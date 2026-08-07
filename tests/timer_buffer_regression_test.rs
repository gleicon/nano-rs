//! Regression tests for timer and Buffer API fixes.
//!
//! Covers three regressions found in v2.0.0-alpha:
//!
//! - [REGR-TIMER-01] setTimeout must respect delay and fire after >= delay ms
//! - [REGR-TIMER-02] setInterval must return a valid ID (not hang / no-op)
//! - [REGR-TIMER-03] clearInterval must remove the entry without panic
//! - [REGR-BUF-01]   Buffer.from(hexStr, 'hex') must decode hex pairs, not return raw string
//! - [REGR-BUF-02]   Buffer.from(str) (no encoding) must remain unchanged (utf-8 passthrough)
//! - [REGR-BUF-03]   Buffer.from(str, 'utf8') explicit encoding must work
//!
//! Unit-level tests run on every `cargo test`.
//! Integration tests that need a full pump loop are guarded by NANO_TIMER_TESTS=1.

#[path = "common.rs"]
mod common;

use nano::runtime::apis::RuntimeAPIs;
use nano::v8::initialize_platform;
use std::sync::Once;

static INIT_V8: Once = Once::new();

fn init_v8_once() {
    INIT_V8.call_once(|| {
        initialize_platform().expect("V8 platform init failed");
        nano::data_plane::init_code_cache();
    });
}

/// Execute JS in a fresh V8 context with all runtime APIs bound.
/// Returns the JS result as a Rust string.
fn run_js(code: &str) -> String {
    init_v8_once();

    let mut iso = common::create_test_isolate();
    let iso_mut = iso.isolate();

    v8::scope!(scope, iso_mut);
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let src = v8::String::new(ctx_scope, code).expect("JS source alloc");
    let script = v8::Script::compile(ctx_scope, src, None).expect("JS compile");
    let result = script.run(ctx_scope).expect("JS run");
    result
        .to_string(ctx_scope)
        .map(|s| s.to_rust_string_lossy(ctx_scope))
        .unwrap_or_else(|| "undefined".to_string())
}

/// Whether full pump-loop timer integration tests are enabled.
fn timer_tests_enabled() -> bool {
    std::env::var("NANO_TIMER_TESTS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// [REGR-TIMER-01] setTimeout fires callback and respects delay
// ---------------------------------------------------------------------------

/// Verify setTimeout returns a valid ID (callback registered, not fired synchronously).
///
/// New architecture: setTimeout is pump-loop-driven — the callback does NOT
/// fire synchronously inside the setTimeout() call. It registers in a thread-
/// local and fires when the pump loop's Pending arm calls fire_pending_timeouts().
/// In this unit-test context (no pump loop), only registration is verified.
#[test]
fn settimeout_returns_valid_id() {
    let result = run_js(
        r#"
        const id = setTimeout(() => {}, 10);
        typeof id === 'number' && id > 0
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-TIMER-01] setTimeout must return a positive numeric ID"
    );
}

/// Verify setTimeout does NOT block the calling thread (regression guard).
///
/// Previous broken impl: std::thread::sleep(delay_ms) inside the callback →
/// CPU timeout guard fired for delays ≥ cpu_time_limit_ms → HTTP 500.
/// Correct impl: returns immediately, pump loop drives the callback later.
#[test]
fn settimeout_does_not_block() {
    let result = run_js(
        r#"
        const start = Date.now();
        setTimeout(() => {}, 200);
        const elapsed = Date.now() - start;
        // If blocking: elapsed ≥ 200ms → test would be slow AND likely crash
        // Correct: elapsed < 5ms (just a thread-local push)
        elapsed < 5
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-TIMER-01] setTimeout must not block — old impl slept in callback causing CPU-guard crash"
    );
}

/// Verify two setTimeout calls return distinct IDs.
#[test]
fn settimeout_ids_are_unique() {
    let result = run_js(
        r#"
        const id1 = setTimeout(() => {}, 10);
        const id2 = setTimeout(() => {}, 10);
        id1 !== id2
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-TIMER-01] setTimeout must return unique IDs"
    );
}

// ---------------------------------------------------------------------------
// [REGR-TIMER-02] setInterval returns a valid ID
// ---------------------------------------------------------------------------

/// Verify setInterval returns a numeric ID > 0.
///
/// The regression was a no-op that returned a dummy ID of 2, which always
/// collided and never fired. A real implementation returns a unique ID >= 100.
#[test]
fn setinterval_returns_nonzero_id() {
    let result = run_js(
        r#"
        const id = setInterval(() => {}, 10);
        typeof id === 'number' && id > 0
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-TIMER-02] setInterval must return a positive numeric ID"
    );
}

/// Verify two setInterval calls return distinct IDs.
#[test]
fn setinterval_ids_are_unique() {
    let result = run_js(
        r#"
        const id1 = setInterval(() => {}, 10);
        const id2 = setInterval(() => {}, 10);
        id1 !== id2
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-TIMER-02] setInterval must return unique IDs for each call"
    );
}

// ---------------------------------------------------------------------------
// [REGR-TIMER-03] clearInterval removes entry without panic
// ---------------------------------------------------------------------------

/// clearInterval with a valid ID must not throw.
#[test]
fn clearinterval_valid_id_no_throw() {
    let result = run_js(
        r#"
        const id = setInterval(() => {}, 50);
        clearInterval(id);
        'ok'
        "#,
    );
    assert_eq!(
        result, "ok",
        "[REGR-TIMER-03] clearInterval must not throw or panic"
    );
}

/// clearInterval with an unknown ID must not throw (spec says ignore).
#[test]
fn clearinterval_unknown_id_no_throw() {
    let result = run_js(
        r#"
        clearInterval(9999);
        'ok'
        "#,
    );
    assert_eq!(
        result, "ok",
        "[REGR-TIMER-03] clearInterval with unknown ID must be a no-op"
    );
}

// ---------------------------------------------------------------------------
// [REGR-BUF-01] Buffer.from(hexStr, 'hex') decodes hex
// ---------------------------------------------------------------------------

/// "hello" in hex — the canonical test case from the regression report.
#[test]
fn buffer_from_hex_hello() {
    let result = run_js(
        r#"
        Buffer.from('68656c6c6f', 'hex').toString()
        "#,
    );
    assert_eq!(
        result, "hello",
        "[REGR-BUF-01] Buffer.from('68656c6c6f', 'hex') must decode to 'hello'"
    );
}

/// Verify byte values, not just toString() round-trip.
#[test]
fn buffer_from_hex_byte_values() {
    let result = run_js(
        r#"
        const b = Buffer.from('0102ff', 'hex');
        [b[0], b[1], b[2]].join(',')
        "#,
    );
    assert_eq!(
        result, "1,2,255",
        "[REGR-BUF-01] Buffer.from hex bytes must equal decoded values"
    );
}

/// Empty hex string produces empty buffer.
#[test]
fn buffer_from_hex_empty() {
    let result = run_js(
        r#"
        Buffer.from('', 'hex').length
        "#,
    );
    assert_eq!(
        result, "0",
        "[REGR-BUF-01] Buffer.from('', 'hex') must produce zero-length buffer"
    );
}

/// Odd-length hex string: last incomplete byte is dropped (Node.js behaviour).
#[test]
fn buffer_from_hex_odd_length_truncates() {
    let result = run_js(
        r#"
        // "abc" hex — only "ab" decodes, "c" is dropped
        const b = Buffer.from('abc', 'hex');
        b.length === 1 && b[0] === 0xab
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-BUF-01] Odd-length hex: last incomplete byte must be dropped"
    );
}

/// Invalid hex chars produce zero bytes for invalid pairs (best-effort).
#[test]
fn buffer_from_hex_invalid_chars_skipped() {
    // "zz" is not valid hex — from_str_radix fails, pair is skipped.
    let result = run_js(
        r#"
        const b = Buffer.from('41zz42', 'hex');
        // Only '41' ('A') decodes; 'zz' fails; '42' ('B') decodes.
        b.length === 2 && b[0] === 0x41 && b[1] === 0x42
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-BUF-01] Invalid hex pairs must be skipped silently"
    );
}

// ---------------------------------------------------------------------------
// [REGR-BUF-02] Buffer.from without encoding stays utf-8
// ---------------------------------------------------------------------------

/// No encoding argument → treat input as UTF-8 bytes.
#[test]
fn buffer_from_string_no_encoding_utf8() {
    let result = run_js(
        r#"
        const b = Buffer.from('hello');
        b.length === 5 && b[0] === 104  // 'h'
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-BUF-02] Buffer.from(str) without encoding must use UTF-8"
    );
}

// ---------------------------------------------------------------------------
// [REGR-BUF-03] Explicit utf8 encoding works
// ---------------------------------------------------------------------------

/// Explicit 'utf8' encoding must behave identically to no encoding.
#[test]
fn buffer_from_string_explicit_utf8() {
    let result = run_js(
        r#"
        const a = Buffer.from('test');
        const b = Buffer.from('test', 'utf8');
        a.length === b.length && a[0] === b[0]
        "#,
    );
    assert_eq!(
        result, "true",
        "[REGR-BUF-03] Buffer.from(str, 'utf8') must match Buffer.from(str)"
    );
}

// ---------------------------------------------------------------------------
// Integration: setInterval fires repeatedly via pump loop
// Requires NANO_TIMER_TESTS=1 and a full HTTP server.
// ---------------------------------------------------------------------------

/// Write JS to a temp file and return the path.
#[allow(dead_code)]
fn write_js(name: &str, code: &str) -> String {
    let dir = std::env::temp_dir().join("nano_timer_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, code).unwrap();
    path.to_str().unwrap().to_string()
}

/// JS handler: counts setInterval fires, resolves after 3 ticks, returns count.
const INTERVAL_COUNT_JS: &str = r#"
async function fetch(_request) {
    let count = 0;
    await new Promise(resolve => {
        const id = setInterval(() => {
            count += 1;
            if (count >= 3) {
                clearInterval(id);
                resolve();
            }
        }, 10);
    });
    return new Response(String(count), { status: 200 });
}
"#;

/// [REGR-TIMER-01] setTimeout fires via pump loop — response arrives after delay.
///
/// Requires NANO_TIMER_TESTS=1.
#[tokio::test]
#[ignore = "requires NANO_TIMER_TESTS=1 and full V8 server setup"]
async fn settimeout_fires_via_pump_loop() {
    if !timer_tests_enabled() {
        return;
    }
    init_v8_once();

    use nano::http::router::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
    use nano::http::server::{create_app_with_shutdown, AppStateWithShutdown};
    use nano::signal::ShutdownState;
    use std::sync::Arc;

    let js = r#"
    async function fetch(_request) {
        const start = Date.now();
        await new Promise(resolve => setTimeout(resolve, 100));
        const elapsed = Date.now() - start;
        return new Response(elapsed >= 90 ? "ok" : "too_fast:" + elapsed, { status: 200 });
    }
    "#;

    let entrypoint = write_js(&format!("settimeout_pump_{}.js", std::process::id()), js);
    let mut vhr = VirtualHostRouter::new(RouteTarget {
        hostname: "localhost".to_string(),
        handler_type: HandlerType::WinterTCHandler(entrypoint.clone()),
    });
    vhr.register(
        "localhost".to_string(),
        RouteTarget {
            hostname: "localhost".to_string(),
            handler_type: HandlerType::WinterTCHandler(entrypoint),
        },
    );
    let state = Arc::new(AppStateWithShutdown::new(
        AppState::new(vhr, 1),
        ShutdownState::default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, create_app_with_shutdown(state))
            .await
            .unwrap()
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let body = reqwest::get(format!("http://127.0.0.1:{}/", addr.port()))
        .await
        .expect("request")
        .text()
        .await
        .expect("body");
    assert_eq!(
        body, "ok",
        "[REGR-TIMER-01] setTimeout pump-loop: expected delay ≥90ms, got '{}'",
        body
    );
}

/// [REGR-TIMER-02] setInterval fires via pump loop — count reaches 3.
///
/// Requires NANO_TIMER_TESTS=1. Starts a full HTTP server, makes one request,
/// and checks the response body equals "3".
#[tokio::test]
#[ignore = "requires NANO_TIMER_TESTS=1 and full V8 server setup"]
async fn setinterval_fires_via_pump_loop() {
    if !timer_tests_enabled() {
        return;
    }
    init_v8_once();

    use nano::http::router::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
    use nano::http::server::{create_app_with_shutdown, AppStateWithShutdown};
    use nano::signal::ShutdownState;
    use std::sync::Arc;

    let entrypoint = write_js(
        &format!("timer_test_{}.js", std::process::id()),
        INTERVAL_COUNT_JS,
    );

    let mut vhr = VirtualHostRouter::new(RouteTarget {
        hostname: "localhost".to_string(),
        handler_type: HandlerType::WinterTCHandler(entrypoint.clone()),
    });
    vhr.register(
        "localhost".to_string(),
        RouteTarget {
            hostname: "localhost".to_string(),
            handler_type: HandlerType::WinterTCHandler(entrypoint.clone()),
        },
    );

    let app_state = AppState::new(vhr, 1);
    let shutdown_state = ShutdownState::default();
    let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));
    let app = create_app_with_shutdown(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let url = format!("http://127.0.0.1:{}/", addr.port());
    let resp = reqwest::get(&url).await.expect("HTTP request failed");
    let body = resp.text().await.expect("body read");

    assert_eq!(
        body, "3",
        "[REGR-TIMER-02] setInterval pump-loop: expected 3 fires, got '{}'",
        body
    );
}

/// Helper: spin up a fresh in-process server with a JS handler, make one GET, return body.
#[cfg(test)]
async fn one_shot_request(js: &str) -> String {
    use nano::http::router::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
    use nano::http::server::{create_app_with_shutdown, AppStateWithShutdown};
    use nano::signal::ShutdownState;
    use std::sync::Arc;

    let entrypoint = write_js(&format!("oneshot_{}.js", uuid_fragment()), js);
    let mut vhr = VirtualHostRouter::new(RouteTarget {
        hostname: "localhost".to_string(),
        handler_type: HandlerType::WinterTCHandler(entrypoint.clone()),
    });
    vhr.register(
        "localhost".to_string(),
        RouteTarget {
            hostname: "localhost".to_string(),
            handler_type: HandlerType::WinterTCHandler(entrypoint),
        },
    );
    let state = Arc::new(AppStateWithShutdown::new(
        AppState::new(vhr, 1),
        ShutdownState::default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, create_app_with_shutdown(state))
            .await
            .unwrap()
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    reqwest::get(format!("http://127.0.0.1:{}/", addr.port()))
        .await
        .expect("request")
        .text()
        .await
        .expect("body")
}

fn uuid_fragment() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("{}_{}", std::process::id(), nanos)
}

/// [REGR-TIMER-04] clearTimeout prevents the callback from executing.
///
/// Pattern: register a timer that sets a flag, immediately clear it,
/// wait via a second timer, assert flag is still false.
#[tokio::test]
#[ignore = "requires NANO_TIMER_TESTS=1 and full V8 server setup"]
async fn cleartimeout_cancels_callback() {
    if !timer_tests_enabled() {
        return;
    }
    init_v8_once();

    let js = r#"
    async function fetch(_request) {
        let fired = false;
        const id = setTimeout(() => { fired = true; }, 30);
        clearTimeout(id);
        // Use a second timer (different ID) to wait past the cancelled deadline.
        await new Promise(resolve => setTimeout(resolve, 80));
        return new Response(fired ? "timer_fired" : "ok", { status: 200 });
    }
    "#;

    let body = one_shot_request(js).await;
    assert_eq!(
        body, "ok",
        "[REGR-TIMER-04] clearTimeout must cancel callback — fired after cancel: '{}'",
        body
    );
}

/// [REGR-TIMER-06] clearInterval called from within its own callback fires exactly once.
///
/// The drain-and-reinsert logic in fire_pending_intervals uses
/// INTERVALS_CLEARED_DURING_FIRE to skip re-insertion when clearInterval(id)
/// is called from the interval's own callback. Without that guard the interval
/// would re-insert and fire on every subsequent pump tick.
#[tokio::test]
#[ignore = "requires NANO_TIMER_TESTS=1 and full V8 server setup"]
async fn clearinterval_from_within_callback_fires_once() {
    if !timer_tests_enabled() {
        return;
    }
    init_v8_once();

    let js = r#"
    async function fetch(_request) {
        let count = 0;
        await new Promise(resolve => {
            const id = setInterval(() => {
                count += 1;
                clearInterval(id);  // cancel self — must not re-insert
                resolve();
            }, 10);
        });
        // Wait one more interval period to confirm no second fire.
        await new Promise(r => setTimeout(r, 30));
        return new Response(String(count), { status: 200 });
    }
    "#;

    let body = one_shot_request(js).await;
    assert_eq!(
        body, "1",
        "[REGR-TIMER-06] clearInterval inside callback must fire exactly once, got '{}'",
        body
    );
}

/// [REGR-TIMER-05] Promise chain with sequential awaited timers completes.
///
/// Three chained awaits each backed by a separate setTimeout.
/// Total expected delay: ~60ms. Verifies the pump loop propagates microtasks
/// correctly across multiple await boundaries inside one async handler.
#[tokio::test]
#[ignore = "requires NANO_TIMER_TESTS=1 and full V8 server setup"]
async fn promise_chain_sequential_timers() {
    if !timer_tests_enabled() {
        return;
    }
    init_v8_once();

    let js = r#"
    async function fetch(_request) {
        const start = Date.now();
        await new Promise(r => setTimeout(r, 10));
        await new Promise(r => setTimeout(r, 20));
        await new Promise(r => setTimeout(r, 30));
        const elapsed = Date.now() - start;
        return new Response(elapsed >= 55 ? "ok" : "too_fast:" + elapsed, { status: 200 });
    }
    "#;

    let body = one_shot_request(js).await;
    assert_eq!(
        body, "ok",
        "[REGR-TIMER-05] Promise chain: three sequential timers must all fire, got '{}'",
        body
    );
}
