//! Worker pool dispatch tests — unique tests extracted from src/worker/pool.rs
//!
//! Tests that are redundant with unified_worker_pool_test.rs have been removed.
//! `test_round_robin_dispatch` remains embedded in pool.rs (needs private field access).

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::sliver::{pack_sliver, SliverMetadata, UnpackedSliver};
use nano::vfs::{VfsFile, VfsNamespace, VfsPath};
use nano::worker::pool::{SliverWorkerPool, WorkerPool};
use nano::worker::HandlerTask;
use std::fs;
use std::io::Write;
use tempfile::TempDir;
use tokio::sync::oneshot;

// ─── helpers ────────────────────────────────────────────────────────────────

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

fn create_test_sliver_for_pool(hostname: &str) -> UnpackedSliver {
    let metadata = SliverMetadata::new(hostname, "1.1.0");
    let archive = pack_sliver(&metadata, None, None).unwrap();
    nano::sliver::unpack_sliver(&archive).unwrap()
}

fn create_sliver_with_handler(hostname: &str, js_code: &str) -> UnpackedSliver {
    let metadata = SliverMetadata::new(hostname, "1.1.0");
    let vfs_entries = vec![(
        VfsPath::new("index.js").unwrap(),
        VfsFile::new(js_code.as_bytes().to_vec()),
    )];
    let archive = pack_sliver(&metadata, None, Some(&vfs_entries)).unwrap();
    nano::sliver::unpack_sliver(&archive).unwrap()
}

// ─── WorkerPool dispatch tests ───────────────────────────────────────────────

#[test]
fn test_dispatch_and_response() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

    let js_code = format!(
        r#"
function fetch(request) {{
    return {{ status: 200, headers: {{ "Content-Type": "text/plain" }}, body: "{}" }};
}}
"#,
        dynamic_token
    );
    let entrypoint = create_test_handler(&temp_dir, "test.js", &js_code);

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive response");

    assert!(
        response.is_ok(),
        "Handler execution failed: {:?}",
        response.err()
    );
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("Content-Type"),
        Some("text/plain".to_string())
    );
    assert!(resp.body().is_some());

    let body_text = String::from_utf8_lossy(resp.body().unwrap());
    assert!(
        body_text.contains(&dynamic_token),
        "Response must contain dynamic token '{}', got: {}",
        dynamic_token,
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}

/// Live telemetry must reflect a real dispatched request: after a worker serves
/// a request, its isolate appears in the telemetry snapshot with request_count >= 1
/// and a real used-heap measurement. Proves the pool → admin-plane wiring.
#[test]
fn test_dispatch_publishes_live_telemetry() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let hostname = format!("telemetry-{}.example.com", uuid::Uuid::new_v4());
    let js_code = r#"function fetch(request) { return { status: 200, body: "ok" }; }"#;
    let entrypoint = create_test_handler(&temp_dir, "tel.js", js_code);

    let pool = WorkerPool::new(hostname.clone(), 1, 0);

    let url = NanoUrl::parse("http://test/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);
    let (tx, rx) = oneshot::channel();
    pool.dispatch(HandlerTask::new(entrypoint, request, tx))
        .expect("dispatch");
    let response = rx.blocking_recv().expect("recv");
    assert!(response.is_ok(), "handler failed: {:?}", response.err());

    // The serving isolate should now be visible in live telemetry.
    let snap = nano::worker::telemetry::snapshot();
    let mine: Vec<_> = snap.iter().filter(|s| s.hostname == hostname).collect();
    assert_eq!(mine.len(), 1, "one live isolate for this host");
    assert!(mine[0].request_count >= 1, "request counted");
    assert!(
        mine[0].memory_bytes.is_some(),
        "real used-heap recorded after a request"
    );

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_concurrent_requests() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let entrypoint = create_test_handler(
        &temp_dir,
        "handler.js",
        r#"
function fetch(request) {
    return { status: 200, headers: {}, body: "OK" };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 4, 0);

    let mut receivers = vec![];
    for i in 0..10 {
        let url = NanoUrl::parse(&format!("http://test/{}", i)).unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

        let (tx, rx) = oneshot::channel();
        let task = HandlerTask::new(entrypoint.clone(), request, tx);

        pool.dispatch(task).unwrap();
        receivers.push(rx);
    }

    for (i, rx) in receivers.into_iter().enumerate() {
        let response = rx
            .blocking_recv()
            .unwrap_or_else(|_| panic!("Failed to receive response {}", i));
        assert!(
            response.is_ok(),
            "Request {} failed: {:?}",
            i,
            response.err()
        );
        let resp = response.unwrap();
        assert_eq!(resp.status(), 200);
    }

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_full_request_object_passed() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let entrypoint = create_test_handler(
        &temp_dir,
        "full_request.js",
        r#"
function fetch(request) {
    const info = {
        method: request.method,
        url: request.url,
        headers: request.headers,
        hasBody: request.body !== null
    };
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(info)
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test.example.com/api/items/123?expand=true").unwrap();
    let mut headers = NanoHeaders::new();
    headers.set("Content-Type", "application/json");
    headers.set("X-Custom-Header", "custom-value");
    let body = Some(bytes::Bytes::from(r#"{"key":"value"}"#));
    let request = NanoRequest::new("POST".to_string(), url, headers, body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive");

    assert!(response.is_ok(), "Handler failed: {:?}", response.err());
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = String::from_utf8_lossy(resp.body().unwrap());
    assert!(
        body_text.contains("POST"),
        "Method not found: {}",
        body_text
    );
    assert!(
        body_text.contains("http://test.example.com/api/items/123"),
        "URL not found: {}",
        body_text
    );
    assert!(
        body_text.contains("custom-value"),
        "Header not found: {}",
        body_text
    );
    assert!(
        body_text.contains("true"),
        "Body flag not found: {}",
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_async_handler_promise() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

    let js_code = format!(
        r#"
async function fetch(request) {{
    const data = await Promise.resolve({{ token: "{}" }});
    return {{
        status: 200,
        headers: {{ "Content-Type": "application/json" }},
        body: JSON.stringify(data)
    }};
}}
"#,
        dynamic_token
    );
    let entrypoint = create_test_handler(&temp_dir, "async_handler.js", &js_code);

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive");

    assert!(
        response.is_ok(),
        "Async handler failed: {:?}",
        response.err()
    );
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = String::from_utf8_lossy(resp.body().unwrap());
    assert!(
        body_text.contains(&dynamic_token),
        "Async response must contain dynamic token '{}', got: {}",
        dynamic_token,
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_request_body_passed() {
    init_platform();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let entrypoint = create_test_handler(
        &temp_dir,
        "body_check.js",
        r#"
function fetch(request) {
    const hasBody = request.body !== null;
    const bodyUsed = request.bodyUsed;
    return {
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ hasBody, bodyUsed })
    };
}
"#,
    );

    let pool = WorkerPool::new("test.example.com".to_string(), 1, 0);

    let url = NanoUrl::parse("http://test/").unwrap();
    let body = Some(bytes::Bytes::from("Hello from client"));
    let request = NanoRequest::new("POST".to_string(), url, NanoHeaders::new(), body);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new(entrypoint, request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive");

    assert!(
        response.is_ok(),
        "Body passing failed: {:?}",
        response.err()
    );
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = String::from_utf8_lossy(resp.body().unwrap());
    assert!(
        body_text.contains("true"),
        "Body flags not correct: {}",
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_worker_pool_vfs_isolation() {
    init_platform();

    let pool1 = WorkerPool::new("app1.example.com".to_string(), 1, 0);
    let pool2 = WorkerPool::new("app2.example.com".to_string(), 1, 0);

    let namespace1 = VfsNamespace::from_hostname("app1.example.com");
    let path1 = nano::vfs::VfsPath::new(&format!("{}::secret.txt", namespace1.as_str())).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        pool1
            .vfs_backend()
            .write(&path1, b"app1-secret-data")
            .await
            .unwrap();
    });

    let exists_in_pool1: bool = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { pool1.vfs_backend().exists(&path1).await.unwrap() });
    assert!(exists_in_pool1, "File should exist in pool1's VFS");

    let namespace2 = VfsNamespace::from_hostname("app2.example.com");
    let path2 = nano::vfs::VfsPath::new(&format!("{}::secret.txt", namespace2.as_str())).unwrap();

    let exists_in_pool2: bool = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { pool2.vfs_backend().exists(&path2).await.unwrap() });
    assert!(
        !exists_in_pool2,
        "File should NOT exist in pool2's VFS (isolated)"
    );

    pool1.shutdown().expect("Pool1 shutdown failed");
    pool2.shutdown().expect("Pool2 shutdown failed");
}

// ─── SliverWorkerPool tests ──────────────────────────────────────────────────

#[test]
fn test_sliver_worker_pool_creation() {
    init_platform();
    let unpacked = create_test_sliver_for_pool("sliver-test.example.com");

    let pool = SliverWorkerPool::new("sliver-test.example.com".to_string(), 2, 0, unpacked);

    assert_eq!(pool.worker_count(), 2);
    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_sliver_worker_pool_single_worker() {
    init_platform();
    let unpacked = create_test_sliver_for_pool("single.example.com");

    let pool = SliverWorkerPool::new("single.example.com".to_string(), 1, 0, unpacked);

    assert_eq!(pool.worker_count(), 1);
    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_sliver_worker_pool_dispatch() {
    init_platform();

    let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

    let js_code = format!(
        r#"function fetch(request) {{ return {{ status: 200, headers: {{}}, body: "{}" }}; }}"#,
        dynamic_token
    );

    let unpacked = create_sliver_with_handler("dispatch.example.com", &js_code);
    let pool = SliverWorkerPool::new("dispatch.example.com".to_string(), 1, 0, unpacked);

    let url = NanoUrl::parse("http://test/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new("/index.js".to_string(), request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive response");

    assert!(
        response.is_ok(),
        "Handler execution failed: {:?}",
        response.err()
    );
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);

    let body_text = String::from_utf8_lossy(resp.body().map(|b| &b[..]).unwrap_or(&[]));
    assert!(
        body_text.contains(&dynamic_token),
        "Sliver response must contain dynamic token '{}', got: {}",
        dynamic_token,
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}

/// Blue-green hot-swap: the same slot (same hostname) serves v1, then after a
/// `hotswap` serves v2 from a fully new pool — no hostname change, no restart.
#[test]
fn test_sliver_pool_slot_hotswap_serves_new_version() {
    use nano::worker::SliverPoolSlot;
    init_platform();

    let host = "hotswap.example.com";
    let v1_token = format!("v1-{}", uuid::Uuid::new_v4());
    let v2_token = format!("v2-{}", uuid::Uuid::new_v4());

    let v1 = create_sliver_with_handler(
        host,
        &format!(r#"function fetch(r){{return{{status:200,headers:{{}},body:"{v1_token}"}};}}"#),
    );
    let slot = SliverPoolSlot::new(host.to_string(), 1, 0, v1);

    // Helper: dispatch one GET / through the pool currently in the slot.
    let dispatch_once = |slot: &SliverPoolSlot| -> String {
        let url = NanoUrl::parse("http://test/").unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);
        let (tx, rx) = oneshot::channel();
        let task = HandlerTask::new("/index.js".to_string(), request, tx);
        slot.current().dispatch(task).expect("dispatch failed");
        let resp = rx
            .blocking_recv()
            .expect("recv failed")
            .expect("handler err");
        assert_eq!(resp.status(), 200);
        String::from_utf8_lossy(resp.body().map(|b| &b[..]).unwrap_or(&[])).to_string()
    };

    // v1 is live.
    assert!(
        dispatch_once(&slot).contains(&v1_token),
        "expected v1 before swap"
    );

    // Swap in v2; the returned old pool has no further traffic — retire it.
    let v2 = create_sliver_with_handler(
        host,
        &format!(r#"function fetch(r){{return{{status:200,headers:{{}},body:"{v2_token}"}};}}"#),
    );
    let old = slot.hotswap(v2);
    std::sync::Arc::try_unwrap(old)
        .expect("old pool should be sole-owned after swap")
        .shutdown()
        .expect("drain old pool");

    // Same slot, same hostname — now serves v2.
    let body = dispatch_once(&slot);
    assert!(
        body.contains(&v2_token),
        "expected v2 after swap, got: {body}"
    );
    assert!(!body.contains(&v1_token), "must not serve v1 after swap");
    assert_eq!(
        slot.hostname(),
        host,
        "hostname must not change across swap"
    );

    std::sync::Arc::try_unwrap(slot.current())
        .ok()
        .map(|p| p.shutdown());
}

/// `hotswap_and_drain` (the SIGHUP path): swaps in v2 and spawns a drain task for
/// the old pool. Verifies the new version serves and the retired pool is dropped
/// without panicking. Runs inside a runtime context so the spawned drain task has
/// somewhere to run.
#[test]
fn test_sliver_pool_slot_hotswap_and_drain() {
    use nano::worker::SliverPoolSlot;
    init_platform();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter(); // gives hotswap_and_drain's tokio::spawn a runtime

    let host = "drain.example.com";
    let v1_token = format!("v1-{}", uuid::Uuid::new_v4());
    let v2_token = format!("v2-{}", uuid::Uuid::new_v4());

    let v1 = create_sliver_with_handler(
        host,
        &format!(r#"function fetch(r){{return{{status:200,headers:{{}},body:"{v1_token}"}};}}"#),
    );
    let slot = SliverPoolSlot::new(host.to_string(), 1, 0, v1);

    let dispatch_once = |slot: &SliverPoolSlot| -> String {
        let url = NanoUrl::parse("http://test/").unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);
        let (tx, rx) = oneshot::channel();
        let task = HandlerTask::new("/index.js".to_string(), request, tx);
        slot.current().dispatch(task).expect("dispatch failed");
        let resp = rx
            .blocking_recv()
            .expect("recv failed")
            .expect("handler err");
        String::from_utf8_lossy(resp.body().map(|b| &b[..]).unwrap_or(&[])).to_string()
    };

    assert!(dispatch_once(&slot).contains(&v1_token));

    let v2 = create_sliver_with_handler(
        host,
        &format!(r#"function fetch(r){{return{{status:200,headers:{{}},body:"{v2_token}"}};}}"#),
    );
    slot.hotswap_and_drain(v2, std::time::Duration::from_millis(100));

    assert!(
        dispatch_once(&slot).contains(&v2_token),
        "expected v2 after drain-swap"
    );

    // Let the drain task fire (drops and retires the old pool).
    std::thread::sleep(std::time::Duration::from_millis(300));
}

#[test]
fn test_sliver_worker_pool_accessors() {
    init_platform();
    let unpacked = create_test_sliver_for_pool("accessors.example.com");
    let sliver_hostname = unpacked.metadata.hostname.clone();

    let pool = SliverWorkerPool::new("accessors.example.com".to_string(), 1, 0, unpacked);

    let sliver_data = pool.sliver_data();
    assert_eq!(sliver_data.metadata.hostname, sliver_hostname);

    let _vfs_backend = pool.vfs_backend();

    pool.shutdown().expect("Shutdown failed");
}

#[test]
fn test_sliver_worker_pool_with_temp_vfs() {
    init_platform();

    let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

    let temp_handler_code = format!(
        r#"function fetch(request) {{ return {{ status: 200, headers: {{ "Content-Type": "text/plain" }}, body: "{}" }}; }}"#,
        dynamic_token
    );

    let unpacked = create_sliver_with_handler("temp-vfs.example.com", &temp_handler_code);

    let pool = SliverWorkerPool::new("temp-vfs.example.com".to_string(), 1, 0, unpacked);

    let url = NanoUrl::parse("http://test/").unwrap();
    let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

    let (tx, rx) = oneshot::channel();
    let task = HandlerTask::new("/index.js".to_string(), request, tx);

    pool.dispatch(task).expect("Failed to dispatch");
    let response = rx.blocking_recv().expect("Failed to receive response");

    assert!(
        response.is_ok(),
        "Handler execution failed: {:?}",
        response.err()
    );
    let resp = response.unwrap();
    assert_eq!(resp.status(), 200);

    let body = resp.body().cloned().unwrap_or_default();
    let body_text = String::from_utf8_lossy(&body);
    assert!(
        body_text.contains(&dynamic_token),
        "Expected response with dynamic token '{}', got: {}",
        dynamic_token,
        body_text
    );

    pool.shutdown().expect("Shutdown failed");
}
