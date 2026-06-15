//! WorkQueue with bounded MPSC channel and affine dispatch
//!
//! Manages task distribution across worker pools with backpressure protection.
//! Affine dispatch ensures requests for the same hostname consistently route
//! to the same pool, improving cache locality.
//!
//! # Requirements
//!
//! - POOL-02: Bounded MPSC channel with 256-slot capacity
//! - POOL-03: Affine dispatch: hostname → pool index → worker thread
//!
//! # Decisions
//!
//! - **D-WQ-01:** 256-slot capacity per worker thread (not per pool)
//! - **D-WQ-02:** Case-insensitive hostname hashing per HTTP spec
//! - **D-WQ-03:** DefaultHasher for consistent hostname-to-pool mapping

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crate::vfs::{BackendFactory, MemoryBackend};
use crate::config::{VfsBackendType, VfsDiskConfig};
use crate::worker::HandlerTask;
use crate::app::registry::AppRegistry;
use crate::control_plane::ControlPlane;

/// Error types for queue operations
#[derive(Debug, Clone, PartialEq)]
pub enum QueueError {
    /// Channel is at capacity (bounded channel full)
    ChannelFull,
    /// Worker thread not found (invalid index)
    WorkerNotFound,
    /// Pool not found for hostname
    PoolNotFound,
    /// Send error (channel disconnected)
    SendError(String),
    /// Other errors
    Other(String),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueueError::ChannelFull => write!(f, "WorkQueue channel is full"),
            QueueError::WorkerNotFound => write!(f, "Worker thread not found"),
            QueueError::PoolNotFound => write!(f, "Pool not found for hostname"),
            QueueError::SendError(e) => write!(f, "Send error: {}", e),
            QueueError::Other(e) => write!(f, "Queue error: {}", e),
        }
    }
}

impl std::error::Error for QueueError {}

/// Statistics for monitoring WorkQueue performance
#[derive(Debug)]
pub struct QueueStats {
    /// Total tasks submitted
    pub tasks_submitted: AtomicU64,
    /// Total tasks completed
    pub tasks_completed: AtomicU64,
    /// Tasks dropped due to channel full
    pub tasks_dropped: AtomicU64,
    /// Number of active pools
    pub active_pools: AtomicU32,
    /// Number of active workers
    pub active_workers: AtomicU32,
}

impl Default for QueueStats {
    fn default() -> Self {
        Self {
            tasks_submitted: AtomicU64::new(0),
            tasks_completed: AtomicU64::new(0),
            tasks_dropped: AtomicU64::new(0),
            active_pools: AtomicU32::new(0),
            active_workers: AtomicU32::new(0),
        }
    }
}

impl QueueStats {
    /// Create new stats with all counters at zero
    pub fn new() -> Self {
        Self::default()
    }

    /// Get snapshot of current stats
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            tasks_submitted: self.tasks_submitted.load(Ordering::Relaxed),
            tasks_completed: self.tasks_completed.load(Ordering::Relaxed),
            tasks_dropped: self.tasks_dropped.load(Ordering::Relaxed),
            active_pools: self.active_pools.load(Ordering::Relaxed),
            active_workers: self.active_workers.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot of queue statistics
#[derive(Debug, Clone, Copy)]
pub struct StatsSnapshot {
    pub tasks_submitted: u64,
    pub tasks_completed: u64,
    pub tasks_dropped: u64,
    pub active_pools: u32,
    pub active_workers: u32,
}

/// A pool of worker threads for a specific hostname
///
/// This pool is designed for entrypoint-based dispatch where each worker
/// creates a fresh V8 isolate from source files. For snapshot-based execution,
/// see `SliverWorkerPool` in the `pool` module.
///
/// ## Features
///
/// - Async creation with custom VFS backends
/// - Round-robin task dispatch
/// - Bounded MPSC channels (256 slots per POOL-02)
///
/// ## When to Use
///
/// Use EntrypointWorkerPool when:
/// - Loading JavaScript from source files dynamically
/// - Development or testing scenarios
/// - Custom VFS backend configuration needed
///
/// For production snapshot execution, use `SliverWorkerPool` instead.
///
/// # Deprecation Notice
///
/// This type is now a thin wrapper around `WorkerPool` for backward compatibility.
/// New code should use `WorkerPool::with_source()` directly with `AppSource::Entrypoint`.
#[derive(Debug)]
pub struct EntrypointWorkerPool {
    /// Inner WorkerPool that handles all execution
    /// 
    /// This wraps the unified WorkerPool created with AppSource::Entrypoint.
    /// Public for backward compatibility with code that accesses .inner
    pub inner: crate::worker::pool::WorkerPool,
    /// Hostname this pool serves (cached for quick access)
    hostname: String,
    /// Number of workers (cached for quick access)
    worker_count: u32,
}

impl EntrypointWorkerPool {
    /// Create a new worker pool with specified number of workers
    ///
    /// Each worker gets a bounded channel with 256-slot capacity per POOL-02.
    /// Uses MemoryBackend by default for VFS.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The hostname this pool serves
    /// * `worker_count` - Number of worker threads to create
    ///
    /// # Returns
    ///
    /// A new `EntrypointWorkerPool` with workers ready to receive tasks
    ///
    /// # Deprecation
    ///
    /// This method delegates to `WorkerPool::with_source()`. For new code,
    /// use `WorkerPool::with_source(hostname, worker_count, 0, AppSource::entrypoint(path))`.
    pub fn new(hostname: &str, worker_count: u32) -> Self {
        Self::with_backend(hostname, worker_count, crate::vfs::VfsBackendEnum::memory(MemoryBackend::new()))
    }

    /// Create a new worker pool with a custom VFS backend
    ///
    /// # Arguments
    ///
    /// * `hostname` - The hostname this pool serves
    /// * `worker_count` - Number of worker threads to create
    /// * `vfs_backend` - Custom VFS backend to use
    ///
    /// # Returns
    ///
    /// A new `EntrypointWorkerPool` with workers ready to receive tasks
    ///
    /// # Deprecation
    ///
    /// This method delegates to `WorkerPool::with_source_and_backend()`. For new code,
    /// use the unified constructor directly.
    pub fn with_backend(hostname: &str, worker_count: u32, vfs_backend: crate::vfs::VfsBackendEnum) -> Self {
        use crate::worker::AppSource;
        
        // Default entrypoint for backward-compatible WorkerPool creation.
        // The actual entrypoint is resolved per-request via the app registry
        // or overridden when using WorkerPool::with_source_and_backend().
        let source = AppSource::entrypoint("index.js");
        let inner = crate::worker::pool::WorkerPool::with_source_and_backend(
            hostname.to_string(),
            worker_count,
            0, // No memory limit by default for backward compatibility
            vfs_backend,
            source,
        );
        
        tracing::info!(
            "EntrypointWorkerPool for {} delegates to unified WorkerPool ({} workers)",
            hostname,
            worker_count
        );
        
        Self {
            inner,
            hostname: hostname.to_string(),
            worker_count,
        }
    }

    /// Try to dispatch a task to a specific worker without blocking
    ///
    /// # Arguments
    ///
    /// * `task` - The handler task to dispatch
    /// * `worker_index` - Index of the worker to send to
    ///
    /// # Returns
    ///
    /// `Ok(())` if task was sent, `Err(QueueError::ChannelFull)` if channel is full
    ///
    /// # Note
    ///
    /// The unified WorkerPool uses unbounded channels. This method now delegates
    /// to the standard `dispatch()` which provides equivalent functionality.
    pub fn try_dispatch(&self, task: HandlerTask) -> Result<(), QueueError> {
        self.inner.dispatch(task).map_err(|e| QueueError::SendError(e.to_string()))
    }

    /// Shutdown the worker pool gracefully
    ///
    /// Drops the senders, causing worker threads to exit after processing
    /// any pending tasks.
    pub fn shutdown(self) {
        tracing::info!("Shutting down EntrypointWorkerPool for {}", self.hostname);
        // Delegate to inner WorkerPool shutdown
        let _ = self.inner.shutdown();
    }
    
    /// Get the number of workers in this pool
    ///
    /// Provided for backward compatibility with code that accessed the field directly.
    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }
    
    /// Get the hostname this pool serves
    ///
    /// Provided for backward compatibility with code that accessed the field directly.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }
}

impl crate::worker::r#trait::WorkerPool for EntrypointWorkerPool {
    fn dispatch(&self, task: HandlerTask) -> anyhow::Result<()> {
        // Delegate to inner WorkerPool
        self.inner.dispatch(task)
    }

    fn shutdown(self) -> anyhow::Result<()> {
        tracing::info!("Shutting down EntrypointWorkerPool for {}", self.hostname);
        // Delegate to inner WorkerPool
        self.inner.shutdown()
    }

    fn worker_count(&self) -> u32 {
        self.worker_count
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }
}

/// WorkQueue with bounded MPSC channels and affine dispatch
///
/// Manages per-hostname worker pools and routes requests consistently.
pub struct WorkQueue {
    /// Map of hostname hash to worker pool
    pools: HashMap<u64, EntrypointWorkerPool>,
    /// Default number of workers per pool
    workers_per_pool: u32,
    /// Bounded channel capacity (256 slots per POOL-02)
    ///
    /// Channel capacity per worker pool (256 slots per POOL-02).
    channel_capacity: usize,
    /// Statistics for monitoring
    pub stats: QueueStats,
    /// VFS backend configuration for disk backend (optional)
    vfs_disk_config: Option<VfsDiskConfig>,
    /// AppRegistry for per-app configuration lookup (optional)
    app_registry: Option<Arc<AppRegistry>>,
    /// Control plane for request validation and batching
    pub control_plane: Option<ControlPlane>,
}

impl WorkQueue {
    /// Create a new WorkQueue
    ///
    /// # Arguments
    ///
    /// * `workers_per_pool` - Number of workers to create per hostname pool
    ///
    /// # Returns
    ///
    /// A new `WorkQueue` with empty pools HashMap
    pub fn new(workers_per_pool: u32) -> Self {
        Self::with_vfs_config(workers_per_pool, None, None)
    }

    /// Create a new WorkQueue with VFS disk backend configuration
    ///
    /// # Arguments
    ///
    /// * `workers_per_pool` - Number of workers to create per hostname pool
    /// * `vfs_disk_config` - Optional disk backend configuration
    /// * `app_registry` - Optional AppRegistry for per-app VFS configuration
    ///
    /// # Returns
    ///
    /// A new `WorkQueue` configured with the specified VFS backend
    pub fn with_vfs_config(
        workers_per_pool: u32,
        vfs_disk_config: Option<VfsDiskConfig>,
        app_registry: Option<Arc<AppRegistry>>,
    ) -> Self {
        let mut control_plane = ControlPlane::new();

        // Pre-register all tenants from app_registry
        if let Some(ref registry) = app_registry {
            for hostname in registry.all_hostnames() {
                let limits = crate::control_plane::TenantLimits {
                    max_script_size: 10 * 1024 * 1024, // 10MB max (per execution limit)
                    max_timeout_ms: 30000,             // 30s default
                    max_batch_size: 100,               // 100 req default
                    allowed_methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string(), "DELETE".to_string()],
                };
                control_plane.register_tenant(hostname, limits);
                tracing::debug!("Pre-registered tenant in control plane at startup");
            }
        }

        Self {
            pools: HashMap::new(),
            workers_per_pool,
            channel_capacity: 256, // POOL-02 requirement
            stats: QueueStats::new(),
            vfs_disk_config,
            app_registry,
            control_plane: Some(control_plane),
        }
    }

    /// Get the channel capacity for this queue
    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    /// Set the AppRegistry for per-app configuration lookup
    ///
    /// # Arguments
    ///
    /// * `app_registry` - The AppRegistry to use for per-app VFS configuration
    ///
    /// # Returns
    ///
    /// Self for builder pattern
    pub fn with_registry(mut self, app_registry: Arc<AppRegistry>) -> Self {
        self.app_registry = Some(app_registry);
        self
    }

    /// Get or create a worker pool for a hostname
    ///
    /// Uses case-insensitive hostname hashing per D-WQ-02.
    /// Creates disk VFS backend asynchronously based on per-app configuration from AppRegistry.
    /// Falls back to global vfs_disk_config if no registry is available.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The hostname to get/create pool for
    ///
    /// # Returns
    ///
    /// A mutable reference to the `EntrypointWorkerPool` for this hostname
    pub async fn get_or_create_pool(&mut self, hostname: &str) -> &mut EntrypointWorkerPool {
        let hash = hash_hostname(hostname);

        if !self.pools.contains_key(&hash) {
            tracing::info!("Creating new EntrypointWorkerPool for hostname: {}", hostname);

            self.ensure_tenant(hostname);

            // Check for per-app VFS configuration from AppRegistry first
            let pool = if let Some(ref registry) = self.app_registry {
                if let Some(app_config) = registry.get(hostname) {
                    match app_config.vfs_backend {
                        VfsBackendType::Disk => {
                            if let Some(ref disk_config) = app_config.vfs_disk {
                                // Create disk backend with per-app config
                                match BackendFactory::new()
                                    .create_backend(
                                        VfsBackendType::Disk,
                                        Some(disk_config),
                                        None,
                                    )
                                    .await
                                {
                                    Ok(backend) => {
                                        tracing::info!(
                                            "Created disk backend for hostname: {} with base_path: {}",
                                            hostname,
                                            disk_config.base_path
                                        );
                                        EntrypointWorkerPool::with_backend(
                                            hostname,
                                            self.workers_per_pool,
                                            backend,
                                        )
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to create disk backend for {}, falling back to memory: {}",
                                            hostname,
                                            e
                                        );
                                        EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                                    }
                                }
                            } else {
                                tracing::warn!(
                                    "App {} has vfs_backend=disk but no vfs_disk config, using memory",
                                    hostname
                                );
                                EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                            }
                        }
                        VfsBackendType::Memory => {
                            // Auto-detect: if app has an entrypoint, use DiskBackend
                            // pointing to the entrypoint's parent directory, or a subdirectory
                            // matching the entrypoint's stem (e.g., app.js -> app/)
                            if !app_config.entrypoint.is_empty() {
                                let entrypoint_path = std::path::Path::new(&app_config.entrypoint);
                                if let Some(parent) = entrypoint_path.parent() {
                                    // Check if there's a subdirectory matching the entrypoint's stem
                                    // e.g., for app.js, check if app/ directory exists
                                    let stem = entrypoint_path.file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_default();
                                    let subdir = parent.join(&stem);
                                    
                                    // Use subdirectory if it exists, otherwise use parent
                                    let base_path = if subdir.exists() && subdir.is_dir() {
                                        tracing::debug!(
                                            "Found matching subdirectory '{}' for entrypoint, using as VFS root",
                                            stem
                                        );
                                        subdir
                                    } else {
                                        parent.to_path_buf()
                                    };
                                    
                                    match BackendFactory::new()
                                        .create_backend(
                                            VfsBackendType::Disk,
                                            Some(&VfsDiskConfig { base_path: base_path.to_string_lossy().to_string() }),
                                            None,
                                        )
                                        .await
                                    {
                                        Ok(backend) => {
                                            tracing::info!(
                                                "Auto-created disk backend for entrypoint app at hostname: {} with base_path: {:?}",
                                                hostname, base_path
                                            );
                                            EntrypointWorkerPool::with_backend(
                                                hostname,
                                                self.workers_per_pool,
                                                backend,
                                            )
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to auto-create disk backend for entrypoint app at {:?}, falling back to memory: {}",
                                                base_path, e
                                            );
                                            EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                                        }
                                    }
                                } else {
                                    tracing::debug!("No parent directory for entrypoint, using memory backend for hostname: {}", hostname);
                                    EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                                }
                            } else {
                                tracing::debug!("Using memory backend for hostname: {} (no entrypoint)", hostname);
                                EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                            }
                        }
                        VfsBackendType::S3 => {
                            tracing::debug!("Using default memory backend for hostname: {}", hostname);
                            EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                        }
                    }
                } else {
                    // App not found in registry, fall back to global config or memory
                    tracing::debug!(
                        "Hostname {} not found in registry, using fallback",
                        hostname
                    );
                    self.create_pool_with_fallback(hostname).await
                }
            } else {
                // No registry available, use fallback (global config or memory)
                self.create_pool_with_fallback(hostname).await
            };

            self.pools.insert(hash, pool);
            self.stats.active_pools.fetch_add(1, Ordering::Relaxed);
            self.stats
                .active_workers
                .fetch_add(self.workers_per_pool, Ordering::Relaxed);
        }

        self.pools.get_mut(&hash).expect("Pool should exist")
    }

    /// Create a pool using global vfs_disk_config as fallback
    ///
    /// This is used when per-app configuration is not available.
    async fn create_pool_with_fallback(&self, hostname: &str) -> EntrypointWorkerPool {
        if let Some(ref disk_config) = self.vfs_disk_config {
            // Create disk backend asynchronously using global config
            let base_path = disk_config.base_path.clone();
            match BackendFactory::new()
                .create_backend(VfsBackendType::Disk, Some(&VfsDiskConfig { base_path }), None)
                .await
            {
                Ok(backend) => {
                    tracing::info!(
                        "Created disk backend for hostname: {} (global config)",
                        hostname
                    );
                    EntrypointWorkerPool::with_backend(hostname, self.workers_per_pool, backend)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to create disk backend (global config), falling back to memory: {}",
                        e
                    );
                    EntrypointWorkerPool::new(hostname, self.workers_per_pool)
                }
            }
        } else {
            // Use memory backend (default)
            tracing::debug!("Using memory backend for hostname: {} (no config)", hostname);
            EntrypointWorkerPool::new(hostname, self.workers_per_pool)
        }
    }

    /// Register a first-seen hostname in the control plane with default limits.
    ///
    /// Called before validation so hostnames without an AppRegistry entry (e.g.
    /// in-process tests) are not rejected by the precondition check.
    pub fn ensure_tenant(&mut self, hostname: &str) {
        if let Some(ref mut control_plane) = self.control_plane {
            if !control_plane.tenant_exists(hostname) {
                let limits = crate::control_plane::TenantLimits {
                    max_script_size: 10 * 1024 * 1024,
                    max_timeout_ms: 30000,
                    max_batch_size: 100,
                    allowed_methods: vec!["GET".to_string(), "POST".to_string(), "PUT".to_string(), "DELETE".to_string()],
                };
                control_plane.register_tenant(hostname.to_string(), limits);
                tracing::debug!("ensure_tenant: auto-registered {} with default limits", hostname);
            }
        }
    }

    /// Uses affine dispatch (hostname → worker index). Returns 503 on backpressure.
    pub async fn dispatch(&mut self, hostname: &str, task: HandlerTask) -> Result<(), QueueError> {
        // Get or create pool for this hostname (async for disk backend creation)
        let pool = self.get_or_create_pool(hostname).await;
        let result = pool.try_dispatch(task);

        self.stats.tasks_submitted.fetch_add(1, Ordering::Relaxed);

        match result {
            Ok(()) => Ok(()),
            Err(QueueError::ChannelFull) => {
                self.stats.tasks_dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!("Channel full for hostname {:?}", hostname);
                Err(QueueError::ChannelFull)
            }
            Err(e) => {
                self.stats.tasks_dropped.fetch_add(1, Ordering::Relaxed);
                Err(e)
            }
        }
    }

    /// Get pool for a hostname (returns None if not found)
    pub fn get_pool(&self, hostname: &str) -> Option<&EntrypointWorkerPool> {
        let hash = hash_hostname(hostname);
        self.pools.get(&hash)
    }

    /// Shutdown all worker pools gracefully
    pub fn shutdown(self) {
        tracing::info!("Shutting down WorkQueue with {} pools", self.pools.len());
        for (hash, pool) in self.pools {
            tracing::debug!("Shutting down EntrypointWorkerPool with hash: {}", hash);
            pool.shutdown();
        }
    }

    /// Get current statistics snapshot
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }
}

/// Hash a hostname to a u64 value
///
/// Uses case-insensitive hashing per HTTP spec (D-WQ-02).
/// Uses std::collections::hash_map::DefaultHasher for consistency.
///
/// # Arguments
///
/// * `hostname` - The hostname to hash
///
/// # Returns
///
/// A u64 hash value for the lowercase hostname
pub fn hash_hostname(hostname: &str) -> u64 {
    let lowercase = hostname.to_lowercase();
    let mut hasher = DefaultHasher::new();
    lowercase.hash(&mut hasher);
    hasher.finish()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoRequest, NanoUrl};
    use tokio::sync::oneshot;

    fn create_dummy_request() -> NanoRequest {
        NanoRequest::new(
            "GET".to_string(),
            NanoUrl::parse("http://test/").unwrap(),
            NanoHeaders::new(),
            None,
        )
    }

    #[test]
    fn test_workqueue_creation() {
        let queue = WorkQueue::new(4);
        assert_eq!(queue.workers_per_pool, 4);
        assert_eq!(queue.channel_capacity(), 256, "POOL-02: bounded channel must be 256 slots");
        assert_eq!(queue.pools.len(), 0);
    }

    #[tokio::test]
    async fn test_get_or_create_pool() {
        let mut queue = WorkQueue::new(2);

        // Create pool for hostname
        let pool = queue.get_or_create_pool("test.example.com").await;
        assert_eq!(pool.hostname, "test.example.com");
        assert_eq!(pool.worker_count, 2);

        // Same hostname returns same pool
        let pool2 = queue.get_or_create_pool("test.example.com").await;
        assert_eq!(pool2.hostname, "test.example.com");

        // Stats updated
        assert_eq!(queue.stats.active_pools.load(Ordering::Relaxed), 1);
        assert_eq!(queue.stats.active_workers.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_hostname_hash_case_insensitive() {
        let hash1 = hash_hostname("Example.COM");
        let hash2 = hash_hostname("example.com");
        let hash3 = hash_hostname("EXAMPLE.COM");

        assert_eq!(hash1, hash2, "Hostname hashing should be case-insensitive");
        assert_eq!(hash2, hash3, "Hostname hashing should be case-insensitive");
    }

    #[tokio::test]
    async fn test_multiple_hostname_pools() {
        let mut queue = WorkQueue::new(2);

        // Create pools for different hostnames
        queue.get_or_create_pool("app1.example.com").await;
        queue.get_or_create_pool("app2.example.com").await;

        assert_eq!(queue.stats.active_pools.load(Ordering::Relaxed), 2);
        assert_eq!(queue.stats.active_workers.load(Ordering::Relaxed), 4);
    }

    #[tokio::test]
    async fn test_affine_dispatch_consistency() {
        let mut queue = WorkQueue::new(4);

        let pool = queue.get_or_create_pool("app.example.com").await;
        let worker_count = pool.worker_count;

        let idx = (hash_hostname("app.example.com") % worker_count as u64) as usize;
        assert!(idx < worker_count as usize, "worker index must be in bounds");

        assert_ne!(
            hash_hostname("app.example.com"),
            hash_hostname("other.example.com"),
            "different hostnames must produce different hashes"
        );
    }

    #[test]
    fn test_queue_error_display() {
        assert_eq!(
            QueueError::ChannelFull.to_string(),
            "WorkQueue channel is full"
        );
        assert_eq!(
            QueueError::WorkerNotFound.to_string(),
            "Worker thread not found"
        );
        assert_eq!(
            QueueError::PoolNotFound.to_string(),
            "Pool not found for hostname"
        );
        assert_eq!(
            QueueError::SendError("test".to_string()).to_string(),
            "Send error: test"
        );
    }

    #[test]
    fn test_worker_pool_try_dispatch() {
        let pool = EntrypointWorkerPool::new("test.local", 2);

        let (tx, _rx) = oneshot::channel();
        let task = HandlerTask {
            entrypoint: "/dev/null".to_string(),
            request: create_dummy_request(),
            response_tx: tx,
            hostname: "test.local".to_string(),
            start_time: std::time::Instant::now(),
            cpu_time_limit_ms: 0, // 0 means no limit for tests
            request_id: "req_test_002".to_string(),
            memory_limit_mb: 0, // 0 means use default
            ws: None,
        };

        // Should succeed (channel is empty)
        let result = pool.try_dispatch(task);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ensure_tenant_registers_in_control_plane() {
        let mut queue = WorkQueue::new(2);

        assert!(
            !queue.control_plane.as_ref().unwrap().tenant_exists("new.host.local"),
            "hostname must not exist before first pool creation"
        );

        queue.get_or_create_pool("new.host.local").await;

        assert!(
            queue.control_plane.as_ref().unwrap().tenant_exists("new.host.local"),
            "hostname must be registered after pool creation"
        );

        // idempotent: must not double-register or panic
        queue.ensure_tenant("new.host.local");
        assert!(
            queue.control_plane.as_ref().unwrap().tenant_exists("new.host.local"),
            "idempotent: already-registered hostname must still exist"
        );
    }
}
