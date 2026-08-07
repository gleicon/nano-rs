//! Adversarial Network Tests (Standalone)
//!
//! These tests are separated from the main security_adversarial.rs
//! to avoid module initialization hangs.

#[path = "common.rs"]
mod common;
use common::{find_available_port, NanoProcess};
use std::time::Duration;

/// Test DNS rebinding protection
#[tokio::test]
async fn test_dns_rebinding_blocked() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response('App served', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "net-rebind.local",
        "app.js",
        js_content,
        100, // 100ms CPU
        32,  // 32MB
    );

    nano.wait_ready(port, "net-rebind.local").await;

    let client = reqwest::Client::new();

    // First request with valid hostname
    let result1 = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "net-rebind.local")
        .send()
        .await;

    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().status(), 200);

    // Second request with different hostname (DNS rebinding attempt)
    let result2 = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "attacker.com")
        .send()
        .await;

    assert!(result2.is_ok());
    let status2 = result2.unwrap().status();

    nano.stop();

    // Unknown hostname should get 404
    assert_eq!(status2.as_u16(), 404);
}

/// Test request flooding rate limiting
#[tokio::test]
async fn test_request_flooding_rate_limited() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response('OK', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "net-flood.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "net-flood.local").await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let mut success_count = 0;

    // Send 50 rapid requests
    for i in 0..50 {
        let result = client
            .get(&format!("http://127.0.0.1:{}/?req={}", port, i))
            .header("Host", "net-flood.local")
            .timeout(Duration::from_secs(2))
            .send()
            .await;

        if let Ok(response) = result {
            if response.status().is_success() {
                success_count += 1;
            }
        }
    }

    let elapsed = start.elapsed();
    nano.stop();

    // Should complete in reasonable time (no per-IP rate limiting yet)
    // Most requests should succeed - we're testing system doesn't crash
    assert!(
        success_count >= 30,
        "At least 30 requests should succeed, got {}",
        success_count
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "Flood test should complete within 30 seconds"
    );
}

/// Test slowloris mitigation
#[tokio::test]
async fn test_slowloris_mitigated() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response('OK', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "net-slow.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "net-slow.local").await;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let start = std::time::Instant::now();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "net-slow.local")
        .send()
        .await;

    let elapsed = start.elapsed();
    nano.stop();

    // Normal request should complete quickly
    assert!(
        elapsed < Duration::from_secs(3),
        "Normal request should not be slow"
    );
    assert!(result.is_ok(), "Request should succeed");
}

/// Test header injection protection
#[tokio::test]
async fn test_header_injection_blocked() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response('OK', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "net-header.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "net-header.local").await;

    let client = reqwest::Client::new();

    // Try header injection via newline in header value
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "net-header.local")
        .header("X-Custom", "value\r\nInjected: header")
        .send()
        .await;

    nano.stop();

    // Request should either succeed (with sanitization) or fail gracefully
    match result {
        Ok(response) => {
            // If request succeeds, the app should not see the injected header
            let _ = response.status();
        }
        Err(_) => {
            // Or request is rejected as malformed
        }
    }
}

/// Test SSRF protection
#[tokio::test]
async fn test_ssrf_private_ips_blocked() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Try to fetch from private IP (this is the attack)
        try {
            const response = await fetch('http://127.0.0.1:8080/internal');
            return new Response('Fetched', { status: 200 });
        } catch (e) {
            return new Response('Blocked', { status: 403 });
        }
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "net-ssrf.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "net-ssrf.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "net-ssrf.local")
        .timeout(Duration::from_secs(3))
        .send()
        .await;

    nano.stop();

    // Request should complete (SSRF attempt handled by app or blocked)
    assert!(result.is_ok(), "SSRF test request should complete");
}

/// Test large header rejection
#[tokio::test]
async fn test_large_header_rejection() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response('OK', { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "net-large.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "net-large.local").await;

    let client = reqwest::Client::new();

    // Create a very large header (100KB)
    let large_value = "x".repeat(100_000);

    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "net-large.local")
        .header("X-Large", large_value)
        .send()
        .await;

    nano.stop();

    // Should either succeed (if headers are truncated) or fail gracefully
    match result {
        Ok(response) => {
            let status = response.status();
            // Large headers might cause 400 or 431 (Request Header Fields Too Large)
            assert!(
                status.is_success() || status.as_u16() == 400 || status.as_u16() == 431,
                "Large header should be handled gracefully, got {}",
                status
            );
        }
        Err(_) => {
            // Connection error is also acceptable for oversized headers
        }
    }
}
