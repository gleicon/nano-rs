//! Admin API endpoint handlers
//!
//! Provides HTTP handlers for the Admin API endpoints including:
//! - Health and readiness probes
//! - Isolate diagnostics
//! - Application management
//! - Metrics (via metrics.rs in parent module)

pub mod apps;
pub mod health;
pub mod isolates;

pub use apps::{
    activate_app, create_app, delete_app, disable_app, drain_app, enable_app, get_app, list_apps,
    reload_app, scale_app, update_app, AppActionResponse, AppError, AppInfo, AppStatus,
    CreateAppRequest, CreateAppResponse, ListAppsResponse, ScaleRequest, UpdateAppRequest,
    UpdateAppResponse,
};
pub use health::{
    health_handler, ready_handler, ready_handler_with_state, HealthResponse, ReadyResponse,
};
pub use isolates::{
    app_metrics_handler, get_isolates_by_hostname, list_isolates, metrics_summary,
    prometheus_metrics_handler, tenant_metrics_json, AppSummary, IsolatesError,
    IsolatesListResponse,
};
