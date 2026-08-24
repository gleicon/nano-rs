//! Unix domain socket admin server for NANO Edge Runtime
//!
//! Provides local admin access via Unix domain socket at a configurable path
//! (default: `/var/run/nano/control.sock`). Unix socket access bypasses the
//! network stack and uses filesystem permissions for security.
//!
//! # Security
//!
//! - Socket file permissions are set to 0o660 (owner+group read/write)
//! - Access control is enforced via Unix group membership
//! - No API key required for Unix socket access (filesystem auth only)
//!
//! # Lifecycle
//!
//! - Stale socket files are cleaned up on startup
//! - Socket is removed on graceful shutdown
//! - Parent directory is created if it doesn't exist
//!
//! # Example
//!
//! ```bash
//! # Access via socat
//! $ echo '{"action":"list_apps"}' | socat - UNIX-CONNECT:/var/run/nano/control.sock
//!
//! # Check permissions
//! $ ls -la /var/run/nano/control.sock
//! srwxrwx--- 1 nano nano 0 Apr 19 14:30 control.sock
//! ```

use axum::extract::State;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;

use crate::admin::auth::AdminAuth;
use crate::admin::handlers::{
    activate_app, create_app, delete_app, disable_app, drain_app, enable_app, get_app, list_apps,
    list_isolates, reload_app, scale_app, update_app,
};
use crate::admin::server::{AdminState, AdminStateAxum};

/// Unix socket configuration
#[derive(Debug, Clone)]
pub struct UnixSocketConfig {
    /// Path to the Unix socket (default: /var/run/nano/control.sock)
    pub path: std::path::PathBuf,
    /// Socket file permissions (default: 0o660)
    pub permissions: u32,
}

impl Default for UnixSocketConfig {
    fn default() -> Self {
        Self {
            path: std::path::PathBuf::from("/var/run/nano/control.sock"),
            permissions: 0o660,
        }
    }
}

impl UnixSocketConfig {
    /// Create a new Unix socket config with the specified path
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            permissions: 0o660,
        }
    }

    /// Set custom permissions
    pub fn with_permissions(mut self, perms: u32) -> Self {
        self.permissions = perms;
        self
    }
}

/// Create and bind a Unix socket at the specified path
///
/// This function:
/// 1. Creates parent directory if it doesn't exist
/// 2. Removes stale socket file if it exists
/// 3. Binds the Unix socket
/// 4. Sets permissions (default 0o660)
///
/// # Arguments
///
/// * `path` - The path to create the Unix socket
///
/// # Returns
///
/// `Ok(UnixListener)` on success, `Err(std::io::Error)` on failure
///
/// # Example
///
/// ```rust,no_run
/// use std::path::Path;
/// use nano::admin::unix_socket::create_unix_socket;
///
/// # async fn example() {
/// let listener = create_unix_socket(Path::new("/var/run/nano/control.sock")).await.unwrap();
/// # }
/// ```
pub async fn create_unix_socket(path: &Path) -> Result<UnixListener, std::io::Error> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tracing::info!(
                "Creating Unix socket parent directory: {}",
                parent.display()
            );
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    // Remove stale socket file if it exists
    if path.exists() {
        tracing::info!("Removing stale Unix socket: {}", path.display());
        tokio::fs::remove_file(path).await?;
    }

    // Bind the Unix socket
    let listener = UnixListener::bind(path)?;
    tracing::info!("Unix socket bound to: {}", path.display());

    // Set socket permissions (owner + group read/write) on Unix only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o660);
        std::fs::set_permissions(path, perms)?;
        tracing::info!("Unix socket permissions set to 0o660 (owner+group)");
    }

    Ok(listener)
}

/// Unix socket server handle
///
/// Returned when starting the Unix socket server, can be used to
/// gracefully shut down the server and clean up the socket file.
pub struct UnixSocketServer {
    /// The shutdown signal sender
    shutdown_tx: tokio::sync::watch::Sender<()>,
    /// Path to the socket file (for cleanup on shutdown)
    socket_path: std::path::PathBuf,
}

impl UnixSocketServer {
    /// Trigger graceful shutdown and clean up socket file
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

/// Start the Unix socket admin server
///
/// Creates a Unix socket at the configured path and starts serving
/// the admin API over it. Unix socket requests bypass API key authentication
/// (filesystem permissions provide security instead).
///
/// # Arguments
///
/// * `config` - Unix socket configuration (path, permissions)
/// * `admin_state` - Shared admin state (registry, metrics, etc.)
/// * `auth` - AdminAuth for creating the router (though not used for socket auth)
///
/// # Returns
///
/// `Ok(UnixSocketServer)` on successful bind, `Err(String)` on failure.
///
/// # Example
///
/// ```rust,no_run
/// use nano::admin::unix_socket::{UnixSocketConfig, start_unix_socket_server};
/// use nano::admin::server::{AdminState, AdminConfig};
/// use nano::admin::auth::AdminAuth;
/// use nano::app::registry::AppRegistry;
/// use std::sync::Arc;
/// use tokio::sync::RwLock;
///
/// # async fn example() {
/// let config = UnixSocketConfig::default();
/// let registry = Arc::new(RwLock::new(AppRegistry::default()));
/// let http_router = Arc::new(RwLock::new(nano::http::router::VirtualHostRouter::default()));
/// let state = AdminState::new(registry, http_router);
/// let auth = Arc::new(AdminAuth::new("unused-key"));
/// let server = start_unix_socket_server(config, state, auth).await.unwrap();
/// # }
/// ```
pub async fn start_unix_socket_server(
    config: UnixSocketConfig,
    admin_state: AdminState,
    auth: Arc<AdminAuth>,
) -> Result<UnixSocketServer, String> {
    // Create and bind the Unix socket
    let listener = create_unix_socket(&config.path).await.map_err(|e| {
        format!(
            "Failed to bind Unix socket to {}: {}",
            config.path.display(),
            e
        )
    })?;

    tracing::info!(
        "Unix socket admin server listening at {}",
        config.path.display()
    );

    // Unix socket access is gated by filesystem permissions; no API key required
    let _ = auth; // auth param kept for API compatibility; not used for socket
    let router = create_unix_socket_router_no_auth(admin_state);

    // Create shutdown channel
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(());
    let socket_path = config.path.clone();

    // Spawn server task
    tokio::spawn(async move {
        let path = socket_path.clone();
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
                tracing::info!("Unix socket admin server shutting down");
            })
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Unix socket server error: {}", e);
            });

        // Clean up socket file on shutdown
        if path.exists() {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                tracing::warn!(
                    "Failed to remove Unix socket file {}: {}",
                    path.display(),
                    e
                );
            } else {
                tracing::info!("Unix socket file removed: {}", path.display());
            }
        }
    });

    Ok(UnixSocketServer {
        shutdown_tx,
        socket_path: config.path,
    })
}

/// Create admin router without authentication for Unix socket
///
/// This creates the admin router without the API key middleware,
/// since Unix socket access is controlled by filesystem permissions.
pub fn create_unix_socket_router_no_auth(state: AdminState) -> axum::Router {
    use axum::routing::{get, post};
    use tower_http::trace::TraceLayer;

    use crate::admin::handlers::health_handler;

    let state_axum = Arc::new(AdminStateAxum::new(state));

    // Create router with all routes but NO auth middleware
    let router = axum::Router::new()
        // Public routes (same as TCP server)
        .route("/admin/health", get(health_handler))
        .route("/admin/ready", get(ready_handler_unix))
        // Protected routes on TCP become public on Unix socket
        .route("/admin/isolates", get(list_isolates_handler_unix))
        .route(
            "/admin/apps",
            get(list_apps_handler_unix).post(create_app_handler_unix),
        )
        .route(
            "/admin/apps/{hostname}",
            get(get_app_handler_unix)
                .patch(update_app_handler_unix)
                .delete(delete_app_handler_unix),
        )
        .route(
            "/admin/apps/{hostname}/activate",
            post(activate_app_handler_unix),
        )
        .route(
            "/admin/apps/{hostname}/disable",
            post(disable_app_handler_unix),
        )
        .route(
            "/admin/apps/{hostname}/enable",
            post(enable_app_handler_unix),
        )
        .route(
            "/admin/apps/{hostname}/reload",
            post(reload_app_handler_unix),
        )
        .route("/admin/apps/{hostname}/scale", post(scale_app_handler_unix))
        .route("/admin/apps/{hostname}/drain", post(drain_app_handler_unix))
        .route("/admin/metrics", get(admin_metrics_handler_unix))
        .with_state(state_axum)
        .layer(TraceLayer::new_for_http());

    router
}

// Unix socket handler wrappers that extract state from Arc<AdminStateAxum>

/// Readiness probe reflecting real shutdown state (returns 503 during drain),
/// mirroring the TCP admin server. Previously routed the always-ready stub.
async fn ready_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
) -> impl axum::response::IntoResponse {
    crate::admin::handlers::ready_handler_with_state(state.inner.is_shutting_down()).await
}

async fn list_isolates_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
) -> impl axum::response::IntoResponse {
    list_isolates(axum::extract::State(state.inner.registry.clone())).await
}

async fn list_apps_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
) -> impl axum::response::IntoResponse {
    list_apps(axum::extract::State(state.inner.http_router.clone())).await
}

async fn create_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    body: axum::extract::Json<crate::admin::handlers::CreateAppRequest>,
) -> impl axum::response::IntoResponse {
    create_app(axum::extract::State(state.inner.http_router.clone()), body).await
}

async fn get_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    get_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.registry.clone()),
    )
    .await
}

async fn update_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
    body: axum::extract::Json<crate::admin::handlers::UpdateAppRequest>,
) -> impl axum::response::IntoResponse {
    update_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
        body,
    )
    .await
}

async fn delete_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    delete_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn activate_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    activate_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn disable_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    disable_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn enable_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    enable_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn reload_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    reload_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn scale_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
    body: axum::extract::Json<crate::admin::handlers::ScaleRequest>,
) -> impl axum::response::IntoResponse {
    scale_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
        body,
    )
    .await
}

async fn drain_app_handler_unix(
    State(state): State<Arc<AdminStateAxum>>,
    axum::extract::Path(hostname): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    drain_app(
        axum::extract::Path(hostname),
        axum::extract::State(state.inner.http_router.clone()),
    )
    .await
}

async fn admin_metrics_handler_unix() -> impl axum::response::IntoResponse {
    use crate::metrics::{PrometheusExporter, METRICS};
    use axum::http::header;
    use axum::response::Response;

    // Update uptime before export
    METRICS.update_uptime();

    // Export metrics in Prometheus format
    let exporter = PrometheusExporter::new();
    let output = exporter.export(&METRICS);

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn unix_ready_route_reflects_shutdown_state() {
        use crate::app::registry::AppRegistry;
        use crate::http::router::VirtualHostRouter;
        use std::sync::atomic::Ordering;
        use tokio::sync::RwLock as TokioRwLock;
        use tower::ServiceExt; // oneshot

        let registry = Arc::new(TokioRwLock::new(AppRegistry::default()));
        let http_router = Arc::new(TokioRwLock::new(VirtualHostRouter::default()));
        let state = AdminState::new(registry, http_router);
        let shutting_down = state.shutting_down.clone();
        let app = create_unix_socket_router_no_auth(state);

        let make_req = || {
            axum::http::Request::builder()
                .uri("/admin/ready")
                .body(axum::body::Body::empty())
                .unwrap()
        };

        // Before shutdown → 200.
        let resp = app.clone().oneshot(make_req()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // After shutdown → 503 (previously routed the always-ready stub and stayed 200).
        shutting_down.store(true, Ordering::SeqCst);
        let resp = app.oneshot(make_req()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_unix_socket_config_default() {
        let config = UnixSocketConfig::default();
        assert_eq!(
            config.path,
            std::path::PathBuf::from("/var/run/nano/control.sock")
        );
        assert_eq!(config.permissions, 0o660);
    }

    #[test]
    fn test_unix_socket_config_new() {
        let config = UnixSocketConfig::new("/tmp/test.sock");
        assert_eq!(config.path, std::path::PathBuf::from("/tmp/test.sock"));
        assert_eq!(config.permissions, 0o660);
    }

    #[test]
    fn test_unix_socket_config_custom_permissions() {
        let config = UnixSocketConfig::new("/tmp/test.sock").with_permissions(0o600);
        assert_eq!(config.permissions, 0o600);
    }

    #[tokio::test]
    async fn test_create_unix_socket() {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        // Create the socket
        let listener = create_unix_socket(&socket_path).await.unwrap();
        drop(listener);

        // Verify socket file exists
        assert!(socket_path.exists());

        // Verify permissions (this test is Unix-specific)
        #[cfg(all(unix, not(windows)))]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&socket_path).unwrap();
            let mode = metadata.permissions().mode();
            // Check that the file mode includes 0o660 (may have extra bits set)
            assert_eq!(mode & 0o777, 0o660);
        }
    }

    #[tokio::test]
    async fn test_create_unix_socket_removes_stale() {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = temp_dir.path().join("test.sock");

        // Create a stale file (not a real socket)
        tokio::fs::write(&socket_path, "stale data").await.unwrap();
        assert!(socket_path.exists());

        // Create the socket (should remove stale file)
        let listener = create_unix_socket(&socket_path).await.unwrap();
        drop(listener);

        // Verify socket file still exists (was replaced)
        assert!(socket_path.exists());
    }

    #[tokio::test]
    async fn test_create_unix_socket_creates_parent_dir() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir
            .path()
            .join("nested")
            .join("deep")
            .join("test.sock");

        // Directory doesn't exist yet
        assert!(!nested_path.parent().unwrap().exists());

        // Create the socket
        let listener = create_unix_socket(&nested_path).await.unwrap();
        drop(listener);

        // Verify directory was created
        assert!(nested_path.parent().unwrap().exists());
        assert!(nested_path.exists());
    }
}
