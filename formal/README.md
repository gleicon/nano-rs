# Formal methods for nano-rs

Machine-checked models of nano-rs's core concurrency protocols. Two tools:

- **TLA+ / TLC** checks a *protocol* — a state machine you write — by exhaustively
  exploring every reachable state for invariant and liveness violations.
- **loom** checks an *implementation* — real Rust — by exhaustively exploring every
  thread interleaving and memory ordering of the code under test, under the C11
  memory model.

They are complementary: TLA+ establishes that a design is correct; loom establishes
that the hand-written synchronization implements it without a data race or ordering
defect. [COVERAGE.md](COVERAGE.md) maps every nano-rs subsystem to the technique
that verifies it and states what is deliberately not model-checked.

New to these tools: [DIAGRAMS.md](DIAGRAMS.md) is a diagram-first walkthrough of one
protocol (the sliver hot-swap) — the state machine, the defect the check rules out,
and how loom explores interleavings.

## Scope

TLA+ verifies a model, not the Rust; a correct spec does not by itself establish
that the code matches it. loom removes that gap for the specific synchronization it
runs (it executes the real `RwLock`/`Arc`/atomics), but only for the code path it
exercises. Neither tool models the V8 FFI, `unsafe`, or the OS threads and Tokio
runtime — those are covered by the Rust type system, Miri, and tests (see
[COVERAGE.md](COVERAGE.md)). The models are checked at small finite bounds, which
is sufficient to exhaust the interleavings that produce ordering defects.

## Protocols

| Protocol | Code | Spec | loom |
|----------|------|------|------|
| HTTP request lifecycle — exactly-one response, bounded-queue backpressure, no hang | `http/router.rs`, `worker/queue.rs`, `worker/pool.rs` | [`RequestLifecycle.tla`](RequestLifecycle.tla) | — |
| Sliver hot-swap — blue-green swap + drain | `worker/sliver_pool.rs` | [`HotSwap.tla`](HotSwap.tla) | [`loom/src/slot.rs`](loom/src/slot.rs) |
| Graceful shutdown / drain | `app/drain.rs`, `signal.rs` | [`Shutdown.tla`](Shutdown.tla) | [`loom/src/drain.rs`](loom/src/drain.rs) |

### RequestLifecycle

Every accepted request reaches exactly one terminal outcome — `ok`, `rejected`
(queue full → 503), or `timeout` — and no request hangs. Invariants:
`DoneIffResponded` (a response exists iff the request is terminal), `QueueBounded`
(backpressure holds), `AtMostOneRunning`. Liveness: `EveryRequestResponds`. The
`timeout` action models the CPU-time limit and the server timeout layer; it is what
makes liveness hold regardless of handler behavior. Strong fairness on `StartWork`
models the FIFO `sync_channel` (no starvation).

### HotSwap

A retired pool's workers exit only once the drain timer has elapsed **and** no
request still holds the pool (Arc refcount zero). Invariants: `SlotIsLive`,
`NoDispatchToDead` (no in-flight request bound to a torn-down pool). Liveness:
`EventuallyReaped`. The `HardKill` constant switches to a variant that reaps on the
timer alone; TLC then produces a 5-state counterexample to `NoDispatchToDead`,
which is the justification for the refcount-based teardown.

### Shutdown

On the shutdown signal the accept path closes, then in-flight requests drain up to a
timeout. Safety: `NoAcceptAfterSignal` (the in-flight count never rises after the
signal — the property that makes `RequestDrain::await_complete`'s empty-check-then-
wait sound), `DrainedMeansEmpty` (a clean drain result means zero in-flight),
`OutcomeStable`. Liveness: `ShutdownTerminates` (via clean drain or timeout).

## loom checks

`loom/` is a standalone crate (not a nano-rs workspace member): loom compiles its
whole dependency graph under `--cfg loom`, which would rebuild tokio/hyper in loom
mode inside nano-rs. It depends only on loom.

- `slot.rs` — `SliverPoolSlot`'s `RwLock<Arc<Pool>>`: a reader holding a pool's
  `Arc` always sees it alive across a concurrent swap-and-drop
  (`reader_never_observes_a_dropped_pool`), and a reader after a swap sees the new
  pool (`swap_is_observed_and_new_pool_is_alive`). The implementation-level
  counterpart of `HotSwap`'s `NoDispatchToDead`.
- `drain.rs` — `RequestDrain`'s counter: concurrent completions reach zero without
  underflow and signal the drain **exactly once**, on the 1→0 edge
  (`drain_signals_exactly_once`). The counterpart of `Shutdown`'s `DrainedMeansEmpty`.

Each module also has a `#[should_panic]` test that introduces the corresponding
defect (hard-kill on timer; racy re-read instead of `fetch_sub`'s return) and
confirms loom reports the failing schedule.

## Running

```bash
make tla               # run TLC on all specs (HotSwap, RequestLifecycle, Shutdown)
make tla-counterexample # HotSwap hard-kill variant: prints the 5-state violation
make loom              # run all loom checks
```

`make tla` downloads the checker into `formal/.cache/` (a repo-local, non-world-
writable path), verifies it against a SHA256 pinned in the Makefile, and re-checks
that hash before each run, so a tampered jar is never handed to the JVM. The pinned
jar is TLA+ **1.7.1**, the last Java-8-compatible build; on Java 11+ point `TLA_JAR`
at a current `tla2tools.jar` and update `TLA_JAR_SHA256`. `make loom` builds only
the standalone crate under `--cfg loom`; normal nano-rs builds and `cargo test` are
unaffected.

## Extending

The pattern for adding a protocol: write `<Name>.tla` + `<Name>.cfg`, add the name
to `TLA_SPECS` in the Makefile, and — if the protocol rests on hand-written
synchronization — add a module to `loom/`. Keep each model to one protocol and to
small bounds. Candidates not yet modelled are noted in [COVERAGE.md](COVERAGE.md).
