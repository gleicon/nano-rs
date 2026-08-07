//! SliverWorkerPool — thin wrapper around WorkerPool for sliver-based apps.

use super::pool::WorkerPool;
use crate::vfs::MemoryBackend;
use crate::worker::HandlerTask;
use anyhow::Result;
use tracing::info;

pub struct SliverWorkerPool {
    inner: WorkerPool,
    pub hostname: String,
    pub worker_count: u32,
    unpacked_sliver: crate::sliver::UnpackedSliver,
}

impl std::fmt::Debug for SliverWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliverWorkerPool")
            .field("worker_count", &self.worker_count)
            .field("hostname", &self.hostname)
            .finish()
    }
}

impl SliverWorkerPool {
    pub fn new(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked_sliver: crate::sliver::UnpackedSliver,
    ) -> Self {
        use crate::worker::AppSource;

        let source = AppSource::sliver(unpacked_sliver.clone());
        let vfs_backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let inner = WorkerPool::with_source_and_backend(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            vfs_backend,
            source,
        );

        info!(
            "SliverWorkerPool for {} created with {} workers",
            hostname, worker_count
        );

        Self {
            inner,
            hostname,
            worker_count,
            unpacked_sliver,
        }
    }

    pub fn dispatch(&self, task: HandlerTask) -> Result<()> {
        self.inner.dispatch(task)
    }

    pub fn shutdown(self) -> Result<()> {
        info!("Shutting down SliverWorkerPool for {}", self.hostname);
        self.inner.shutdown()
    }

    pub fn worker_count(&self) -> u32 {
        self.worker_count
    }
    pub fn hostname(&self) -> &str {
        &self.hostname
    }
    pub fn sliver_data(&self) -> &crate::sliver::UnpackedSliver {
        &self.unpacked_sliver
    }
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
        self.worker_count()
    }
    fn hostname(&self) -> &str {
        self.hostname()
    }
}
