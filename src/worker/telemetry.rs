//! Live per-isolate telemetry.
//!
//! Worker threads publish real runtime stats for each active V8 isolate here;
//! the admin plane (`GET /admin/isolates`) reads them back. This is the single
//! source of truth for live isolate stats — before it existed, the diagnostics
//! endpoint fabricated numbers.
//!
//! Lifecycle: a worker calls [`register_isolate`] when it creates an isolate and
//! holds the returned [`IsolateTelemetryGuard`] for the isolate's lifetime. When
//! the isolate is recycled or the worker exits, the guard drops and the entry is
//! removed — so the registry only ever reflects *live* isolates.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use dashmap::DashMap;

/// Live stats for one active isolate. Runtime counters are atomic so worker
/// threads update them without locking while the admin plane reads concurrently.
struct IsolateStats {
    hostname: String,
    worker_id: u32,
    created_at: Instant,
    request_count: AtomicU64,
    busy: AtomicBool,
    /// Last-observed V8 used-heap bytes; `0` means not yet measured.
    memory_bytes: AtomicUsize,
    env_keys: Vec<String>,
}

fn registry() -> &'static DashMap<String, Arc<IsolateStats>> {
    static REGISTRY: OnceLock<DashMap<String, Arc<IsolateStats>>> = OnceLock::new();
    REGISTRY.get_or_init(DashMap::new)
}

/// RAII guard tying a telemetry entry to an isolate's lifetime. Dropping it
/// removes the entry, so a recycled or dead isolate never lingers in the registry.
pub struct IsolateTelemetryGuard {
    isolate_id: String,
}

impl Drop for IsolateTelemetryGuard {
    fn drop(&mut self) {
        registry().remove(&self.isolate_id);
    }
}

/// Register a freshly created isolate. Returns a guard that deregisters on drop.
pub fn register_isolate(
    isolate_id: String,
    hostname: String,
    worker_id: u32,
    env_keys: Vec<String>,
) -> IsolateTelemetryGuard {
    registry().insert(
        isolate_id.clone(),
        Arc::new(IsolateStats {
            hostname,
            worker_id,
            created_at: Instant::now(),
            request_count: AtomicU64::new(0),
            busy: AtomicBool::new(false),
            memory_bytes: AtomicUsize::new(0),
            env_keys,
        }),
    );
    IsolateTelemetryGuard { isolate_id }
}

/// Mark an isolate busy (processing a request) or idle.
pub fn mark_busy(isolate_id: &str, busy: bool) {
    if let Some(stats) = registry().get(isolate_id) {
        stats.busy.store(busy, Ordering::Relaxed);
    }
}

/// Record a completed request: bumps the request count, refreshes the observed
/// memory, and clears the busy flag.
pub fn record_request(isolate_id: &str, memory_bytes: usize) {
    if let Some(stats) = registry().get(isolate_id) {
        stats.request_count.fetch_add(1, Ordering::Relaxed);
        stats.memory_bytes.store(memory_bytes, Ordering::Relaxed);
        stats.busy.store(false, Ordering::Relaxed);
    }
}

/// An immutable point-in-time view of one isolate's live stats.
pub struct IsolateSnapshot {
    pub isolate_id: String,
    pub hostname: String,
    pub worker_id: u32,
    pub created_at: Instant,
    pub request_count: u64,
    pub busy: bool,
    /// `None` if no request has been served yet (no memory reading taken).
    pub memory_bytes: Option<usize>,
    pub env_keys: Vec<String>,
}

/// Snapshot every currently-live isolate. Used by the admin diagnostics collector.
pub fn snapshot() -> Vec<IsolateSnapshot> {
    registry()
        .iter()
        .map(|entry| {
            let s = entry.value();
            let mem = s.memory_bytes.load(Ordering::Relaxed);
            IsolateSnapshot {
                isolate_id: entry.key().clone(),
                hostname: s.hostname.clone(),
                worker_id: s.worker_id,
                created_at: s.created_at,
                request_count: s.request_count.load(Ordering::Relaxed),
                busy: s.busy.load(Ordering::Relaxed),
                memory_bytes: if mem == 0 { None } else { Some(mem) },
                env_keys: s.env_keys.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_snapshot_and_deregister_on_drop() {
        // Unique id so this test is isolated from other tests sharing the global
        // registry in parallel — assert only about our own entry, never the count.
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "iso_unit_test_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );

        {
            let _guard = register_isolate(
                id.clone(),
                "app.example.com".to_string(),
                7,
                vec!["API_KEY".to_string()],
            );
            mark_busy(&id, true);
            record_request(&id, 2 * 1024 * 1024);
            record_request(&id, 3 * 1024 * 1024);

            let snap = snapshot();
            let mine = snap
                .iter()
                .find(|s| s.isolate_id == id)
                .expect("registered");
            assert_eq!(mine.hostname, "app.example.com");
            assert_eq!(mine.worker_id, 7);
            assert_eq!(mine.request_count, 2, "two requests recorded");
            assert!(!mine.busy, "record_request clears busy");
            assert_eq!(mine.memory_bytes, Some(3 * 1024 * 1024), "latest memory");
            assert_eq!(mine.env_keys, vec!["API_KEY".to_string()]);
        }

        // Guard dropped → our entry is gone (no leak of this isolate).
        assert!(
            snapshot().iter().all(|s| s.isolate_id != id),
            "deregistered on drop"
        );
    }

    #[test]
    fn updates_on_missing_id_are_noops() {
        // Must not panic when the isolate isn't registered.
        mark_busy("nonexistent", true);
        record_request("nonexistent", 1024);
    }
}
