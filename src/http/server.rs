//! HTTP server implementation
//!
//! Provides the HTTP server for NANO runtime using axum. Includes
//! configurable middleware stack (tracing, timeout, compression),
//! health endpoints, and virtual host routing. Supports graceful
//! shutdown with request draining.

use anyhow::{Context, Result};
use axum::{
    extract::State as AxumState,
    http::StatusCode,
    response::Json,
    routing::{any, get},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, timeout::TimeoutLayer, trace::TraceLayer};

use crate::admin::metrics::metrics_handler;
use crate::app::registry::AppRegistry;
use crate::config::NanoConfig;
use crate::http::config::ServerConfig;
use crate::http::content_type_from_ext;
use crate::http::router::{
    dispatch_to_worker_pool, AppState, HandlerType, RouteTarget, VirtualHostRouter,
};
use crate::signal::ShutdownState;
use crate::vfs::{loader::load_directory_to_vfs, IsolateVfs, MemoryBackend, VfsNamespace};

/// Create a TCP listener with SO_REUSEADDR enabled for quick port reuse
///
/// This allows the server to be restarted immediately after shutdown,
/// preventing "Address already in use" errors during development and testing.
async fn create_reuse_listener(addr: &std::net::SocketAddr) -> Result<TcpListener> {
    let socket = tokio::net::TcpSocket::new_v4().context("Failed to create TCP socket")?;

    // Enable SO_REUSEADDR to allow immediate port reuse after shutdown
    socket
        .set_reuseaddr(true)
        .context("Failed to set SO_REUSEADDR on socket")?;

    // Also enable SO_REUSEPORT on Unix systems for better load balancing
    #[cfg(unix)]
    socket
        .set_reuseport(true)
        .context("Failed to set SO_REUSEPORT on socket")?;

    if addr.ip().is_unspecified() {
        tracing::warn!(
            "Binding to {} — all network interfaces exposed. \
             Set server.host to a specific IP or add firewall rules in production.",
            addr
        );
    }

    socket
        .bind(*addr)
        .with_context(|| format!("Failed to bind to {}", addr))?;

    let listener = socket.listen(128).context("Failed to listen on socket")?;

    Ok(listener)
}

/// Health check response
#[derive(Debug, Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    version: String,
}

/// Readiness check response
#[derive(Debug, Serialize, Deserialize)]
struct ReadyResponse {
    ready: bool,
    message: String,
}

/// Basic health check handler (liveness probe)
///
/// Returns HTTP 200 OK for load balancer and orchestrator health checks.
/// This endpoint always succeeds and indicates the server process is running.
async fn health_handler() -> (StatusCode, Json<HealthResponse>) {
    tracing::debug!("Health check (liveness) received");
    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }),
    )
}

/// Admin health check handler
///
/// Same as basic health but under `/_admin` path for consistency
/// with other admin endpoints.
async fn admin_health_handler() -> (StatusCode, Json<HealthResponse>) {
    tracing::debug!("Admin health check received");
    health_handler().await
}

/// Readiness check handler (readiness probe)
///
/// Returns HTTP 200 if the server is ready to accept traffic,
/// or HTTP 503 if the server is shutting down or not initialized.
/// Used by load balancers to stop sending traffic before shutdown.
async fn ready_handler(
    AxumState(state): AxumState<Arc<AppStateWithShutdown>>,
) -> (StatusCode, Json<ReadyResponse>) {
    let shutting_down = state.shutdown_state.is_shutting_down();
    let active_count = state.shutdown_state.active_requests();

    tracing::debug!(
        shutting_down = shutting_down,
        active_requests = active_count,
        "Readiness check received"
    );

    if shutting_down {
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
                message: "Server is ready".to_string(),
            }),
        )
    }
}

/// Application state with shutdown tracking
///
/// Wraps the existing AppState and adds shutdown state for coordination
/// between graceful shutdown and readiness checks.
#[derive(Debug)]
pub struct AppStateWithShutdown {
    /// Original application state with router and work queue
    pub app_state: AppState,
    /// Shutdown state for readiness checks
    pub shutdown_state: ShutdownState,
}

impl AppStateWithShutdown {
    /// Create new state with app state and shutdown state
    pub fn new(app_state: AppState, shutdown_state: ShutdownState) -> Self {
        Self {
            app_state,
            shutdown_state,
        }
    }
}

/// Legacy application state shared across all request handlers
///
/// This is a simpler version without shutdown tracking, kept for
/// backward compatibility with existing tests and simple use cases.
#[derive(Debug)]
pub struct State {
    // Future fields:
    // - worker_pool: Arc<WorkerPool>
    // - virtual_hosts: Arc<RwLock<HashMap<String, App>>>
    // - metrics: Arc<Metrics>
}

impl State {
    /// Creates a new empty application state
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Create the axum application router with virtual host routing
///
/// Builds the router with:
/// 1. Health endpoint at /health (for load balancer checks)
/// 2. Admin endpoints at /_admin/* (health, ready)
/// 3. Virtual host routing for all other paths
/// 4. Middleware stack per D-01: Tracing → Timeout → Compression
///
/// # Arguments
///
/// * `state` - Shared application state with shutdown tracking
///
/// # Returns
/// A configured `Router` ready to be passed to `axum::serve()`.
pub fn create_app_with_shutdown(state: Arc<AppStateWithShutdown>) -> Router {
    // Create a clone for the virtual host handler
    let app_state_clone = Arc::new(state.app_state.clone());

    // Build axum router with middleware
    Router::new()
        // Basic health endpoint (backward compatible)
        .route("/health", get(health_handler))
        // Admin endpoints
        .route("/_admin/health", get(admin_health_handler))
        .route("/_admin/ready", get(ready_handler))
        .route("/_admin/metrics", get(metrics_handler))
        // Root path - dispatch to worker pool for JS execution
        .route(
            "/",
            any({
                let state = app_state_clone.clone();
                move |req| dispatch_to_worker_pool(AxumState(state), req)
            }),
        )
        // Catch-all for virtual hosts - dispatch to worker pool
        .route(
            "/{*path}",
            any({
                let state = app_state_clone;
                move |req| dispatch_to_worker_pool(AxumState(state), req)
            }),
        )
        // Middleware stack (applied in reverse order)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(CompressionLayer::new())
        .with_state(state)
}

/// Create the default axum application router (backward compatible)
///
/// Creates an app with default state and no shutdown tracking.
/// For graceful shutdown support, use `create_app_with_shutdown()`.
pub fn create_app() -> Router {
    // Create default handler (per D-04)
    let default_target = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("NANO Runtime".to_string()),
    };

    // Create router with example routes for testing
    let mut router = VirtualHostRouter::new(default_target);

    // Register example routes
    router.register(
        "api.example.com".to_string(),
        RouteTarget {
            hostname: "api.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("API Handler".to_string()),
        },
    );

    router.register(
        "blog.example.com".to_string(),
        RouteTarget {
            hostname: "blog.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("Blog Handler".to_string()),
        },
    );

    tracing::info!(
        "Virtual host router initialized with {} routes",
        router.route_count()
    );

    // Create state with the router and WorkQueue
    let app_state = AppState::new(router, 4); // 4 workers per hostname pool
    let shutdown_state = ShutdownState::default();
    let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));

    create_app_with_shutdown(state)
}

/// Single source of truth for the bind → serve loop shared by every
/// `start_server_*` entry point: resolve the address, bind a reuse listener,
/// log, and serve `app` to completion. The `start_server_*` functions differ
/// only in how they build `app`; the orchestration lives here.
async fn serve_app(config: &ServerConfig, app: Router) -> Result<()> {
    let addr = config
        .socket_addr()
        .context("Failed to parse server address")?;
    let listener = create_reuse_listener(&addr).await?;
    tracing::info!("HTTP server listening on {}", addr);
    axum::serve(listener, app).await.context("Server error")?;
    tracing::info!("HTTP server shut down gracefully");
    Ok(())
}

/// Like [`serve_app`], but serves with graceful shutdown driven by `shutdown_signal`.
async fn serve_app_with_shutdown<F>(
    config: &ServerConfig,
    app: Router,
    shutdown_signal: F,
) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let addr = config
        .socket_addr()
        .context("Failed to parse server address")?;
    let listener = create_reuse_listener(&addr).await?;
    tracing::info!("HTTP server listening on {}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .context("Server error")?;
    tracing::info!("HTTP server shut down gracefully");
    Ok(())
}

/// Start the HTTP server
///
/// Binds to the configured address and starts serving requests.
/// This function runs until the server is shut down (e.g., via SIGTERM).
///
/// # Arguments
///
/// * `config` - Server configuration including port and host
///
/// # Errors
///
/// Returns an error if:
/// - The TCP listener cannot be bound to the configured address
/// - The server encounters an error while running
///
/// # Examples
///
/// ```rust,no_run
/// use nano::http::{start_server, ServerConfig};
///
/// # async fn example() -> anyhow::Result<()> {
/// let config = ServerConfig::default();
/// start_server(config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn start_server(config: ServerConfig) -> Result<()> {
    serve_app(&config, create_app()).await
}

/// Start the HTTP server with graceful shutdown support
///
/// Binds to the configured address and starts serving requests.
/// The server will gracefully shut down when the provided signal resolves.
///
/// # Arguments
///
/// * `config` - Server configuration including port and host
/// * `shutdown_signal` - Future that triggers graceful shutdown when resolved
///
/// # Errors
///
/// Returns an error if:
/// - The TCP listener cannot be bound to the configured address
/// - The server encounters an error while running
///
/// # Examples
///
/// ```rust,no_run
/// use nano::http::{start_server_with_shutdown, ServerConfig};
/// use nano::signal::GracefulShutdown;
/// use std::time::Duration;
///
/// # async fn example() -> anyhow::Result<()> {
/// let config = ServerConfig::default();
/// let (tx, mut rx) = tokio::sync::broadcast::channel::<()>(1);
///
/// start_server_with_shutdown(config, async move {
///     let _ = rx.recv().await;
/// }).await?;
/// # Ok(())
/// # }
/// ```
pub async fn start_server_with_shutdown<F>(config: ServerConfig, shutdown_signal: F) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    serve_app_with_shutdown(&config, create_app(), shutdown_signal).await
}

/// Start the HTTP server with graceful shutdown and full state integration
///
/// This is the full-featured version that integrates with the graceful
/// shutdown system and tracks in-flight requests for the readiness probe.
///
/// # Arguments
///
/// * `config` - Server configuration including port and host
/// * `shutdown_state` - Shutdown state for readiness checks and drain tracking
///
/// # Returns
///
/// Returns a `Result` indicating success or failure. The server runs
/// until a shutdown signal is received.
///
/// # Examples
///
/// ```rust,no_run
/// use nano::http::{start_server_with_state, ServerConfig};
/// use nano::signal::ShutdownState;
///
/// # async fn example() -> anyhow::Result<()> {
/// let config = ServerConfig::default();
/// let shutdown_state = ShutdownState::default();
///
/// start_server_with_state(config, shutdown_state).await?;
/// # Ok(())
/// # }
/// ```
pub async fn start_server_with_state(
    config: ServerConfig,
    shutdown_state: ShutdownState,
) -> Result<()> {
    // Create default handler
    let default_target = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("NANO Runtime".to_string()),
    };

    // Create router with example routes
    let mut router = VirtualHostRouter::new(default_target);

    router.register(
        "api.example.com".to_string(),
        RouteTarget {
            hostname: "api.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("API Handler".to_string()),
        },
    );

    router.register(
        "blog.example.com".to_string(),
        RouteTarget {
            hostname: "blog.example.com".to_string(),
            handler_type: HandlerType::StaticResponse("Blog Handler".to_string()),
        },
    );

    tracing::info!(
        "Virtual host router initialized with {} routes",
        router.route_count()
    );

    // Create state with the router and shutdown tracking
    let app_state = AppState::new(router, 4);
    let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));

    serve_app(&config, create_app_with_shutdown(state)).await
}

/// Start the HTTP server with a custom router
///
/// This version allows passing a pre-configured VirtualHostRouter,
/// which is useful for sliver-based serving where routes are
/// determined by the sliver's VFS contents.
///
/// # Arguments
///
/// * `router` - The pre-configured virtual host router
/// * `config` - Server configuration including port and host
/// * `shutdown_state` - Shutdown state for graceful shutdown
///
/// # Returns
///
/// Returns a `Result` indicating success or failure.
/// Start the HTTP server with a shared `Arc<RwLock<VirtualHostRouter>>`.
///
/// Used by `run_server()` so the admin API and HTTP server share the same
/// live routing table — admin API writes are visible to in-flight dispatches.
pub async fn start_server_with_shared_router(
    router: std::sync::Arc<tokio::sync::RwLock<VirtualHostRouter>>,
    config: ServerConfig,
    shutdown_state: ShutdownState,
) -> Result<()> {
    let app_state = AppState::new_shared(router, 4);
    let state = std::sync::Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));
    serve_app(&config, create_app_with_shutdown(state)).await
}

pub async fn start_server_with_router(
    router: VirtualHostRouter,
    config: ServerConfig,
    shutdown_state: ShutdownState,
) -> Result<()> {
    // Create app state with the provided router
    let app_state = AppState::new(router, 4);
    let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));
    serve_app(&config, create_app_with_shutdown(state)).await
}

/// Start the HTTP server for sliver-based JavaScript execution
///
/// This server routes ALL requests to the SliverWorkerPool for JS execution,
/// enabling full WinterTC-compatible request handling from heap snapshots.
///
/// # Arguments
///
/// * `worker_pool` - The SliverWorkerPool containing snapshot-restored isolates
/// * `entrypoint` - The JS entrypoint file (e.g., "index.js") to execute
/// * `config` - Server configuration including port and host
/// * `shutdown_signal` - Future that triggers graceful shutdown when resolved
///
/// # Returns
///
/// Returns a `Result` indicating success or failure.
pub async fn start_server_with_sliver_pool<F>(
    pool_slot: Arc<crate::worker::SliverPoolSlot>,
    entrypoint: String,
    config: ServerConfig,
    shutdown_signal: F,
) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    use crate::http::sliver_handler::{sliver_js_handler, SliverHandlerState};
    use axum::routing::{any, get};

    tracing::info!(
        "Starting sliver JS execution server (entrypoint: {})",
        entrypoint
    );

    // Create sliver handler state
    let handler_state = SliverHandlerState {
        pool_slot,
        entrypoint,
    };

    // Build router with sliver JS execution
    let app = Router::new()
        // Health check endpoints
        .route("/health", get(health_handler))
        .route("/_admin/health", get(admin_health_handler))
        // ALL requests go to JS execution (WinterTC style)
        .route(
            "/",
            any({
                let state = handler_state.clone();
                move |req| sliver_js_handler(axum::extract::State(state.clone()), req)
            }),
        )
        .route(
            "/{*path}",
            any({
                let state = handler_state;
                move |req| sliver_js_handler(axum::extract::State(state), req)
            }),
        )
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(CompressionLayer::new());

    serve_app_with_shutdown(&config, app, shutdown_signal).await
}

/// Start HTTP server with configuration from NanoConfig
///
/// This function creates an AppRegistry from the config, builds a VirtualHostRouter
/// for all configured apps, and starts the HTTP server with the configured port and host.
///
/// # Arguments
///
/// * `nano_config` - The loaded NanoConfig with apps and server settings
/// * `shutdown_state` - Shutdown state for graceful shutdown coordination
///
/// # Returns
///
/// Returns a `Result` indicating success or failure.
///
/// # Config Mode Features
///
/// - Supports sliver-based apps (snapshot-restored isolates)
/// - Virtual host routing based on hostname in config
/// - Per-app limits configured via the limits section
/// - Server bind address from config (not hardcoded)
pub async fn start_server_with_config(
    nano_config: NanoConfig,
    shutdown_state: ShutdownState,
) -> Result<()> {
    // Create AppRegistry from config
    let registry = AppRegistry::from_config(nano_config.clone());
    tracing::info!("Created AppRegistry with {} app(s)", registry.count());

    // Build VirtualHostRouter from config apps
    // Empty default response triggers 404 for unknown hosts
    let default_target = RouteTarget {
        hostname: "default".to_string(),
        handler_type: HandlerType::StaticResponse("".to_string()),
    };
    let mut router = VirtualHostRouter::new(default_target);

    // Register routes for each app in config
    for app in &nano_config.apps {
        let target = if let Some(ref _sliver_path) = app.sliver {
            // Sliver-based app (entrypoint type detection for sliver entrypoint)
            use crate::http::router::detect_entrypoint_type;
            let entrypoint_type = detect_entrypoint_type(&app.entrypoint);

            match entrypoint_type {
                crate::http::router::EntrypointType::JavaScript(_) => {
                    // JavaScript entrypoint - use sliver handler
                    RouteTarget {
                        hostname: app.hostname.clone(),
                        handler_type: HandlerType::WinterTCSliverHandler {
                            hostname: app.hostname.clone(),
                            entrypoint: app.entrypoint.clone(),
                        },
                    }
                }
                crate::http::router::EntrypointType::StaticFile(path) => {
                    // Static file entrypoint - serve directly
                    let ext = Path::new(&path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let content_type = content_type_from_ext(ext).to_string();
                    RouteTarget {
                        hostname: app.hostname.clone(),
                        handler_type: HandlerType::StaticFile { path, content_type },
                    }
                }
                crate::http::router::EntrypointType::StaticDir(root) => {
                    // Directory entrypoint - load into VFS for better performance
                    // This loads all files into memory at startup for fast serving
                    let vfs = IsolateVfs::new(
                        VfsNamespace::from_hostname(&app.hostname),
                        crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
                    );

                    // Load directory contents into VFS
                    match load_directory_to_vfs(&vfs, &root, "/").await {
                        Ok(count) => {
                            tracing::info!(
                                "Loaded {} files from '{}' into VFS for {}",
                                count,
                                root,
                                app.hostname
                            );

                            // Build files HashMap from VFS backend
                            let mut files = std::collections::HashMap::new();
                            let backend = vfs.backend();

                            // Get all entries from the VFS backend
                            // Note: We need to get the entries from the MemoryBackend
                            if let Some(mem_backend) =
                                backend.as_any().downcast_ref::<MemoryBackend>()
                            {
                                for (path, file) in mem_backend.snapshot_entries() {
                                    // Determine content type from path
                                    let ext = std::path::Path::new(path.as_str())
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("");
                                    let content_type = content_type_from_ext(ext).to_string();

                                    files.insert(
                                        path.as_str().to_string(),
                                        (file.content, content_type),
                                    );
                                }
                            }

                            RouteTarget {
                                hostname: app.hostname.clone(),
                                handler_type: HandlerType::VfsStaticFiles {
                                    files,
                                    default_file: Some("index.html".to_string()),
                                },
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to load directory '{}' into VFS for {}: {}",
                                root,
                                app.hostname,
                                e
                            );
                            // Fallback to filesystem-based StaticDir handler
                            RouteTarget {
                                hostname: app.hostname.clone(),
                                handler_type: HandlerType::StaticDir {
                                    root,
                                    default_file: "index.html".to_string(),
                                },
                            }
                        }
                    }
                }
            }
        } else {
            // Entrypoint-based app (non-sliver) - use entrypoint type detection
            use crate::http::router::detect_entrypoint_type;
            let entrypoint_type = detect_entrypoint_type(&app.entrypoint);

            match entrypoint_type {
                crate::http::router::EntrypointType::JavaScript(path) => {
                    // JavaScript entrypoint - execute as Worker
                    RouteTarget {
                        hostname: app.hostname.clone(),
                        handler_type: HandlerType::WinterTCHandler(path),
                    }
                }
                crate::http::router::EntrypointType::StaticFile(path) => {
                    // Static file entrypoint - serve directly
                    let ext = Path::new(&path)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    let content_type = content_type_from_ext(ext).to_string();
                    RouteTarget {
                        hostname: app.hostname.clone(),
                        handler_type: HandlerType::StaticFile { path, content_type },
                    }
                }
                crate::http::router::EntrypointType::StaticDir(root) => {
                    // Directory entrypoint - serve from directory
                    RouteTarget {
                        hostname: app.hostname.clone(),
                        handler_type: HandlerType::StaticDir {
                            root,
                            default_file: "index.html".to_string(),
                        },
                    }
                }
            }
        };

        // Log registration with handler type
        let handler_name = match &target.handler_type {
            HandlerType::WinterTCHandler(_) => "javascript",
            HandlerType::WinterTCSliverHandler { .. } => "sliver-js",
            HandlerType::StaticFile { .. } => "static-file",
            HandlerType::StaticDir { .. } => "static-dir",
            HandlerType::StaticResponse(_) => "static-response",
            HandlerType::VfsStaticFiles { .. } => "vfs-static",
        };

        router.register(app.hostname.clone(), target);
        tracing::info!(
            "Registered app '{}' with {} handler (entrypoint: {})",
            app.hostname,
            handler_name,
            app.entrypoint
        );
    }

    tracing::info!(
        "Virtual host router initialized with {} routes",
        router.route_count()
    );

    // Find first app with disk VFS config to pass to WorkQueue
    // Note: This is a temporary solution - proper per-app VFS backends require
    // architectural changes to create pools lazily with app-specific configs
    let disk_config = nano_config.apps.iter().find_map(|app| {
        if let crate::config::VfsBackendType::Disk = app.vfs_backend {
            app.vfs_disk.clone()
        } else {
            None
        }
    });

    // Wrap registry in Arc for sharing with AppState
    let registry_arc = Arc::new(registry);

    // Create app state with the router, optional disk VFS config, and app registry
    // Use the first app's worker limit, or default to 4 if not specified
    let workers_per_pool = nano_config
        .apps
        .first()
        .map(|app| app.limits.workers)
        .filter(|&w| w > 0)
        .unwrap_or(4);

    let app_state = if let Some(ref disk) = disk_config {
        tracing::info!("Using disk VFS backend with base_path: {}", disk.base_path);
        AppState::with_vfs_config(
            router,
            workers_per_pool,
            Some(disk.clone()),
            Some(registry_arc),
        )
    } else {
        AppState::with_vfs_config(router, workers_per_pool, None, Some(registry_arc))
    };
    let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));

    // Convert server config and serve.
    let server_config = ServerConfig::from(nano_config.server);
    serve_app(&server_config, create_app_with_shutdown(state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_health_endpoint() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ready_endpoint_when_healthy() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_ready_endpoint_when_shutting_down() {
        // Create app with shutdown state marked
        let drain = crate::app::drain::RequestDrain::new();
        let shutdown_state = ShutdownState::new(drain);
        shutdown_state.mark_shutting_down();

        let app_state = AppState::new(
            VirtualHostRouter::new(RouteTarget {
                hostname: "default".to_string(),
                handler_type: HandlerType::StaticResponse("NANO Runtime".to_string()),
            }),
            4,
        );
        let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));
        let app = create_app_with_shutdown(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.host, "127.0.0.1");

        // Verify socket_addr works
        let addr = config.socket_addr().unwrap();
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn test_state_creation() {
        let _state = State::new();
        let _arc_state = Arc::new(State::new());
        // Just verify State can be created and wrapped in Arc
    }

    #[tokio::test]
    async fn test_app_creation() {
        let app = create_app();
        // Verify the app can be created without panicking
        let _ = app;
    }

    #[tokio::test]
    async fn test_health_response_format() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let health: HealthResponse = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(health.status, "healthy");
        assert!(!health.version.is_empty());
    }

    #[tokio::test]
    async fn test_ready_response_format() {
        let app = create_app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let ready: ReadyResponse = serde_json::from_slice(&body_bytes).unwrap();

        assert!(ready.ready);
        assert_eq!(ready.message, "Server is ready");
    }

    #[tokio::test]
    async fn test_ready_response_when_shutting_down() {
        let drain = crate::app::drain::RequestDrain::new();
        let shutdown_state = ShutdownState::new(drain);
        shutdown_state.mark_shutting_down();

        let app_state = AppState::new(
            VirtualHostRouter::new(RouteTarget {
                hostname: "default".to_string(),
                handler_type: HandlerType::StaticResponse("NANO Runtime".to_string()),
            }),
            4,
        );
        let state = Arc::new(AppStateWithShutdown::new(app_state, shutdown_state));
        let app = create_app_with_shutdown(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/_admin/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let ready: ReadyResponse = serde_json::from_slice(&body_bytes).unwrap();

        assert!(!ready.ready);
        assert_eq!(ready.message, "Server is shutting down");
    }

    #[tokio::test]
    async fn test_socket_reuse_addr() {
        // Verify SO_REUSEADDR lets us rebind a concrete address immediately after
        // dropping a listener on it. Use an OS-assigned ephemeral port rather than
        // the fixed default port — the default may already be held by another
        // process, which was the source of prior flakiness.
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        // First bind on the concrete port.
        let listener1 = create_reuse_listener(&addr).await;
        assert!(listener1.is_ok(), "First bind should succeed on {}", addr);

        // Drop it, then immediately rebind the same address. With SO_REUSEADDR this
        // succeeds without waiting out TIME_WAIT — no sleep needed.
        drop(listener1);

        let listener2 = create_reuse_listener(&addr).await;
        assert!(
            listener2.is_ok(),
            "Second bind on {} should succeed with SO_REUSEADDR enabled",
            addr
        );
    }
}
