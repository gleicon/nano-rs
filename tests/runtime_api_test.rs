//! Runtime API integration tests
//!
//! Tests the handler execution functionality for Phase 3 Plan 01.
//!
//! # V8 v147 Compatibility Note
//! All V8 operations (platform init, isolate creation, execution) must happen
//! on the same thread to avoid "Cannot create a handle without a HandleScope" errors.
//! We use std::sync::Once for thread-safe initialization within spawn_blocking.

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::runtime::{execute_handler, HandlerContext};
use nano::v8::{initialize_platform, NanoIsolate};
use std::sync::Once;

/// Thread-safe V8 platform initialization
/// Must be called inside the spawn_blocking thread, not the async test thread
fn init_platform() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        initialize_platform().expect("Failed to initialize V8 platform");
    });
}

/// Test handler context creation
#[test]
fn test_handler_context_creation() {
    let url = NanoUrl::parse("https://example.com/api").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let context = HandlerContext {
        entrypoint: "/app/index.js".to_string(),
        request,
        memory_limit_mb: 0,
        hostname: String::new(),
    };

    assert_eq!(context.entrypoint, "/app/index.js");
    assert_eq!(context.request.method(), "GET");
}

/// Test handler execution with a simple JavaScript file
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_execute_handler_no_fetch() {
    // Create a simple JS file that doesn't define fetch
    let js_code = r#"console.log("Hello from handler");"#;
    let temp_dir = std::env::temp_dir();
    let js_path = temp_dir.join("test_handler_no_fetch.js");
    std::fs::write(&js_path, js_code).expect("Failed to write test JS file");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init and all V8 operations must be in the same thread
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

        let context = HandlerContext {
            entrypoint: js_path_str,
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        execute_handler(&mut isolate, context)
    })
    .await
    .unwrap();

    // A handler with no fetch function is misconfigured and must fail loudly
    // with 500 — not mask a broken deployment behind a 200.
    assert!(response.is_ok());
    let response = response.unwrap();
    assert_eq!(response.status(), 500);
    let body = response
        .body()
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_default();
    assert!(
        body.contains("no fetch function defined"),
        "error body should explain the misconfiguration, got: {body}"
    );
}

/// Test handler execution with a fetch function that returns a response
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_execute_handler_with_fetch() {
    // Create a JS file that defines fetch and returns a response
    let js_code = r#"
        function fetch(request) {
            return {
                status: 200,
                headers: { "Content-Type": "application/json" },
                body: '{"success": true}'
            };
        }
    "#;
    let temp_dir = std::env::temp_dir();
    let js_path = temp_dir.join("test_handler_with_fetch.js");
    std::fs::write(&js_path, js_code).expect("Failed to write test JS file");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let request = NanoRequest::new("POST".to_string(), url, NanoHeaders::new(), None);

        let context = HandlerContext {
            entrypoint: js_path_str,
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        execute_handler(&mut isolate, context)
    })
    .await
    .unwrap();

    assert!(
        response.is_ok(),
        "Handler execution failed: {:?}",
        response.err()
    );
    let response = response.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("Content-Type"),
        Some("application/json".to_string())
    );
}

/// Test handler execution with custom status code
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_execute_handler_custom_status() {
    // Create a JS file that returns a 404 response
    let js_code = r#"
        function fetch(request) {
            return {
                status: 404,
                headers: { "Content-Type": "text/plain" },
                body: "Not Found"
            };
        }
    "#;
    let temp_dir = std::env::temp_dir();
    let js_path = temp_dir.join("test_handler_404.js");
    std::fs::write(&js_path, js_code).expect("Failed to write test JS file");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("https://example.com/not-found").unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

        let context = HandlerContext {
            entrypoint: js_path_str,
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        execute_handler(&mut isolate, context)
    })
    .await
    .unwrap();

    assert!(response.is_ok());
    let response = response.unwrap();
    assert_eq!(response.status(), 404);
}

/// Test handler execution with request method access
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_execute_handler_request_access() {
    // Create a JS file that accesses request properties
    let js_code = r#"
        function fetch(request) {
            return {
                status: 200,
                headers: { "X-Request-Method": request.method },
                body: "Method: " + request.method
            };
        }
    "#;
    let temp_dir = std::env::temp_dir();
    let js_path = temp_dir.join("test_handler_request.js");
    std::fs::write(&js_path, js_code).expect("Failed to write test JS file");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let request = NanoRequest::new("DELETE".to_string(), url, NanoHeaders::new(), None);

        let context = HandlerContext {
            entrypoint: js_path_str,
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        execute_handler(&mut isolate, context)
    })
    .await
    .unwrap();

    assert!(response.is_ok());
    let response = response.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("X-Request-Method"),
        Some("DELETE".to_string())
    );
}
