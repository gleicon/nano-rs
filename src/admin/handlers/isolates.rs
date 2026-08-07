//! Isolate diagnostics handler for Admin API
//!
//! Provides ps-style listing of active V8 isolates across all worker pools.
//! Similar to `ps` command but for NANO runtime isolates.

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::admin::diagnostics::{DiagnosticsCollector, IsolateInfo};
use crate::app::registry::AppRegistry;

/// Isolate information for API response
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IsolateResponse {
    /// Hostname this isolate serves
    pub hostname: String,
    /// Worker thread/pool ID
    pub worker_id: u32,
    /// When the isolate was created (ISO 8601)
    pub created_at: String,
    /// Uptime in human-readable format
    pub uptime: String,
    /// Number of requests processed
    pub request_count: u64,
    /// Current memory usage in bytes (if available)
    pub memory_bytes: Option<usize>,
    /// Whether the isolate is currently processing a request
    pub busy: bool,
    /// Environment variable keys (not values, for privacy)
    pub env_keys: Vec<String>,
}

/// App summary information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSummary {
    /// Hostname
    pub hostname: String,
    /// Number of active workers
    pub worker_count: u32,
    /// Total requests served
    pub total_requests: u64,
    /// Average memory per isolate in MB
    pub avg_memory_mb: f64,
    /// Uptime of the oldest isolate
    pub uptime: String,
    /// Memory limit in MB
    pub memory_limit_mb: u32,
    /// Timeout in seconds
    pub timeout_secs: u32,
}

/// Isolates list response
#[derive(Debug, Serialize, Deserialize)]
pub struct IsolatesListResponse {
    /// Total number of isolates across all apps
    pub total_isolates: usize,
    /// Total requests since startup
    pub total_requests: u64,
    /// Number of apps
    pub app_count: usize,
    /// List of all isolates
    pub isolates: Vec<IsolateResponse>,
    /// Per-app summaries
    pub apps: Vec<AppSummary>,
    /// Timestamp of the snapshot
    pub timestamp: String,
}

/// Error response for isolate endpoint failures
#[derive(Debug, Serialize)]
pub struct IsolatesError {
    error: String,
    message: String,
    code: u16,
}

impl IsolatesError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: "InternalError".to_string(),
            message: message.into(),
            code: 500,
        }
    }

    pub fn into_response(self) -> (StatusCode, Json<Self>) {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self))
    }
}

/// Format an Instant as ISO 8601 string
fn format_instant(instant: Instant) -> String {
    // Calculate the actual time by subtracting elapsed from now
    let elapsed = instant.elapsed();
    let actual_time = std::time::SystemTime::now() - elapsed;

    // Convert to RFC3339 format
    actual_time
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| {
            let secs = d.as_secs();
            let nanos = d.subsec_nanos();
            format!(
                "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                1970 + secs / 31_536_000,
                (secs % 31_536_000) / 2_592_000 + 1,
                (secs % 2_592_000) / 86_400 + 1,
                (secs % 86_400) / 3_600,
                (secs % 3_600) / 60,
                secs % 60,
                nanos / 1_000_000
            )
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Format current timestamp as ISO 8601
fn current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        1970 + secs / 31_536_000,
        (secs % 31_536_000) / 2_592_000 + 1,
        (secs % 2_592_000) / 86_400 + 1,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
        nanos / 1_000_000
    )
}

/// Convert IsolateInfo to IsolateResponse
fn convert_isolate(info: &IsolateInfo) -> IsolateResponse {
    IsolateResponse {
        hostname: info.hostname.clone(),
        worker_id: info.worker_id,
        created_at: format_instant(info.created_at),
        uptime: info.uptime(),
        request_count: info.request_count,
        memory_bytes: info.memory_bytes,
        busy: info.busy,
        env_keys: info.env_keys.clone(),
    }
}

/// List all isolates handler
///
/// Returns a ps-style listing of all active V8 isolates across all
/// worker pools. Includes per-isolate details and app-level summaries.
///
/// # Arguments
///
/// * `State(registry)` - Shared app registry for looking up configurations
///
/// # Returns
///
/// JSON response with isolate and app information, or an error response.
///
/// # Example
///
/// ```text
/// GET /admin/isolates
///
/// Response:
/// {
///   "total_isolates": 8,
///   "total_requests": 1523,
///   "app_count": 2,
///   "isolates": [
///     {
///       "hostname": "api.example.com",
///       "worker_id": 0,
///       "created_at": "2026-04-19T21:41:09.123Z",
///       "uptime": "5m 30s",
///       "request_count": 42,
///       "memory_bytes": 16777216,
///       "busy": false,
///       "env_keys": ["API_KEY", "DB_URL"]
///     }
///   ],
///   "apps": [
///     {
///       "hostname": "api.example.com",
///       "worker_count": 4,
///       "total_requests": 892,
///       "avg_memory_mb": 32.5,
///       "uptime": "5m 30s",
///       "memory_limit_mb": 128,
///       "timeout_secs": 30
///     }
///   ],
///   "timestamp": "2026-04-19T21:46:39.456Z"
/// }
/// ```
pub async fn list_isolates(
    State(registry): State<Arc<RwLock<AppRegistry>>>,
) -> Result<Json<IsolatesListResponse>, (StatusCode, Json<IsolatesError>)> {
    tracing::debug!("Listing isolates");

    // Create diagnostics collector
    let collector = DiagnosticsCollector::new(registry);

    // Collect current system state
    let diagnostics = collector.collect().await;

    // Convert to response format
    let isolates: Vec<IsolateResponse> = diagnostics.isolates.iter().map(convert_isolate).collect();

    let apps: Vec<AppSummary> = diagnostics
        .app_stats
        .iter()
        .map(|app| AppSummary {
            hostname: app.hostname.clone(),
            worker_count: app.worker_count,
            total_requests: app.total_requests,
            avg_memory_mb: app.avg_memory_mb,
            uptime: app.uptime.clone(),
            memory_limit_mb: app.config.memory_limit_mb,
            timeout_secs: app.config.timeout_secs,
        })
        .collect();

    let response = IsolatesListResponse {
        total_isolates: diagnostics.total_isolates,
        total_requests: diagnostics.total_requests,
        app_count: diagnostics.app_stats.len(),
        isolates,
        apps,
        timestamp: current_timestamp(),
    };

    tracing::debug!(
        total_isolates = response.total_isolates,
        total_requests = response.total_requests,
        app_count = response.app_count,
        "Isolates listed successfully"
    );

    Ok(Json(response))
}

/// Get isolates for a specific hostname
///
/// Returns isolate information filtered by hostname.
///
/// # Arguments
///
/// * `hostname` - The hostname to filter by
/// * `registry` - Shared app registry
///
/// # Returns
///
/// JSON response with filtered isolate information.
pub async fn get_isolates_by_hostname(
    hostname: &str,
    registry: Arc<RwLock<AppRegistry>>,
) -> Result<Json<IsolatesListResponse>, (StatusCode, Json<IsolatesError>)> {
    tracing::debug!(hostname = hostname, "Getting isolates for hostname");

    let collector = DiagnosticsCollector::new(registry);
    let diagnostics = collector.collect().await;

    // Filter isolates by hostname
    let isolates: Vec<IsolateResponse> = diagnostics
        .isolates
        .iter()
        .filter(|i| i.hostname == hostname)
        .map(convert_isolate)
        .collect();

    // Filter apps by hostname
    let apps: Vec<AppSummary> = diagnostics
        .app_stats
        .iter()
        .filter(|a| a.hostname == hostname)
        .map(|app| AppSummary {
            hostname: app.hostname.clone(),
            worker_count: app.worker_count,
            total_requests: app.total_requests,
            avg_memory_mb: app.avg_memory_mb,
            uptime: app.uptime.clone(),
            memory_limit_mb: app.config.memory_limit_mb,
            timeout_secs: app.config.timeout_secs,
        })
        .collect();

    let response = IsolatesListResponse {
        total_isolates: isolates.len(),
        total_requests: apps.iter().map(|a| a.total_requests).sum(),
        app_count: apps.len(),
        isolates,
        apps,
        timestamp: current_timestamp(),
    };

    Ok(Json(response))
}

/// Prometheus metrics endpoint handler
///
/// Returns all metrics in Prometheus text format 0.0.4.
/// Combines global metrics and per-tenant metrics.
///
/// # Example
///
/// ```text
/// GET /admin/metrics
///
/// Response:
/// # HELP nano_requests_total Total HTTP requests
/// # TYPE nano_requests_total counter
/// nano_requests_total{hostname="api.example.com",status="200"} 1423
///
/// # HELP nano_tenant_requests_total Total requests per tenant
/// nano_tenant_requests_total{hostname="api.example.com"} 1423
/// ```
pub async fn prometheus_metrics_handler() -> impl axum::response::IntoResponse {
    use crate::metrics::{PrometheusExporter, METRICS, TENANT_METRICS};
    use axum::http::header;
    use axum::response::Response;

    // Update uptime before export
    METRICS.update_uptime();

    // Export global metrics in Prometheus format
    let exporter = PrometheusExporter::new();
    let mut output = exporter.export(&METRICS);

    // Add per-tenant metrics
    output.push('\n');
    output.push_str(&TENANT_METRICS.to_prometheus());

    // Build response with correct content type
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .body(output)
        .unwrap()
}

/// Tenant metrics JSON endpoint handler
///
/// Returns per-tenant metrics in JSON format for programmatic access.
///
/// # Example
///
/// ```text
/// GET /admin/metrics/tenants
///
/// Response:
/// {
///   "tenants": [
///     {
///       "hostname": "api.example.com",
///       "requests": {
///         "total": 1423,
///         "success": 1400,
///         "error": 20,
///         "timeout": 3,
///         "active": 5
///       },
///       "cpu": {
///         "total_seconds": 45.5,
///         "avg_per_request_ms": 32.0
///       },
///       "memory": {
///         "current_bytes": 16777216,
///         "external_bytes": 2097152,
///         "peak_bytes": 33554432
///       },
///       "latency": {
///         "p50_ms": 25.0,
///         "p95_ms": 75.0,
///         "p99_ms": 150.0
///       }
///     }
///   ],
///   "summary": {
///     "total_tenants": 1,
///     "total_requests": 1423,
///     "total_cpu_seconds": 45.5
///   }
/// }
/// ```
pub async fn tenant_metrics_json() -> Json<serde_json::Value> {
    use crate::metrics::TENANT_METRICS;

    let snapshot = TENANT_METRICS.snapshot();

    let tenants: Vec<serde_json::Value> = snapshot
        .tenants
        .iter()
        .map(|t| {
            serde_json::json!({
                "hostname": t.hostname,
                "requests": {
                    "total": t.requests_total,
                    "success": t.requests_success,
                    "error": t.requests_error,
                    "timeout": t.requests_timeout,
                    "active": t.requests_active,
                },
                "cpu": {
                    "total_seconds": t.cpu_seconds_total,
                    "avg_per_request_ms": t.cpu_avg_ms,
                },
                "memory": {
                    "current_bytes": t.memory_used_bytes,
                    "external_bytes": t.memory_external_bytes,
                    "peak_bytes": t.memory_peak_bytes,
                },
                "latency": {
                    "p50_ms": t.latency_p50_ms,
                    "p95_ms": t.latency_p95_ms,
                    "p99_ms": t.latency_p99_ms,
                },
                "context_resets": t.context_resets_total,
                "isolates_active": t.isolates_active,
            })
        })
        .collect();

    Json(serde_json::json!({
        "tenants": tenants,
        "summary": {
            "total_tenants": snapshot.tenants.len(),
            "total_requests": snapshot.total_requests,
            "total_cpu_seconds": snapshot.total_cpu_seconds,
        },
        "timestamp": current_timestamp(),
    }))
}

/// Metrics summary endpoint handler
///
/// Returns high-level overview of system metrics.
///
/// # Example
///
/// ```text
/// GET /admin/metrics/summary
///
/// Response:
/// {
///   "global": {
///     "total_requests": 1523,
///     "requests_per_second": 45.2
///   },
///   "tenants": {
///     "count": 5,
///     "top_by_requests": [
///       ["api.example.com", 892],
///       ["blog.example.com", 631]
///     ],
///     "top_by_cpu": [
///       ["api.example.com", 30.5],
///       ["blog.example.com", 15.0]
///     ]
///   },
///   "system": {
///     "timestamp": "2026-04-20T12:34:56.789Z",
///     "version": "1.2.0"
///   }
/// }
/// ```
pub async fn metrics_summary() -> Json<serde_json::Value> {
    use crate::metrics::{METRICS, TENANT_METRICS};

    // Calculate RPS approximation (requests since startup / uptime)
    let uptime_secs = METRICS.uptime_seconds();
    let global_requests = METRICS
        .requests_total
        .get_all()
        .iter()
        .map(|(_, v)| *v)
        .sum::<u64>();
    let rps = if uptime_secs > 0 {
        global_requests as f64 / uptime_secs as f64
    } else {
        0.0
    };

    Json(serde_json::json!({
        "global": {
            "total_requests": global_requests,
            "requests_per_second": rps,
            "uptime_seconds": uptime_secs,
        },
        "tenants": {
            "count": TENANT_METRICS.tenant_count(),
            "hostnames": TENANT_METRICS.tenant_hostnames(),
            "top_by_requests": TENANT_METRICS.top_tenants_by_requests(10),
            "top_by_cpu": TENANT_METRICS.top_tenants_by_cpu(10),
        },
        "system": {
            "timestamp": current_timestamp(),
            "version": env!("CARGO_PKG_VERSION"),
        }
    }))
}

/// Get metrics for a specific app/hostname
///
/// Returns detailed metrics for a single tenant.
///
/// # Arguments
///
/// * `hostname` - The hostname to get metrics for
///
/// # Returns
///
/// JSON response with tenant metrics or 404 if not found.
///
/// # Example
///
/// ```text
/// GET /admin/metrics/apps/api.example.com
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "requests_total": 1423,
///   "requests_per_second": 45.2,
///   "cpu_seconds_total": 45.5,
///   "memory_bytes": 16777216,
///   "request_duration_p99": 150.0
/// }
/// ```
pub async fn app_metrics_handler(
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use crate::metrics::TENANT_METRICS;

    match TENANT_METRICS.get_tenant(&hostname) {
        Some(metrics) => {
            let m = metrics.read().unwrap();
            Ok(Json(serde_json::json!({
                "hostname": hostname,
                "requests_total": m.requests_total.get(),
                "requests_success": m.requests_success.get(),
                "requests_error": m.requests_error.get(),
                "requests_timeout": m.requests_timeout.get(),
                "requests_active": m.requests_active.get(),
                "cpu_seconds_total": m.cpu_seconds_total.get(),
                "memory_used_bytes": m.memory_used_bytes.get(),
                "memory_external_bytes": m.memory_external_bytes.get(),
                "memory_peak_bytes": m.memory_peak_bytes(),
                "context_resets_total": m.context_resets_total.get(),
                "request_duration_p99_ms": m.request_duration_p99() * 1000.0,
                "isolates_active": m.isolates_active.get(),
            })))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_timestamp_format() {
        let ts = current_timestamp();
        // Should be in ISO 8601 format with Z suffix
        assert!(ts.contains("T"));
        assert!(ts.ends_with("Z"));
        assert!(ts.len() > 20);
    }

    #[test]
    fn test_isolates_error() {
        let error = IsolatesError::new("Test error");
        assert_eq!(error.error, "InternalError");
        assert_eq!(error.message, "Test error");
        assert_eq!(error.code, 500);
    }

    #[test]
    fn test_isolate_response_serialization() {
        let isolate = IsolateResponse {
            hostname: "api.example.com".to_string(),
            worker_id: 0,
            created_at: "2026-04-19T21:41:09.123Z".to_string(),
            uptime: "5m 30s".to_string(),
            request_count: 42,
            memory_bytes: Some(16777216),
            busy: false,
            env_keys: vec!["API_KEY".to_string()],
        };

        let json = serde_json::to_string(&isolate).unwrap();
        assert!(json.contains("api.example.com"));
        assert!(json.contains("worker_id"));
        assert!(json.contains("request_count"));
    }

    #[test]
    fn test_app_summary_serialization() {
        let app = AppSummary {
            hostname: "api.example.com".to_string(),
            worker_count: 4,
            total_requests: 892,
            avg_memory_mb: 32.5,
            uptime: "5m 30s".to_string(),
            memory_limit_mb: 128,
            timeout_secs: 30,
        };

        let json = serde_json::to_string(&app).unwrap();
        assert!(json.contains("api.example.com"));
        assert!(json.contains("32.5"));
        assert!(json.contains("128"));
    }
}
