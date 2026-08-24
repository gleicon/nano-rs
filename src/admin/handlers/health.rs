//! Health check handler for Admin API
//!
//! Provides liveness and readiness probe endpoints for the Admin API.
//! These endpoints are publicly accessible (no authentication required).

use axum::{http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,
    /// Service version
    pub version: String,
    /// Service name
    pub service: String,
}

/// Readiness check response
#[derive(Debug, Serialize, Deserialize)]
pub struct ReadyResponse {
    /// Whether the service is ready to accept traffic
    pub ready: bool,
    /// Human-readable status message
    pub message: String,
}

/// Admin health check handler (liveness probe)
///
/// Returns HTTP 200 OK indicating the admin API server is running.
/// This endpoint always succeeds and is used by load balancers
/// and orchestrators to check if the process is alive.
///
/// # Returns
///
/// JSON response with status, version, and service name.
///
/// # Example
///
/// ```text
/// GET /admin/health
///
/// Response:
/// {
///   "status": "healthy",
///   "version": "0.1.0",
///   "service": "nano-admin"
/// }
/// ```
pub async fn health_handler() -> (StatusCode, Json<HealthResponse>) {
    tracing::debug!("Admin health check (liveness) received");
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            service: "nano-admin".to_string(),
        }),
    )
}

/// Health handler with shutdown state awareness
///
/// Extended version that checks the shutdown state to return
/// appropriate readiness status during graceful shutdown.
///
/// # Arguments
///
/// * `is_shutting_down` - Whether the server is currently shutting down
///
/// # Returns
///
/// JSON response with appropriate status code.
pub async fn ready_handler_with_state(is_shutting_down: bool) -> (StatusCode, Json<ReadyResponse>) {
    tracing::debug!(
        shutting_down = is_shutting_down,
        "Admin readiness check with state"
    );

    if is_shutting_down {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadyResponse {
                ready: false,
                message: "Server is shutting down".to_string(),
            }),
        )
    } else {
        (
            StatusCode::OK,
            Json(ReadyResponse {
                ready: true,
                message: "Admin API is ready".to_string(),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_handler() {
        let (status, json) = health_handler().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json.status, "healthy");
        assert_eq!(json.service, "nano-admin");
        assert!(!json.version.is_empty());
    }

    #[tokio::test]
    async fn test_ready_handler_with_state_shutting_down() {
        let (status, json) = ready_handler_with_state(true).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(!json.ready);
        assert_eq!(json.message, "Server is shutting down");
    }

    #[tokio::test]
    async fn test_ready_handler_with_state_ready() {
        let (status, json) = ready_handler_with_state(false).await;
        assert_eq!(status, StatusCode::OK);
        assert!(json.ready);
        assert_eq!(json.message, "Admin API is ready");
    }
}
