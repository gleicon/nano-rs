//! HTTP Request Handler for Sliver-based JS Execution
//!
//! This module bridges HTTP requests to the SliverWorkerPool for
//! WinterTC-compatible JavaScript execution from heap snapshots.

use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
};
use tokio::sync::oneshot;

use crate::http::{NanoRequest, NanoResponse};
use crate::logging::create_request_span;
use crate::worker::HandlerTask;

/// State for sliver-based request handling
#[derive(Clone)]
pub struct SliverHandlerState {
    /// Hot-swappable holder for the sliver worker pool. Each request reads the
    /// pool currently in effect, so a blue-green swap repoints traffic with no
    /// change to this state or the hostname.
    pub pool_slot: std::sync::Arc<crate::worker::SliverPoolSlot>,
    /// The JS entrypoint (e.g., "index.js" or "app.js")
    pub entrypoint: String,
}

/// Handle HTTP request by dispatching to sliver worker pool
///
/// This handler:
/// 1. Converts axum request to NanoRequest (WinterTC format)
/// 2. Creates a HandlerTask with the request
/// 3. Dispatches to SliverWorkerPool
/// 4. Waits for JS execution result
/// 5. Returns NanoResponse as HTTP response
pub async fn sliver_js_handler(
    State(state): State<SliverHandlerState>,
    request: Request<Body>,
) -> Response<Body> {
    let start = std::time::Instant::now();

    // Snapshot the pool in effect for this request. A concurrent hot-swap only
    // affects requests that read the slot after it — this one runs to completion
    // on the pool it captured here.
    let pool = state.pool_slot.current();

    // Extract request components
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();

    // Generate request ID for distributed tracing
    let request_id = format!("req_{}", uuid::Uuid::new_v4().to_string()[..8].to_string());

    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    // Create request span for distributed tracing
    let span = create_request_span(&pool.hostname, &request_id);
    let _enter = span.enter();

    tracing::debug!(
        "Sliver handler received request: {} {}",
        method,
        path_and_query
    );

    // Read body (1MB limit per D-05)
    let body_bytes = match axum::body::to_bytes(request.into_body(), 1048576).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read request body: {}", e);
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("Bad request: failed to read body"))
                .unwrap();
        }
    };

    let nano_request =
        match NanoRequest::from_axum_parts(&method, &uri, host, &headers, Some(body_bytes)) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to parse URL: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("Bad request: invalid URL"))
                    .unwrap();
            }
        };

    // Create oneshot channel for response
    let (tx, rx) = oneshot::channel();

    // Create handler task with hostname and request_id for distributed tracing
    let task = HandlerTask::with_hostname(
        state.entrypoint.clone(),
        nano_request,
        tx,
        pool.hostname.clone(),
    )
    .with_request_id(request_id.clone());

    // Dispatch to the pool currently in effect
    if let Err(e) = pool.dispatch(task) {
        tracing::error!("Failed to dispatch to worker pool: {}", e);
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body(Body::from(format!("Service unavailable: {}", e)))
            .unwrap();
    }

    // Wait for response
    let nano_response = match rx.await {
        Ok(Ok(response)) => response,
        Ok(Err(e)) => {
            tracing::error!("Handler returned error: {}", e);
            NanoResponse::with_status(500)
                .with_header("Content-Type", "text/plain")
                .with_body(format!("Handler error: {}", e))
        }
        Err(_) => {
            tracing::error!("Handler channel closed unexpectedly");
            NanoResponse::with_status(500)
                .with_header("Content-Type", "text/plain")
                .with_body("Internal error: handler channel closed")
        }
    };

    // Extract worker_id, isolate_id and status for logging
    let worker_id = nano_response.worker_id();
    let isolate_id = nano_response.isolate_id().map(|s| s.to_string());
    let status = nano_response.status();

    // Convert to axum response
    let axum_response = nano_response.to_axum_response();

    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Include worker_id and isolate_id in log when available
    match (worker_id, isolate_id) {
        (Some(wid), Some(iso)) => {
            let worker_id_u64 = wid as u64;
            tracing::info!(
                worker_id = worker_id_u64,
                isolate_id = %iso,
                "Sliver JS handler completed: {} {} - {} in {:.2}ms (worker: {}, isolate: {})",
                method,
                path_and_query,
                status,
                duration_ms,
                wid,
                iso
            );
        }
        (Some(wid), None) => {
            let worker_id_u64 = wid as u64;
            tracing::info!(
                worker_id = worker_id_u64,
                "Sliver JS handler completed: {} {} - {} in {:.2}ms (worker: {})",
                method,
                path_and_query,
                status,
                duration_ms,
                wid
            );
        }
        _ => {
            tracing::info!(
                "Sliver JS handler completed: {} {} - {} in {:.2}ms",
                method,
                path_and_query,
                status,
                duration_ms
            );
        }
    }

    axum_response
}
