# Cold Start Terminology Guide

**Version:** 2.7.0
**Last Updated:** 2026-08-24

"Cold start" is ambiguous — it can mean process boot, isolate creation, or the
per-request context reset. This guide defines the distinct timing categories used
across NANO docs and states what actually optimizes each one today.

> NANO does **not** currently restore isolates from a V8 heap snapshot. The
> create/restore primitives exist and are tested, but serving from a baked heap
> blob needs an external-reference table for the native runtime APIs — see
> The live cold-start optimization is **bytecode caching**
> (below): a sliver packed with `nano sliver create` carries precompiled V8
> bytecode that the runtime loads to skip JavaScript parse + compile.

---

## Timing categories

### 1. Process boot

Time from binary execution to HTTP server ready — one-time per process start.
Covers V8 platform initialization, config loading, HTTP socket binding, and worker
pool creation. This is infrastructure startup, not a per-request metric. It matters
on every container/pod restart and deployment rollout.

### 2. First request on a new isolate (cold)

When a worker creates a fresh isolate, the first request must compile the
entrypoint. Two paths:

- **With cached bytecode (fast path).** If the app's sliver carries
  `bytecode.v8bc` and its recorded V8 cache-version tag matches the running V8,
  the runtime compiles via `ScriptCompiler::Source::new_with_cached_data`, skipping
  the parse+compile step. This is the current cold-start optimization.
- **From source (fallback).** No bytecode, or a version-tag mismatch → the isolate
  parses and compiles the entrypoint from source. Always correct; just not as fast.

Either way the runtime then binds the WinterTC/native APIs (`RuntimeAPIs::bind_all`)
into the context before serving.

### 3. Context reset (per request)

Between requests on the same isolate, NANO uses a fresh context for isolation
rather than tearing down and recreating the isolate. This provides request-to-request
isolation without paying isolate-creation cost each time.

### 4. Fresh isolate creation

Creating a new isolate from scratch — heap allocation, V8 internal setup, context
creation, entrypoint compilation, and API binding. Happens on pool expansion, after
eviction, or for a new hostname. Bytecode caching shortens the compilation part; the
rest is inherent V8 cost.

---

## What optimizes what

| Category | Live optimization |
|----------|-------------------|
| Process boot | Small config; prebuilt V8; right-sized worker pools |
| First request (cold) | Precompiled `bytecode.v8bc` in the sliver (skips parse+compile) |
| Context reset | Already minimal; keep handler global state small |
| Fresh isolate | Bytecode caching for the compile step; pre-warm pools |

Restoring an isolate from a baked V8 heap snapshot (which would also skip
`bind_all`) is not implemented — not the current cold-start path.

---

## Related documentation

- [Performance](PERFORMANCE.md) — tuning guide
- [Slivers](SLIVER_WORKFLOW.md) — creating and managing sliver bundles
