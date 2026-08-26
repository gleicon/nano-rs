//! Loom model-checks of nano-rs's custom concurrency primitives.
//!
//! Loom exhaustively explores every thread interleaving and memory ordering of
//! real Rust code under the C11 memory model. It complements the TLA+ specs in
//! `../*.tla`, which check the *protocols*; loom checks the *implementations* of
//! the hand-written synchronization those protocols rely on.
//!
//! Scope note: loom compiles its whole dependency graph in loom mode, so this is
//! a standalone crate (not a nano-rs workspace member) depending only on loom.
//! It also cannot drive OS threads, Tokio, V8, or third-party concurrent
//! structures (`DashMap`, `tokio::Semaphore`). So each module reproduces the
//! *hand-written* synchronization skeleton of one primitive with loom types,
//! keeping the exact shape of the shipped code. Data structures whose
//! concurrency is provided by a trusted library are out of scope here and are
//! covered by property tests instead (see `../COVERAGE.md`).
//!
//! Modules:
//!   - `slot`  — `SliverPoolSlot`'s `RwLock<Arc<..>>` swap/drop (hot-swap safety).
//!   - `drain` — `RequestDrain`'s exactly-once drain signal on the 1->0 edge.
//!
//! Run: `RUSTFLAGS="--cfg loom" cargo test --release` (or `make loom`).

pub mod drain;
pub mod slot;
