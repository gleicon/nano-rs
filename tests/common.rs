//! Security Test Module
//!
//! Shared utilities and test harness for adversarial security testing.
//! Provides common setup functions, test context management, and assertion helpers.

use std::sync::Arc;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::fs;
use std::path::PathBuf;
use std::net::TcpListener;

use nano::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};
use nano::runtime::fs_polyfill::set_current_vfs;
use nano::v8::platform;
use nano::v8::NanoIsolate;

#[allow(dead_code)]
pub fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Find an available port for testing
pub fn find_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("Failed to bind to find available port")
        .local_addr()
        .expect("Failed to get local address")
        .port()
}

#[allow(dead_code)]
pub fn create_test_vfs(hostname: &str) -> Arc<IsolateVfs> {
    Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname(hostname),
        nano::vfs::VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ))
}

#[allow(dead_code)]
pub struct SecurityTestContext {
    pub vfs: Arc<IsolateVfs>,
    pub hostname: String,
}

impl SecurityTestContext {
    /// Create a new test context
    pub fn new(hostname: &str) -> Self {
        init_platform();
        let vfs = create_test_vfs(hostname);
        set_current_vfs(Some(vfs.clone()));
        
        Self {
            vfs,
            hostname: hostname.to_string(),
        }
    }
    
    /// Pre-populate a file in the VFS
    pub async fn create_file(&self, path: &str, content: &[u8]) {
        self.vfs.write(path, content).await.expect("Failed to create test file");
    }
}

/// Assert that an operation was blocked with expected error
pub fn assert_blocked(result: Result<impl std::fmt::Debug, impl std::fmt::Debug>, expected_error: &str) {
    match result {
        Ok(_) => panic!("Expected operation to be blocked, but it succeeded"),
        Err(e) => {
            let error_str = format!("{:?}", e);
            assert!(
                error_str.contains(expected_error) || 
                error_str.to_lowercase().contains(&expected_error.to_lowercase()),
                "Expected error containing '{}', got: {}",
                expected_error,
                error_str
            );
        }
    }
}

/// Assert that a VFS error has the expected code
pub fn assert_vfs_error(result: Result<impl std::fmt::Debug, nano::vfs::VfsError>, expected_code: &str) {
    match result {
        Ok(_) => panic!("Expected VFS error, but operation succeeded"),
        Err(e) => {
            let error_str = format!("{:?}", e);
            assert!(
                error_str.contains(expected_code),
                "Expected VFS error code '{}', got: {}",
                expected_code,
                error_str
            );
        }
    }
}

/// Helper for spawning NANO process in end-to-end tests
pub struct NanoProcess {
    pub child: std::process::Child,
    pub temp_dir: PathBuf,
}

impl NanoProcess {
    /// Start NANO with given configuration
    pub fn start(
        port: u16,
        hostname: &str,
        entrypoint: &str,
        js_content: &[u8],
        cpu_time_ms: u64,
        memory_mb: usize,
    ) -> (Self, PathBuf) {
        let temp_dir = create_test_dir(&hostname.replace('.', "_"));
        
        // Write JS entrypoint
        fs::write(temp_dir.join(entrypoint), js_content)
            .expect(&format!("Failed to write {}", entrypoint));
        
        // Create config with limits
        let entrypoint_abs = temp_dir.join(entrypoint).to_str().unwrap().to_string();
        let _base_path = temp_dir.to_str().unwrap();
        
        let config = format!(r#"{{
  "apps": [{{
    "hostname": "{}",
    "entrypoint": "{}",
    "limits": {{
      "memory_mb": {},
      "timeout_secs": 30,
      "workers": 1,
      "cpu_time_ms": {},
      "cpu_time_enabled": true
    }},
    "vfs_backend": "memory"
  }}],
  "server": {{
    "port": {},
    "host": "127.0.0.1"
  }}
}}"#, hostname, entrypoint_abs, memory_mb, cpu_time_ms, port);
        
        fs::write(temp_dir.join("config.json"), config.as_bytes())
            .expect("Failed to write config.json");
        
        // Find binary
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
        
        (Self { child, temp_dir: temp_dir.clone() }, temp_dir)
    }
    
    /// Wait for server to be ready
    pub async fn wait_ready(&mut self, port: u16, hostname: &str) {
        // Initial V8 startup delay
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        let client = reqwest::Client::new();
        let start = Instant::now();
        let max_wait = Duration::from_secs(30);
        
        while start.elapsed() < max_wait {
            match client
                .get(format!("http://127.0.0.1:{}/", port))
                .header("Host", hostname)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(_) => return,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        
        // Capture stderr before panicking
        let mut stderr = String::new();
        if let Some(ref mut err) = self.child.stderr {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            stderr = String::from_utf8_lossy(&buf).to_string();
        }
        eprintln!("=== NANO STDERR ===\n{}\n===================", stderr);
        self.stop();
        panic!("Server failed to start on port {} within 15 seconds", port);
    }
    
    /// Stop the NANO process
    pub fn stop(&mut self) {
        self.child.kill().ok();
        let _ = self.child.wait();
        cleanup_test_dir(&self.temp_dir);
    }
}

impl Drop for NanoProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Find NANO binary path
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

/// Create temporary test directory
#[allow(dead_code)]
fn create_test_dir(name: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir()
        .join(format!("nano_security_{}_{}_{}", name, std::process::id(), timestamp));
    fs::remove_dir_all(&temp_dir).ok();
    fs::create_dir_all(&temp_dir).expect("Failed to create test dir");
    temp_dir
}

/// Cleanup test directory
#[allow(dead_code)]
fn cleanup_test_dir(path: &PathBuf) {
    fs::remove_dir_all(path).ok();
}

/// Wait for server to be ready (synchronous version)
pub fn wait_for_server_sync(port: u16, _hostname: &str, max_wait_secs: u64) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(500));
    
    let start = Instant::now();
    let max_wait = Duration::from_secs(max_wait_secs);
    
    while start.elapsed() < max_wait {
        match std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
            Ok(_) => return Ok(()),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    
    Err(format!("Server failed to start on port {} within {} seconds", port, max_wait_secs))
}

/// Create a V8 isolate for testing
/// 
/// This creates a NanoIsolate and returns the raw v8::Isolate handle
/// for use in test scopes. The isolate is owned by NanoIsolate.
pub fn create_test_isolate() -> NanoIsolate {
    NanoIsolate::new().expect("Failed to create test isolate")
}
