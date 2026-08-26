//! Regression test for the graceful-shutdown drain wiring.
//!
//! `RequestDrain` used to be created and awaited at shutdown, but nothing on the
//! request path incremented it — so `await_complete` returned instantly and the
//! app-level drain was a no-op. These tests pin the fix: a request flowing through
//! `dispatch_to_worker_pool` is counted in-flight on the *same* drain that
//! `ShutdownState` waits on, and the count returns to zero when the request ends.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use futures::channel::mpsc;
use tower::ServiceExt;

use nano::app::drain::RequestDrain;
use nano::http::router::{AppState, HandlerType, RouteTarget, VirtualHostRouter};
use nano::http::server::{create_app_with_shutdown, AppStateWithShutdown};
use nano::signal::ShutdownState;

fn app_with_shared_drain(drain: RequestDrain) -> axum::Router {
    let router = VirtualHostRouter::new(RouteTarget {
        hostname: "drain-test.local".to_string(),
        handler_type: HandlerType::WinterTCHandler("/index.js".to_string()),
    });
    let app_state = AppState::new(router, 1);
    let shutdown_state = ShutdownState::new(drain);
    create_app_with_shutdown(Arc::new(AppStateWithShutdown::new(app_state, shutdown_state)))
}

/// A request in flight through the real dispatch path is counted on the shutdown
/// drain, and the count returns to zero when it completes.
///
/// The request body is a stream that stays pending until we close its sender, so
/// `dispatch_to_worker_pool` blocks reading the body — after it has created the
/// per-request `DrainHandle` but before it does anything else. That gives a
/// deterministic window in which the in-flight count must read 1.
#[tokio::test]
async fn in_flight_request_is_counted_on_the_shutdown_drain() {
    let drain = RequestDrain::new();
    let app = app_with_shared_drain(drain.clone());

    // Body backed by a channel we control; no item is sent, so reading it blocks.
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let body = Body::from_stream(rx);
    let req = Request::builder()
        .uri("/")
        .header("host", "drain-test.local")
        .body(body)
        .unwrap();

    assert_eq!(drain.active_count(), 0, "no requests in flight yet");

    let task = tokio::spawn(app.oneshot(req));

    // Wait for dispatch to create the DrainHandle (it is now blocked on the body).
    let mut waited = 0;
    while drain.active_count() == 0 && waited < 300 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        waited += 1;
    }
    assert_eq!(
        drain.active_count(),
        1,
        "an in-flight request must be counted on the shutdown drain"
    );

    // Close the body stream → the request finishes.
    drop(tx);
    let _ = task.await.expect("request task");

    assert_eq!(
        drain.active_count(),
        0,
        "the drain count returns to zero once the request completes"
    );
}

/// End-to-end consequence: while a request is in flight, `await_complete` (what
/// `ShutdownState::shutdown` calls) does NOT report drained; it does once the
/// request finishes. This is the behavior the wiring restores.
#[tokio::test]
async fn shutdown_waits_for_an_in_flight_request() {
    let drain = RequestDrain::new();
    let app = app_with_shared_drain(drain.clone());

    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, std::io::Error>>(1);
    let req = Request::builder()
        .uri("/")
        .header("host", "drain-test.local")
        .body(Body::from_stream(rx))
        .unwrap();
    let task = tokio::spawn(app.oneshot(req));

    let mut waited = 0;
    while drain.active_count() == 0 && waited < 300 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        waited += 1;
    }
    assert_eq!(drain.active_count(), 1, "request in flight");

    // Draining must NOT complete while the request is in flight.
    let drained = drain.await_complete(Duration::from_millis(100)).await;
    assert!(!drained, "await_complete must not report drained while a request runs");

    // Finish the request, then draining completes promptly.
    drop(tx);
    let _ = task.await.expect("request task");
    let drained = drain.await_complete(Duration::from_secs(1)).await;
    assert!(drained, "await_complete reports drained once the request finishes");
}
