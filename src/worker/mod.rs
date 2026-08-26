//! Worker pool architecture for multi-tenant JavaScript execution
//!
//! This module provides a unified worker pool implementation for NANO, managing V8 isolate
//! execution across multiple threads with per-tenant isolation.
//!
//! ## Architecture Overview (Unified)
//!
//! All application types (Static, JS) flow through a single unified engine.
//! (WebAssembly is available to JS handlers via the `WebAssembly` global, not as
//! a standalone deployable app type — there is no `AppSource::Wasm`.)
//!
//! ### Unified WorkerPool with AppSource
//!
//! The [`WorkerPool`] now accepts an [`AppSource`] enum that unifies all app types:
//!
//! - **Entrypoint** ([`AppSource::Entrypoint`]): JavaScript from filesystem
//!   - Creates fresh isolates from source files
//!   - Full feature set: CPU limits, memory monitoring, eviction, tracing
//!   - Use when: Config mode, development, dynamic loading
//!
//! - **Sliver** ([`AppSource::Sliver`]): Snapshot-based execution
//!   - Restores isolates from V8 snapshots (~1-2ms cold start)
//!   - Same full feature set as entrypoint mode
//!   - Use when: Production deployment from .sliver files
//!
//! - **Static** ([`AppSource::Static`]): Static file serving
//!   - No isolate creation (pure file serving)
//!   - Same routing and tracing as dynamic apps
//!   - Use when: Static sites without JavaScript execution
//!
//! ### Unified Features (All App Types)
//!
//! All applications now receive identical treatment:
//! - ✅ V8 isolates for code execution
//! - ✅ CPU time limits and enforcement
//! - ✅ Per-isolate heap cap + OOM termination + recycle-after-N + soft eviction
//!   (per-worker `MemoryMonitor` recycles the isolate under memory pressure). The
//!   cross-isolate LRU `EvictionManager` is implemented + tested but not yet wired
//!   — see docs/ROADMAP.md.
//! - ✅ Request tracing: `request_id` + `worker_id` + `isolate_id`
//! - ✅ Sliver packaging support (all types can be slivered)
//! - ✅ VFS for code and static artifacts
//! - ✅ Async/await support (no "Promise still pending")
//!
//! ### Architecture Diagram
//!
//! ```mermaid
//! flowchart TB
//!     subgraph AppSource["AppSource Enum"]
//!         EP[Entrypoint<br/>JS files]
//!         SL[Sliver<br/>V8 snapshot]
//!         ST[Static<br/>No isolate]
//!     end
//!
//!     subgraph WorkerPool["WorkerPool::with_source()"]
//!         RT[Tokio Runtime<br/>per thread]
//!         OM[OOM Monitor<br/>memory_limit_mb]
//!         MM[Memory Monitor<br/>pressure tracking]
//!         EM[Eviction Manager<br/>soft/hard eviction]
//!     end
//!
//!     subgraph Workers["Worker Threads"]
//!         W0[Worker 0<br/>V8 Isolate]
//!         W1[Worker 1<br/>V8 Isolate]
//!         WN[Worker N<br/>V8 Isolate]
//!     end
//!
//!     EP -->|fresh isolate| WorkerPool
//!     SL -->|snapshot restore| WorkerPool
//!     ST -->|file serving| StaticPool
//!
//!     WorkerPool --> W0
//!     WorkerPool --> W1
//!     WorkerPool --> WN
//!
//!     W0 -->|HandlerTask| MPSC[MPSC Channel<br/>request/response]
//!     W1 -->|HandlerTask| MPSC
//!     WN -->|HandlerTask| MPSC
//!
//!     MPSC -->|NanoResponse| HTTP[HTTP Layer]
//!
//!     style AppSource fill:#e1f5fe
//!     style WorkerPool fill:#fff3e0
//!     style Workers fill:#e8f5e9
//!     style StaticPool fill:#f3e5f5
//! ```
//!
//! ### Legacy Types (Unified)
//!
//! These types are maintained for backward compatibility but delegate to the unified
//! [`WorkerPool`] implementation internally:
//!
//! - **SliverWorkerPool** ([`pool::SliverWorkerPool`]): Thin wrapper for sliver mode
//!   - Wraps WorkerPool with AppSource::Sliver
//!   - Supports temp entrypoint override for extracted VFS
//!
//! - **EntrypointWorkerPool** ([`queue::EntrypointWorkerPool`]): Thin wrapper for entrypoint mode
//!   - Wraps WorkerPool with AppSource::Entrypoint
//!   - ~240 lines of duplicate worker logic removed
//!
//! - **WorkQueue** ([`queue::WorkQueue`]): Multi-hostname manager
//!   - Manages WorkerPools with per-hostname affinity
//!   - Affine dispatch for cache locality
//!
//! ### Trait-Based Design
//!
//! All pool types implement the [`WorkerPoolTrait`] for common operations:
//! - [`WorkerPoolTrait::dispatch`]: Send tasks to workers
//! - [`WorkerPoolTrait::shutdown`]: Graceful shutdown
//! - [`WorkerPoolTrait::worker_count`], [`WorkerPoolTrait::hostname`]: Introspection
//!
//! The trait is object-safe, enabling polymorphic usage:
//! ```rust,ignore
//! use nano::worker::{WorkerPoolTrait, SliverWorkerPool, EntrypointWorkerPool};
//!
//! fn use_pool(pool: &dyn WorkerPoolTrait) {
//!     println!("Pool {} has {} workers", pool.hostname(), pool.worker_count());
//! }
//! ```
//!
//! ### Pool Selection Guide
//!
//! All use cases now flow through the unified [`WorkerPool`] via [`AppSource`]:
//!
//! | Use Case | Recommended API | Legacy API (still works) |
//! |----------|-----------------|-------------------------|
//! | Production sliver execution | `WorkerPool::with_source(AppSource::sliver(data))` | `SliverWorkerPool::new(...)` |
//! | Dynamic app loading | `WorkerPool::with_source(AppSource::entrypoint(path))` | `EntrypointWorkerPool::new(...)` |
//! | Multi-tenant hosting | `WorkQueue` with `WorkerPool` per hostname | Same |
//! | Testing/development | `WorkerPool::with_source(AppSource::entrypoint(path))` | `EntrypointWorkerPool::new(...)` |
//!
//! **New code should use `WorkerPool::with_source()`** for direct access to all features.
//!
//! ### VFS Backend Configuration
//!
//! Both SliverWorkerPool and EntrypointWorkerPool support custom VFS backends:
//! - [`SliverWorkerPool::with_backend`]: Snapshot execution with custom storage
//! - [`EntrypointWorkerPool::with_backend`]: Entrypoint dispatch with custom storage
//! - [`WorkQueue::with_vfs_config`]: Async disk backend creation for multi-tenant
//!
//! ## Thread Safety
//!
//! Each worker thread creates and owns its `NanoIsolate` (thread-local ownership).
//! Isolates are `!Send + !Sync` via `PhantomData<*mut ()>`, preventing cross-thread
//! movement. This is critical for V8 stability (see POOL-05).
//!
//! ## Task Flow
//!
//! 1. HTTP layer creates a [`HandlerTask`] with entrypoint, request, and response channel
//! 2. WorkerPool dispatches the task via MPSC to a worker thread
//! 3. Worker thread executes the JavaScript handler using its isolate
//! 4. Response is sent back via oneshot channel
//!
//! ## Graceful Shutdown
//!
//! Calling [`WorkerPoolTrait::shutdown`] or dropping the pool signals workers to exit
//! via MPSC channel closure. All worker threads are joined to ensure clean isolate cleanup.

pub mod app_source;
pub mod cpu_tracker;
pub mod eviction;
pub mod limits;
pub mod memory_monitor;
pub mod oom;
pub mod pool;
pub mod queue;
pub mod sliver_pool;
pub mod telemetry;
pub mod tenant_pool;
pub mod timeout;
pub mod r#trait;

// Re-export types
pub use app_source::AppSource;
pub use cpu_tracker::{CpuTimeError, CpuTimeSnapshot, CpuTracker};
pub use eviction::{EvictionAction, EvictionManager, EvictionPolicy, IsolateMetadata};
pub use limits::{HeapStatistics, MemoryLimiter, OomError, RequestMemoryTracker};
pub use memory_monitor::{
    MemoryMonitor, MemoryMonitorConfig, MemoryPressureLevel, MemorySnapshot, MemoryTrend,
};
pub use oom::{OomMonitor, OomMonitorBuilder};
pub use pool::{WorkerHandle, WorkerPool};
pub use queue::{
    hash_hostname, EntrypointWorkerPool, QueueError, QueueStats, StatsSnapshot, WorkQueue,
};
pub use r#trait::{BoxedWorkerPool, WorkerPool as WorkerPoolTrait, WorkerPoolConfig};
pub use sliver_pool::{SliverPoolSlot, SliverWorkerPool};
pub use timeout::{ExecutionTimer, TimeoutConfig, TimeoutError};

use crate::http::{NanoRequest, NanoResponse};
use tokio::sync::oneshot;

/// Async-to-sync bridge channels for WebSocket connections.
///
/// Uses `std::sync::mpsc` (NOT `tokio::sync::mpsc`) because the worker side
/// needs `Receiver::recv_timeout()` for idle shrink-to-zero. tokio's mpsc has
/// no `recv_timeout`, making that pattern impossible (per D-06b, D-07b).
///
/// - `inbound_rx`: Worker receives frames arriving from the client
/// - `outbound_tx`: Worker sends frames to be forwarded to the client
pub struct WsChannels {
    /// Frames received from the WebSocket client (worker reads these)
    pub inbound_rx: std::sync::mpsc::Receiver<tungstenite::Message>,
    /// Frames to send to the WebSocket client (worker writes these)
    pub outbound_tx: std::sync::mpsc::SyncSender<tungstenite::Message>,
}

impl std::fmt::Debug for WsChannels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WsChannels {{ .. }}")
    }
}

/// Task sent to worker threads for JavaScript handler execution
///
/// This struct is `Send` so it can safely cross thread boundaries via MPSC channels.
/// The response is sent back via the oneshot channel.
#[derive(Debug)]
pub struct HandlerTask {
    /// Path to the JavaScript entrypoint file
    pub entrypoint: String,
    /// The incoming HTTP request (WinterTC-compatible)
    pub request: NanoRequest,
    /// Channel to send the response back to the caller
    pub response_tx: oneshot::Sender<anyhow::Result<NanoResponse>>,
    /// Hostname (tenant identifier) for metrics tracking
    pub hostname: String,
    /// Start time for request duration tracking
    pub start_time: std::time::Instant,
    /// CPU time limit in milliseconds (0 = no limit)
    pub cpu_time_limit_ms: u32,
    /// Request ID for distributed tracing (e.g., "req_abc123")
    pub request_id: String,
    /// Memory limit per request in MB (0 = use default 16MB)
    pub memory_limit_mb: u32,
    /// WebSocket channels — Some means WS upgrade mode, None means normal HTTP
    pub ws: Option<WsChannels>,
}

// Safety: NanoRequest is Clone + contains String/Bytes which are Send
// This explicit impl documents and verifies the Send contract
unsafe impl Send for HandlerTask {}

impl HandlerTask {
    /// Create a new handler task
    ///
    /// # Arguments
    ///
    /// * `entrypoint` - Path to the JavaScript file
    /// * `request` - The HTTP request to process
    /// * `response_tx` - Oneshot channel sender for the response
    pub fn new(
        entrypoint: String,
        request: NanoRequest,
        response_tx: oneshot::Sender<anyhow::Result<NanoResponse>>,
    ) -> Self {
        Self {
            entrypoint,
            request,
            response_tx,
            hostname: String::new(),
            start_time: std::time::Instant::now(),
            cpu_time_limit_ms: 0,
            request_id: format!("req_{}", uuid::Uuid::new_v4().to_string()[..8].to_string()),
            memory_limit_mb: 0,
            ws: None,
        }
    }

    /// Create a new handler task with a specific request_id for testing/tracing
    ///
    /// # Arguments
    ///
    /// * `entrypoint` - Path to the JavaScript file
    /// * `request` - The HTTP request to process
    /// * `response_tx` - Oneshot channel sender for the response
    /// * `hostname` - Tenant hostname for metrics tracking
    /// * `request_id` - Specific request ID for tracing
    pub fn new_with_request_id(
        entrypoint: String,
        request: NanoRequest,
        response_tx: oneshot::Sender<anyhow::Result<NanoResponse>>,
        hostname: String,
        request_id: String,
    ) -> Self {
        Self {
            entrypoint,
            request,
            response_tx,
            hostname,
            start_time: std::time::Instant::now(),
            cpu_time_limit_ms: 0,
            request_id,
            memory_limit_mb: 0,
            ws: None,
        }
    }

    /// Create a new handler task with hostname for metrics tracking
    ///
    /// # Arguments
    ///
    /// * `entrypoint` - Path to the JavaScript file
    /// * `request` - The HTTP request to process
    /// * `response_tx` - Oneshot channel sender for the response
    /// * `hostname` - Tenant hostname for metrics tracking
    pub fn with_hostname(
        entrypoint: String,
        request: NanoRequest,
        response_tx: oneshot::Sender<anyhow::Result<NanoResponse>>,
        hostname: String,
    ) -> Self {
        Self {
            entrypoint,
            request,
            response_tx,
            hostname,
            start_time: std::time::Instant::now(),
            cpu_time_limit_ms: 0,
            request_id: format!("req_{}", uuid::Uuid::new_v4().to_string()[..8].to_string()),
            memory_limit_mb: 0,
            ws: None,
        }
    }

    /// Create a new handler task with hostname, CPU limits, and request_id
    ///
    /// # Arguments
    ///
    /// * `entrypoint` - Path to the JavaScript file
    /// * `request` - The HTTP request to process
    /// * `response_tx` - Oneshot channel sender for the response
    /// * `hostname` - Tenant hostname for metrics tracking
    /// * `cpu_time_limit_ms` - CPU time limit in milliseconds (0 = no limit)
    /// * `request_id` - Request ID for distributed tracing
    pub fn with_hostname_and_limits(
        entrypoint: String,
        request: NanoRequest,
        response_tx: oneshot::Sender<anyhow::Result<NanoResponse>>,
        hostname: String,
        cpu_time_limit_ms: u32,
        request_id: String,
    ) -> Self {
        Self {
            entrypoint,
            request,
            response_tx,
            hostname,
            start_time: std::time::Instant::now(),
            cpu_time_limit_ms,
            request_id,
            memory_limit_mb: 0,
            ws: None,
        }
    }

    /// Set the request ID for distributed tracing
    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = request_id;
        self
    }

    /// Set CPU time limit
    pub fn with_cpu_limit(mut self, cpu_time_limit_ms: u32) -> Self {
        self.cpu_time_limit_ms = cpu_time_limit_ms;
        self
    }

    /// Set memory limit per request in MB
    pub fn with_memory_limit(mut self, memory_limit_mb: u32) -> Self {
        self.memory_limit_mb = memory_limit_mb;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoUrl};

    #[test]
    fn test_handler_task_creation() {
        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let request = NanoRequest::new("GET".to_string(), url, NanoHeaders::new(), None);

        let (tx, _rx) = oneshot::channel();
        let task = HandlerTask::new("/app/index.js".to_string(), request, tx);

        assert_eq!(task.entrypoint, "/app/index.js");
        assert_eq!(task.request.method(), "GET");
    }

    #[test]
    fn test_handler_task_is_send() {
        // Compile-time check: HandlerTask must be Send
        fn assert_send<T: Send>() {}
        assert_send::<HandlerTask>();
    }
}
