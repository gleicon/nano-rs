//! Admin module for diagnostics and monitoring
//!
//! Provides visibility into the NANO runtime state including:
//! - Active isolates and worker pools
//! - App statistics and resource usage
//! - System-wide diagnostics
//! - Prometheus metrics endpoint
//! - HTTP Admin API with API key authentication
//! - Unix domain socket for local admin access

pub mod auth;
pub mod diagnostics;
pub mod handlers;
pub mod metrics;
pub mod server;
pub mod unix_socket;

pub use auth::{api_key_middleware, api_key_middleware_forbidden, AdminAuth, AuthError};
pub use diagnostics::{AppStats, DiagnosticsCollector, IsolateInfo, SystemDiagnostics};
pub use handlers::*;
pub use metrics::metrics_handler;
pub use server::{create_admin_router, AdminConfig, AdminServer};
pub use unix_socket::{
    create_unix_socket, start_unix_socket_server, UnixSocketConfig, UnixSocketServer,
};
