//! Data Plane: Optimized request execution.
//!
//! Per TigerStyle:
//! - NO validation checks in hot path (control plane handles validation)
//! - NO dynamic allocations (pre-allocated by control plane)
//! - Minimal branching (lookup tables over conditionals)
//! - Zero-copy where possible
//! - CPU sprints through batches

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use anyhow::{anyhow, Result};

// Thread-local storage for the worker thread's Tokio runtime handle.
// This allows fetch() and other async operations to access the runtime.
thread_local! {
    static WORKER_RUNTIME: RefCell<Option<tokio::runtime::Handle>> = const { RefCell::new(None) };
}

/// Get the worker thread's Tokio runtime handle if available.
pub fn with_worker_runtime<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&tokio::runtime::Handle) -> R,
{
    WORKER_RUNTIME.with(|runtime| runtime.borrow().as_ref().map(f))
}

/// Set the worker runtime handle for the current thread.
pub fn set_worker_runtime(handle: tokio::runtime::Handle) {
    WORKER_RUNTIME.with(|runtime| {
        *runtime.borrow_mut() = Some(handle);
    });
}

/// Code cache entry with modification time tracking.
struct CodeCacheEntry {
    code: Arc<str>,
    modified: SystemTime,
}

/// Thread-safe code cache to avoid disk reads on every request.
///
/// This significantly reduces latency for frequently accessed entrypoints
/// by caching the file contents in memory and only re-reading when the
/// file modification time changes.
static CODE_CACHE: RwLock<Option<HashMap<String, CodeCacheEntry>>> = RwLock::new(None);

/// V8 bytecode cache — stores compiled bytecode per entrypoint path.
///
/// On isolate recycle (every MAX_REQUESTS_PER_ISOLATE requests), feeding
/// back the bytecode via ConsumeCodeCache skips V8 parsing and compilation
/// entirely. Invalidated when the source file changes (tied to CODE_CACHE).
static BYTECODE_CACHE: RwLock<Option<HashMap<String, Arc<[u8]>>>> = RwLock::new(None);

fn bytecode_map_read() -> std::sync::RwLockReadGuard<'static, Option<HashMap<String, Arc<[u8]>>>> {
    BYTECODE_CACHE.read().unwrap()
}

fn bytecode_map_write() -> std::sync::RwLockWriteGuard<'static, Option<HashMap<String, Arc<[u8]>>>>
{
    let mut w = BYTECODE_CACHE.write().unwrap();
    if w.is_none() {
        *w = Some(HashMap::new());
    }
    w
}

/// Get cached bytecode for an entrypoint, if available.
pub fn get_bytecode_cache(entrypoint: &str) -> Option<Arc<[u8]>> {
    bytecode_map_read().as_ref()?.get(entrypoint).cloned()
}

/// Store compiled bytecode for an entrypoint.
pub fn set_bytecode_cache(entrypoint: &str, bytes: Arc<[u8]>) {
    bytecode_map_write()
        .as_mut()
        .unwrap()
        .insert(entrypoint.to_string(), bytes);
}

/// Invalidate bytecode cache for an entrypoint (called when source changes).
pub fn invalidate_bytecode_cache(entrypoint: &str) {
    if let Some(map) = bytecode_map_write().as_mut() {
        map.remove(entrypoint);
    }
}

/// Initialize the code cache on first use.
pub fn init_code_cache() {
    let mut cache = CODE_CACHE.write().unwrap();
    if cache.is_none() {
        *cache = Some(HashMap::new());
    }
}

/// Read code from cache or disk, with automatic cache invalidation.
///
/// This function caches file contents to avoid repeated disk reads,
/// which is a significant latency optimization (can save 1-5ms per request).
pub fn read_code_cached(entrypoint: &str) -> Result<Arc<str>> {
    // Fast path: check if we can read from cache
    {
        let cache_read = CODE_CACHE.read().unwrap();
        if let Some(cache) = cache_read.as_ref() {
            if let Some(entry) = cache.get(entrypoint) {
                // Check if file has been modified since we cached it
                if let Ok(metadata) = std::fs::metadata(entrypoint) {
                    if let Ok(modified) = metadata.modified() {
                        if modified == entry.modified {
                            // Cache hit - return cached code
                            return Ok(entry.code.clone());
                        }
                    }
                }
            }
        }
    }

    // Slow path: read from disk and update cache
    let code = std::fs::read_to_string(entrypoint)
        .map_err(|e| anyhow!("Failed to read entrypoint '{}': {}", entrypoint, e))?;

    let modified = std::fs::metadata(entrypoint)
        .and_then(|m| m.modified())
        .unwrap_or_else(|_| std::time::SystemTime::now());

    let code_arc: Arc<str> = code.into();

    // Update source cache; invalidate bytecode cache — source changed.
    {
        let mut cache_write = CODE_CACHE.write().unwrap();
        if cache_write.is_none() {
            *cache_write = Some(HashMap::new());
        }
        if let Some(cache) = cache_write.as_mut() {
            cache.insert(
                entrypoint.to_string(),
                CodeCacheEntry {
                    code: code_arc.clone(),
                    modified,
                },
            );
        }
    }
    invalidate_bytecode_cache(entrypoint);

    Ok(code_arc)
}

thread_local! {
    /// Shared with the active CpuTimeoutGuard so that any code on the worker thread
    /// (fetch, VFS, fs) can signal async I/O waits without holding the guard directly.
    static CPU_ASYNC_WAIT_FLAG: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Signal the active CPU timer that the current thread is about to wait for async I/O.
///
/// Prefer `AsyncWaitGuard::begin()` — the guard calls this in both the normal
/// path and on panic unwind, preventing indefinite timer suppression.
/// No-op if no `CpuTimeoutGuard` is active.
pub fn signal_cpu_async_waiting(waiting: bool) {
    CPU_ASYNC_WAIT_FLAG.with(|cell| {
        if let Some(flag) = cell.borrow().as_ref() {
            flag.store(waiting, Ordering::Relaxed);
        }
    });
}

/// RAII guard that pauses the CPU timer for the duration of a blocking async wait.
///
/// ```rust,ignore
/// let _w = AsyncWaitGuard::begin();
/// handle.block_on(async { ... }); // CPU timer paused; resumes when _w drops
/// ```
///
/// Panic-safe: Drop always calls `signal_cpu_async_waiting(false)` even when
/// an unwind skips the normal return path.
pub struct AsyncWaitGuard;

impl AsyncWaitGuard {
    pub fn begin() -> Self {
        signal_cpu_async_waiting(true);
        Self
    }
}

impl Drop for AsyncWaitGuard {
    fn drop(&mut self) {
        signal_cpu_async_waiting(false);
    }
}

/// Inner timer loop, extracted so it can be tested without a real V8 isolate.
///
/// Ticks every 1 ms. Each tick counts toward `limit_ms` only when
/// `is_async_waiting` is false. Calls `on_expire` when the CPU budget is
/// exhausted, or exits early when `should_stop` is set (called from Drop).
fn run_cpu_timer_thread(
    limit_ms: u32,
    is_async_waiting: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    on_expire: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut elapsed_ms: u64 = 0;
        while elapsed_ms < limit_ms as u64 {
            std::thread::sleep(std::time::Duration::from_millis(1));
            if should_stop.load(Ordering::Relaxed) {
                return;
            }
            if !is_async_waiting.load(Ordering::Relaxed) {
                elapsed_ms += 1;
            }
        }
        on_expire();
    })
}

/// Guard that enforces a CPU-time budget on a V8 isolate.
///
/// Measures actual JS execution time, not wall-clock time: call
/// `set_async_waiting(true)` before any async sleep and `set_async_waiting(false)`
/// when JS resumes. The timer thread skips accumulation while waiting.
///
/// Dropping the guard signals the timer thread to stop and joins it — it exits
/// within one 1 ms tick, so Drop never blocks for more than ~2 ms regardless
/// of whether the budget was consumed.
pub struct CpuTimeoutGuard {
    is_async_waiting: Arc<AtomicBool>,
    should_stop: Arc<AtomicBool>,
    terminated: Arc<AtomicBool>,
    timer_thread: Option<std::thread::JoinHandle<()>>,
    isolate_ptr: *mut v8::Isolate,
}

impl CpuTimeoutGuard {
    pub fn new(isolate: &mut v8::Isolate, limit_ms: u32) -> Self {
        let isolate_ptr: *mut v8::Isolate = isolate as *mut _;
        let is_async_waiting = Arc::new(AtomicBool::new(false));
        let should_stop = Arc::new(AtomicBool::new(false));
        let terminated = Arc::new(AtomicBool::new(false));

        // Cast to usize so the closure is Send. The pointer is valid for the timer
        // thread's entire lifetime because Drop joins the thread before the isolate
        // can be destroyed. V8's terminate_execution is safe to call from any thread.
        let isolate_usize = isolate_ptr as usize;
        let term_clone = terminated.clone();
        let on_expire = move || {
            term_clone.store(true, Ordering::SeqCst);
            let ptr = isolate_usize as *mut v8::Isolate;
            // SAFETY: see above.
            unsafe {
                if let Some(iso) = ptr.as_ref() {
                    iso.terminate_execution();
                }
            }
            crate::metrics::METRICS.record_cpu_timeout();
        };

        let timer_thread = run_cpu_timer_thread(
            limit_ms,
            is_async_waiting.clone(),
            should_stop.clone(),
            on_expire,
        );

        // Register the flag so fetch/VFS/fs bindings can signal waits via
        // signal_cpu_async_waiting() without holding the guard directly.
        CPU_ASYNC_WAIT_FLAG.with(|cell| {
            *cell.borrow_mut() = Some(is_async_waiting.clone());
        });

        Self {
            is_async_waiting,
            should_stop,
            terminated,
            timer_thread: Some(timer_thread),
            isolate_ptr,
        }
    }

    /// Signal whether the worker is currently waiting for async I/O.
    /// Pass `true` before sleeping in the event loop, `false` immediately after.
    pub fn set_async_waiting(&self, waiting: bool) {
        self.is_async_waiting.store(waiting, Ordering::Relaxed);
    }

    /// True if the CPU budget was exhausted and terminate_execution was called.
    pub fn is_terminated(&self) -> bool {
        self.terminated.load(Ordering::SeqCst)
    }
}

impl Drop for CpuTimeoutGuard {
    fn drop(&mut self) {
        // Deregister the thread-local signal hook before stopping the timer.
        CPU_ASYNC_WAIT_FLAG.with(|cell| {
            *cell.borrow_mut() = None;
        });
        // Signal early exit; the thread sees this within 1 ms.
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.timer_thread.take() {
            let _ = thread.join();
        }
        if self.terminated.load(Ordering::SeqCst) {
            // SAFETY: this drop runs on the worker thread that owns the isolate.
            unsafe {
                if let Some(isolate) = self.isolate_ptr.as_mut() {
                    isolate.cancel_terminate_execution();
                }
            }
        }
    }
}

// CpuTimeoutGuard is !Send + !Sync because it holds `isolate_ptr: *mut v8::Isolate`.
// No explicit impl needed — the compiler derives it from the raw pointer field.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_timer_stops_within_2ms_on_abort() {
        let waiting = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let fired2 = fired.clone();

        let handle = run_cpu_timer_thread(1000, waiting, stop.clone(), move || {
            fired2.store(true, Ordering::SeqCst);
        });

        let start = std::time::Instant::now();
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        assert!(
            start.elapsed() < std::time::Duration::from_millis(5),
            "timer should exit within 2 ms of abort; took {:?}",
            start.elapsed()
        );
        assert!(
            !fired.load(Ordering::SeqCst),
            "on_expire must not fire on abort"
        );
    }

    #[test]
    fn cpu_timer_pauses_while_async_waiting() {
        let waiting = Arc::new(AtomicBool::new(true)); // start paused
        let stop = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let fired2 = fired.clone();

        let limit_ms = 5u32;
        let handle = run_cpu_timer_thread(limit_ms, waiting.clone(), stop.clone(), move || {
            fired2.store(true, Ordering::SeqCst);
        });

        std::thread::sleep(std::time::Duration::from_millis(limit_ms as u64 * 3));
        assert!(
            !fired.load(Ordering::SeqCst),
            "must not fire while async_waiting=true"
        );

        // Release: should fire within ~limit_ms more ms
        waiting.store(false, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(limit_ms as u64 + 15));
        assert!(
            fired.load(Ordering::SeqCst),
            "must fire after CPU budget elapses"
        );

        handle.join().unwrap();
    }

    #[test]
    fn cpu_timer_fires_after_limit_ms_of_cpu_time() {
        let waiting = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let fired2 = fired.clone();

        let limit_ms = 5u32;
        let handle = run_cpu_timer_thread(limit_ms, waiting, stop, move || {
            fired2.store(true, Ordering::SeqCst);
        });

        std::thread::sleep(std::time::Duration::from_millis(limit_ms as u64 + 15));
        assert!(
            fired.load(Ordering::SeqCst),
            "must fire after limit_ms of CPU time"
        );

        handle.join().unwrap();
    }
}
