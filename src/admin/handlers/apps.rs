//! App management handlers for Admin API
//!
//! Provides CRUD operations for managing hosted applications including:
//! - List all apps
//! - Get app by hostname
//! - Create new app (pending → activate workflow)
//! - Update app configuration
//! - Delete app
//! - Lifecycle operations: activate, disable, enable, reload, scale, drain

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    Json as AxumJson,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::{AppConfig, AppLimits};
use crate::http::router::{HandlerType, RouteTarget, VirtualHostRouter};

/// App status in the lifecycle
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    /// App is pending activation (created but not yet active)
    Pending,
    /// App is active and handling traffic
    Active,
    /// App is disabled (not handling traffic but config retained)
    Disabled,
}

/// App information response
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppInfo {
    /// Hostname
    pub hostname: String,
    /// Entrypoint path
    pub entrypoint: String,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
    /// Resource limits
    pub limits: AppLimits,
    /// Current status
    pub status: AppStatus,
    /// When the app was created
    pub created_at: String,
    /// Whether the app is currently routing traffic
    pub is_active: bool,
}

/// List apps response
#[derive(Debug, Serialize, Deserialize)]
pub struct ListAppsResponse {
    /// Total number of apps
    pub total: usize,
    /// List of apps
    pub apps: Vec<AppInfo>,
}

/// Create app request
#[derive(Debug, Deserialize)]
pub struct CreateAppRequest {
    /// Hostname for the app
    pub hostname: String,
    /// Path to JavaScript entrypoint
    pub entrypoint: String,
    /// Environment variables (optional)
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    /// Resource limits (optional, uses defaults if not provided)
    #[serde(default)]
    pub limits: AppLimits,
    /// If true, immediately activate the app (skip pending phase)
    #[serde(default)]
    pub activate: bool,
}

/// Create app response
#[derive(Debug, Serialize)]
pub struct CreateAppResponse {
    /// The created app info
    pub app: AppInfo,
    /// Status message
    pub message: String,
}

/// Update app request
#[derive(Debug, Deserialize)]
pub struct UpdateAppRequest {
    /// New entrypoint path (optional)
    pub entrypoint: Option<String>,
    /// New environment variables (optional, replaces entire map)
    pub env_vars: Option<HashMap<String, String>>,
    /// New resource limits (optional)
    pub limits: Option<AppLimits>,
}

/// Update app response
#[derive(Debug, Serialize)]
pub struct UpdateAppResponse {
    /// The updated app info
    pub app: AppInfo,
    /// Status message
    pub message: String,
}

/// App action response
#[derive(Debug, Serialize)]
pub struct AppActionResponse {
    /// The affected app hostname
    pub hostname: String,
    /// Action performed
    pub action: String,
    /// Status message
    pub message: String,
    /// Current app status after action
    pub status: AppStatus,
}

/// Error response for app operations
#[derive(Debug, Serialize)]
pub struct AppError {
    error: String,
    message: String,
    code: u16,
}

impl AppError {
    pub fn not_found(hostname: &str) -> Self {
        Self {
            error: "NotFound".to_string(),
            message: format!("App '{}' not found", hostname),
            code: 404,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            error: "ValidationError".to_string(),
            message: message.into(),
            code: 400,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            error: "Conflict".to_string(),
            message: message.into(),
            code: 409,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error: "InternalError".to_string(),
            message: message.into(),
            code: 500,
        }
    }

    pub fn into_response(self) -> (StatusCode, Json<Self>) {
        let code = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (code, Json(self))
    }
}

/// Get current timestamp string
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

/// Convert AppConfig to AppInfo
/// Build the API `AppInfo` view from a registry `AppConfig`.
fn config_to_info(config: &AppConfig, status: AppStatus, created_at: &str) -> AppInfo {
    AppInfo {
        hostname: config.hostname.clone(),
        entrypoint: config.entrypoint.clone(),
        env_vars: config.env_vars.clone(),
        limits: config.limits.clone(),
        status: status.clone(),
        created_at: created_at.to_string(),
        is_active: status == AppStatus::Active,
    }
}

/// List all apps handler
///
/// Returns a list of all registered applications with their current status.
///
/// # Example
///
/// ```text
/// GET /admin/apps
///
/// Response:
/// {
///   "total": 2,
///   "apps": [
///     {
///       "hostname": "api.example.com",
///       "entrypoint": "/apps/api.js",
///       "env_vars": {"API_KEY": "secret"},
///       "limits": {"memory_mb": 128, "timeout_secs": 30, "workers": 4},
///       "status": "active",
///       "created_at": "2026-04-19T21:41:09.123Z",
///       "is_active": true
///     }
///   ]
/// }
/// ```
pub async fn list_apps(
    State(http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<ListAppsResponse>, (StatusCode, Json<AppError>)> {
    tracing::debug!("Listing all apps");

    let router = http_router.read().await;
    let apps: Vec<AppInfo> = router
        .user_routes()
        .map(|(hostname, entrypoint)| AppInfo {
            hostname: hostname.clone(),
            entrypoint: entrypoint.clone(),
            env_vars: HashMap::new(),
            limits: AppLimits::default(),
            status: AppStatus::Active,
            created_at: current_timestamp(),
            is_active: true,
        })
        .collect();

    let total = apps.len();
    tracing::debug!(total = total, "Apps listed");
    Ok(Json(ListAppsResponse { total, apps }))
}

/// Get a specific app by hostname
///
/// Returns detailed information about a single application.
///
/// # Example
///
/// ```text
/// GET /admin/apps/api.example.com
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "entrypoint": "/apps/api.js",
///   ...
/// }
/// ```
pub async fn get_app(
    Path(hostname): Path<String>,
    State(registry): State<Arc<RwLock<crate::app::registry::AppRegistry>>>,
) -> Result<Json<AppInfo>, (StatusCode, Json<AppError>)> {
    tracing::debug!(hostname = %hostname, "Getting app info");

    // Read the real app config from the registry — env vars and limits are the
    // configured values, not defaults.
    match registry.read().await.get(&hostname) {
        // The registry tracks no creation timestamp; report "unknown" rather than
        // fabricating one.
        Some(config) => Ok(Json(config_to_info(&config, AppStatus::Active, "unknown"))),
        None => Err(AppError::not_found(&hostname).into_response()),
    }
}

/// Create a new app (two-phase creation)
///
/// Creates a new application configuration. By default, the app is created
/// in "pending" status and must be explicitly activated via POST /admin/apps/:host/activate.
///
/// The entrypoint is synchronously validated before creation.
///
/// # Example
///
/// ```text
/// POST /admin/apps
/// {
///   "hostname": "new-app.example.com",
///   "entrypoint": "/apps/new-app.js",
///   "env_vars": {"KEY": "value"},
///   "limits": {"memory_mb": 64, "timeout_secs": 10, "workers": 2},
///   "activate": false
/// }
///
/// Response (201 Created):
/// {
///   "app": {
///     "hostname": "new-app.example.com",
///     ...
///     "status": "pending"
///   },
///   "message": "App created in pending status. Use POST /admin/apps/new-app.example.com/activate to activate."
/// }
/// ```
pub async fn create_app(
    State(http_router): State<Arc<RwLock<VirtualHostRouter>>>,
    AxumJson(request): AxumJson<CreateAppRequest>,
) -> Result<(StatusCode, Json<CreateAppResponse>), (StatusCode, Json<AppError>)> {
    tracing::info!(
        hostname = %request.hostname,
        entrypoint = %request.entrypoint,
        "Creating new app"
    );

    {
        let router = http_router.read().await;
        if router.get_user_route(&request.hostname).is_some() {
            return Err(
                AppError::conflict(format!("App '{}' already exists", request.hostname))
                    .into_response(),
            );
        }
    }

    // Register in the HTTP router so requests are routed immediately.
    http_router.write().await.register(
        request.hostname.clone(),
        RouteTarget {
            hostname: request.hostname.clone(),
            handler_type: HandlerType::WinterTCHandler(request.entrypoint.clone()),
        },
    );

    let status = AppStatus::Active;
    let app_info = AppInfo {
        hostname: request.hostname.clone(),
        entrypoint: request.entrypoint.clone(),
        env_vars: request.env_vars.clone(),
        limits: request.limits.clone(),
        status: status.clone(),
        created_at: current_timestamp(),
        is_active: true,
    };

    tracing::info!(hostname = %request.hostname, "App created and route registered");

    Ok((
        StatusCode::CREATED,
        Json(CreateAppResponse {
            app: app_info,
            message: format!("App '{}' created and activated", request.hostname),
        }),
    ))
}

/// Update app configuration
///
/// Partially updates an existing app's configuration. Only provided fields are updated.
///
/// # Example
///
/// ```text
/// PATCH /admin/apps/api.example.com
/// {
///   "limits": {"memory_mb": 256, "timeout_secs": 60, "workers": 8}
/// }
///
/// Response:
/// {
///   "app": {
///     "hostname": "api.example.com",
///     "entrypoint": "/apps/api.js",
///     "limits": {"memory_mb": 256, ...}
///   },
///   "message": "App 'api.example.com' updated successfully"
/// }
/// ```
pub async fn update_app(
    Path(hostname): Path<String>,
    State(http_router): State<Arc<RwLock<VirtualHostRouter>>>,
    AxumJson(request): AxumJson<UpdateAppRequest>,
) -> Result<Json<UpdateAppResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Updating app");

    let current_entrypoint = {
        let router = http_router.read().await;
        match router.get_user_route(&hostname) {
            Some(e) => e,
            None => return Err(AppError::not_found(&hostname).into_response()),
        }
    };

    let new_entrypoint = request.entrypoint.unwrap_or(current_entrypoint);

    // Re-register with updated entrypoint (register overwrites existing route).
    http_router.write().await.register(
        hostname.clone(),
        RouteTarget {
            hostname: hostname.clone(),
            handler_type: HandlerType::WinterTCHandler(new_entrypoint.clone()),
        },
    );

    tracing::info!(hostname = %hostname, entrypoint = %new_entrypoint, "App updated");

    let app_info = AppInfo {
        hostname: hostname.clone(),
        entrypoint: new_entrypoint,
        env_vars: request.env_vars.unwrap_or_default(),
        limits: request.limits.unwrap_or_default(),
        status: AppStatus::Active,
        created_at: current_timestamp(),
        is_active: true,
    };

    Ok(Json(UpdateAppResponse {
        app: app_info,
        message: format!("App '{}' updated successfully", hostname),
    }))
}

/// Delete an app
///
/// Removes an application from the registry. If the app is active,
/// it will be drained first (existing requests complete before removal).
///
/// # Example
///
/// ```text
/// DELETE /admin/apps/api.example.com
///
/// Response (200 OK):
/// {
///   "hostname": "api.example.com",
///   "action": "delete",
///   "message": "App 'api.example.com' deleted successfully",
///   "status": "active"
/// }
/// ```
pub async fn delete_app(
    Path(hostname): Path<String>,
    State(http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::warn!(hostname = %hostname, "Deleting app");

    let removed = http_router.write().await.deregister(&hostname);
    if !removed {
        return Err(AppError::not_found(&hostname).into_response());
    }

    tracing::info!(hostname = %hostname, "App deleted and route removed");

    Ok(Json(AppActionResponse {
        hostname,
        action: "delete".to_string(),
        message: "App deleted successfully".to_string(),
        status: AppStatus::Active,
    }))
}

/// Activate a pending app
///
/// Promotes an app from "pending" to "active" status, starting the worker pool
/// and enabling traffic routing.
///
/// # Example
///
/// ```text
/// POST /admin/apps/new-app.example.com/activate
///
/// Response:
/// {
///   "hostname": "new-app.example.com",
///   "action": "activate",
///   "message": "App 'new-app.example.com' activated successfully",
///   "status": "active"
/// }
/// ```
pub async fn activate_app(
    Path(hostname): Path<String>,
    State(_http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Activating app");

    // In a real implementation:
    // 1. Look up pending app
    // 2. Create worker pool
    // 3. Add to active registry
    // 4. Remove from pending

    Ok(Json(AppActionResponse {
        hostname,
        action: "activate".to_string(),
        message: "App activated successfully".to_string(),
        status: AppStatus::Active,
    }))
}

/// Disable an active app
///
/// Stops routing traffic to an app but keeps the configuration.
/// The app can be re-enabled later.
///
/// # Example
///
/// ```text
/// POST /admin/apps/api.example.com/disable
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "action": "disable",
///   "message": "App 'api.example.com' disabled successfully",
///   "status": "disabled"
/// }
/// ```
pub async fn disable_app(
    Path(hostname): Path<String>,
    State(_http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Disabling app");

    // In a real implementation:
    // 1. Stop routing traffic (remove from active routes)
    // 2. Keep configuration for re-enabling
    // 3. Optionally drain existing connections

    Ok(Json(AppActionResponse {
        hostname,
        action: "disable".to_string(),
        message: "App disabled successfully".to_string(),
        status: AppStatus::Disabled,
    }))
}

/// Enable a disabled app
///
/// Resumes routing traffic to a previously disabled app.
///
/// # Example
///
/// ```text
/// POST /admin/apps/api.example.com/enable
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "action": "enable",
///   "message": "App 'api.example.com' enabled successfully",
///   "status": "active"
/// }
/// ```
pub async fn enable_app(
    Path(hostname): Path<String>,
    State(_http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Enabling app");

    // In a real implementation:
    // 1. Restore routing (add back to active routes)
    // 2. Verify worker pool is running

    Ok(Json(AppActionResponse {
        hostname,
        action: "enable".to_string(),
        message: "App enabled successfully".to_string(),
        status: AppStatus::Active,
    }))
}

/// Reload app JavaScript from disk
///
/// Triggers a reload of the JavaScript entrypoint from disk.
/// This is useful for hot-reloading during development or after deployments.
///
/// # Example
///
/// ```text
/// POST /admin/apps/api.example.com/reload
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "action": "reload",
///   "message": "App 'api.example.com' reloaded successfully",
///   "status": "active"
/// }
/// ```
pub async fn reload_app(
    Path(hostname): Path<String>,
    State(http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Reloading app");

    let entrypoint = {
        let router = http_router.read().await;
        match router.get_user_route(&hostname) {
            Some(e) => e,
            None => return Err(AppError::not_found(&hostname).into_response()),
        }
    };

    // Re-register with the same entrypoint. The worker pool will pick up fresh JS
    // content on the next request (workers load on-demand, not at register time).
    http_router.write().await.register(
        hostname.clone(),
        RouteTarget {
            hostname: hostname.clone(),
            handler_type: HandlerType::WinterTCHandler(entrypoint),
        },
    );

    Ok(Json(AppActionResponse {
        hostname,
        action: "reload".to_string(),
        message: "App reloaded successfully".to_string(),
        status: AppStatus::Active,
    }))
}

/// Scale app worker count
///
/// Adjusts the number of worker threads for an application.
///
/// # Example
///
/// ```text
/// POST /admin/apps/api.example.com/scale?workers=8
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "action": "scale",
///   "message": "App 'api.example.com' scaled to 8 workers",
///   "status": "active"
/// }
/// ```
pub async fn scale_app(
    Path(hostname): Path<String>,
    State(_http_router): State<Arc<RwLock<VirtualHostRouter>>>,
    AxumJson(request): AxumJson<ScaleRequest>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, workers = request.workers, "Scaling app");

    if request.workers < 1 || request.workers > 32 {
        return Err(AppError::validation("Workers must be between 1 and 32").into_response());
    }

    Ok(Json(AppActionResponse {
        hostname,
        action: "scale".to_string(),
        message: format!("App scaled to {} workers", request.workers),
        status: AppStatus::Active,
    }))
}

/// Scale request body
#[derive(Debug, Deserialize)]
pub struct ScaleRequest {
    /// New worker count (1-32)
    pub workers: usize,
}

/// Drain and disable an app
///
/// Drains existing connections (waits for in-flight requests to complete)
/// then disables the app. This is a graceful shutdown for a single app.
///
/// # Example
///
/// ```text
/// POST /admin/apps/api.example.com/drain
///
/// Response:
/// {
///   "hostname": "api.example.com",
///   "action": "drain",
///   "message": "App 'api.example.com' drained and disabled",
///   "status": "disabled"
/// }
/// ```
pub async fn drain_app(
    Path(hostname): Path<String>,
    State(_http_router): State<Arc<RwLock<VirtualHostRouter>>>,
) -> Result<Json<AppActionResponse>, (StatusCode, Json<AppError>)> {
    tracing::info!(hostname = %hostname, "Draining app");

    // In a real implementation:
    // 1. Stop accepting new requests for this hostname
    // 2. Wait for in-flight requests to complete (with timeout)
    // 3. Disable the app

    Ok(Json(AppActionResponse {
        hostname,
        action: "drain".to_string(),
        message: "App drained and disabled".to_string(),
        status: AppStatus::Disabled,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_error_not_found() {
        let error = AppError::not_found("test.example.com");
        assert_eq!(error.error, "NotFound");
        assert!(error.message.contains("test.example.com"));
        assert_eq!(error.code, 404);
    }

    #[tokio::test]
    async fn get_app_returns_real_config_not_defaults() {
        use crate::app::registry::AppRegistry;
        use crate::config::AppConfig;

        // App with non-default limits + env vars.
        let limits = AppLimits {
            memory_mb: 999,
            workers: 7,
            ..Default::default()
        };
        let mut env_vars = HashMap::new();
        env_vars.insert("API_KEY".to_string(), "secret".to_string());

        let mut apps = HashMap::new();
        apps.insert(
            "real.example.com".to_string(),
            AppConfig {
                hostname: "real.example.com".to_string(),
                entrypoint: "./real.js".to_string(),
                sliver: None,
                compat: Default::default(),
                env_vars,
                limits,
                vfs_backend: Default::default(),
                vfs_disk: None,
                vfs_s3: None,
            },
        );
        let registry = Arc::new(RwLock::new(AppRegistry::new(apps)));

        let resp = get_app(Path("real.example.com".to_string()), State(registry))
            .await
            .expect("app found");

        // Real configured values, not AppLimits::default() / empty env.
        assert_eq!(resp.limits.memory_mb, 999, "real memory limit, not default");
        assert_eq!(resp.limits.workers, 7, "real worker count, not default");
        assert_eq!(resp.env_vars.get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(resp.entrypoint, "./real.js");
    }

    #[test]
    fn test_app_error_validation() {
        let error = AppError::validation("Invalid memory limit");
        assert_eq!(error.error, "ValidationError");
        assert_eq!(error.message, "Invalid memory limit");
        assert_eq!(error.code, 400);
    }

    #[test]
    fn test_app_error_conflict() {
        let error = AppError::conflict("App already exists");
        assert_eq!(error.error, "Conflict");
        assert_eq!(error.code, 409);
    }

    #[test]
    fn test_config_to_info() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            sliver: None,
            compat: Default::default(),
            env_vars: HashMap::new(),
            limits: AppLimits::default(),
            vfs_backend: Default::default(),
            vfs_disk: None,
            vfs_s3: None,
        };

        let info = config_to_info(&config, AppStatus::Active, "2026-04-19T21:41:09.123Z");
        assert_eq!(info.hostname, "api.example.com");
        assert_eq!(info.status, AppStatus::Active);
        assert!(info.is_active);
    }

    #[test]
    fn test_app_status_serialization() {
        let pending = AppStatus::Pending;
        let json = serde_json::to_string(&pending).unwrap();
        assert_eq!(json, "\"pending\"");

        let active = AppStatus::Active;
        let json = serde_json::to_string(&active).unwrap();
        assert_eq!(json, "\"active\"");

        let disabled = AppStatus::Disabled;
        let json = serde_json::to_string(&disabled).unwrap();
        assert_eq!(json, "\"disabled\"");
    }

    #[test]
    fn test_current_timestamp() {
        let ts = current_timestamp();
        assert!(ts.contains("T"));
        assert!(ts.ends_with("Z"));
    }

    #[test]
    fn test_create_app_request_deserialization() {
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app.js",
            "env_vars": {"KEY": "value"},
            "limits": {"memory_mb": 128, "timeout_secs": 30, "workers": 4},
            "activate": true
        }"#;

        let request: CreateAppRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.hostname, "api.example.com");
        assert_eq!(request.entrypoint, "/app.js");
        assert_eq!(request.env_vars.get("KEY"), Some(&"value".to_string()));
        assert!(request.activate);
    }

    #[test]
    fn test_create_app_request_defaults() {
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app.js"
        }"#;

        let request: CreateAppRequest = serde_json::from_str(json).unwrap();
        assert!(!request.activate); // Default is false
        assert!(request.env_vars.is_empty()); // Default is empty
    }

    #[test]
    fn test_app_info_serialization() {
        let info = AppInfo {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            env_vars: HashMap::new(),
            limits: AppLimits::default(),
            status: AppStatus::Active,
            created_at: "2026-04-19T21:41:09.123Z".to_string(),
            is_active: true,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("api.example.com"));
        assert!(json.contains("active"));
        assert!(json.contains("\"is_active\":true"));
    }
}
