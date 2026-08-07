//! NANO Edge Runtime - Multi-tenant JavaScript edge runtime
//!
//! A Rust-based edge runtime using rusty_v8 to execute JavaScript in isolated
//! V8 contexts. Supports multi-tenancy with thread-local isolates and context
//! reset between requests for fast cold starts.

use anyhow::Result;

pub mod admin;
pub mod app;
pub mod assertions;
pub mod config;
pub mod control_plane;
pub mod data_plane;
pub mod http;
pub mod limits;
pub mod logging;
pub mod metrics;
pub mod runtime;
pub mod signal;
pub mod sliver;
pub mod v8;
pub mod vfs;
pub mod wasm;
pub mod worker;

/// Library entry point — stub used by tests and binary glue.
///
/// ## Integration boundary
///
/// External systems (PaaS layers, orchestrators) integrate with nano-rs via the
/// **admin HTTP API** (`POST /admin/apps`, `reload`, `scale`, etc.) over the Unix
/// socket or TCP address configured at startup. This is the only supported
/// integration surface.
///
/// The `[lib]` crate target exists for internal test infrastructure. The module
/// tree (`worker`, `runtime`, `wasm`, `vfs`, etc.) is `pub` for test access, but
/// its API is **not stable** and may change between versions without notice.
///
/// ## Future embedding (not yet designed)
///
/// A `NanoRuntime::builder()` embedding API is a potential future path, but
/// requires a design doc covering public API surface, lifecycle contract
/// (start/reload/shutdown), and config model. Do not implement without that design.
pub fn run() -> Result<()> {
    tracing::info!("NANO runtime initialized");
    Ok(())
}
