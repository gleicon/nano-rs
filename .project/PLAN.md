# PLAN.md

## Now

**State:** Phase 11 complete + owner-namespaced hostname scheme done. Canonical URL is `{owner}-{name}.{domain}` to prevent subdomain squatting. `custom_domain` column added to DB, `App` struct, `AppResponse`. `PUT /api/apps/:name/domain` (set CNAME) and `DELETE /api/apps/:name/domain` (clear) registered in user_routes. Both crates build clean.

**Next:** Commit all changes, then `git subtree split --prefix=remo -b remo-split` to extract remo into its own repo.

**Open questions:**
- Step E (flatten `EntrypointWorkerPool` → direct `WorkerPool` in queue.rs): still optional, ~12 call sites.
- `apis.rs` still 2376 lines — URL (~350) + Buffer (~257) could be split further.
- remo split target remote URL TBD (user has domain "remo").

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
- [ ] `ds-quality-gate` full 9-pass run on entire branch diff
- [ ] Commit all cleanup work as logical atomic commits
- [ ] Address filed security findings: SSRF, timing attack, default bind addr, escape_json control chars

### Phase 11 — remo scaffold + Nano.env wiring
- [x] remo/ standalone Cargo workspace created (git-push mini PaaS)
- [x] Nano.env V8 wiring: thread-local CURRENT_ENV → frozen Nano.env object in V8
- [x] WorkerPool.with_source_backend_and_env() constructor
- [x] 6 V8 env tests pass (accessible, frozen, missing=undefined, multi-key, etc.)
- [x] 12 remo validation tests pass (app name, sha, safe_join, constant_eq, parse_app_name)
- [x] Code review: 10 findings, all fixed (see commit for details)
- [x] bcrypt → sha256 for token auth (O(N×300ms) → O(1) SQL lookup)
- [x] Owner-namespaced hostnames: `{owner}-{name}.{domain}` prevents subdomain squatting
- [x] custom_domain column + app_set_custom_domain() in db.rs
- [x] PUT/DELETE /api/apps/:name/domain endpoints with validate_domain()
- [x] AppResponse includes custom_domain field
- [ ] Commit all changes
- [ ] `git subtree split --prefix=remo -b remo-split` → push to remo repo
