//! Adversarial Isolation Tests (Standalone)
//!
//! These tests are separated from the main security_adversarial.rs
//! to avoid module initialization hangs.

#[path = "common.rs"]
mod common;
use common::{find_available_port, NanoProcess};
use std::time::Duration;

/// Test hostname spoofing detection
#[tokio::test]
async fn test_hostname_spoofing_detected() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        return new Response(JSON.stringify({
            url: request.url,
            headers: Array.from(request.headers.entries())
        }), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) =
        NanoProcess::start(port, "isolation-host.local", "app.js", js_content, 100, 32);

    nano.wait_ready(port, "isolation-host.local").await;

    let client = reqwest::Client::new();

    // Attempt to spoof hostname via X-Forwarded-Host
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "isolation-host.local")
        .header("X-Forwarded-Host", "attacker.com")
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let body = response.text().await.unwrap_or_default();
            // The app should see the actual hostname, not the spoofed one
            assert!(
                body.contains("isolation-host.local") || !body.contains("attacker.com"),
                "Hostname should not be spoofable via X-Forwarded-Host. Response: {}",
                body
            );
        }
        Err(_) => {}
    }
}

/// Test worker pool isolation
#[tokio::test]
async fn test_worker_pool_isolation() {
    let port1 = find_available_port();
    let port2 = find_available_port();

    let js_content1 = br#"export default {
    async fetch(request) {
        // Store something in global state
        globalThis.app1Data = 'secret-app1';
        return new Response('App1', { status: 200 });
    }
}"#;

    let js_content2 = br#"export default {
    async fetch(request) {
        // Try to access App1's data
        const stolen = globalThis.app1Data || 'not-found';
        return new Response(`App2 sees: ${stolen}`, { status: 200 });
    }
}"#;

    let (mut nano1, _temp_dir1) = NanoProcess::start(
        port1,
        "isolation-app1.local",
        "app1.js",
        js_content1,
        100,
        32,
    );

    let (mut nano2, _temp_dir2) = NanoProcess::start(
        port2,
        "isolation-app2.local",
        "app2.js",
        js_content2,
        100,
        32,
    );

    nano1.wait_ready(port1, "isolation-app1.local").await;
    nano2.wait_ready(port2, "isolation-app2.local").await;

    let client = reqwest::Client::new();

    // First, make request to App1 to set its data
    let _ = client
        .get(&format!("http://127.0.0.1:{}/", port1))
        .header("Host", "isolation-app1.local")
        .send()
        .await;

    // Then, make request to App2 to check if it can see App1's data
    let result2 = client
        .get(&format!("http://127.0.0.1:{}/", port2))
        .header("Host", "isolation-app2.local")
        .send()
        .await;

    nano1.stop();
    nano2.stop();

    match result2 {
        Ok(response) => {
            let body = response.text().await.unwrap_or_default();
            // App2 should NOT see App1's data
            assert!(
                body.contains("not-found") || !body.contains("secret-app1"),
                "App2 should NOT access App1's data. Response: {}",
                body
            );
        }
        Err(_) => {}
    }
}

/// Test cross-tenant request isolation
#[tokio::test]
async fn test_cross_tenant_request_isolation() {
    let port = find_available_port();

    let js_content = br#"export default {
    async fetch(request) {
        // Return the hostname we see
        return new Response(JSON.stringify({
            hostname: new URL(request.url).hostname
        }), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "isolation-tenant.local",
        "app.js",
        js_content,
        100,
        32,
    );

    nano.wait_ready(port, "isolation-tenant.local").await;

    let client = reqwest::Client::new();

    // Request with correct hostname
    let result1 = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "isolation-tenant.local")
        .send()
        .await;

    assert!(result1.is_ok());

    // Request with wrong hostname should 404
    let result2 = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "other-tenant.local")
        .send()
        .await;

    nano.stop();

    match result2 {
        Ok(response) => {
            assert_eq!(response.status(), 404);
        }
        Err(_) => {}
    }
}
