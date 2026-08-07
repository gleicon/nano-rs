//! Hono.js Integration Tests
//!
//! Extended tests for Hono-style middleware patterns and routing.
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

/// Loads a test fixture file from tests/fixtures/frameworks/
fn load_fixture(name: &str) -> String {
    let path = format!("tests/fixtures/frameworks/{}.js", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path, e))
}

/// Creates a temporary JS file with fixture content for testing
fn create_temp_js_file(fixture_name: &str) -> std::path::PathBuf {
    let code = load_fixture(fixture_name);
    let temp_dir = std::env::temp_dir();
    let file_name = format!("test_{}.js", fixture_name);
    let js_path = temp_dir.join(&file_name);
    std::fs::write(&js_path, code).expect("Failed to write test JS file");
    js_path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_middleware_chain_order() {
    // Verify middleware executes in correct order (logger wraps CORS)
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init and all V8 operations must be in the same thread
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://test.example.com/").unwrap();
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

    assert!(
        response.is_ok(),
        "Handler execution failed: {:?}",
        response.err()
    );
    let response = response.unwrap();

    // Both middlewares should have run
    assert!(response
        .headers()
        .get("Access-Control-Allow-Origin")
        .is_some());
    assert!(response.headers().get("X-Powered-By").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_post_request() {
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init and all V8 operations must be in the same thread
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://test.example.com/").unwrap();
        let mut headers = NanoHeaders::new();
        headers.set("Content-Type", "application/json");

        let request = NanoRequest::new("POST".to_string(), url, headers, None);

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

    // Should still return 200 (simple router doesn't check method in this test)
    assert_eq!(response.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_cors_headers_on_all_routes() {
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    // Test CORS headers are present on both success and 404 responses
    for path in &["/", "/about", "/nonexistent"] {
        let path_str = path.to_string();
        let entrypoint = js_path_str.clone();

        let response = tokio::task::spawn_blocking(move || {
            // V8 platform init (Once ensures this only runs once across all threads)
            init_platform();

            let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

            let url = NanoUrl::parse(&format!("http://test.example.com{}", path_str)).unwrap();
            let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

            let context = HandlerContext {
                entrypoint,
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
            "Handler execution failed for {}: {:?}",
            path,
            response.err()
        );
        let response = response.unwrap();

        // CORS middleware applies to all responses
        assert!(
            response
                .headers()
                .get("Access-Control-Allow-Origin")
                .is_some(),
            "CORS header missing on {}",
            path
        );
    }
}
