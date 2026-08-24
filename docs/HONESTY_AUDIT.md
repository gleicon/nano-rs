# Keeping the codebase honest

We kept finding the same failure classes one pass at a time: endpoints returning
**fabricated data** dressed as real, **tests that can't fail**, and **docs that
describe features the code doesn't have**. This document turns that ad-hoc hunting
into a repeatable process so the classes stop recurring.

The root problem is always the same: **a gap between what something claims and what
it does**, with nothing that fails when they diverge. The defense is layered — each
layer closes the gap for a different surface.

## The failure classes (what to look for)

1. **Mocked / static returns served as real.** A handler returns invented or
   config-derived values that look like live data. Tells: `// in real implementation`,
   `// for test/demo`, `// for now, return …`, hardcoded stats (`42 + worker_id`),
   defaults where real config was available (`AppLimits::default()` in a getter).
2. **False-green tests.** Tests that pass without exercising anything:
   `assert!(true)`, no assertions at all, silent env-gates (`if !x_enabled() { return }`),
   `#[ignore]` hiding a broken or removed feature, or tests that assert the *mock*
   (e.g. `total_isolates == sum_of_configured_workers` when isolates are fabricated).
3. **Doc drift.** README/API status tables and JSON examples that no longer match
   the code: wrong field names, phantom endpoints, "In Progress" for shipped features.
4. **Flaky tests.** Pass alone, fail in the parallel suite — shared process-global
   state (worker pools, V8 pump, thread-locals, ports) without serialization.

## The layers of defense

### Layer 0 — Coverage baseline (`make coverage`) — *the denominator*

Everything else is sampling until you measure the whole. `make coverage`
(`cargo-llvm-cov`) reports line/function coverage across all source in minutes and
names the least-covered files — which is exactly where mocks and weak tests hide.
Without this, "we keep finding new problems" is inevitable: you're discovering
incrementally with no idea how much is left. With it, the unknown becomes a ranked,
finite list.

Baseline (2026-08): **~69% line coverage.** `make coverage-gate` fails the build if
it drops below `COVERAGE_MIN` (currently 68) — ratchet that number *up* as coverage
improves so it can never regress. Remaining worst-covered files are **live-but-
undertested** (they need real infra/fixtures, not more code): `v8/snapshot.rs` (~26%,
needs snapshot blobs), `vfs/mod.rs` (~49%, S3 variant needs a live backend),
`v8/isolate.rs` (~52%, snapshot constructors).

**A low-coverage file is not always undertested — it can be dead.** Two of the worst
offenders were dead code, not test gaps:
- `v8/module.rs` (~37%): three never-called functions (`execute_esm_or_script` /
  `execute_classic_script` / `execute_esm_module`) + a duplicate `extract_js_response`.
  The real path is `compile_*_handler` in `worker/pool.rs`. Deleting ~490 lines took it
  to ~71%.
- `worker/context.rs` (~33%): the entire `ContextManager` abstraction was orphaned —
  the worker pool reimplements isolate recycling inline. Its only "user" was a test that
  gave false confidence an unused feature worked. Deleted the module + its test.

So when coverage flags a file, **first ask "is this reachable?"** — trace callers
(`borescope callers` / grep) before writing tests. Testing dead code is worse than the
gap: it adds false confidence and blocks deletion. Removing dead code fixes coverage
*and* file size at once.

### Static profiling as a dead-code + hot-path finder (`make static-profile`)

`scripts/static-profile.sh` synthesizes a flamegraph from borescope's call graph — no
runtime data. Width = distinct static call paths through a function, a proxy for "hot".
It confirmed the serving lifecycle's three hot subsystems (isolate serving →
`with_source_backend_and_env`, router front → `dispatch_to_worker_pool`, app loading →
`start_server_with_config`) and, as a byproduct, a *reachability* pass surfaced two more
dead functions (`execute_handler_with_context`, `ControlPlane::submit_request`).

**Two caveats learned the hard way:**
1. **The serving path splits at the async channel boundary.** `dispatch_to_worker_pool`
   hands work to a detached worker thread via MPSC, so no call edge crosses it — the
   flamegraph needs *multiple roots* (front + worker loop), not one.
2. **Pure call-graph dead-code detection is unreliable here.** The runtime registers
   ~100 functions as V8 callbacks (`v8::Function::new(scope, my_cb)` — function pointers,
   not calls), so they look unreachable but are live. A "zero-caller" list is mostly these
   false positives. **Coverage + manual caller-tracing is the reliable dead-code method;**
   the call graph is a *lead generator*, not the verdict. Always confirm a zero-caller hit
   by grepping for the name repo-wide before deleting.

Coverage tells you what runs under test; Layer 5 (mutation) tells you whether the
tests that run actually *assert* anything. Use coverage to find untested code, then
mutation to audit the tests covering the rest.

### Layer 1 — Mechanical gate (`make audit`) — *runs in CI, cheap, deterministic*

`scripts/honesty-audit.sh` greps for the known tells and fails the build. It catches
re-introduction of every pattern above. A genuinely-legitimate hit is exempted with a
trailing `// audit-ok: <why>` — which makes each exception a deliberate, reviewable
decision instead of an accident. This is the floor: it can't prove honesty, but it
stops the exact mistakes we already made from coming back.

### Layer 2 — Endpoint contract tests — *one per operator/user-facing surface*

Every admin/API endpoint has a test that builds **real** state, hits the endpoint, and
asserts the response **reflects the input** — never just "constructs without panicking".
Examples now in the tree:
- `get_app_returns_real_config_not_defaults` — real limits/env, not defaults.
- `collect_reflects_live_telemetry` / `test_dispatch_publishes_live_telemetry` — real
  per-isolate stats flow end-to-end.
- `test_ready_route_reflects_shutdown_state` (TCP + Unix) — 503 during drain.

Rule: **if an endpoint returns data, a test must assert that data is real.** A test that
only checks the response shape is not enough.

### Layer 3 — "No fabrication" architectural rule — *enforced in review + Layer 1*

Production code returns real data, or an explicit `None` / error / `"unknown"` — it
**never invents a plausible value**. If the data isn't wired yet, say so honestly (and
Layer 1's markers make "I'll fake it for now" fail CI).

### Layer 4 — Doc/code contract — *docs are generated from or checked against code*

Doc JSON examples drift because they're written by hand. Prefer: derive the example from
the actual serde struct (a test that serializes the real type and compares), or at least
keep one example per response type next to its `#[derive(Serialize)]` and check field
names in review. Phantom endpoints are caught by cross-referencing documented routes
against the router registration.

### Layer 5 — Mutation testing (`make mutants`) — *finds tests that don't test*

Layer 1 catches *known* false-green shapes; mutation testing catches the rest. Run
[`cargo-mutants`](https://mutants.rs): it changes the code (flips a comparison, returns a
default/empty) and checks that **some test fails**. A surviving mutant means the covering
test asserts nothing meaningful — the deepest form of "false green".

```bash
make mutants FILE=src/admin/diagnostics.rs   # lib tests only, per module
```

Worked example: the first run on `diagnostics.rs` scored **13 caught / 14 missed (48%)**.
The survivors named exactly what was untested — the operator-facing `format_ps`/`format_json`
output methods (nothing exercised them), the `avg_memory_mb` averaging arithmetic (asserted
only `> 0`, so `*`↔`/` survived), and the `format_duration`/`truncate` boundary comparisons.
Adding those assertions took the module to **22/22 viable mutants caught (100%)**. None of
these gaps were visible from reading the tests — they *looked* thorough.

Full-repo mutation is slow (each mutant rebuilds and re-tests). **Don't run it as a
blanket per-release sweep** — the value is proportional to *change*, not the calendar.
Use `make mutants-changed` to mutate only the files a release actually touched (diffed
against the latest tag), lib-scoped:

```bash
make mutants-changed                 # since latest tag
make mutants-changed REF=origin/main # since a branch point
```

### Layer 6 — Flakiness surfacing — *the suite runs the way CI runs*

Flakes hide until the suite is loaded. Run tests in parallel (the default) and, for the
integration tests that spin up real servers, **serialize the ones that share global
state** (see `SERVER_TEST_LOCK` in `websocket_integration_test.rs`) rather than
`#[ignore]`-ing them — an ignored test catches nothing.

## The routine

- **Every PR:** `make audit` + `make lint` + `make test` (Layers 1, 2, 3, 6).
- **Before a release:** `make mutants-changed` (Layer 5) and a doc pass cross-checking
  status tables and JSON examples against the code (Layer 4).
- **When you find a new class:** add its tell to `scripts/honesty-audit.sh` so the next
  one is caught mechanically. The audit script is meant to grow.

## A note on "god-files"

`borescope smells` flags large central files. Size alone is not the defect — **cohesion
is.** Judge each by whether it does one thing:

- `v8/isolate.rs` (~1000 lines) is one type (`NanoIsolate`) with many legitimate
  constructors and accessors. Splitting would scatter one API across files — leave it.
- `http/server.rs` had **seven `start_server_*` variants that each copy-pasted the
  bind→serve→shutdown loop**. That's duplication, not cohesion — it was consolidated
  behind `serve_app` / `serve_app_with_shutdown`.

Before splitting a big file, check whether its bulk is one cohesive responsibility
(keep) or repeated boilerplate / mixed concerns (refactor). Don't refactor for line
count; refactor to remove duplication and mixed responsibilities.
