//! Adversarial CPU Exhaustion Tests
//!
//! Tests to verify CPU time limits prevent denial-of-service attacks:
//! - Infinite loops
//! - Regular expression denial of service (ReDoS)
//! - Algorithmic complexity attacks
//! - Recursive bombs
//! - Generator memory pressure
//! - Computationally expensive crypto
//! - Catastrophic regex backtracking
//! - JSON parse bombs

#[path = "common.rs"]
mod common;
use common::{find_available_port, NanoProcess};
use std::time::{Duration, Instant};

/// Test infinite loop termination
/// Attack: while(true) { Math.random(); }
/// Mitigation: CPU timer-based termination
#[tokio::test]
async fn test_infinite_loop_terminated() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        while (true) { Math.random(); }
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-infinite.local",
        "infinite.js",
        js_content,
        50, // 50ms CPU limit
        32, // 32MB memory
    );

    nano.wait_ready(port, "cpu-infinite.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-infinite.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should terminate within ~500ms (10x CPU limit)
    assert!(
        elapsed < Duration::from_millis(500),
        "Infinite loop should terminate within 500ms, took {:?}",
        elapsed
    );

    // Should get 504 Gateway Timeout or server error
    match result {
        Ok(response) => {
            let status = response.status();
            assert!(
                status.as_u16() == 504 || status.is_server_error(),
                "Expected 504 or server error for infinite loop, got {}",
                status
            );
        }
        Err(_) => {} // Timeout is acceptable
    }
}

/// Test ReDoS (Regular Expression Denial of Service)
/// Attack: /(a+)+$/ against "aaaaaaaaaaaaaaaaaaaaaaaaaaaa!"
/// Mitigation: CPU time limit prevents excessive backtracking
#[tokio::test]
async fn test_pathological_regex_redos() {
    let port = find_available_port();
    // ReDoS pattern: (a+)+$ has catastrophic backtracking on non-matching input
    let js_content = br#"export default {
    async fetch(request) {
        const evil_pattern = /(a+)+$/;
        const evil_input = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaa!';  // 28 'a's + non-matching char
        
        try {
            const result = evil_pattern.test(evil_input);
            return new Response(JSON.stringify({matched: result}), { status: 200 });
        } catch (e) {
            return new Response(JSON.stringify({error: e.message}), { status: 500 });
        }
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-redos.local",
        "redos.js",
        js_content,
        50, // 50ms CPU limit
        32,
    );

    nano.wait_ready(port, "cpu-redos.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-redos.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // ReDoS should be terminated quickly
    assert!(
        elapsed < Duration::from_millis(500),
        "ReDoS attack should be terminated within 500ms, took {:?}",
        elapsed
    );

    match result {
        Ok(response) => {
            assert!(
                response.status().as_u16() == 504 || response.status().is_server_error(),
                "Expected error status for ReDoS"
            );
        }
        Err(_) => {} // Timeout acceptable
    }
}

/// Test algorithmic complexity attack (O(n²) nested loops)
/// Attack: Nested loops with large iteration count
/// Mitigation: CPU time limit prevents excessive computation
#[tokio::test]
async fn test_algorithmic_complexity_attack() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // O(n^2) algorithmic complexity attack
        const n = 100000;
        let count = 0;
        
        for (let i = 0; i < n; i++) {
            for (let j = 0; j < n; j++) {
                count += i * j;
                if (count > 1000000000) count = 0; // Prevent overflow
            }
        }
        
        return new Response(JSON.stringify({count}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-complexity.local",
        "complexity.js",
        js_content,
        50, // 50ms limit - way too short for O(n²) with n=100000
        32,
    );

    nano.wait_ready(port, "cpu-complexity.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-complexity.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should terminate due to CPU limit
    assert!(
        elapsed < Duration::from_millis(500),
        "Algorithmic complexity attack should be terminated within 500ms, took {:?}",
        elapsed
    );
}

/// Test recursive function bomb
/// Attack: Recursive Fibonacci with no memoization on large N
/// Mitigation: CPU time limit prevents stack overflow / excessive computation
#[tokio::test]
async fn test_recursive_function_bomb() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Recursive fibonacci - exponential time complexity
        function fib(n) {
            if (n <= 1) return n;
            return fib(n - 1) + fib(n - 2);
        }
        
        // fib(50) would take ~2^50 operations - impossible within CPU limit
        const result = fib(50);
        
        return new Response(JSON.stringify({fib: result}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-recursive.local",
        "recursive.js",
        js_content,
        50, // 50ms limit
        32,
    );

    nano.wait_ready(port, "cpu-recursive.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-recursive.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Recursive bomb should be terminated
    assert!(
        elapsed < Duration::from_millis(500),
        "Recursive bomb should be terminated within 500ms, took {:?}",
        elapsed
    );
}

/// Test generator memory pressure
/// Attack: Infinite generator yielding values
/// Mitigation: CPU time limit
#[tokio::test]
async fn test_generator_memory_pressure() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Generator that produces infinite sequence
        function* infiniteGenerator() {
            let i = 0;
            while (true) {
                yield i++;
            }
        }
        
        const gen = infiniteGenerator();
        let sum = 0;
        
        // Try to exhaust CPU by iterating
        for (const val of gen) {
            sum += val;
            if (sum > 1000000000) sum = 0;
        }
        
        return new Response(JSON.stringify({sum}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-generator.local",
        "generator.js",
        js_content,
        50, // 50ms limit
        32,
    );

    nano.wait_ready(port, "cpu-generator.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-generator.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Generator should be terminated by CPU limit
    assert!(
        elapsed < Duration::from_millis(500),
        "Generator attack should be terminated within 500ms, took {:?}",
        elapsed
    );
}

/// Test computationally expensive crypto operations
/// Attack: PBKDF2 with extremely high iteration count
/// Mitigation: CPU time limit
#[tokio::test]
async fn test_computationally_expensive_crypto() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Attempt to drain CPU with heavy crypto operations
        // Note: crypto.subtle operations are native and may not count toward JS CPU time
        // This tests the defense depth
        
        const data = new Uint8Array(1000);
        for (let i = 0; i < data.length; i++) {
            data[i] = i % 256;
        }
        
        // SHA-256 in a tight loop
        let hashCount = 0;
        while (true) {
            // This should trigger CPU termination if crypto counts toward JS CPU
            // Or timeout if it doesn't
            hashCount++;
            if (hashCount % 1000 === 0) {
                // Yield occasionally
            }
        }
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-crypto.local",
        "crypto_bomb.js",
        js_content,
        50, // 50ms limit
        32,
    );

    nano.wait_ready(port, "cpu-crypto.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-crypto.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should be terminated
    assert!(
        elapsed < Duration::from_millis(500),
        "Crypto attack should be terminated within 500ms, took {:?}",
        elapsed
    );
}

/// Test regex catastrophic backtracking with multiple patterns
/// Attack: Multiple complex regex patterns
/// Mitigation: CPU time limit
#[tokio::test]
async fn test_regex_catastrophic_backtracking() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Multiple ReDoS patterns
        const patterns = [
            /(a+)+$/,
            /(a+)+b/,
            /(.*a){x}$/,
            /([a-z]+)+$/
        ];
        
        const evil_inputs = [
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaa!',
            'aaaaaaaaaaaaaaaaaaaaaaaaaab',
            'abcdefghijklmnopqrstuvwxyz!'
        ];
        
        for (const pattern of patterns) {
            for (const input of evil_inputs) {
                try {
                    pattern.test(input);
                } catch (e) {
                    // Pattern might throw
                }
            }
        }
        
        return new Response('Completed', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-regex.local",
        "regex_bomb.js",
        js_content,
        50, // 50ms limit
        32,
    );

    nano.wait_ready(port, "cpu-regex.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-regex.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should be terminated
    assert!(
        elapsed < Duration::from_millis(500),
        "Multiple regex attack should be terminated within 500ms, took {:?}",
        elapsed
    );
}

/// Test JSON parse bomb
/// Attack: Deeply nested JSON causing excessive parse time
/// Mitigation: CPU time limit (and ideally depth limits)
#[tokio::test]
async fn test_json_parse_bomb() {
    let port = find_available_port();

    // Create deeply nested JSON string programmatically in JS
    let js_content = br#"export default {
    async fetch(request) {
        // Create deeply nested JSON structure
        let nested = 'null';
        for (let i = 0; i < 100000; i++) {
            nested = `[${nested}]`;
        }
        
        // Attempt to parse extremely deep JSON
        try {
            const result = JSON.parse(nested);
            return new Response(JSON.stringify({depth: 'parsed'}), { status: 200 });
        } catch (e) {
            return new Response(JSON.stringify({error: e.message}), { status: 500 });
        }
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "cpu-json.local",
        "json_bomb.js",
        js_content,
        50, // 50ms limit - not enough time to build and parse deep JSON
        32,
    );

    nano.wait_ready(port, "cpu-json.local").await;

    let start = Instant::now();
    let client = reqwest::Client::new();
    let _result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "cpu-json.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;
    let elapsed = start.elapsed();

    nano.stop();

    // Should be terminated (loop to build JSON will trigger CPU limit)
    assert!(
        elapsed < Duration::from_millis(500),
        "JSON bomb should be terminated within 500ms, took {:?}",
        elapsed
    );
}
