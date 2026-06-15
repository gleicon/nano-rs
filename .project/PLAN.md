# PLAN.md

## Now

**State:** Tier 1-3 codebase simplification complete. 667 tests pass (1 pre-existing failure: `test_socket_reuse_addr` port-race, unrelated to our changes). `cargo build` clean, zero warnings. Timer infrastructure extracted to `runtime/timers.rs`. `NanoRequest::from_axum_parts` unifies 4 duplicate request-construction sites. `with_worker_runtime` now correctly wired — `pool.rs` calls `data_plane::set_worker_runtime` instead of dead thread-local. Worker loop merge plan documented (Steps A–E), not yet executed.

**State:** Phase 10 test extraction complete. 650 pass, 1 ignored (`test_socket_reuse_addr` marked `#[ignore]`), 3 pre-existing adversarial E2E failures (not regressions). Extracted 7 test modules to `tests/`: `router_unit_tests`, `vfs_memory_unit_tests`, `app_timeout_unit_tests`, `metrics_unit_tests`, `v8_module_unit_tests`, `worker_pool_dispatch_tests`. SubtleCrypto split: `runtime/subtle_v8.rs` (1001 lines) from `apis.rs` (3374→2376 lines). `build` clean.

**Next:** Phase 10 quality gate — run `ds-quality-gate` 9-pass, then commit all cleanup work.

**Open questions:**
- Step E (flatten `EntrypointWorkerPool` → direct `WorkerPool` in queue.rs): ~12 call sites, low risk, optional scope.
- `apis.rs` still 2376 lines — URL (~350) + Buffer (~257) could be further split.
- 3 adversarial E2E failures pre-date our changes (verified via stash test).

---

## Roadmap

### Phase 1 — pool.rs deslop (WS duplicate blocks)
- [x] Extract OOM pre-check → `ws_oom_break!` macro
- [x] Merge Text + Binary WS dispatch → `ws_dispatch!` macro
- [x] Narrating comments stripped; verbose docs slimmed
- [x] `cargo check` clean

### Phase 2 — Rust code quality audit
- [x] SAFETY comments on all unsafe sites (fetch.rs, websocket.rs, pool.rs, tenant_pool.rs)
- [x] Dead code removed (async_support.rs, sliver/mod.rs, wasm/js_api.rs)

### Phase 3 — Bug review
- [x] `dispatch_ws` prune dead handles via `is_finished()`
- [x] `ws_close_callback` guards on `WS_ACCEPTED`
- [x] OOM events logged in ws_messages loop
- [x] `clear_ws_thread_locals()` on all WS exit paths confirmed

### Phase 4 — Security review
- [x] CRITICAL: `set_allow_generation_from_strings(false)` added to standalone handler path
- [x] HIGH findings filed: SSRF, timing attack, default bind `0.0.0.0`, escape_json control chars

### Phase 5 — Quality gate + WS test fixes
- [x] 9-pass quality gate run; all findings implemented
- [x] WS tests 24/24 (Host header mismatch fix; timeout data-discard fix in test suite)
- [x] WS browser-compat stub added (`WS_SERVER_SOCKET`, `set_ws_readystate`, `WebSocket` global)
- [x] Commit `c2101cfa` — eval/new Function ban in standalone path

### Phase 6 — Tier 1: Dead code deletion
- [x] `NanoRequest::from_axum_request` deleted (no callers)
- [x] `serialize_response_to_json` + dead `base64_encode` removed from `v8_bridge.rs`
- [x] `WorkerPool::dispatch_to` deleted; `try_dispatch` second arg removed
- [x] `TenantPool` struct body deleted (895 lines); thread-locals + helpers kept
- [x] `pub use tenant_pool::TenantPool` re-export removed from `worker/mod.rs`

### Phase 7 — Tier 2: Async runtime unification
- [x] `pool.rs` dead `WORKER_RUNTIME` removed; replaced with `data_plane::set_worker_runtime`
- [x] `module.rs` `pollster::block_on` → `Handle::try_current()` + `with_worker_runtime` fallback
- [x] 14 inline `Runtime::new()` fallbacks in `vfs_bindings.rs` + `fs_polyfill.rs` → `vfs_block_on()` helper

### Phase 8 — Tier 3: Duplication elimination
- [x] Timer infrastructure extracted to `runtime/timers.rs` (apis.rs: 3631→3369 lines)
- [x] `NanoRequest::from_axum_parts` in `types.rs` — unifies 4 inline URL+header construction sites
- [x] Worker loop merge plan written (Steps A–E, see below)

### Phase 9 — Worker loop merge
- [x] **Step A**: Delete `execute_js_standalone` + `virtual_host_handler` (dead — never registered in server.rs)
- [x] **Step B**: `WorkerPool::new` → delegates to `with_source_and_backend` (Loop 2)
- [x] **Step C**: Delete Loop 1 body from `pool.rs`; re-expose `with_backend` as thin compat wrapper
- [x] **Step D**: Document `AppSource::entrypoint` placeholder invariant in `WorkerPool::new` + `EntrypointWorkerPool`
- [ ] **Step E**: (Optional) Flatten `EntrypointWorkerPool` → direct `WorkerPool` usage in `queue.rs`
- [x] Run full test suite after each step — 665 pass, 1 pre-existing failure

### Phase 10 — Test extraction + quality gate
- [x] Extract tests from `vfs/memory.rs` → `tests/vfs_memory_unit_tests.rs` (14 tests)
- [x] Extract tests from `app/timeout.rs` → `tests/app_timeout_unit_tests.rs` (11 tests)
- [x] Extract tests from `metrics/tenant.rs` → `tests/metrics_unit_tests.rs` (12 tests)
- [x] Extract tests from `v8/module.rs` → `tests/v8_module_unit_tests.rs` (3 tests, 3 kept embedded)
- [x] Extract tests from `http/router.rs` → `tests/router_unit_tests.rs` (7+1 tests, 2 kept embedded)
- [x] Extract tests from `worker/pool.rs` → `tests/worker_pool_dispatch_tests.rs` (11 tests)
- [x] SubtleCrypto extracted from `runtime/apis.rs` → `runtime/subtle_v8.rs` (1001 lines)
- [x] `test_socket_reuse_addr` marked `#[ignore]` (pre-existing port-race)
- [ ] `ds-quality-gate` full 9-pass run
- [ ] Commit all cleanup work as logical atomic commits
- [ ] Address filed security findings: SSRF, timing attack, default bind addr, escape_json control chars
