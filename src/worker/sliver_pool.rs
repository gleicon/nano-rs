//! SliverWorkerPool — snapshot-restored worker pool
//!
//! This is a thin wrapper around WorkerPool for backward compatibility.
//! New code should use WorkerPool::with_source() with AppSource::Sliver.

use super::pool::WorkerPool;
use crate::worker::HandlerTask;
use crate::vfs::MemoryBackend;
use anyhow::Result;
use tracing::info;

/// Worker pool for sliver-based (snapshot-restored) applications
///
/// This specialized worker pool creates isolates from V8 heap snapshots
/// rather than fresh isolates. It also restores VFS state from the sliver.
///
/// # Design
///
/// - Each worker restores its isolate from the snapshot blob
/// - VFS entries are restored before the worker accepts tasks
/// - Falls back to fresh isolate if snapshot restoration fails
/// - Shares the same dispatch interface as regular WorkerPool
///
/// # Deprecation Notice
///
/// This type is now a thin wrapper around `WorkerPool` for backward compatibility.
/// New code should use `WorkerPool::with_source()` directly with `AppSource::Sliver`.
pub struct SliverWorkerPool {
    /// Inner WorkerPool that handles all execution
    ///
    /// This wraps the unified WorkerPool created with AppSource::Sliver.
    inner: WorkerPool,
    /// Hostname this pool serves (cached for quick access)
    pub hostname: String,
    /// Number of workers (cached for quick access)
    pub worker_count: u32,
    /// Unpacked sliver data (kept for reference/debugging)
    unpacked_sliver: crate::sliver::UnpackedSliver,
    /// Optional temp entrypoint path (kept for reference/debugging)
    temp_entrypoint: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for SliverWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliverWorkerPool")
            .field("worker_count", &self.worker_count)
            .field("hostname", &self.hostname)
            .field("unpacked_sliver", &self.unpacked_sliver.metadata.hostname)
            .field("temp_entrypoint", &self.temp_entrypoint)
            .finish()
    }
}

impl SliverWorkerPool {
    /// Create a new sliver worker pool with restored isolates
    ///
    /// This now delegates to the unified `WorkerPool::with_source()` constructor
    /// for consistent behavior across all pool types.
    ///
    /// # Arguments
    ///
    /// * `hostname` - Hostname this pool serves (for logging)
    /// * `worker_count` - Number of worker threads to spawn
    /// * `memory_limit_mb` - Memory limit per isolate in MB (0 = no limit)
    /// * `unpacked_sliver` - The unpacked sliver containing snapshot and VFS data
    ///
    /// # Returns
    ///
    /// A new SliverWorkerPool with N workers restored from snapshot
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source()`. For new code,
    /// use `WorkerPool::with_source(hostname, worker_count, memory_limit_mb, AppSource::sliver(data))`.
    pub fn new(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked_sliver: crate::sliver::UnpackedSliver,
    ) -> Self {
        Self::with_temp_entrypoint(
            hostname,
            worker_count,
            memory_limit_mb,
            unpacked_sliver,
            None,
        )
    }

    /// Create a new sliver worker pool with a temp entrypoint path
    ///
    /// This variant is used when the sliver VFS has been extracted to a temp
    /// directory, and the JS entrypoint should be read from that location.
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source()`. For new code,
    /// use `WorkerPool::with_source()` with `AppSource::sliver_with_temp(data, temp)`.
    pub fn with_temp_entrypoint(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked_sliver: crate::sliver::UnpackedSliver,
        temp_entrypoint: Option<std::path::PathBuf>,
    ) -> Self {
        use crate::worker::AppSource;
        use crate::vfs::MemoryBackend;

        let source = if let Some(temp) = temp_entrypoint.clone() {
            AppSource::sliver_with_temp(unpacked_sliver.clone(), temp)
        } else {
            AppSource::sliver(unpacked_sliver.clone())
        };

        let vfs_backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let inner = WorkerPool::with_source_and_backend(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            vfs_backend,
            source,
        );

        info!(
            "SliverWorkerPool for {} created with {} workers (delegates to unified WorkerPool)",
            hostname, worker_count
        );

        Self {
            inner,
            hostname: hostname.clone(),
            worker_count,
            unpacked_sliver,
            temp_entrypoint,
        }
    }

    /// Create a new sliver worker pool with a specific VFS backend
    ///
    /// # Deprecation
    ///
    /// This method now delegates to `WorkerPool::with_source_and_backend()`.
    pub fn with_backend(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        vfs_backend: crate::vfs::VfsBackendEnum,
        unpacked_sliver: crate::sliver::UnpackedSliver,
        temp_entrypoint: Option<std::path::PathBuf>,
    ) -> Self {
        use crate::worker::AppSource;

        let source = if let Some(temp) = temp_entrypoint.clone() {
            AppSource::sliver_with_temp(unpacked_sliver.clone(), temp)
        } else {
            AppSource::sliver(unpacked_sliver.clone())
        };

        let inner = WorkerPool::with_source_and_backend(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            vfs_backend,
            source,
        );

        info!(
            "SliverWorkerPool for {} created with {} workers (custom backend)",
            hostname, worker_count
        );

        Self {
            inner,
            hostname: hostname.clone(),
            worker_count,
            unpacked_sliver,
            temp_entrypoint,
        }
    }

    pub fn dispatch(&self, task: HandlerTask) -> Result<()> {
        self.inner.dispatch(task)
    }

    pub fn shutdown(self) -> Result<()> {
        info!("Shutting down SliverWorkerPool for {}", self.hostname);
        self.inner.shutdown()
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

    /// Access the unpacked sliver data (for debugging/testing)
    pub fn sliver_data(&self) -> &crate::sliver::UnpackedSliver {
        &self.unpacked_sliver
    }

    /// Access the VFS backend (for testing VFS operations)
    pub fn vfs_backend(&self) -> &crate::vfs::VfsBackendEnum {
        &self.inner.vfs_backend
    }
}

impl crate::worker::r#trait::WorkerPool for SliverWorkerPool {
    fn dispatch(&self, task: HandlerTask) -> Result<()> {
        self.inner.dispatch(task)
    }

    fn shutdown(self) -> Result<()> {
        self.inner.shutdown()
    }

    fn worker_count(&self) -> u32 {
        self.worker_count
    }

    fn hostname(&self) -> &str {
        &self.hostname
    }
}

