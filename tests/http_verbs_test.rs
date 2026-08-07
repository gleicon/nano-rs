//! Comprehensive HTTP Verb Tests for Phase 17
//!
//! Tests all HTTP methods with full body, headers, and processing support:
//! - GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS
//! - Methods with bodies (POST, PUT, PATCH)
//! - Methods without bodies (GET, HEAD, DELETE, OPTIONS)
//! - Custom headers per method
//! - Body processing for each method type

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::worker::{HandlerTask, WorkerPool};
use std::fs;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::oneshot;

// Helper to decode base64 in tests
fn base64_decode(input: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .unwrap()
}

fn init_platform() {
    if !nano::v8::is_initialized() {
        nano::v8::initialize_platform().expect("Failed to initialize V8 platform");
    }
}

fn create_test_handler(dir: &TempDir, filename: &str, code: &str) -> String {
    let path = dir.path().join(filename);
    let mut file = fs::File::create(&path).expect("Failed to create test file");
    file.write_all(code.as_bytes())
        .expect("Failed to write test code");
    path.to_string_lossy().to_string()
}

/// Handler that echoes back all request details for verification
fn create_echo_handler() -> &'static str {
    r#"
function fetch(request) {
    // Body is base64 encoded string (or null)
    // We return it as-is to verify the body was passed correctly
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
            bodyUsed: request.bodyUsed,
            hasBody: request.body !== null
        })
    };
}
"#
}

#[test]
fn test_http_get_without_body() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "get_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/resource?filter=active").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Accept", "application/json");
    headers.set("Authorization", "Bearer token123");
    let request = NanoRequest::new("GET".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "GET");
    assert!(data["url"].as_str().unwrap().contains("/api/resource"));
    assert!(data["url"].as_str().unwrap().contains("?filter=active"));
    // Headers are stored in internal __headers__ property of Headers object
    assert!(data["headers"]["__headers__"]["authorization"]
        .as_str()
        .unwrap()
        .contains("Bearer"));
    assert!(!data["hasBody"].as_bool().unwrap());
    assert!(data["body"].is_null());

    pool.shutdown().unwrap();
}

#[test]
fn test_http_get_with_body() {
    // GET with body is valid per HTTP/1.1 RFC 7231
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "get_body_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/query").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    let body = Some(bytes::Bytes::from(r#"{"query":"search term"}"#));
    let request = NanoRequest::new("GET".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "GET");
    assert!(data["hasBody"].as_bool().unwrap());
    // Body is base64 encoded
    let body_b64 = data["body"].as_str().unwrap();
    let body_decoded = String::from_utf8(base64_decode(body_b64)).unwrap();
    assert_eq!(body_decoded, r#"{"query":"search term"}"#);

    pool.shutdown().unwrap();
}

#[test]
fn test_http_post_with_json_body() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "post_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/users").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    headers.set("X-Request-ID", "req-12345");
    let body = Some(bytes::Bytes::from(
        r#"{"name":"John","email":"john@example.com"}"#,
    ));
    let request = NanoRequest::new("POST".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "POST");
    assert!(data["hasBody"].as_bool().unwrap());
    // Body is base64 encoded
    let body_b64 = data["body"].as_str().unwrap();
    let body_decoded = String::from_utf8(base64_decode(body_b64)).unwrap();
    assert!(body_decoded.contains("John"));
    assert_eq!(
        data["headers"]["__headers__"]["x-request-id"]
            .as_str()
            .unwrap(),
        "req-12345"
    );

    pool.shutdown().unwrap();
}

#[test]
fn test_http_post_without_body() {
    // POST without body (e.g., simple action trigger)
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "post_nobody_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/refresh").unwrap();
    let headers = NanoHeaders::new();
    let request = NanoRequest::new("POST".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "POST");
    assert!(!data["hasBody"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_http_put_with_body() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "put_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/users/123").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    headers.set("If-Match", "\"abc123\"");
    let body = Some(bytes::Bytes::from(r#"{"name":"Jane","status":"active"}"#));
    let request = NanoRequest::new("PUT".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "PUT");
    assert!(data["hasBody"].as_bool().unwrap());
    // Body is base64 encoded
    let body_b64 = data["body"].as_str().unwrap();
    let body_decoded = String::from_utf8(base64_decode(body_b64)).unwrap();
    assert!(body_decoded.contains("Jane"));
    assert!(data["url"].as_str().unwrap().contains("/users/123"));

    pool.shutdown().unwrap();
}

#[test]
fn test_http_delete_with_body() {
    // DELETE with body (valid for complex delete operations)
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "delete_body_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/batch-delete").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    let body = Some(bytes::Bytes::from(r#"{"ids":[1,2,3,4,5]}"#));
    let request = NanoRequest::new("DELETE".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "DELETE");
    assert!(data["hasBody"].as_bool().unwrap());
    // Body is base64 encoded
    let body_b64 = data["body"].as_str().unwrap();
    let body_decoded = String::from_utf8(base64_decode(body_b64)).unwrap();
    assert!(body_decoded.contains("1,2,3,4,5"));

    pool.shutdown().unwrap();
}

#[test]
fn test_http_delete_without_body() {
    // Standard DELETE without body
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "delete_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/users/123").unwrap();
    let headers = NanoHeaders::new();
    let request = NanoRequest::new("DELETE".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "DELETE");
    assert!(!data["hasBody"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_http_patch_with_body() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "patch_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/users/123").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/merge-patch+json");
    let body = Some(bytes::Bytes::from(
        r#"{"status":"inactive","lastModified":"2024-01-01"}"#,
    ));
    let request = NanoRequest::new("PATCH".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "PATCH");
    assert!(data["hasBody"].as_bool().unwrap());
    // Body is base64 encoded
    let body_b64 = data["body"].as_str().unwrap();
    let body_decoded = String::from_utf8(base64_decode(body_b64)).unwrap();
    assert!(body_decoded.contains("inactive"));

    pool.shutdown().unwrap();
}

#[test]
fn test_http_head_request() {
    // HEAD requests should not have body per HTTP spec
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "head_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/resource").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Accept", "application/json");
    let request = NanoRequest::new("HEAD".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "HEAD");
    assert!(!data["hasBody"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_http_options_request() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "options_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/resource").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Origin", "https://example.com");
    headers.set("Access-Control-Request-Method", "POST");
    let request = NanoRequest::new("OPTIONS".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "OPTIONS");
    assert!(!data["hasBody"].as_bool().unwrap());
    assert_eq!(
        data["headers"]["__headers__"]["origin"].as_str().unwrap(),
        "https://example.com"
    );

    pool.shutdown().unwrap();
}

#[test]
fn test_http_custom_method() {
    // Test non-standard HTTP methods (WebDAV, etc.)
    init_platform();
    let temp_dir = TempDir::new().unwrap();
    let entrypoint = create_test_handler(&temp_dir, "custom_test.js", create_echo_handler());

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/resource").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "text/xml");
    let body = Some(bytes::Bytes::from("<propfind><allprop/></propfind>"));
    let request = NanoRequest::new("PROPFIND".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert_eq!(data["method"].as_str().unwrap(), "PROPFIND");
    assert!(data["hasBody"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_http_all_methods_with_headers() {
    // Verify headers are passed correctly for all common methods
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let handler_code = r#"
function fetch(request) {
    const headerNames = Object.keys(request.headers);
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            method: request.method,
            headerCount: headerNames.length,
            headers: request.headers
        })
    };
}
"#;

    let entrypoint = create_test_handler(&temp_dir, "headers_all_test.js", handler_code);
    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let methods = vec!["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

    for method in methods {
        let url = NanoUrl::parse(&format!(
            "http://test.example.com/api/{}-test",
            method.to_lowercase()
        ))
        .unwrap();
        let mut headers = NanoHeaders::new();
        headers.set("X-Custom-Header", &format!("value-for-{}", method));
        headers.set("X-Request-Method", method);

        let body =
            if method == "GET" || method == "HEAD" || method == "DELETE" || method == "OPTIONS" {
                None
            } else {
                Some(bytes::Bytes::from(format!(r#"{{"method":"{}"}}"#, method)))
            };

        let request = NanoRequest::new(method.to_string(), url, headers, body);

        let (tx, rx) = oneshot::channel();
        let task = HandlerTask::new(entrypoint.clone(), request, tx);

        pool.dispatch(task).unwrap();
        let response = rx.blocking_recv().unwrap().unwrap();

        assert_eq!(response.status(), 200, "{} request failed", method);

        let body_str = String::from_utf8_lossy(response.body().unwrap());
        let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

        assert_eq!(data["method"].as_str().unwrap(), method);
        assert!(data["headers"]["__headers__"]["x-custom-header"]
            .as_str()
            .unwrap()
            .contains(method));
    }

    pool.shutdown().unwrap();
}
