//! Prometheus metrics endpoint handler
//!
//! Provides the `/_admin/metrics` HTTP endpoint that exposes collected
//! metrics in Prometheus text format for scraping by monitoring tools.
//!
//! # Endpoint
//!
//! - `GET /_admin/metrics` - Returns Prometheus-formatted metrics
//!
//! # Response Format
//!
//! Content-Type: `text/plain; version=0.0.4; charset=utf-8`
//!
//! Example output:
//! ```text
//! # HELP nano_requests_total Total HTTP requests
//! # TYPE nano_requests_total counter
//! nano_requests_total{hostname="api.example.com",status="200"} 1423
//!
//! # HELP nano_request_duration_ms Request latency in milliseconds
//! # TYPE nano_request_duration_ms histogram
//! nano_request_duration_ms_bucket{hostname="api.example.com",le="10"} 892
//! ```

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use crate::http::server::AppStateWithShutdown;
use crate::metrics::exporter::prometheus_content_type;
use crate::metrics::{MetricsRegistry, PrometheusExporter, METRICS};

/// Error response for metrics endpoint failures
/// Prometheus metrics handler
///
/// Returns all collected metrics in Prometheus text exposition format.
/// This endpoint is publicly accessible (no auth required) since metrics
/// are operational data useful for monitoring.
///
/// # Arguments
///
/// * `_state` - Application state (unused but required for handler signature)
///
/// # Returns
///
/// Response with:
/// - Status 200: Prometheus-formatted metrics
/// - Content-Type: `text/plain; version=0.0.4; charset=utf-8`
///
/// # Example
///
/// ```text
/// GET /_admin/metrics
///
/// # HELP nano_requests_total Total HTTP requests
/// # TYPE nano_requests_total counter
/// nano_requests_total{hostname="api.example.com",status="200"} 1423
/// ```
pub async fn metrics_handler(State(_state): State<Arc<AppStateWithShutdown>>) -> impl IntoResponse {
    tracing::debug!("Metrics endpoint requested");

    // Update uptime before export
    METRICS.update_uptime();

    // Export metrics in Prometheus format
    let exporter = PrometheusExporter::new();
    let output = exporter.export(&METRICS);

    // Build response with correct content type
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, prometheus_content_type())
        .body(output)
        .unwrap()
}

/// Alternative metrics handler that takes explicit registry
///
/// Used for testing or when a specific registry instance is needed
/// rather than the global singleton.
pub async fn metrics_handler_with_registry(registry: &MetricsRegistry) -> impl IntoResponse {
    let exporter = PrometheusExporter::new();
    let output = exporter.export(registry);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, prometheus_content_type())
        .body(output)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::router::AppState;
    use crate::http::server::AppStateWithShutdown;
    use crate::metrics::MetricsRegistry;
    use crate::signal::ShutdownState;

    #[tokio::test]
    async fn test_metrics_handler_returns_200() {
        // Create minimal state for testing
        use crate::app::RequestDrain;
        use crate::http::router::{HandlerType, RouteTarget, VirtualHostRouter};

        let default_target = RouteTarget {
            hostname: "default".to_string(),
            handler_type: HandlerType::StaticResponse("Not Found".to_string()),
        };
        let router = VirtualHostRouter::new(default_target);
        let app_state = AppState::new(router, 2);
        let shutdown_state = ShutdownState::new(RequestDrain::new());
        let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));

        let response = metrics_handler(State(state)).await;

        // Convert to axum response and check status
        let response: Response = response.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_metrics_content_type() {
        let registry = MetricsRegistry::new();

        let response = metrics_handler_with_registry(&registry).await;
        let response: Response = response.into_response();

        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        assert!(content_type.contains("text/plain"));
        assert!(content_type.contains("version=0.0.4"));
    }

    #[tokio::test]
    async fn test_metrics_contains_expected_data() {
        let registry = MetricsRegistry::new();

        // Record some test data
        registry.record_request("test.example.com", "200", 42.5);
        registry.record_request("test.example.com", "200", 55.0);
        registry.record_request("test.example.com", "500", 100.0);
        registry.set_memory_bytes("test.example.com", "iso-1", 16777216);

        let response = metrics_handler_with_registry(&registry).await;
        let response: Response = response.into_response();

        // Read body
        let body_bytes = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

        // Verify expected content
        assert!(body_str.contains("nano_requests_total"));
        assert!(body_str.contains("nano_request_duration_ms"));
        assert!(body_str.contains("nano_memory_bytes"));
        assert!(body_str.contains("hostname=\"test.example.com\""));
        assert!(body_str.contains("# HELP nano_requests_total"));
        assert!(body_str.contains("# TYPE nano_requests_total counter"));
    }

    #[test]
    fn test_prometheus_content_type_constant() {
        let ct = prometheus_content_type();
        assert_eq!(ct, "text/plain; version=0.0.4; charset=utf-8");
    }
}
