# nano-rs Design Decisions

Decisions recorded during `/ds-grill-me` session — 2026-08-02.

---

**Q: How should WASM workers be modelled for nano-rs users?**
**A: Option A — WASM as a library called from a JS entrypoint.** The entrypoint is always a `.js` file; it loads the `.wasm` bytes (inline or via `Nano.fs`) and calls exports. nano-rs stays out of the question of which language produced the WASM or what the module's internal conventions are.
*Rationale: nano-rs runs WASM through V8's browser model (WebAssembly.compile/instantiate). No WASI, no host functions beyond what JS provides. A `.wasm` first-class entrypoint would require a caller-convention contract nano-rs doesn't have. Language-agnostic: Rust, Go, AssemblyScript, C — all reduce to the same `.wasm` bytes.*

---

**Q: Who owns the WASM calling convention — nano-rs or the user's toolchain?**
**A: The user's toolchain (B1).** Users ship `.wasm` + the glue `.js` their toolchain generates (wasm-bindgen for Rust, tinygo for Go). The JS glue is the entrypoint. nano-rs never touches calling conventions.
*Rationale: owning a calling convention means owning a versioned contract. B1 keeps nano-rs language-agnostic and defers all ABI concerns to mature per-language tooling. Revisit if a language with no viable glue generator emerges.*

---

**Q: Should WASM limits be compile-time constants or runtime-configurable?**
**A: Compile-time constants in `limits.rs`, new `limits::wasm` module.** Same pattern as all other limits. Per-app overrides are the PaaS layer's responsibility (remo or equivalent), not nano-rs's.
*Rationale: nano-rs has no runtime config loading. Adding it for WASM alone would be inconsistent and add a loading/validation layer nothing else needs.*

---

**Q: What are the hard WASM limit values?**
**A:**
- `wasm::MODULE_SIZE_BYTES_MAX` = 10 MB (matches JS script cap; >10 MB is unoptimized output)
- `wasm::LINEAR_MEMORY_PAGES_MAX` = 512 pages = 32 MB (documents intent; enforced implicitly by 128 MB V8 heap OOM)
- `wasm::IMPORT_COUNT_MAX` = 128 (covers wasm-bindgen glue; >128 signals a design problem)
- `wasm::EXPORT_COUNT_MAX` = 256 (symmetric cap)

*Rationale: module size and import/export counts are cheaply checked pre-instantiation. Linear memory requires WASM binary section parsing — deferred; the V8 heap hard limit already enforces it.*

---

**Q: How is `LINEAR_MEMORY_PAGES_MAX` enforced without a WASM binary parser?**
**A: Documented-only limit, enforced implicitly by the 128 MB V8 heap OOM.** When the heap limit fires, `heap_limit_hits_total` counter increments and an event is logged. A new `wasm_memory_oom_total` counter will be added to distinguish WASM OOM events from JS OOM events in metrics.
*Rationale: writing a WASM memory-section parser adds complexity with no practical safety benefit the heap cap doesn't already provide. The counter gives observability for profiling.*

---

**Q: Should nano-rs be a CLI tool, an embeddable crate, or both?**
**A: CLI-only (D1), explicitly.** The `[lib]` target exists for internal test infrastructure, not as a public embedding surface. External systems (remo, etc.) integrate via the admin HTTP API (`:9000`) — language-agnostic, process-boundary-safe, multi-node compatible. The `pub fn run()` stub in `lib.rs` will be clarified to say this explicitly.
*Rationale: the HTTP admin API already is the right integration boundary. In-process embedding buys nothing for the PaaS use case and creates an API surface to version and maintain. D2 (proper embedding API) is a valid future path but requires a separate design session — the public API contract, lifecycle management (NanoRuntime::start/stop), and config surface are non-trivial. Record as potential v2+ path.*

**D2 (future, needs design):** `use nano::{NanoRuntime, NanoConfig}` — `NanoRuntime::builder().config(cfg).start() -> Result<NanoHandle>`. Would require: stable public API surface, versioning commitment, lifecycle contract (graceful shutdown, reload), config struct distinct from CLI args. Do not implement without a design doc.

---

# Browser APIs, Storage, and Node.js Compat — 2026-08-06

---

**Q: One runtime mode or two (WinterTC vs browser-compat)?**
Two distinct modes. WinterTC stays spec-clean. Browser-compat is opt-in and injects extra globals.
_Rationale: mixing browser globals into the WinterTC context causes spec drift and confuses users writing portable workers._

---

**Q: What signals browser-compat mode?**
Admin API flag (`mode: "browser"` on app registration). CLI shortcut: `--browser-app`.
_Rationale: mode is infrastructure config, not application code. The JS file stays clean._

---

**Q: Entry pattern for browser-compat apps?**
Same as WinterTC: `export default { fetch(request) {} }`. Mode flag controls which extra globals get injected at bind time, not the execution model.
_Rationale: zero new dispatch logic. Works in both modes if using `nano:` imports._

---

**Q: Stateful persistent isolates (tab model) or stateless isolates with durable storage?**
Stateless isolates with durable storage. The isolate is the execution context; state lives in storage backends.
_Rationale: avoids session management, maps to existing VFS hostname-scoping, keeps isolates recyclable. In-isolate JS globals persist only within one worker thread — not consistent across the pool (N workers = N separate JS heaps)._

---

**Q: Module namespace for built-in nano-rs APIs?**
`nano:` prefix — e.g. `import { kv } from 'nano:kv'`. Resolved in the ESM module loader before VFS lookup (prefix check in `resolve_import_path` in `v8/module.rs`).
_Rationale: clearly marks nano-rs specific APIs as distinct from WinterTC spec and Node.js. No collision risk._

---

**Q: KV namespace model?**
Named namespaces: `kv` (default, auto-scoped) + `openKV('name')` for multiple stores per app. Both hostname-scoped as `{hostname}::{kv_namespace}` via existing `IsolateVfs` prefix model.
_Rationale: `kv` covers the simple case; `openKV` covers separation of concerns. Zero cost if only one namespace is used._

---

**Q: KV value types?**
Bytes (`Uint8Array`) as the Rust-side primitive. JS-side convenience wrappers: `kv.getJSON(key)` / `kv.setJSON(key, value)`.
_Rationale: bytes are unambiguous. JSON wrapper lives in the JS module layer, not Rust._

---

**Q: Tenant isolation across a multi-app process?**
Free — inherited from `IsolateVfs.prefix_namespace()`. A worker only holds a reference to its own `IsolateVfs`; no cross-hostname VFS API exists. `nano:kv` routes through the same `IsolateVfs`, so isolation is automatic.
_Rationale: no new isolation code needed._

---

**Q: Implementation order?**
1. Close docs/code gap: `require('path')`, `require('buffer')`, `require('assert')`, `process.env` — documented as complete, not implemented
2. `nano:kv` — async bytes KV, EdgeStore backend, hostname-namespaced
3. `localStorage` shim + `sessionStorage` shim — JS-only wrappers over `nano:kv` and in-isolate Map respectively
4. `CacheStorage` — WinterTC-specified, VFS-backed Response serialization
5. `IndexedDB` — phase 3, backend TBD

_Rationale: unblocks current users first. Storage primitives before high-level shims. IndexedDB last — V8 has no built-in; full implementation required._

---

**Q: Storage backend architecture?**
Two separate stacks: file ops (`Nano.fs`, `require('fs')`) use VFS (memory/disk). KV ops (`nano:kv`) use EdgeStore (`gleicon/edgestore`, embedded crate, no separate process). S3 not wired now — EdgeStore handles S3 recovery through its own replication path when needed.
_Rationale: VFS is designed for named file access; EdgeStore is designed for structured KV with namespaces. EdgeStore's `engine.get(namespace, key)` maps exactly to `openKV('name').get(key)`._

---

**Q: Node.js compat scope?**
Minimal first: `require('path')`, `require('buffer')`, `require('assert')`, `process.env`. `require('events')` and `require('util')` are phase 2. `http`, `https`, `net`, `child_process` out of scope permanently. Node.js `crypto` deferred — Web Crypto (`crypto.subtle`) already complete.
_Rationale: ~80% of npm packages that run in edge runtimes need only path/buffer/assert/process._

---

**Note — IndexedDB backend (unresolved, phase 3):**
Options: SQLite (new dep) or structured JSON in VFS paths. Deferred until CacheStorage is done and usage patterns are clearer.
