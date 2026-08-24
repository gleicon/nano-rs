//! SliverWorkerPool — thin wrapper around WorkerPool for sliver-based apps.

use super::pool::WorkerPool;
use crate::vfs::MemoryBackend;
use crate::worker::HandlerTask;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
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

/// A hot-swappable holder for a sliver app's worker pool — the "router-level"
/// blue-green deploy.
///
/// Serving reads [`current`](Self::current) to get the pool in effect and
/// dispatches to it. [`hotswap`](Self::hotswap) builds a *fully-warm* new pool
/// from a new bundle, atomically repoints the slot, and returns the old pool.
/// New requests immediately route to the new pool; the old one receives no
/// further dispatches and drains — dropping it exits its workers once in-flight
/// requests (which hold their own `Arc` clone) complete. The hostname never
/// changes, so routing and the client-facing subdomain are untouched.
pub struct SliverPoolSlot {
    current: std::sync::RwLock<Arc<SliverWorkerPool>>,
    hostname: String,
    worker_count: u32,
    memory_limit_mb: u32,
}

impl std::fmt::Debug for SliverPoolSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliverPoolSlot")
            .field("hostname", &self.hostname)
            .field("worker_count", &self.worker_count)
            .finish()
    }
}

impl SliverPoolSlot {
    /// Build the initial pool and wrap it in a swappable slot.
    pub fn new(
        hostname: String,
        worker_count: u32,
        memory_limit_mb: u32,
        unpacked: crate::sliver::UnpackedSliver,
    ) -> Arc<Self> {
        let pool = Arc::new(SliverWorkerPool::new(
            hostname.clone(),
            worker_count,
            memory_limit_mb,
            unpacked,
        ));
        Arc::new(Self {
            current: std::sync::RwLock::new(pool),
            hostname,
            worker_count,
            memory_limit_mb,
        })
    }

    /// The pool currently serving traffic. Cheap: clones an `Arc`.
    pub fn current(&self) -> Arc<SliverWorkerPool> {
        self.current
            .read()
            .expect("SliverPoolSlot poisoned")
            .clone()
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Blue-green swap. Builds a new warm pool from `unpacked`, repoints the slot
    /// to it, and returns the old pool so the caller can control draining. The
    /// hostname and worker/memory sizing carry over from this slot.
    pub fn hotswap(&self, unpacked: crate::sliver::UnpackedSliver) -> Arc<SliverWorkerPool> {
        let new_pool = Arc::new(SliverWorkerPool::new(
            self.hostname.clone(),
            self.worker_count,
            self.memory_limit_mb,
            unpacked,
        ));
        let mut guard = self.current.write().expect("SliverPoolSlot poisoned");
        std::mem::replace(&mut *guard, new_pool)
    }

    /// Swap, then let the old pool dry for `drain` before dropping it (which
    /// exits its workers). Must be called from within a Tokio runtime — the drain
    /// wait runs on a spawned task so the swap itself returns immediately.
    pub fn hotswap_and_drain(&self, unpacked: crate::sliver::UnpackedSliver, drain: Duration) {
        let old = self.hotswap(unpacked);
        let host = self.hostname.clone();
        tokio::spawn(async move {
            tokio::time::sleep(drain).await;
            drop(old); // senders drop → old workers exit after finishing in-flight work
            info!("Drained and retired previous sliver pool for {}", host);
        });
    }
}
