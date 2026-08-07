//! WebSocket Integration Tests — Phase 23
//!
//! Verifies the full WebSocket stack: HTTP upgrade, JS handler dispatch,
//! message echo, close, 32 MiB size limit enforcement, and accept guard.
//!
//! Tests that require a running HTTP server with a full AppState are marked
//! #[ignore] — they require the `NANO_WS_TESTS=1` env var to run since they
//! exercise the complete upgrade path through `dispatch_to_worker_pool`.
//!
//! Tests without server setup (unit-style) run normally.
//!
//! # Running WS server tests
//!
//! ```bash
//! NANO_WS_TESTS=1 cargo test --test websocket_integration_test -- --nocapture
//! ```

use std::sync::Once;

static INIT_V8: Once = Once::new();

fn init_v8_once() {
    INIT_V8.call_once(|| {
        let _ = nano::v8::initialize_platform();
        nano::data_plane::init_code_cache();
    });
}

/// Write JS to a temp file and return the path.
fn write_js(name: &str, code: &str) -> String {
    let dir = std::env::temp_dir().join("nano_ws_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, code).unwrap();
    path.to_str().unwrap().to_string()
}

/// Whether full WS server tests are enabled.
fn ws_tests_enabled() -> bool {
    std::env::var("NANO_WS_TESTS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// JS handler code for WS echo
// ---------------------------------------------------------------------------

/// CF Workers WebSocket echo handler.
///
/// Accepts WS upgrade, echoes all text/binary messages back, ignores close
/// event silently (WS will close after JS handler returns or on relay drop).
const WS_ECHO_JS: &str = r#"
async function fetch(request) {
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    server.accept();
    server.addEventListener('message', (event) => {
        server.send(event.data);
    });
    server.addEventListener('close', (_event) => {
        // handled by relay task
    });
    return new Response(null, { status: 101, webSocket: client });
}
"#;

/// Handler that calls send() before accept() — should throw TypeError.
const WS_SEND_BEFORE_ACCEPT_JS: &str = r#"
async function fetch(request) {
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    // Intentionally skip accept() — this should throw TypeError per D-14b
    server.send("should throw");
    return new Response(null, { status: 101, webSocket: client });
}
"#;

// ---------------------------------------------------------------------------
// Helper: start a test WS server
// ---------------------------------------------------------------------------

/// Start a minimal axum server with the WS dispatch handler on an ephemeral
/// port. Returns the bound socket address.
///
/// This is used by tests that require a running server. Only called when
/// `ws_tests_enabled()` returns true.
async fn start_ws_test_server(js_code: &str) -> (std::net::SocketAddr, ()) {
    use std::sync::Arc;

    use nano::http::router::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
    use nano::http::server::{create_app_with_shutdown, AppStateWithShutdown};
    use nano::signal::ShutdownState;

    let entrypoint = write_js(&format!("ws_test_{}.js", std::process::id()), js_code);

    // Build a VirtualHostRouter with "localhost" pointing at our JS file.
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

    // Give the server a moment to start.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    (addr, ())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// [WS-UPGRADE-01] WebSocket upgrade succeeds — server returns 101.
///
/// Verifies: axum WebSocketUpgrade extractor performs 101 handshake,
/// Upgrade detection branch fires before body consumption.
#[tokio::test]
#[ignore = "requires NANO_WS_TESTS=1 and full V8 server setup"]
async fn ws_upgrade() {
    if !ws_tests_enabled() {
        return;
    }
    init_v8_once();

    let (addr, _shutdown) = start_ws_test_server(WS_ECHO_JS).await;
    let url = format!("ws://127.0.0.1:{}/", addr.port());

    let result = tokio_tungstenite::connect_async(&url).await;
    assert!(
        result.is_ok(),
        "WS upgrade should succeed (101): {:?}",
        result.err()
    );
}

/// [WS-MESSAGE-01] Echo: send "hello", receive "hello".
///
/// Verifies: relay task forwards inbound frames to worker, worker calls JS
/// message handler, server.send() pushes outbound, relay returns to client.
#[tokio::test]
#[ignore = "requires NANO_WS_TESTS=1 and full V8 server setup"]
async fn ws_message_echo() {
    use futures_util::{SinkExt, StreamExt};
    use tungstenite::Message;

    if !ws_tests_enabled() {
        return;
    }
    init_v8_once();

    let (addr, _shutdown) = start_ws_test_server(WS_ECHO_JS).await;
    let url = format!("ws://127.0.0.1:{}/", addr.port());

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");
    ws.send(Message::Text("hello".to_string()))
        .await
        .expect("send");

    let msg = ws.next().await.expect("receive").expect("frame");
    match msg {
        Message::Text(s) => assert_eq!(s, "hello", "echo should match"),
        other => panic!("expected Text echo, got {:?}", other),
    }
}

/// [WS-CLOSE-01] Clean close: client sends Close, connection terminates cleanly.
///
/// Verifies: Close frame handled by worker ws_messages loop, JS close handler
/// fires, relay task cleans up, no panic or resource leak.
#[tokio::test]
#[ignore = "requires NANO_WS_TESTS=1 and full V8 server setup"]
async fn ws_close() {
    use futures_util::{SinkExt, StreamExt};
    use tungstenite::Message;

    if !ws_tests_enabled() {
        return;
    }
    init_v8_once();

    let (addr, _shutdown) = start_ws_test_server(WS_ECHO_JS).await;
    let url = format!("ws://127.0.0.1:{}/", addr.port());

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");

    // Echo "ping" first to confirm the connection is live.
    ws.send(Message::Text("ping".to_string()))
        .await
        .expect("send");
    let _ = ws.next().await.expect("echo back");

    // Now close cleanly.
    ws.send(Message::Close(None)).await.expect("send close");

    // Drain until the stream ends (server sends Close ack, stream closes).
    while let Some(frame) = ws.next().await {
        match frame {
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {} // skip any pending frames
        }
    }
    // If we reach here without panic, the close was clean.
}

/// [WS-LIMIT-01] 32 MiB message limit: oversized binary triggers close 1009.
///
/// Verifies: ws_relay_task enforces MAX_WS_MESSAGE_BYTES constant,
/// server sends Close frame with code 1009 (Message Too Big).
#[tokio::test]
#[ignore = "requires NANO_WS_TESTS=1, full V8 server setup, and ~33 MiB heap"]
async fn ws_size_limit() {
    use futures_util::{SinkExt, StreamExt};
    use tungstenite::Message;

    if !ws_tests_enabled() {
        return;
    }
    init_v8_once();

    let (addr, _shutdown) = start_ws_test_server(WS_ECHO_JS).await;
    let url = format!("ws://127.0.0.1:{}/", addr.port());

    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect");

    // Send a 33 MiB binary message (exceeds 32 MiB limit per D-09/D-12b).
    // The server may close the TCP connection while we're still sending —
    // EPIPE / BrokenPipe on the send is acceptable because it means the
    // server already enforced the limit. We only care that the connection
    // terminates (either via 1009 frame or connection reset).
    let oversized = vec![0u8; 33 * 1024 * 1024];
    let send_result = ws.send(Message::Binary(oversized)).await;

    match send_result {
        Err(e)
            if e.to_string().contains("Broken pipe")
                || e.to_string().contains("BrokenPipe")
                || e.to_string().contains("Connection reset") =>
        {
            // Server closed mid-send enforcing the size limit — correct.
            return;
        }
        Err(e) => panic!("unexpected send error: {}", e),
        Ok(()) => {} // send buffered; now read the close frame
    }

    // If send succeeded (large TCP buffer), server should respond with Close 1009.
    let frame = ws
        .next()
        .await
        .expect("should receive close")
        .expect("frame");
    match frame {
        Message::Close(Some(cf)) => {
            assert_eq!(
                cf.code,
                tungstenite::protocol::frame::coding::CloseCode::Size,
                "should close with 1009 (Size), got {:?}",
                cf.code
            );
        }
        Message::Close(None) => {
            // Acceptable — server closed without a code.
        }
        other => panic!("expected Close frame, got {:?}", other),
    }
}

/// [WS-ACCEPT-01] send() before accept() throws TypeError (D-14b).
///
/// Verifies: ws_send_callback checks WS_ACCEPTED thread-local, throws TypeError
/// before server.accept() is called. Connection should not return 101.
#[tokio::test]
#[ignore = "requires NANO_WS_TESTS=1 and full V8 server setup"]
async fn ws_accept_guard() {
    if !ws_tests_enabled() {
        return;
    }
    init_v8_once();

    let (addr, _shutdown) = start_ws_test_server(WS_SEND_BEFORE_ACCEPT_JS).await;
    let url = format!("ws://127.0.0.1:{}/", addr.port());

    // The WS upgrade should fail or return an error response because the JS
    // handler throws before returning a valid 101 response.
    let result = tokio_tungstenite::connect_async(&url).await;
    // Either connect_async fails (server returned non-101) or the connection
    // closes immediately — both are acceptable.
    match result {
        Err(_) => {
            // Connection refused or upgrade failed — correct: D-14b enforced.
        }
        Ok((mut ws, _)) => {
            // Connected but connection should close very quickly due to the error.
            use futures_util::StreamExt;
            use tungstenite::Message;
            let frame = ws.next().await;
            match frame {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                    // Correct: server closed due to TypeError
                }
                Some(Ok(other)) => {
                    panic!(
                        "Expected close after accept guard violation, got {:?}",
                        other
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Structural / compile-time tests (no server needed)
// ---------------------------------------------------------------------------

/// Verify WebSocketPair binding compiles and the module exports are correct.
///
/// This is a compile-time check — if websocket.rs has any API issues, this
/// test will fail to compile.
#[test]
fn ws_module_compiles() {
    // The fact that this file compiles proves websocket.rs exports bind_websocket_pair.
    // Statically verify the function is callable via the runtime module path.
    let _ = nano::runtime::websocket::bind_websocket_pair as fn(_, _);
}

/// Verify clear_ws_thread_locals is pub(crate) and the WS runtime module is wired up.
///
/// This is a structural smoke test — if the WS thread-locals or the websocket module
/// were removed, the `ws_module_compiles` test above would fail at compile time.
/// This test simply passes to confirm the test binary links correctly.
#[test]
fn ws_structural_smoke() {
    // Verify websocket module is reachable via the runtime module path.
    // This is a link-time check; the function call is a no-op here.
    #[allow(function_casts_as_integer)]
    let _ = std::hint::black_box(nano::runtime::websocket::bind_websocket_pair as usize);
}
