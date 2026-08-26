# Verification coverage

Which technique verifies which part of nano-rs, and why. The guiding principle:
each class of defect has a technique that detects it directly. Model checking
targets **concurrency and state-machine** defects — interleavings and orderings
that example-based tests reach only by luck. Sequential input/output correctness
is the domain of property and unit tests. Memory safety is the domain of the type
system and Miri. Applying a model checker to sequential code, or a unit test to a
race, misfiles the defect against a technique that cannot see it.

## Technique legend

| Technique | Detects | Exhaustive over |
|-----------|---------|-----------------|
| **TLA+ / TLC** | protocol/state-machine defects (lost or duplicated work, stuck states, unsafe transitions) | all reachable states of the model |
| **loom** | data races and memory-ordering defects in hand-written synchronization | all thread interleavings + C11 orderings of the code under test |
| **property / adversarial tests** | input-space defects (traversal, injection, malformed input, crypto correctness) | sampled + hand-picked adversarial inputs |
| **unit / integration tests** | algorithmic and wiring defects on concrete cases | enumerated cases |
| **Rust type system + borrow checker** | memory safety, data-race freedom for `Send`/`Sync`, thread-affinity (`!Send` isolates) | every compiled line |
| **Miri** | undefined behavior in `unsafe` (aliasing, uninitialized reads) | executed `unsafe` paths |

## Subsystem coverage

| Subsystem | Concurrency/state protocol | Primary verification | Artifact |
|-----------|----------------------------|----------------------|----------|
| **HTTP request path** (`http/router.rs`, `worker/queue.rs`, `worker/pool.rs`) | request → exactly-one response; bounded-queue backpressure (503); no hang | TLA+ | [`RequestLifecycle.tla`](RequestLifecycle.tla) |
| **Sliver hot-swap** (`worker/sliver_pool.rs`) | blue-green pool swap + drain; no request bound to a torn-down pool | TLA+ + loom | [`HotSwap.tla`](HotSwap.tla), [`loom/src/slot.rs`](loom/src/slot.rs) |
| **Graceful shutdown / drain** (`app/drain.rs`, `signal.rs`) | no accept after signal; drain terminates; a clean drain means zero in-flight; exactly-once drain signal | TLA+ + loom | [`Shutdown.tla`](Shutdown.tla), [`loom/src/drain.rs`](loom/src/drain.rs) |
| **Isolate lifecycle / eviction** (`worker/pool.rs`, `worker/eviction.rs`) | recycle-after-N; LRU eviction selection | unit tests + type system | `worker/eviction.rs` tests; recycle covered structurally by the request-loop |
| **Live telemetry** (`worker/telemetry.rs`) | register/deregister of live isolates | `DashMap` (library-provided sync) + unit/integration tests | `telemetry.rs` tests; `test_dispatch_publishes_live_telemetry` |
| **nano:kv store** (`runtime/kv.rs`) | concurrent get/set/delete; tenant isolation by hostname namespacing | `DashMap` (concurrency) + property tests (key construction ⇒ isolation) | `kv_*` tests |
| **VFS** (`vfs/memory.rs`, `disk.rs`, `s3.rs`) | per-isolate ownership (no cross-thread sharing within a namespace); path isolation | adversarial + integration tests; S3 against live MinIO | `adversarial_vfs.rs`; `s3.rs` `#[ignore]` integration test |
| **fetch / SSRF guard** (`runtime/fetch.rs`) | resolved-address filtering (sequential per call) | adversarial tests | `adversarial_network_*.rs` |
| **WebCrypto** (`runtime/crypto/`) | none (sequential) | RustCrypto primitives + known-answer tests | `crypto_*` tests |
| **URL / Headers / Request / Response** (`runtime/`, `http/`) | none (sequential parsing/marshalling) | unit + WinterTC conformance tests | `http_wintercg_test.rs`, `runtime/*` tests |
| **Bytecode version gate** (`sliver/unpacker.rs`) | none (a predicate on a version tag) | unit tests | `bytecode_matches_v8_*` tests |
| **CPU-time limit** (`worker/cpu_tracker.rs`, `worker/timeout.rs`) | timer fires → V8 terminate → response (the `timeout` outcome) | integration tests; modelled as the `Timeout` escape in `RequestLifecycle.tla` | `cpu_timeout_*` tests |
| **V8 isolate + FFI** (`v8/isolate.rs`, `v8/snapshot.rs`) | thread-affinity (`!Send`), EPT sentinel drop order | Rust type system (`PhantomData<*mut ()>`); Miri for `unsafe` | `into_inner`/`from_v8_isolate`; snapshot round-trip tests |

## What is model-checked, and what is deliberately not

Model-checked (TLA+ and/or loom): the sliver hot-swap, the HTTP request lifecycle
with backpressure, and graceful shutdown/drain — the paths where a defect depends
on ordering between concurrent actors and would not reproduce deterministically in
a test.

Not model-checked, by design:

- **Sequential code** (crypto, URL/headers, request marshalling, bytecode gating).
  These have no interleaving; a model checker would explore a single trivial
  trajectory. Property tests and known-answer tests cover their input space.
- **Library-provided concurrency** (`DashMap` in KV and telemetry, `tokio`
  primitives). Their internal synchronization is verified upstream; nano-rs's
  obligation is correct *use*, which for KV is tenant-key construction (a property
  test) rather than an interleaving.
- **Memory safety and the V8 FFI.** The borrow checker enforces data-race freedom
  and the `!Send` thread-affinity that keeps isolates on their owning thread; Miri
  covers the `unsafe` extraction paths. Neither TLA+ nor loom models C++-side V8
  state.

## Findings

A recurring pattern surfaced by auditing model assumptions against the code: a
per-app limit is stored in one place but the enforcement point reads a different
source, or isn't wired on a serving path. The `AppState::new_shared` / `new` paths
(default `nano-rs run` and admin-driven) pass `app_registry: None`, so anything read
from the registry silently no-ops there. All resolved; each has a regression test.

- **CPU-time limit** — returned 0 with no `app_registry`, so the `CpuTimeoutGuard`
  was never armed and a runaway handler wedged the worker. Surfaced by
  `RequestLifecycle.tla` (`CpuLimit=FALSE`). Fixed: `get_cpu_time_limit_ms` falls
  back to `default_cpu_time_ms` (50ms); only an explicit `cpu_time_enabled: false`
  yields 0. `CpuLimit=FALSE` still runs as the residual foot-gun of disabling it.
- **Memory limit** — the registry-less pool-creation fallback passed
  `memory_limit_mb: 0` (no V8 heap cap, ~V8 default). Fixed: `create_pool_with_fallback`
  uses `default_memory_limit` (128MB), so an isolate is never uncapped.
- **Worker count** — `limits.workers` was ignored; every pool used the global
  `workers_per_pool` (taken from `apps[0]`). Fixed: the per-app pool uses
  `app_config.limits.workers`.
- **Request timeout** — `limits.timeout_secs` was never read; the only bound was a
  fixed 30s `TimeoutLayer`. Fixed: `dispatch_to_worker_pool` wraps the worker
  response in a per-app deadline (`get_request_timeout_secs`, default 30s); the
  global layer is now a 300s ceiling.
- **Control-plane `TenantLimits`** — `allowed_methods` / `max_batch_size` were
  stored but never consulted (no request-batch concept; no per-tenant method
  allowlist). Removed rather than left as inert config; `validate_request_ref` keeps
  enforcing the global script/timeout/body bounds plus tenant existence.

Regression tests: `cpu_time_limit_defaults_on_and_respects_config`,
`request_timeout_defaults_on_and_respects_config` (router), and
`registry_less_pool_caps_memory_and_uses_default_workers`,
`per_app_workers_is_honored_over_pool_default` (queue) — the limit × serving-path
enforcement matrix.

## Bounds

The TLA+ models are checked at small finite bounds (a handful of requests, pools,
and generations) — enough to exhaust the interleavings that produce ordering
defects, which appear at small scale. The loom checks run two-to-three threads,
which is sufficient for the pairwise races in these primitives. Neither is a proof
at unbounded scale; both are exhaustive within their stated bounds.
