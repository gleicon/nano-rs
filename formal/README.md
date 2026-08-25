# Formal methods for nano-rs

Two tools, aimed at the concurrency protocols where interleavings cause bugs that
ordinary tests can't reliably reach. They are **complementary**: TLA+ checks the
*design*, loom checks the *implementation*.

| Tool | Checks | Artifact | What it proves here |
|------|--------|----------|---------------------|
| **TLA+ / TLC** | the *protocol* (a state-machine model) | `HotSwap.tla`, `HotSwap.cfg` | The blue-green hot-swap/drain design never drops a request and never leaks draining pools. |
| **loom** | the *real Rust* (RwLock + Arc, all interleavings under the C11 memory model) | `loom-slot/` | `SliverPoolSlot`'s actual lock/Arc code can't tear a pool down under an in-flight request. |

Both target the same feature: the zero-downtime sliver hot-swap in
`src/worker/sliver_pool.rs` (`SliverPoolSlot`). Start there — it's the smallest
real concurrency protocol in the codebase and a good place to learn these tools.

**New to TLA+ / loom? Read [DIAGRAMS.md](DIAGRAMS.md) first** — a picture-first walk
through the state machine, the bug both tools catch, and how loom explores
interleavings.

> **The one caveat that matters.** TLA+ verifies a *model*, not the Rust. A green
> spec means the design is sound; it does not prove the code matches the design.
> loom closes that gap for the specific data structure it exercises (it runs the
> real `RwLock`/`Arc`), but only for the slice modelled. Neither tool touches the
> V8 FFI, `unsafe`, or the worker threads themselves. Treat these as high-value
> checks on two specific protocols, not a whole-system correctness proof.

---

## TLA+ — the hot-swap protocol

`HotSwap.tla` models each pool as a generation with a lifecycle
(`absent → live → retired → dead`) and the slot as the generation currently
installed. It encodes the design's key rule: a retired pool's workers exit only
once the drain timer elapsed **and** no request still holds it (the Arc refcount
is zero) — not on the wall-clock timer alone.

Invariants checked:

- **`SlotIsLive`** — the slot always points at a live pool, so a new request never
  binds to a retired/dead one.
- **`NoDispatchToDead`** — no in-flight request is ever bound to a torn-down pool.
  This is the "no dropped request during a deploy" safety property.
- **`EventuallyReaped`** (liveness) — every retired pool is eventually reaped, so
  repeated deploys don't accumulate draining pools. It holds *because requests
  complete* (weak fairness on `Complete`) — teardown is bounded by request
  completion, which is exactly the design point.

The constant **`HardKill`** selects the teardown rule:

- `FALSE` (shipped design) — reap only when no request still holds the pool.
- `TRUE` (buggy variant) — hard-kill on the drain timer, ignoring in-flight
  requests. TLC finds a 5-state counterexample violating `NoDispatchToDead`,
  which is *why* the shipped code waits for the Arc refcount.

### Run it

TLC needs Java. On **Java 8**, use tla2tools from TLA+ **1.7.1** (newer releases
need Java 11+):

```bash
# shipped design — expect: "No error has been found" (all invariants + liveness)
make tla

# buggy hard-kill variant — expect: "Invariant NoDispatchToDead is violated"
# plus a 5-state counterexample trace
make tla-counterexample
```

`make tla` downloads the checker into `formal/.cache/` (a repo-local, non-world-
writable path — not `/tmp`), verifies it against a SHA256 pinned in the Makefile,
and re-checks that hash before every run, so a tampered jar is never handed to the
JVM. The pinned jar is TLA+ **1.7.1** (the last Java-8-compatible build). On
Java 11+ you can instead point `TLA_JAR` at a current `tla2tools.jar` and update
`TLA_JAR_SHA256`.

---

## loom — the slot's real synchronization

`loom-slot/` is a **standalone crate** (not a nano-rs workspace member). Loom
compiles its entire dependency graph in loom mode; running that against nano-rs
would rebuild tokio/hyper under `--cfg loom` and fail. Isolating the check in a
crate whose only dependency is loom avoids that. It reproduces the slot's
synchronization skeleton — `RwLock<Arc<Pool>>` with `current()` (clone under a
read lock) and `hotswap()` (replace under a write lock, return the old Arc) — with
a stub `Pool` whose `Drop` marks it not-alive.

Tests (each explores *all* interleavings via `loom::model`):

- **`reader_never_observes_a_dropped_pool`** — a reader holding a pool's `Arc`
  always sees it alive, even while another thread swaps and drops the old pool.
  The implementation-level counterpart of TLA+ `NoDispatchToDead`.
- **`swap_is_observed_and_new_pool_is_alive`** — a reader after a swap sees the new
  pool (the write lock's happens-before edge; no lost swap).
- **`hard_kill_variant_is_caught_by_loom`** (`#[should_panic]`) — the loom analog
  of TLA+ `HardKill`: tearing the pool down without waiting for holders, and loom
  finds the failing schedule. Proves the check has teeth.

### Run it

```bash
cd formal/loom-slot
RUSTFLAGS="--cfg loom" cargo test --release
```

`make loom` wraps this. It does **not** run in normal `cargo test` — the crate is
outside the nano-rs package, so day-to-day builds and CI of nano-rs are unaffected.

---

## Extending this

The same treatment fits two more protocols, in rough order of value:

1. **Graceful shutdown / request draining** (`app/drain.rs`, `signal.rs`) — a
   textbook drain: stop accepting, wait for `active_requests == 0`, time out.
   Safety: nothing accepted after the signal. Liveness: shutdown terminates.
2. **Isolate lifecycle + eviction** (`worker/eviction.rs`, recycle-after-N,
   memory-pressure eviction, the telemetry register/deregister guard) — invariant:
   no dispatch to an isolate mid-recycle; the telemetry registry never shows a
   dead isolate.

Keep each spec small and single-protocol so it stays a reasoning aid, not a
maintenance burden.
