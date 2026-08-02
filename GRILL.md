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
