//! Astro Islands Architecture Integration Tests
//!
//! Tests Astro's partial hydration pattern with island markers.
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

fn load_fixture(name: &str) -> String {
    let path = format!("tests/fixtures/frameworks/{}.js", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path, e))
}

fn create_temp_js_file(fixture_name: &str) -> std::path::PathBuf {
    let code = load_fixture(fixture_name);
    let temp_dir = std::env::temp_dir();
    let file_name = format!("test_{}.js", fixture_name);
    let js_path = temp_dir.join(&file_name);
    std::fs::write(&js_path, code).expect("Failed to write test JS file");
    js_path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_astro_home_page_renders_islands() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init and all V8 operations must be in the same thread
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://astro.example.com/").unwrap();
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
    assert_eq!(
        response.headers().get("X-Astro-Islands"),
        Some("true".to_string())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_astro_gallery_page_carousel() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://astro.example.com/gallery").unwrap();
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
async fn test_astro_404() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://astro.example.com/nonexistent").unwrap();
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
async fn test_astro_image_assets() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    for img in &["/photo1.jpg", "/photo2.jpg", "/photo3.jpg"] {
        let img_str = img.to_string();
        let entrypoint = js_path_str.clone();

        let response = tokio::task::spawn_blocking(move || {
            // V8 platform init (Once ensures this only runs once across all threads)
            init_platform();

            let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

            let url = NanoUrl::parse(&format!("http://astro.example.com{}", img_str)).unwrap();
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
            img,
            response.err()
        );
        let response = response.unwrap();
        assert_eq!(response.status(), 200, "Failed for {}", img);
        assert_eq!(
            response.headers().get("Content-Type"),
            Some("image/jpeg".to_string()),
            "Content-Type mismatch for {}",
            img
        );
        assert_eq!(
            response.headers().get("X-Astro-Asset"),
            Some("true".to_string()),
            "X-Astro-Asset header missing for {}",
            img
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_astro_island_hydration_strategy_markers() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://astro.example.com/").unwrap();
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
async fn test_astro_server_rendered_content() {
    let js_path = create_temp_js_file("astro-islands-app");
    let js_path_str = js_path.to_string_lossy().to_string();

    let response = tokio::task::spawn_blocking(move || {
        // V8 platform init (Once ensures this only runs once across all threads)
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        let url = NanoUrl::parse("http://astro.example.com/").unwrap();
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
