# ADR-007: ESM Module Execution Strategy

**Status:** Accepted  
**Date:** 2026-04-21  
**Deciders:** Core Team  
**Technical Story:** Support ES modules (`export default { fetch }`) with V8

---

## Context and Problem Statement

Modern JavaScript uses ES modules:

```javascript
export default {
  async fetch(request) {
    return new Response("Hello");
  }
};
```

But classic scripts use:

```javascript
// No exports, just top-level code
var handler = {
  fetch: async (request) => {
    return new Response("Hello");
  }
};
```

V8 has two ways to execute JavaScript:
1. **`Script::compile`** — Classic scripts, top-level execution
2. **`Module::compile`** — ES modules with import/export resolution

We need ESM support for frameworks like Hono.js, Next.js, Astro. But implementing full ESM Module API with VFS-backed imports is complex.

---

## Decision Drivers

* **Compatibility** — Must run modern framework bundles
* **Complexity** — Balance implementation effort
* **Performance** — Fast compilation/execution
* **Standards** — Eventually full ESM support
* **Timeline** — v1.x needs immediate framework support

---

## Considered Options

### Option 1: Full V8 Module API

Proper ESM with `Module::Instantiate`, VFS-backed import resolution.

### Option 2: Transformation

Transform ESM syntax to classic script-compatible code.

### Option 3: Hybrid

Detect and use appropriate method per file.

### Option 4: Wait for rusty_v8

Defer until Module API stabilizes.

---

## Decision Outcome

**Chosen option for v1.x: "Transformation"**

Transform ESM syntax to classic script-compatible code:

```javascript
// Original ESM
export default { fetch: handler };

// Transformed to classic script
var __nano_exports = { fetch: handler };
__nano_module.exports = __nano_exports;
```

This provides:
- Immediate framework compatibility
- Works with existing `Script::compile` infrastructure
- No V8 Module API complexity

**Alternative considered: full Module API (not adopted)**

A full `v8::Module` implementation (not adopted) would use:
- VFS-backed import resolution
- Dynamic `import()` support
- True ES module semantics

---

## Implementation Details

### Transformation Pipeline

```rust
pub fn transform_esm(source: &str) -> Result<String, Error> {
    // 1. Parse with swc or regex-based
    let ast = parse_module(source)?;
    
    // 2. Transform exports
    if has_default_export(&ast) {
        // export default { ... } → var __nano_exports = { ... }
        source = transform_default_export(source);
    }
    
    // 3. Transform imports
    // import { Hono } from 'hono' → const Hono = require('hono').Hono
    source = transform_imports(source);
    
    // 4. Wrap with module object
    source = wrap_with_module_object(source);
    
    Ok(source)
}
```

### Execution Model

```rust
// After transformation, execute as classic script
let script = v8::Script::compile(scope, code)?;
let result = script.run(scope)?;

// Extract exports
let exports = scope.get_global()
    .get(scope, "__nano_module")
    .get(scope, "exports");

// Call handler
let handler = exports.get(scope, "fetch");
```

### Supported Patterns

| Pattern | Status | Transformed To |
|---------|--------|----------------|
| `export default {}` | ✅ | `var __nano_exports = {}` |
| `export const x = 1` | ✅ | `var x = 1; __nano_exports.x = x` |
| `import x from 'y'` | ✅ | `var x = require('y').default` |
| `import { x } from 'y'` | ✅ | `var x = require('y').x` |
| `import('./dynamic')` | ❌ | Not supported (v2.0) |
| `export * from 'y'` | ❌ | Not supported (v2.0) |

---

## Positive Consequences

* **Works immediately** — Hono.js, Next.js, Astro run today
* **Minimal implementation complexity** — No Module API to implement
* **Fast compilation** — Script::compile is fast
* **Backward compatible** — Classic scripts still work
* **Framework ecosystem unlocked** — Can use bundlers

---

## Negative Consequences

* **Not "real" ESM** — No import resolution (user must bundle)
* **Requires bundling** — Dependencies must be resolved at build time
* **Limited dynamic imports** — `import()` not supported
* **Spec non-compliant** — Some ESM semantics differ
* **v2.0 migration needed** — Full Module API still required

---

## Bundler Requirement

Because we don't resolve imports, users must bundle:

```bash
# Hono.js example
npm install hono
npx esbuild src/index.js --bundle --outfile=dist/app.js --format=esm
nano-rs run --entrypoint dist/app.js
```

Or create sliver:
```bash
nano-rs sliver create dist/ --output app.sliver
nano-rs run --sliver app.sliver
```

**Key point:** Bundling is NANO's philosophy (no npm resolution in runtime).

---

## Future: Full Module API (v2.0)

### Alternative implementation (not adopted)

```rust
// V8 Module API with VFS-backed resolution
let module = v8::Module::create_module(
    scope,
    source,
    |specifier| resolve_import(specifier, vfs),  // VFS lookup
)?;

module.instantiate(scope)?;
let result = module.evaluate(scope)?;
```

### Features

- VFS-backed `import` resolution
- Dynamic `import()` support
- Cyclic dependency handling
- Import maps
- Bare specifier resolution

### Timeline

- **v1.x:** Transformation (current)
- Full Module API implementation (not adopted; the shim approach shipped)

---

## Alternatives Rejected

### Option 1: Full V8 Module API — Deferred to v2.0

**Why (deferred):** Complex implementation, VFS import resolution, cyclic dependencies, longer timeline. Transformation unlocks frameworks immediately.

### Option 3: Hybrid — Rejected

**Why:** Adds complexity (two code paths). Either transformation works or full Module API needed. Hybrid is worst of both.

### Option 4: Wait — Rejected

**Why:** Would block all framework support. Cannot ship without Hono.js/Next.js/Astro support.

---

## Technical Debt

This is **accepted technical debt**:

```markdown
### ESM-01: Full V8 Module API

**Status:** Accepted  
**Status:** not implemented — the addEventListener/ESM shim approach was adopted

**Rationale:**
- Transformation works for all v1.x use cases
- Full Module API provides spec compliance
- User impact: Low (bundlers work today)
- Effort: High (import resolution, cyclic deps, etc.)

**Migration path:**
- v1.x: Bundled apps work via transformation
- v2.0: Can use native imports (optional upgrade)
```

---

## Related Decisions

* [ADR-004: VFS Architecture](004-vfs-architecture.md) — VFS needed for v2.0 import resolution
* Full Module API: not implemented (shim adopted)

---

## Code References

- `src/v8/module.rs` — Module loading and transformation
- `src/v8/transform.rs` — ESM-to-script transformation
- `src/v8/script.rs` — Script execution (classic)
- Phase 18 plans — Implementation details

---

## Examples

### Hono.js (Works Today)

```javascript
import { Hono } from 'hono';

const app = new Hono();
app.get('/', (c) => c.text('Hello'));

export default app;
```

**Build:**
```bash
npx esbuild src/index.js --bundle --outfile=dist/app.js
nano-rs run --entrypoint dist/app.js
```

### Next.js Static Export (Works Today)

```bash
next build  # Outputs to dist/
nano-rs run dist/
```

### Dynamic Import (v2.0)

```javascript
// NOT supported in v1.x
const module = await import('./config.js');

// Workaround: Use conditionals at build time
import config from './config.js';  // Bundler handles
```

---

*Last updated: 2026-04-21*
