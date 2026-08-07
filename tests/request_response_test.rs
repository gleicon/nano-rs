//! Integration tests for Request/Response fixes (Phase 17)
//!
//! These tests verify that:
//! 1. Full WinterTC Request object is passed to JS (method, url, headers, body, bodyUsed)
//! 2. Async handlers with await resolve correctly
//! 3. Request body is readable

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::worker::{HandlerTask, WorkerPool};
use std::fs;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::oneshot;

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

#[test]
fn test_wintertc_request_object() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "wintertc_test.js",
        r#"
function fetch(request) {
    // Verify all WinterTC Request properties exist
    const checks = {
        hasMethod: typeof request.method === 'string',
        hasUrl: typeof request.url === 'string',
        hasHeaders: typeof request.headers === 'object',
        hasBody: request.hasOwnProperty('body'),
        hasBodyUsed: typeof request.bodyUsed === 'boolean'
    };

    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(checks)
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/path?query=value").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Accept", "application/json");
    let request = NanoRequest::new("GET".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let checks: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(checks["hasMethod"].as_bool().unwrap());
    assert!(checks["hasUrl"].as_bool().unwrap());
    assert!(checks["hasHeaders"].as_bool().unwrap());
    assert!(checks["hasBody"].as_bool().unwrap());
    assert!(checks["hasBodyUsed"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_request_headers_available() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "headers_test.js",
        r#"
function fetch(request) {
    const headers = request.headers;

    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            hasContentType: !!headers.get('content-type'),
            hasAuthorization: !!headers.get('authorization'),
            contentType: headers.get('content-type'),
            authorization: headers.get('authorization')
        })
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    headers.set("Authorization", "Bearer token123");
    let request = NanoRequest::new("POST".to_string(), url, headers, None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(data["hasContentType"].as_bool().unwrap());
    assert!(data["hasAuthorization"].as_bool().unwrap());
    assert_eq!(data["contentType"].as_str().unwrap(), "application/json");
    assert_eq!(data["authorization"].as_str().unwrap(), "Bearer token123");

    pool.shutdown().unwrap();
}

#[test]
fn test_async_handler_resolves() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "async_test.js",
        r#"
async function fetch(request) {
    // Test async/await works
    const data = await Promise.resolve({ success: true, async: true });

    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(data)
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(data["success"].as_bool().unwrap());
    assert!(data["async"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_promise_fulfilled_state() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "promise_test.js",
        r#"
async function fetch(request) {
    // Create a promise chain
    const value = await Promise.resolve(42)
        .then(v => v * 2)
        .then(v => ({ doubled: v }));

    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(value)
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(data["doubled"].as_i64().unwrap(), 84);

    pool.shutdown().unwrap();
}

#[test]
fn test_request_body_presence() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "body_test.js",
        r#"
function fetch(request) {
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            hasBody: request.body !== null && request.body !== undefined,
            bodyPresent: !!request.body,
            bodyType: typeof request.body
        })
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    // Test with body
    let url = NanoUrl::parse("http://test.example.com/").unwrap();
    let body = Some(bytes::Bytes::from(r#"{"test":"data"}"#));
    let request = NanoRequest::new("POST".to_string(), url, NanoHeaders::new(), body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint.clone(), request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body_str = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body_str).unwrap();

    assert!(data["hasBody"].as_bool().unwrap());
    assert!(data["bodyPresent"].as_bool().unwrap());
    assert_eq!(data["bodyType"].as_str().unwrap(), "string"); // base64 encoded

    // Test without body
    let url2 = NanoUrl::parse("http://test.example.com/").unwrap();
    let request2 = NanoRequest::new("GET".to_string(), url2, NanoHeaders::new(), None);

    let (tx2, rx2) = oneshot::channel();
    let task2 = HandlerTask::new(entrypoint, request2, tx2);

    pool.dispatch(task2).unwrap();
    let response2 = rx2.blocking_recv().unwrap().unwrap();

    assert_eq!(response2.status(), 200);

    let body_str2 = String::from_utf8_lossy(response2.body().unwrap());
    let data2: serde_json::Value = serde_json::from_str(&body_str2).unwrap();

    assert!(!data2["hasBody"].as_bool().unwrap()); // null body
    assert!(!data2["bodyPresent"].as_bool().unwrap());

    pool.shutdown().unwrap();
}

#[test]
fn test_request_url_parsing() {
    init_platform();
    let temp_dir = TempDir::new().unwrap();

    let entrypoint = create_test_handler(
        &temp_dir,
        "url_test.js",
        r#"
function fetch(request) {
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
            url: request.url,
            method: request.method
        })
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url =
        NanoUrl::parse("http://test.example.com:8080/path/to/resource?foo=bar&baz=qux").unwrap();
    let request = NanoRequest::new("PUT".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).unwrap();
    let response = rx.blocking_recv().unwrap().unwrap();

    assert_eq!(response.status(), 200);

    let body = String::from_utf8_lossy(response.body().unwrap());
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(data["url"]
        .as_str()
        .unwrap()
        .contains("test.example.com:8080"));
    assert!(data["url"].as_str().unwrap().contains("/path/to/resource"));
    assert!(data["url"].as_str().unwrap().contains("?foo=bar"));
    assert_eq!(data["method"].as_str().unwrap(), "PUT");

    pool.shutdown().unwrap();
}
