//! V8 isolate endurance tests — end-to-end through WorkerPool.
//!
//! These tests prove the correctness properties from STAB-02, STAB-03, and STAB-04
//! through the WorkerPool dispatch path, including exception recovery across sequential
//! requests on the same persistent isolate.
//!
//! Tests:
//!   [ENDURE-01] Exception at request N does not break request N+1 (STAB-02)
//!   [ENDURE-02] Module-level state persists within one isolate lifetime (STAB-03, CF-Workers semantics)
//!   [ENDURE-03] 15+ requests per worker with no degradation (STAB-04)

use nano::http::{NanoHeaders, NanoRequest, NanoUrl};
use nano::vfs::VfsBackendEnum;
use nano::worker::pool::WorkerPool;
use nano::worker::HandlerTask;

use std::sync::Arc;

fn init_v8() {
    let _ = nano::v8::initialize_platform();
    nano::data_plane::init_code_cache();
}

fn make_backend() -> VfsBackendEnum {
    VfsBackendEnum::Memory(Arc::new(nano::vfs::MemoryBackend::default()))
}

fn write_js(name: &str, code: &str) -> String {
    let dir = std::env::temp_dir().join("nano_endurance_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, code).unwrap();
    path.to_str().unwrap().to_string()
}

fn make_get(url: &str) -> NanoRequest {
    NanoRequest::new(
        "GET".to_string(),
        NanoUrl::parse(url).unwrap(),
        NanoHeaders::new(),
        None,
    )
}

// ─── [ENDURE-01] Exception recovery ──────────────────────────────────────────
//
// Send 30 requests to a 1-worker pool. The JS handler throws on every 3rd call
// (call_count % 3 == 0). Each throw must NOT poison the isolate — the immediately
// following request must return Ok(200).

#[test]
fn endure_01_exception_recovery() {
    init_v8();

    // The handler increments call_count first, then throws if call_count % 3 == 0.
    // This means requests 3, 6, 9, ... (1-indexed) throw.
    let entrypoint = write_js(
        "endure01.js",
        r#"
var call_count = 0;
function __nano_user_fetch(req) {
    call_count++;
    if (call_count % 3 === 0) {
        throw new Error("intentional at " + call_count);
    }
    return { status: 200, headers: {}, body: "ok:" + call_count };
}
"#,
    );

    let pool = WorkerPool::with_backend(
        "endurance.test".into(),
        1, // exactly 1 worker — all requests hit the same persistent scope
        0,
        make_backend(),
    );

    let mut prev_threw = false;
    for i in 0..30 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = HandlerTask::new(entrypoint.clone(), make_get("http://endurance.test/"), tx);
        pool.dispatch(task).unwrap();
        let result = rx
            .blocking_recv()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("channel closed")));

        // call_count inside JS = i+1 (1-indexed)
        let is_throw_req = (i + 1) % 3 == 0;

        if is_throw_req {
            // Throw request: Err(_) is the expected outcome (TryCatch → JS exception)
            match &result {
                Err(_) => {} // expected
                Ok(r) => {
                    // status 500 is also acceptable if the worker converts exceptions
                    assert!(
                        r.status() == 500,
                        "[ENDURE-01] request {} expected throw/500, got status {}",
                        i,
                        r.status()
                    );
                }
            }
            prev_threw = true;
        } else {
            if prev_threw {
                // CRITICAL: request immediately after a throw must succeed
                assert!(
                    result.is_ok(),
                    "[ENDURE-01] request {} (after throw) got Err: {:?}",
                    i,
                    result.err()
                );
                let r = result.unwrap();
                assert_eq!(
                    r.status(),
                    200,
                    "[ENDURE-01] request {} (after throw) expected 200, got {}",
                    i,
                    r.status()
                );
                prev_threw = false;
            } else {
                // Non-throw, non-recovery request: should be 200
                assert!(
                    result.is_ok(),
                    "[ENDURE-01] request {} got error: {:?}",
                    i,
                    result.err()
                );
                assert_eq!(
                    result.unwrap().status(),
                    200,
                    "[ENDURE-01] request {} not 200",
                    i
                );
            }
        }
    }
}

// ─── [ENDURE-02] Module-level state persists within one isolate lifetime ──────
//
// [ENDURE-02] Module-level state persists within one isolate lifetime — CF-Workers semantics.
// This is correct behaviour, NOT a bug.
//
// The JS handler increments a module-level counter and returns it.
// With a 1-worker pool, the counter must monotonically increase: 1, 2, 3, 4, 5.
// This documents Cloudflare Workers semantics: module global state is shared within
// one isolate's lifetime. Fresh state only appears after isolate recycle.

#[test]
fn endure_02_module_state_persists() {
    init_v8();

    let entrypoint = write_js(
        "endure02.js",
        r#"
var request_count = 0;
function __nano_user_fetch(req) {
    request_count++;
    return { status: 200, headers: {}, body: String(request_count) };
}
"#,
    );

    let pool = WorkerPool::with_backend("endurance.test".into(), 1, 0, make_backend());

    let expected = [1u32, 2, 3, 4, 5];
    for (i, &exp) in expected.iter().enumerate() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        pool.dispatch(HandlerTask::new(
            entrypoint.clone(),
            make_get("http://endurance.test/"),
            tx,
        ))
        .unwrap();
        let result = rx
            .blocking_recv()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("channel closed")));
        assert!(
            result.is_ok(),
            "[ENDURE-02] request {} error: {:?}",
            i,
            result.err()
        );
        let r = result.unwrap();
        assert_eq!(
            r.status(),
            200,
            "[ENDURE-02] request {} status={}",
            i,
            r.status()
        );
        let body = r
            .body()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        let got: u32 = body.trim().parse().unwrap_or(0);
        assert_eq!(
            got, exp,
            "[ENDURE-02] request {} expected counter={}, got={}",
            i, exp, got
        );
    }
}

// ─── [ENDURE-03] 15+ requests per worker, no degradation ─────────────────────
//
// Send 15 requests to a 1-worker pool. All must return Ok(200).
// Verifies STAB-04: 10+ requests per worker with no degradation.

#[test]
fn endure_03_ten_plus_requests_no_degradation() {
    init_v8();

    let entrypoint = write_js(
        "endure03.js",
        r#"
function __nano_user_fetch(req) {
    return { status: 200, headers: {}, body: "ok" };
}
"#,
    );

    let pool = WorkerPool::with_backend("endurance.test".into(), 1, 0, make_backend());

    for i in 0..15 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        pool.dispatch(HandlerTask::new(
            entrypoint.clone(),
            make_get("http://endurance.test/"),
            tx,
        ))
        .unwrap();
        let result = rx
            .blocking_recv()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("channel closed")));
        assert!(
            result.is_ok(),
            "[ENDURE-03] request {} got error: {:?}",
            i,
            result.err()
        );
        assert_eq!(
            result.unwrap().status(),
            200,
            "[ENDURE-03] request {} degraded",
            i
        );
    }
}
