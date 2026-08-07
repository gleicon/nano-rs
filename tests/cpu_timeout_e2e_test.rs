//! End-to-end tests for CPU time limits
//!
//! These tests spawn the NANO binary and verify actual timeout behavior.
//! Run with: cargo test --test cpu_timeout_e2e_test -- --test-threads=1
//!
//! NOTE: All tests use the disk VFS backend for per-app VFS configuration.
//! The WASM tests that read files via Nano.fs.readFile() require the disk
//! VFS backend to be properly configured via the AppRegistry.
//!
//! All tests:
//! - test_js_cpu_timeout: JavaScript infinite loop termination
//! - test_js_within_cpu_limit: Normal JS execution within limits  
//! - test_cpu_limit_per_isolate: Per-isolate CPU limits
//! - test_wasm_cpu_timeout: WASM with file read (disk VFS)
//! - test_wasm_within_cpu_limit: WASM with file read (disk VFS)

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn find_available_port() -> u16 {
    // Find an available port by binding to port 0 (OS assigns available port)
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to find available port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    // Small delay to ensure port is released
    std::thread::sleep(Duration::from_millis(100));
    port
}

fn nano_binary_path() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let project_root = PathBuf::from(manifest_dir);

    let release_path = project_root.join("target/release/nano-rs");
    if release_path.exists() {
        return release_path;
    }

    let debug_path = project_root.join("target/debug/nano-rs");
    if debug_path.exists() {
        return debug_path;
    }

    panic!("NANO binary not found. Build with: cargo build");
}

fn create_test_dir(name: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "nano_e2e_{}_{}_{}",
        name,
        std::process::id(),
        timestamp
    ));
    fs::remove_dir_all(&temp_dir).ok();
    fs::create_dir_all(&temp_dir).expect("Failed to create test dir");
    temp_dir
}

fn cleanup_test_dir(path: &PathBuf) {
    fs::remove_dir_all(path).ok();
}

fn write_test_file(dir: &PathBuf, filename: &str, content: &[u8]) {
    fs::write(dir.join(filename), content).expect(&format!("Failed to write {}", filename));
}

async fn wait_for_server(port: u16, hostname: &str, max_wait_secs: u64) -> Result<(), String> {
    // Wait for V8 initialization
    tokio::time::sleep(Duration::from_millis(500)).await;

    let client = reqwest::Client::new();
    let start = Instant::now();
    let max_wait = Duration::from_secs(max_wait_secs);

    while start.elapsed() < max_wait {
        match client
            .get(format!("http://127.0.0.1:{}/", port))
            .header("Host", hostname)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(_) => return Ok(()),
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    Err(format!(
        "Server failed to start on port {} within {} seconds",
        port, max_wait_secs
    ))
}

struct NanoProcess {
    child: std::process::Child,
    temp_dir: PathBuf,
}

impl NanoProcess {
    fn start(
        port: u16,
        hostname: &str,
        entrypoint: &str,
        js_content: &[u8],
        wasm_file: Option<(&str, &[u8])>,
    ) -> (Self, PathBuf) {
        let temp_dir = create_test_dir(&hostname.replace('.', "_"));

        // Entrypoint is read directly from filesystem (not through VFS)
        // Write JS file at temp_dir root
        fs::write(temp_dir.join(entrypoint), js_content)
            .expect(&format!("Failed to write {}", entrypoint));

        // DiskBackend (with per-app base_path) resolves paths directly under
        // base_path — no namespace subdirectory. Files accessed via Nano.fs
        // must be placed at base_path/<filename>, not base_path/<hostname>/<filename>.
        if let Some((wasm_name, wasm_bytes)) = wasm_file {
            fs::write(temp_dir.join(wasm_name), wasm_bytes)
                .expect(&format!("Failed to write {}", wasm_name));
        }

        // Create config with absolute paths
        let base_path = temp_dir.to_str().unwrap();
        // Escape backslashes for Windows compatibility in JSON
        let base_path_escaped = base_path.replace('\\', "\\\\");
        // Use absolute path for entrypoint to ensure workers can find it
        let entrypoint_abs = temp_dir
            .join(entrypoint)
            .to_str()
            .unwrap()
            .replace('\\', "\\\\");

        let config = format!(
            r#"{{
  "apps": [{{
    "hostname": "{}",
    "entrypoint": "{}",
    "limits": {{
      "memory_mb": 128,
      "timeout_secs": 30,
      "workers": 1,
      "cpu_time_ms": 100,
      "cpu_time_enabled": true
    }},
    "vfs_backend": "disk",
    "vfs_disk": {{
      "base_path": "{}"
    }}
  }}],
  "server": {{
    "port": {},
    "host": "127.0.0.1"
  }}
}}"#,
            hostname, entrypoint_abs, base_path_escaped, port
        );

        fs::write(temp_dir.join("config.json"), config.as_bytes())
            .expect("Failed to write config.json");

        // Debug: print config
        eprintln!("Test dir: {:?}", temp_dir);
        eprintln!("Config: {}", config);

        let nano_path = nano_binary_path();
        let child = Command::new(&nano_path)
            .arg("run")
            .arg("--config")
            .arg(temp_dir.join("config.json"))
            .current_dir(&temp_dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to spawn NANO");

        (
            Self {
                child,
                temp_dir: temp_dir.clone(),
            },
            temp_dir,
        )
    }

    async fn wait_ready(&mut self, port: u16, hostname: &str) {
        if let Err(e) = wait_for_server(port, hostname, 15).await {
            // Kill the process first so read_to_end doesn't block forever.
            // A running child keeps its stderr pipe open; read_to_end only
            // returns once EOF is received, which happens when the process exits.
            self.child.kill().ok();
            let _ = self.child.wait();
            let mut stderr = String::new();
            if let Some(ref mut err) = self.child.stderr {
                use std::io::Read;
                let mut buf = Vec::new();
                let _ = err.read_to_end(&mut buf);
                stderr = String::from_utf8_lossy(&buf).to_string();
            }
            eprintln!("=== NANO STDERR ===\n{}\n===================", stderr);
            panic!("{}", e);
        }
    }

    fn stop(&mut self) {
        self.child.kill().ok();
        let _ = self.child.wait();
        // Print stderr so test failures are diagnosable
        if let Some(ref mut err) = self.child.stderr {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            if !buf.is_empty() {
                eprintln!(
                    "=== NANO STDERR ===\n{}\n===================",
                    String::from_utf8_lossy(&buf)
                );
            }
        }
        cleanup_test_dir(&self.temp_dir);
    }
}

impl Drop for NanoProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

#[tokio::test]
async fn test_js_cpu_timeout() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        while (true) { Math.random(); }
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "timeout.local", "infinite.js", js_content, None);

    nano.wait_ready(port, "timeout.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "timeout.local")
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should timeout quickly (within ~500ms real time for 100ms CPU limit)
    assert!(
        elapsed < Duration::from_millis(500),
        "CPU timeout took too long: {:?}. Expected <500ms",
        elapsed
    );

    match result {
        Ok(response) => {
            assert!(
                response.status().is_server_error() || response.status().as_u16() == 504,
                "Expected error for CPU timeout, got {}",
                response.status()
            );
        }
        Err(_) => {} // Timeout is expected
    }

    println!("JS CPU timeout test passed: elapsed={:?}", elapsed);
}

#[tokio::test]
async fn test_js_within_cpu_limit() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        let sum = 0;
        for (let i = 0; i < 1000; i++) { sum += i; }
        return new Response(JSON.stringify({sum}), {
            status: 200,
            headers: {'Content-Type': 'application/json'}
        });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "normal.local", "normal.js", js_content, None);

    nano.wait_ready(port, "normal.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "normal.local")
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            assert!(
                response.status().is_success(),
                "Expected success, got {}",
                response.status()
            );
            let body = response.text().await.unwrap_or_default();
            assert!(
                body.contains("499500"),
                "Expected sum 499500, got: {}",
                body
            );
        }
        Err(e) => panic!("Request failed: {}", e),
    }

    println!("JS normal execution test passed");
}

#[tokio::test]
async fn test_wasm_cpu_timeout() {
    let port = find_available_port();
    let wasm_bytes = include_bytes!("../examples/wasm-test/add.wasm");
    let js_content = br#"export default {
    async fetch(request) {
        const wasmBytes = await Nano.fs.readFile('add.wasm');
        const module = await WebAssembly.compile(wasmBytes);
        const instance = await WebAssembly.instantiate(module, {});
        while (true) { instance.exports.add(1, 1); }
    }
}"#;

    let (mut nano, temp_dir) = NanoProcess::start(
        port,
        "wasm-timeout.local",
        "wasm_infinite.js",
        js_content,
        Some(("add.wasm", wasm_bytes)),
    );

    // Verify files exist
    // Entrypoint at temp_dir root (read directly by runtime)
    assert!(
        temp_dir.join("wasm_infinite.js").exists(),
        "wasm_infinite.js should exist in temp dir"
    );
    assert!(
        temp_dir.join("add.wasm").exists(),
        "add.wasm should exist in base_path"
    );

    nano.wait_ready(port, "wasm-timeout.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "wasm-timeout.local")
        .timeout(Duration::from_secs(5))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    assert!(
        elapsed < Duration::from_millis(500),
        "WASM CPU timeout took too long: {:?}. Expected <500ms",
        elapsed
    );

    match result {
        Ok(response) => {
            assert!(
                response.status().is_server_error() || response.status().as_u16() == 504,
                "Expected error for WASM CPU timeout, got {}",
                response.status()
            );
        }
        Err(_) => {}
    }

    println!("WASM CPU timeout test passed: elapsed={:?}", elapsed);
}

#[tokio::test]
async fn test_wasm_within_cpu_limit() {
    let port = find_available_port();
    let wasm_bytes = include_bytes!("../examples/wasm-test/add.wasm");
    let js_content = br#"export default {
    async fetch(request) {
        const url = new URL(request.url);
        const a = parseInt(url.searchParams.get('a') || '5');
        const b = parseInt(url.searchParams.get('b') || '3');
        const wasmBytes = await Nano.fs.readFile('add.wasm');
        const module = await WebAssembly.compile(wasmBytes);
        const instance = await WebAssembly.instantiate(module, {});
        const result = instance.exports.add(a, b);
        return new Response(JSON.stringify({a, b, result}), {
            status: 200,
            headers: {'Content-Type': 'application/json'}
        });
    }
}"#;

    let (mut nano, temp_dir) = NanoProcess::start(
        port,
        "wasm-normal.local",
        "wasm_normal.js",
        js_content,
        Some(("add.wasm", wasm_bytes)),
    );

    // Verify files exist
    // Entrypoint at temp_dir root (read directly by runtime)
    assert!(
        temp_dir.join("wasm_normal.js").exists(),
        "wasm_normal.js should exist in temp dir"
    );
    assert!(
        temp_dir.join("add.wasm").exists(),
        "add.wasm should exist in base_path"
    );

    nano.wait_ready(port, "wasm-normal.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/?a=10&b=20", port))
        .header("Host", "wasm-normal.local")
        .timeout(Duration::from_secs(5))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            eprintln!("Response: status={}, body={}", status, body);
            assert!(
                status.is_success(),
                "Expected success, got {} with body: {}",
                status,
                body
            );
            assert!(body.contains("30"), "Expected result 30, got: {}", body);
        }
        Err(e) => panic!("Request failed: {}", e),
    }

    println!("WASM normal execution test passed");
}

#[tokio::test]
async fn test_cpu_limit_per_isolate() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        let iterations = 0;
        const endTime = Date.now() + 50;
        while (Date.now() < endTime) { iterations++; }
        return new Response(JSON.stringify({iterations}), {
            status: 200,
            headers: {'Content-Type': 'application/json'}
        });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "compute.local", "compute.js", js_content, None);

    nano.wait_ready(port, "compute.local").await;

    let client = reqwest::Client::new();

    // Send concurrent requests
    let futures = (0..3).map(|_| {
        client
            .get(&format!("http://127.0.0.1:{}/", port))
            .header("Host", "compute.local")
            .timeout(Duration::from_secs(5))
            .send()
    });

    let results = futures::future::join_all(futures).await;
    nano.stop();

    let mut success_count = 0;
    for result in results {
        match result {
            Ok(response) if response.status().is_success() => {
                success_count += 1;
            }
            Ok(response) => {
                println!("Got error status: {}", response.status());
            }
            Err(e) => {
                println!("Request error: {}", e);
            }
        }
    }

    assert!(
        success_count >= 2,
        "Expected at least 2 successful requests, got {}",
        success_count
    );
    println!(
        "Per-isolate CPU limit test passed: {}/3 succeeded",
        success_count
    );
}
