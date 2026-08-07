//! Framework Compatibility Tests
//!
//! Tests that Hono.js-style and generic WinterTC apps execute correctly
//! in NANO's V8 runtime with all WinterTC APIs available.
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
async fn test_generic_wintertc_app_root_route() {
    let js_path = create_temp_js_file("generic-wintertc-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init and all V8 operations must be in the same thread
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        // Create a request to the root route
        let url = NanoUrl::parse("http://test.example.com/").unwrap();
        let mut headers = NanoHeaders::new();
        headers.set("User-Agent", "Test/1.0");
        headers.set("Accept", "application/json");

        let request = NanoRequest::new("GET".to_string(), url, headers, None);

        let context = HandlerContext {
            entrypoint: js_path_str,
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        // Execute the handler
        execute_handler(&mut isolate, context)
    })
    .await
    .unwrap();

    // Verify response
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
    assert_eq!(
        response.headers().get("X-Generic-App"),
        Some("true".to_string())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_generic_wintertc_app_health_route() {
    let js_path = create_temp_js_file("generic-wintertc-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://test.example.com/health").unwrap();
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
    assert_eq!(response.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_generic_wintertc_app_api_data_route() {
    let js_path = create_temp_js_file("generic-wintertc-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://test.example.com/api/data").unwrap();
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
    assert_eq!(response.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_generic_wintertc_app_404() {
    let js_path = create_temp_js_file("generic-wintertc-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://test.example.com/nonexistent").unwrap();
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
    assert_eq!(response.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_style_app_root_route() {
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://hono.example.com/").unwrap();
        let mut headers = NanoHeaders::new();
        headers.set("Accept", "application/json");

        let request = NanoRequest::new("GET".to_string(), url, headers, None);

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
    assert_eq!(
        response.headers().get("X-Powered-By"),
        Some("NANO/Hono-Sim".to_string())
    );
    // Verify CORS headers from middleware
    assert_eq!(
        response.headers().get("Access-Control-Allow-Origin"),
        Some("*".to_string())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_style_app_about_route() {
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://hono.example.com/about").unwrap();
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
    assert_eq!(response.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hono_style_app_404() {
    let js_path = create_temp_js_file("hono-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://hono.example.com/unknown").unwrap();
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
    assert_eq!(response.status(), 404);
}
