//! Virtual host routing for HTTP requests
//!
//! Provides virtual host routing that directs HTTP requests to different
//! handlers based on the Host header. Supports exact hostname matching
//! with case-insensitive lookup and a fallback default handler.
//!
//! # Decisions
//!
//! - **D-03:** Exact hostname match only (no wildcards or regex patterns for v1)
//! - **D-04:** Fallback to default handler when no hostname matches
//! - Hostname lookup is case-insensitive per HTTP spec
//!
//! # WinterTC Integration
//!
//! This module integrates with WinterTC types (NanoRequest/NanoResponse)
//! to enable JavaScript handler execution.
//!
//! # Static File Serving
//!
//! Entrypoint type detection automatically determines how to handle entrypoints:
//! - JavaScript files (.js, .mjs, .ts) → Execute as Workers
//! - Static files (.html, .css, images, etc.) → Serve with correct content-type
//! - Directories → Serve index.html with automatic content-type detection

#[path = "ws_relay.rs"]
mod ws_relay;
use ws_relay::handle_ws_upgrade;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, Response, StatusCode},
    response::IntoResponse,
};
use tokio::sync::Mutex;

use crate::app::registry::AppRegistry;
use crate::http::{content_type_from_ext, NanoRequest, NanoResponse};
use crate::logging::create_request_span;
use crate::metrics::METRICS;
use crate::worker::{HandlerTask, QueueError, WorkQueue};
use uuid::Uuid;

/// Entrypoint type for automatic file type detection
///
/// Determines how to handle an entrypoint based on its file extension:
/// - JavaScript files (.js, .mjs, .ts) → Execute as Workers
/// - Static files (.html, .css, images, etc.) → Serve with correct content-type
/// - Directories → Serve index.html with automatic content-type detection
#[derive(Debug, Clone)]
pub enum EntrypointType {
    /// Path to a JavaScript file that should be executed as a Worker
    JavaScript(String),
    /// Path to a specific static file to serve
    StaticFile(String),
    /// Path to a directory (serves index.html for root path)
    StaticDir(String),
}

/// Detect the type of entrypoint based on file extension
///
/// Analyzes the file path to determine whether it should be:
/// - Executed as JavaScript (js, mjs, ts extensions)
/// - Served as a static file (html, css, images, etc.)
/// - Served as a directory (with index.html fallback)
///
/// # Arguments
///
/// * `path` - The file or directory path to analyze
///
/// # Returns
///
/// An `EntrypointType` indicating how the entrypoint should be handled
///
/// # Examples
///
/// ```rust
/// use nano::http::router::detect_entrypoint_type;
///
/// let js = detect_entrypoint_type("./app.js");
/// // Returns EntrypointType::JavaScript("./app.js")
///
/// let html = detect_entrypoint_type("./index.html");
/// // Returns EntrypointType::StaticFile("./index.html")
///
/// let dir = detect_entrypoint_type("./dist");
/// // Returns EntrypointType::StaticDir("./dist")
/// ```
pub fn detect_entrypoint_type(path: &str) -> EntrypointType {
    let path_obj = Path::new(path);

    // Check if it's a directory first
    if path_obj.is_dir() {
        return EntrypointType::StaticDir(path.to_string());
    }

    // Get file extension
    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        // JavaScript files - execute as Worker (.gs = Google Apps Script classic scripts)
        "js" | "mjs" | "ts" | "gs" => EntrypointType::JavaScript(path.to_string()),
        // All other files - serve statically
        _ => EntrypointType::StaticFile(path.to_string()),
    }
}

/// Handler type for routed requests
///
/// Defines how a request should be processed based on the route configuration.
/// Supports static responses for testing, WinterTC handlers for JS execution,
/// and static file serving for HTML/CSS/assets.
#[derive(Debug, Clone)]
pub enum HandlerType {
    /// Returns a fixed response string (for testing)
    StaticResponse(String),
    /// WinterTC handler that uses NanoRequest/NanoResponse
    WinterTCHandler(String),
    /// WinterTC handler for sliver-based (snapshot-restored) apps
    ///
    /// Contains the entrypoint path and optional sliver data reference
    WinterTCSliverHandler {
        /// Path to the JavaScript entrypoint
        entrypoint: String,
        /// Reference to hostname for looking up sliver data in registry
        hostname: String,
    },
    /// Serve static files from VFS entries
    ///
    /// This handler serves files directly from the VFS entries
    /// stored in the sliver. It's used for static sites and assets.
    VfsStaticFiles {
        /// Map of path -> (content, content_type)
        files: std::collections::HashMap<String, (Vec<u8>, String)>,
        /// Default file to serve for root path (e.g., "index.html")
        default_file: Option<String>,
    },
    /// Serve a single static file from the filesystem
    ///
    /// Used for HTML entrypoints and other static files.
    /// Files are read at request time from the filesystem.
    StaticFile {
        /// Path to the file on disk
        path: String,
        /// Content-Type header value
        content_type: String,
    },
    /// Serve static files from a directory
    ///
    /// Used for directory entrypoints (e.g., Astro build output).
    /// Serves index.html for root path and maps other paths to files.
    StaticDir {
        /// Root directory path
        root: String,
        /// Default file to serve for root path (e.g., "index.html")
        default_file: String,
    },
}

/// Target for a routed request
///
/// Associates a hostname with its handler configuration. This is stored
/// in the router's route table and returned when a hostname matches.
#[derive(Debug, Clone)]
pub struct RouteTarget {
    /// The hostname this route targets
    pub hostname: String,
    /// The handler type for this route
    pub handler_type: HandlerType,
}

impl RouteTarget {
    /// Handle a static request directly, bypassing the worker pool.
    ///
    /// Only valid for `StaticResponse`, `VfsStaticFiles`, `StaticFile`, and `StaticDir`
    /// variants. JS variants (`WinterTCHandler`, `WinterTCSliverHandler`) are routed
    /// through the worker pool by `dispatch_to_worker_pool` and must never reach here.
    pub async fn handle(&self, _request: NanoRequest) -> NanoResponse {
        match &self.handler_type {
            HandlerType::WinterTCHandler(_) | HandlerType::WinterTCSliverHandler { .. } => {
                unreachable!("JS handlers must be dispatched via the worker pool, not handle()")
            }
            HandlerType::StaticResponse(response) => {
                if response.is_empty() {
                    // Empty response means "not found" - return HTTP 404
                    NanoResponse::not_found()
                        .with_header("Content-Type", "text/plain")
                        .with_body("Not Found")
                } else {
                    NanoResponse::ok()
                        .with_header("Content-Type", "text/plain")
                        .with_body(response.clone())
                }
            }
            HandlerType::VfsStaticFiles {
                files,
                default_file,
            } => {
                // Serve static files from VFS
                let path = _request.url().pathname();

                // Special handling for root path
                let is_root = path == "/" || path.is_empty();

                // Get the default file name
                let default = default_file.as_deref().unwrap_or("index.html");

                // Determine lookup path
                let lookup_path = if is_root {
                    default.to_string()
                } else {
                    // Remove leading slash
                    path.strip_prefix('/')
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| path.to_string())
                };

                // Debug: log available files and lookup attempt
                tracing::debug!(
                    "VFS lookup: path='{}' is_root={} -> lookup='{}' | files count={}",
                    path,
                    is_root,
                    lookup_path,
                    files.len()
                );

                // STRATEGY 1: Try exact match first
                if let Some((content, content_type)) = files.get(&lookup_path) {
                    tracing::debug!(
                        "VFS hit (exact): '{}' ({} bytes)",
                        lookup_path,
                        content.len()
                    );
                    return NanoResponse::ok()
                        .with_header("Content-Type", content_type)
                        .with_body_bytes(content.clone());
                }

                // STRATEGY 2: For root path, try JS entry points first (frameworks), then HTML
                if is_root {
                    // JavaScript frameworks typically use index.js as entry point
                    let entry_points = vec![
                        "index.js",   // Most common JS framework entry
                        "app.js",     // Alternative JS entry
                        "main.js",    // Another common JS entry
                        "server.js",  // Server-side JS entry
                        "index.html", // Static site fallback
                        "index.htm",  // Legacy HTML
                    ];
                    for entry_point in entry_points {
                        if let Some((content, content_type)) = files.get(entry_point) {
                            tracing::debug!("VFS hit (root entry point): '{}'", entry_point);
                            return NanoResponse::ok()
                                .with_header("Content-Type", content_type)
                                .with_body_bytes(content.clone());
                        }
                    }
                }

                // STRATEGY 3: Try with /index.html suffix (for directory paths)
                let index_path = format!("{}/index.html", lookup_path);
                if let Some((content, content_type)) = files.get(&index_path) {
                    tracing::debug!("VFS hit (dir index): '{}'", index_path);
                    return NanoResponse::ok()
                        .with_header("Content-Type", content_type)
                        .with_body_bytes(content.clone());
                }

                // STRATEGY 4: Try with .html extension
                let html_path = format!("{}.html", lookup_path);
                if let Some((content, content_type)) = files.get(&html_path) {
                    tracing::debug!("VFS hit (.html ext): '{}'", html_path);
                    return NanoResponse::ok()
                        .with_header("Content-Type", content_type)
                        .with_body_bytes(content.clone());
                }

                // File not found - return clean 404
                tracing::debug!(
                    "VFS miss: path='{}' lookup='{}' not found in {} files",
                    path,
                    lookup_path,
                    files.len()
                );

                NanoResponse::not_found()
            }
            HandlerType::StaticFile { path, content_type } => {
                // Serve a single static file from the filesystem
                tracing::debug!(
                    "Serving static file: {} (content-type: {})",
                    path,
                    content_type
                );

                match tokio::fs::read_to_string(path).await {
                    Ok(content) => NanoResponse::ok()
                        .with_header("Content-Type", content_type)
                        .with_body(content),
                    Err(e) => {
                        tracing::warn!("Failed to read static file {}: {}", path, e);
                        NanoResponse::not_found()
                    }
                }
            }
            HandlerType::StaticDir { root, default_file } => {
                // Serve files from a directory
                let path = _request.url().pathname();

                // Determine file path
                let file_path = if path == "/" || path.is_empty() {
                    format!("{}/{}", root, default_file)
                } else {
                    // Remove leading slash and construct path
                    let clean_path = path.strip_prefix('/').unwrap_or_else(|| path.as_str());
                    // Security: prevent path traversal
                    if clean_path.contains("..") {
                        tracing::warn!("Path traversal attempt blocked: {}", path);
                        return NanoResponse::not_found();
                    }
                    format!("{}/{}", root, clean_path)
                };

                tracing::debug!("Serving from directory: {} -> {}", path, file_path);

                // Determine content type from extension
                let ext = Path::new(&file_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                let content_type = content_type_from_ext(ext);

                // Read and serve the file
                match tokio::fs::read(&file_path).await {
                    Ok(bytes) => NanoResponse::ok()
                        .with_header("Content-Type", content_type)
                        .with_body_bytes(bytes),
                    Err(e) => {
                        tracing::debug!("File not found: {} (error: {})", file_path, e);
                        NanoResponse::not_found()
                    }
                }
            }
        }
    }
}

/// Virtual host router
///
/// Routes HTTP requests based on the Host header using exact hostname
/// matching. Hostnames are compared case-insensitively by storing and
/// looking up lowercase versions.
#[derive(Debug, Clone)]
pub struct VirtualHostRouter {
    /// Route table: lowercase hostname -> route target
    routes: HashMap<String, RouteTarget>,
    /// Default handler for unmatched hosts
    default: RouteTarget,
}

impl VirtualHostRouter {
    /// Creates a new virtual host router with a default fallback handler
    ///
    /// The default handler is returned when no registered hostname matches
    /// the request's Host header. This ensures every request gets handled
    /// per D-04.
    ///
    /// # Arguments
    ///
    /// * `default` - The route target to use when no hostname matches
    ///
    /// # Returns
    ///
    /// A new `VirtualHostRouter` with empty routes and the specified default
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::http::router::{VirtualHostRouter, RouteTarget, HandlerType};
    ///
    /// let default = RouteTarget {
    ///     hostname: "default".to_string(),
    ///     handler_type: HandlerType::StaticResponse("Not Found".to_string()),
    /// };
    /// let router = VirtualHostRouter::new(default);
    /// ```
    pub fn new(default: RouteTarget) -> Self {
        Self {
            routes: HashMap::new(),
            default,
        }
    }

    /// Returns the number of registered routes
    ///
    /// Useful for logging and monitoring the router state.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Registers a new hostname route
    ///
    /// Adds a hostname -> handler mapping to the route table. The hostname
    /// is stored in lowercase for case-insensitive matching per HTTP spec.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The hostname to register (e.g., "api.example.com")
    /// * `target` - The route target defining how to handle requests
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::http::router::{VirtualHostRouter, RouteTarget, HandlerType};
    ///
    /// let default = RouteTarget {
    ///     hostname: "default".to_string(),
    ///     handler_type: HandlerType::StaticResponse("default".to_string()),
    /// };
    /// let mut router = VirtualHostRouter::new(default);
    ///
    /// router.register(
    ///     "api.example.com".to_string(),
    ///     RouteTarget {
    ///         hostname: "api.example.com".to_string(),
    ///         handler_type: HandlerType::StaticResponse("api".to_string()),
    ///     },
    /// );
    /// ```
    pub fn register(&mut self, hostname: String, target: RouteTarget) {
        let lowercase_host = hostname.to_lowercase();
        tracing::info!(
            "Registering route: {} -> {:?}",
            hostname,
            target.handler_type
        );
        self.routes.insert(lowercase_host, target);
    }

    /// Resolves a hostname to its route target
    ///
    /// Performs case-insensitive exact match lookup. If no route matches,
    /// returns the default handler per D-04.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname from the HTTP Host header
    ///
    /// # Returns
    ///
    /// A reference to the `RouteTarget` for this hostname (or default)
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::http::router::{VirtualHostRouter, RouteTarget, HandlerType};
    ///
    /// let default = RouteTarget {
    ///     hostname: "default".to_string(),
    ///     handler_type: HandlerType::StaticResponse("default".to_string()),
    /// };
    /// let router = VirtualHostRouter::new(default);
    ///
    /// // Unknown host returns default
    /// let target = router.resolve("unknown.com");
    /// // assert!(matches!(target.handler_type, HandlerType::StaticResponse(s) if s == "default"));
    /// ```
    pub fn resolve(&self, host: &str) -> &RouteTarget {
        let lowercase_host = host.to_lowercase();
        self.routes.get(&lowercase_host).unwrap_or(&self.default)
    }

    /// Remove a route by hostname. Returns true if the route existed.
    pub fn deregister(&mut self, hostname: &str) -> bool {
        self.routes.remove(&hostname.to_lowercase()).is_some()
    }

    /// Return the entrypoint path for a user-registered WinterTC route, if any.
    pub fn get_user_route(&self, hostname: &str) -> Option<String> {
        self.routes
            .get(&hostname.to_lowercase())
            .and_then(|t| match &t.handler_type {
                HandlerType::WinterTCHandler(path) => Some(path.clone()),
                HandlerType::WinterTCSliverHandler { entrypoint, .. } => Some(entrypoint.clone()),
                _ => None,
            })
    }

    /// Iterate over all user-registered WinterTC routes as (hostname, entrypoint) pairs.
    pub fn user_routes(&self) -> impl Iterator<Item = (&String, &String)> {
        self.routes
            .iter()
            .filter_map(|(host, target)| match &target.handler_type {
                HandlerType::WinterTCHandler(path) => Some((host, path)),
                HandlerType::WinterTCSliverHandler { entrypoint, .. } => Some((host, entrypoint)),
                _ => None,
            })
    }
}

impl Default for VirtualHostRouter {
    /// Creates a default router with a simple "NANO Runtime" handler
    ///
    /// This is useful for testing and bootstrapping. Production code
    /// should create a router with a custom default handler.
    fn default() -> Self {
        let default_target = RouteTarget {
            hostname: "default".to_string(),
            handler_type: HandlerType::StaticResponse("NANO Runtime".to_string()),
        };
        Self::new(default_target)
    }
}

/// Application state shared with axum handlers
///
/// Contains the virtual host router and WorkQueue for request dispatch.
/// Wrapped in Arc for thread-safe sharing across requests.
#[derive(Clone)]
pub struct AppState {
    /// The virtual host router for hostname-based request routing.
    /// Wrapped in Arc<RwLock<>> so the admin API can register routes at runtime
    /// without restarting the HTTP server.
    pub router: Arc<tokio::sync::RwLock<VirtualHostRouter>>,
    /// The WorkQueue for dispatching requests to worker pools
    pub work_queue: Arc<Mutex<WorkQueue>>,
    /// Optional AppRegistry for looking up app limits
    app_registry: Option<Arc<AppRegistry>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("router", &"<VirtualHostRouter>")
            .field("work_queue", &"<WorkQueue>")
            .field("has_app_registry", &self.app_registry.is_some())
            .finish()
    }
}

impl AppState {
    /// Create a new AppState with the given router and worker configuration
    pub fn new(router: VirtualHostRouter, workers_per_pool: u32) -> Self {
        Self::with_vfs_config(router, workers_per_pool, None, None)
    }

    /// Create a new AppState from a shared router Arc (used when admin API shares the router).
    /// Uses disk VFS with "/" as base so admin-registered absolute entrypoint paths are readable.
    pub fn new_shared(
        router: Arc<tokio::sync::RwLock<VirtualHostRouter>>,
        workers_per_pool: u32,
    ) -> Self {
        let vfs_disk = Some(crate::config::VfsDiskConfig {
            base_path: "/".to_string(),
        });
        Self {
            router,
            work_queue: Arc::new(Mutex::new(WorkQueue::with_vfs_config(
                workers_per_pool,
                vfs_disk,
                None,
            ))),
            app_registry: None,
        }
    }

    /// Create a new AppState with VFS disk backend configuration
    pub fn with_vfs_config(
        router: VirtualHostRouter,
        workers_per_pool: u32,
        vfs_disk_config: Option<crate::config::VfsDiskConfig>,
        app_registry: Option<Arc<AppRegistry>>,
    ) -> Self {
        Self {
            router: Arc::new(tokio::sync::RwLock::new(router)),
            work_queue: Arc::new(Mutex::new(WorkQueue::with_vfs_config(
                workers_per_pool,
                vfs_disk_config,
                app_registry.clone(),
            ))),
            app_registry,
        }
    }

    /// Get CPU time limit for a hostname from the app registry
    ///
    /// Returns the configured CPU time limit in milliseconds if the app
    /// is found and CPU time tracking is enabled. Returns 0 if disabled
    /// or app not found (no limit).
    fn get_cpu_time_limit_ms(&self, hostname: &str) -> u32 {
        match &self.app_registry {
            None => 0,
            Some(registry) => match registry.get(hostname) {
                None => 0,
                Some(app_config) => {
                    if app_config.limits.cpu_time_enabled {
                        app_config.limits.cpu_time_ms
                    } else {
                        0
                    }
                }
            },
        }
    }
}

/// Dispatch request to worker pool via WorkQueue
///
/// This handler integrates the virtual host router with the WorkQueue,
/// enabling affine dispatch: same hostname always routes to same worker.
/// Records metrics for each request: count by hostname/status and latency.
/// Returns HTTP 503 with Retry-After header when channel is full.
///
/// # Arguments
///
/// * `state` - Application state containing the router and WorkQueue
/// * `request` - The full HTTP request
///
/// # Returns
///
/// An HTTP response from the worker pool or an error response
pub async fn dispatch_to_worker_pool(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    // Start timing the request
    let start = std::time::Instant::now();
    // Extract Host header from the request and strip port if present
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|s| {
            // Strip port from host:port format (e.g., "localhost:9999" -> "localhost")
            s.split(':').next().unwrap_or(s).to_string()
        })
        .unwrap_or_else(|| "default".to_string());

    // Generate request ID and create span with context
    let request_id = format!("req_{}", Uuid::new_v4().to_string()[..8].to_string());
    let span = create_request_span(&host, &request_id);
    let _enter = span.enter();

    tracing::debug!("Dispatching request to worker pool for host: {}", host);

    if request
        .headers()
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return handle_ws_upgrade(state, request, host)
            .await
            .into_response();
    }

    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers = request.headers().clone();
    let body = request.into_body();

    // Read body (1MB limit per D-05)
    let body_bytes = match axum::body::to_bytes(body, 1048576).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read body: {}", e);
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"error":"BadRequest","message":"Failed to read body","code":400}"#,
                ))
                .unwrap();
        }
    };

    let nano_request =
        match NanoRequest::from_axum_parts(&method, &uri, &host, &headers, Some(body_bytes)) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to parse URL: {}", e);
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"error":"BadRequest","message":"Invalid URL","code":400}"#,
                    ))
                    .unwrap();
            }
        };

    // Look up route target — clone to release the read lock before async dispatch.
    let target = state.router.read().await.resolve(&host).clone();

    // Extract entrypoint from target or handle directly
    let entrypoint = match &target.handler_type {
        HandlerType::WinterTCHandler(path) => path.clone(),
        HandlerType::WinterTCSliverHandler {
            entrypoint: path, ..
        } => path.clone(),
        HandlerType::StaticResponse(_)
        | HandlerType::VfsStaticFiles { .. }
        | HandlerType::StaticFile { .. }
        | HandlerType::StaticDir { .. } => {
            // These handler types don't need worker dispatch - serve directly
            let nano_response = target.handle(nano_request).await;
            return nano_response.to_axum_response();
        }
    };

    // Create oneshot channel for response
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Get CPU time limit from app registry (0 means no limit)
    let cpu_time_limit_ms = state.get_cpu_time_limit_ms(&host);

    // Create handler task with hostname and request_id for distributed tracing
    let task = HandlerTask {
        entrypoint,
        request: nano_request,
        response_tx: tx,
        hostname: host.clone(),
        start_time: std::time::Instant::now(),
        cpu_time_limit_ms,
        request_id: request_id.clone(),
        memory_limit_mb: 0,
        ws: None,
    };

    // Get request path for access log
    let path = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");

    // Dispatch to WorkQueue — hold lock only through dispatch, not rx.await.
    // Previously the MutexGuard lived through the entire rx.await (full V8 execution),
    // serializing all tenants behind a single lock. Now the guard drops as soon as
    // the task lands in the worker's bounded channel.
    let dispatch_result = {
        let mut queue = state.work_queue.lock().await;
        queue.ensure_tenant(&host);
        if let Some(ref control_plane) = queue.control_plane {
            if let Err(e) = control_plane.validate_request_ref(&task) {
                tracing::warn!("Control plane validation failed: {}", e);
                let response = Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("content-type", "text/plain")
                    .body(Body::from(format!("Validation error: {}", e)))
                    .unwrap();
                return response;
            }
        }
        queue.dispatch(&host, task).await
    }; // MutexGuard dropped here — concurrent requests can now be dispatched

    let (response, status_code, worker_id, isolate_id) = match dispatch_result {
        Ok(()) => {
            // Wait for response from worker — lock NOT held
            match rx.await {
                Ok(Ok(nano_response)) => {
                    let status = nano_response.status();
                    let worker_id = nano_response.worker_id();
                    let isolate_id = nano_response.isolate_id().map(|s| s.to_string());
                    (
                        nano_response.to_axum_response(),
                        status,
                        worker_id,
                        isolate_id,
                    )
                }
                Ok(Err(e)) => {
                    tracing::error!("Handler error: {}", e);
                    let response = Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header("content-type", "text/plain")
                        .body(Body::from("Internal Server Error"))
                        .unwrap();
                    (response, 500, None, None)
                }
                Err(_) => {
                    tracing::error!("Response channel closed");
                    let response = Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .header("content-type", "text/plain")
                        .body(Body::from("Internal Server Error"))
                        .unwrap();
                    (response, 500, None, None)
                }
            }
        }
        Err(QueueError::ChannelFull) => {
            tracing::warn!("WorkQueue full for hostname: {}", host);
            let response = Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Retry-After", "1")
                .header("content-type", "text/plain")
                .body(Body::from("Service Unavailable - Queue Full"))
                .unwrap();
            (response, 503, None, None)
        }
        Err(e) => {
            tracing::error!("Dispatch error: {}", e);
            let response = Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "text/plain")
                .body(Body::from("Internal Server Error"))
                .unwrap();
            (response, 500, None, None)
        }
    };

    // Calculate duration and record metrics
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    METRICS.record_request(&host, &status_code.to_string(), duration_ms);

    // HTTP Access Log - single line per request with all key info
    // Include worker_id and isolate_id when available to show which worker/isolate processed the request
    match (worker_id, isolate_id) {
        (Some(wid), Some(iso)) => {
            let worker_id_u64 = wid as u64;
            tracing::info!(
                method = %method,
                path = %path,
                host = %host,
                status = status_code,
                worker_id = worker_id_u64,
                isolate_id = %iso,
                duration_ms = format!("{:.2}", duration_ms),
                "HTTP {} {} - {} in {}ms (worker: {}, isolate: {})",
                method,
                path,
                status_code,
                format!("{:.2}", duration_ms),
                wid,
                iso
            );
        }
        (Some(wid), None) => {
            let worker_id_u64 = wid as u64;
            tracing::info!(
                method = %method,
                path = %path,
                host = %host,
                status = status_code,
                worker_id = worker_id_u64,
                duration_ms = format!("{:.2}", duration_ms),
                "HTTP {} {} - {} in {}ms (worker: {})",
                method,
                path,
                status_code,
                format!("{:.2}", duration_ms),
                wid
            );
        }
        _ => {
            tracing::info!(
                method = %method,
                path = %path,
                host = %host,
                status = status_code,
                duration_ms = format!("{:.2}", duration_ms),
                "HTTP {} {} - {} in {}ms",
                method,
                path,
                status_code,
                format!("{:.2}", duration_ms)
            );
        }
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_ws_upgrade_forbidden_for_static_handler() {
        let router = VirtualHostRouter::new(RouteTarget {
            hostname: "static.host".to_string(),
            handler_type: HandlerType::StaticResponse("hello".to_string()),
        });
        let state = Arc::new(AppState::new(router, 0));

        let request = Request::builder()
            .method("GET")
            .uri("http://static.host/ws")
            .header("upgrade", "websocket")
            .header("connection", "upgrade")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("sec-websocket-version", "13")
            .body(Body::empty())
            .unwrap();

        let response = handle_ws_upgrade(state, request, "static.host".to_string()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_ws_upgrade_rejects_malformed_request() {
        let router = VirtualHostRouter::new(RouteTarget {
            hostname: "ws.host".to_string(),
            handler_type: HandlerType::WinterTCHandler("/app/index.js".to_string()),
        });
        let state = Arc::new(AppState::new(router, 0));

        // Missing Sec-WebSocket-Key and Sec-WebSocket-Version → extractor rejects → 400
        let request = Request::builder()
            .method("GET")
            .uri("http://ws.host/ws")
            .header("upgrade", "websocket")
            .body(Body::empty())
            .unwrap();

        let response = handle_ws_upgrade(state, request, "ws.host".to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
