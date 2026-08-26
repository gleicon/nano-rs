//! `SliverPoolSlot`'s synchronization discipline (hot-swap safety).
//!
//! Reproduces the slot's skeleton with loom types, keeping the exact shape of
//! `src/worker/sliver_pool.rs`: a reader clones the inner `Arc` under a read lock
//! (`current()`) then uses it; a swapper replaces the inner `Arc` under a write
//! lock and drops the old one (`hotswap` + the drain task's drop). A stub `Pool`
//! stands in for `SliverWorkerPool` (which loom cannot run).
//!
//! The property is the implementation-level counterpart of the TLA+
//! `NoDispatchToDead` invariant in `../HotSwap.tla`:
//!
//!   A pool obtained from the slot stays alive for as long as the caller holds
//!   its `Arc` — a concurrent swap-and-drop can never tear it down underneath an
//!   in-flight request.

use loom::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use loom::sync::{Arc, RwLock};

/// Stand-in for `SliverWorkerPool`. `alive` starts true and is flipped false in
/// `Drop` — i.e. when the last `Arc` is released and the "workers" exit.
pub struct Pool {
    pub id: u64,
    pub alive: AtomicBool,
}

impl Pool {
    pub fn new(id: u64) -> Self {
        Pool {
            id,
            alive: AtomicBool::new(true),
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // If any reader ever observed this on a pool it was still holding, the
        // Arc discipline would be broken.
        self.alive.store(false, Ordering::Release);
    }
}

/// The synchronization skeleton of `SliverPoolSlot`: the current pool behind an
/// `RwLock<Arc<..>>`, with the generation atomic alongside it (as shipped).
pub struct Slot {
    current: RwLock<Arc<Pool>>,
    generation: AtomicU64,
}

impl Slot {
    pub fn new(pool: Pool) -> Arc<Self> {
        Arc::new(Slot {
            current: RwLock::new(Arc::new(pool)),
            generation: AtomicU64::new(0),
        })
    }

    /// `current()`: clone the Arc of the pool in effect, under a read lock.
    pub fn current(&self) -> Arc<Pool> {
        self.current.read().unwrap().clone()
    }

    /// `hotswap()`: install a new pool under a write lock, return the old Arc.
    pub fn hotswap(&self, pool: Pool) -> Arc<Pool> {
        let mut guard = self.current.write().unwrap();
        self.generation.fetch_add(1, Ordering::Release);
        std::mem::replace(&mut *guard, Arc::new(pool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that captured a pool from the slot must always see it alive while
    /// it still holds the `Arc`, even as another thread swaps in a new pool and
    /// drops the old one. Loom explores every interleaving; a torn-down-underneath
    /// race would trip the assertion in some schedule.
    #[test]
    fn reader_never_observes_a_dropped_pool() {
        loom::model(|| {
            let slot = Slot::new(Pool::new(0));

            let reader = {
                let slot = slot.clone();
                loom::thread::spawn(move || {
                    // current() — capture the pool in effect.
                    let pool = slot.current();
                    // Use it: while we hold this Arc, the pool cannot be dropped.
                    assert!(
                        pool.alive.load(Ordering::Acquire),
                        "in-flight request observed a torn-down pool (id={})",
                        pool.id
                    );
                })
            };

            // Concurrently: deploy a new version and drain (drop) the old pool.
            let old = slot.hotswap(Pool::new(1));
            drop(old); // the drain task dropping its Arc clone

            reader.join().unwrap();
        });
    }

    /// A reader that runs entirely after a swap observes the new pool — the swap
    /// is never lost, and the slot never yields a stale-and-dead pool. Pins the
    /// happens-before edge the write lock establishes.
    #[test]
    fn swap_is_observed_and_new_pool_is_alive() {
        loom::model(|| {
            let slot = Slot::new(Pool::new(0));

            let old = slot.hotswap(Pool::new(1));

            let reader = {
                let slot = slot.clone();
                loom::thread::spawn(move || {
                    let pool = slot.current();
                    assert!(
                        pool.alive.load(Ordering::Acquire),
                        "current pool must be alive"
                    );
                    assert_eq!(pool.id, 1, "reader after swap must see the new pool");
                })
            };

            drop(old);
            reader.join().unwrap();
        });
    }

    /// Teeth check — the loom counterpart of the TLA+ `HardKill` counterexample.
    /// Here the swapper tears the old pool down (`alive = false`) on the drain
    /// timer *without* waiting for in-flight holders — the buggy design. In the
    /// interleaving where a reader reads `alive` on a pool it still holds after
    /// that store, the assertion fires. Loom finds that schedule, so this test is
    /// expected to panic. (Delete `#[should_panic]` to see loom print the failing
    /// interleaving.)
    #[test]
    #[should_panic]
    fn hard_kill_variant_is_caught_by_loom() {
        loom::model(|| {
            let slot = Slot::new(Pool::new(0));

            let reader = {
                let slot = slot.clone();
                loom::thread::spawn(move || {
                    let pool = slot.current();
                    assert!(
                        pool.alive.load(Ordering::Acquire),
                        "in-flight request observed a hard-killed pool"
                    );
                })
            };

            let old = slot.hotswap(Pool::new(1));
            old.alive.store(false, Ordering::Release); // BUG: kill ignoring holders
            drop(old);

            reader.join().unwrap();
        });
    }
}
