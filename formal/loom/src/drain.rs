//! `RequestDrain`'s exactly-once drain signal.
//!
//! Reproduces the counter logic of `src/app/drain.rs` with loom atomics.
//! `request_completed` decrements the in-flight count and must release the drain
//! waiter *exactly once* — on the 1 -> 0 transition, and only then. The shipped
//! code does this correctly by acting on the value `fetch_sub` returns:
//!
//! ```ignore
//! let count = self.active_requests.fetch_sub(1, SeqCst);
//! if count == 1 { self.drain_semaphore.add_permits(1); }  // last one out
//! ```
//!
//! The property is the implementation-level counterpart of the TLA+ `Shutdown`
//! model's `DrainedMeansEmpty`: a drain is signalled precisely when the last
//! in-flight request leaves. Loom checks it across every interleaving of
//! concurrent completions, plus the no-underflow invariant.

use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;

/// The counter half of `RequestDrain`: `active` in-flight, and `signals` counting
/// drain-permit releases (a stand-in for `Semaphore::add_permits`).
struct Drain {
    active: AtomicUsize,
    signals: AtomicUsize,
}

impl Drain {
    fn with_active(n: usize) -> Arc<Self> {
        Arc::new(Drain {
            active: AtomicUsize::new(n),
            signals: AtomicUsize::new(0),
        })
    }

    /// Faithful port of `request_completed`: signal only on the 1 -> 0 edge, keyed
    /// off the value `fetch_sub` returned (the pre-decrement count).
    fn request_completed(&self) {
        let count = self.active.fetch_sub(1, Ordering::SeqCst);
        if count == 1 {
            self.signals.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two requests complete concurrently. Across every interleaving: the count
    /// reaches exactly 0 (no underflow, no lost decrement) and the drain is
    /// signalled exactly once — the thread that observed the 1 -> 0 edge.
    #[test]
    fn drain_signals_exactly_once() {
        loom::model(|| {
            let drain = Drain::with_active(2);

            let t1 = {
                let drain = drain.clone();
                loom::thread::spawn(move || drain.request_completed())
            };
            let t2 = {
                let drain = drain.clone();
                loom::thread::spawn(move || drain.request_completed())
            };

            t1.join().unwrap();
            t2.join().unwrap();

            assert_eq!(drain.active.load(Ordering::SeqCst), 0, "count must reach 0");
            assert_eq!(
                drain.signals.load(Ordering::SeqCst),
                1,
                "drain must be signalled exactly once"
            );
        });
    }

    /// Teeth check — the loom counterpart of a subtly-wrong drain. Here
    /// `request_completed` decides to signal by *re-reading* the counter after the
    /// decrement (`load() == 0`) instead of using `fetch_sub`'s return value. Two
    /// threads can both observe 0 and both signal, or the reads can interleave so
    /// none does. Loom finds the double-signal schedule, so this test is expected
    /// to panic. (Delete `#[should_panic]` to see the failing interleaving.)
    #[test]
    #[should_panic]
    fn racy_reread_variant_is_caught_by_loom() {
        loom::model(|| {
            let drain = Drain::with_active(2);

            let broken_complete = |d: &Drain| {
                d.active.fetch_sub(1, Ordering::SeqCst);
                if d.active.load(Ordering::SeqCst) == 0 {
                    d.signals.fetch_add(1, Ordering::SeqCst);
                }
            };

            let t1 = {
                let drain = drain.clone();
                loom::thread::spawn(move || broken_complete(&drain))
            };
            let t2 = {
                let drain = drain.clone();
                loom::thread::spawn(move || broken_complete(&drain))
            };

            t1.join().unwrap();
            t2.join().unwrap();

            assert_eq!(
                drain.signals.load(Ordering::SeqCst),
                1,
                "a correct drain signals exactly once"
            );
        });
    }
}
