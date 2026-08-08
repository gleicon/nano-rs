# NANO Runtime Compatibility Matrix

**Version:** 1.5.0  
**Last Updated:** 2026-05-02

---

## WinterTC Minimum Common APIs

| API | Status | Notes |
|-----|--------|-------|
| fetch() | ✅ Complete | Full implementation with Request/Response |
| Request | ✅ Complete | Constructor with all standard properties |
| Response | ✅ Complete | Constructor with status, headers, body |
| Headers | ✅ Complete | Map-like interface, case-insensitive |
| URL | ✅ Complete | Full URL parsing |
| URLSearchParams | ✅ Complete | Query string manipulation |
| TextEncoder | ✅ Complete | UTF-8 encoding |
| TextDecoder | ✅ Complete | UTF-8 decoding |
| console | ✅ Complete | log, error, warn methods |
| ReadableStream | ✅ Complete | WinterTC streams |
| WritableStream | ✅ Complete | WinterTC streams |
| AbortController | ✅ Complete | Signal-based cancellation |
| Blob | ✅ Complete | Binary data wrapper |
| FormData | ✅ Complete | Multipart form data |
| DOMException | ✅ Complete | Standard error types |
| structuredClone | ✅ Complete | Deep object cloning |
| performance.now() | ✅ Complete | High-res timer |

**Coverage:** 16/16 core WinterTC APIs (100%)

---

## WebCrypto APIs

| API | Status | Algorithms | Notes |
|-----|--------|------------|-------|
| crypto.getRandomValues | ✅ Complete | All TypedArray types | |
| crypto.subtle.digest | ✅ Complete | SHA-256, SHA-512 | |
| crypto.subtle.generateKey | ✅ Complete | AES-GCM, HMAC | |
| crypto.subtle.importKey | ✅ Complete | JWK format | AES-GCM, HMAC only |
| crypto.subtle.exportKey | ✅ Complete | JWK format | AES-GCM, HMAC only |
| crypto.subtle.encrypt | ✅ Complete | AES-GCM | |
| crypto.subtle.decrypt | ✅ Complete | AES-GCM | |
| crypto.subtle.sign | ✅ Complete | HMAC | |
| crypto.subtle.verify | ✅ Complete | HMAC | |
| crypto.subtle.deriveKey | ❌ Not Implemented | | Planned for v2.0 |
| RSA key operations | ❌ Not Implemented | | Planned for v2.0 (Phase 24) |
| ECDSA operations | ❌ Not Implemented | | Planned for v2.0 (Phase 24) |

**Coverage:** 9/12 implemented (75%)  
**v2.0 Planned:** RSA, ECDSA, deriveKey

---

## Node.js API Polyfills

| API | Status | Implementation | Notes |
|-----|--------|----------------|-------|
| Buffer.from() | ⚠️ Partial | From string, array, hex/base64 | Limited encodings |
| Buffer.alloc() | ✅ Complete | Allocate with size | |
| Buffer.toString() | ✅ Complete | UTF-8 decode | |
| setTimeout | ✅ Complete | Basic timer support | |
| setInterval | ✅ Complete | Basic timer support | |
| clearTimeout | ✅ Complete | Timer cancellation | |
| clearInterval | ✅ Complete | Timer cancellation | |
| require('fs') | ⚠️ Partial | Via VFS polyfill | Async methods only |
| fs.readFileSync | ⚠️ Partial | Limited support | Use async readFile |
| fs.writeFileSync | ⚠️ Partial | Limited support | Use async writeFile |
| fs.existsSync | ✅ Complete | Sync check | |

**Implemented via `require()`:**
- `require('path')` — join, dirname, basename, extname, resolve, isAbsolute, normalize
- `require('buffer')` — Buffer.from, alloc, isBuffer, concat (returns Uint8Array)
- `require('assert')` — ok, equal, strictEqual, notEqual
- `process.env` — app env vars (set via `env_vars` in config); `process.version`, `process.platform`

**NOT Implemented (by design):**
- Node.js http module — Use WinterTC fetch() instead
- Node.js net module — Raw sockets not supported
- Node.js os module — Not available
- Node.js stream module — Use WinterTC streams
- Node.js crypto module — Use WebCrypto instead

**Coverage:** ~14/20+ common APIs (~65%)

**Important:** NANO is NOT a Node.js replacement. It targets WinterTC (Web-interoperable Runtimes Community Group) APIs first, with Node.js polyfills for convenience.

---

## NANO-Specific APIs

| API | Status | Notes |
|-----|--------|-------|
| Nano.fs.readFile | ✅ Complete | Async file read from VFS |
| Nano.fs.writeFile | ✅ Complete | Async file write to VFS |
| Nano.fs.exists | ✅ Complete | Check file existence |
| Nano.fs.deleteFile | ✅ Complete | Remove files |
| Nano.fs.listDir | ⚠️ Partial | Directory listing (basic implementation) |
| Nano.fs.mkdir | ❌ Not Implemented | Planned for v2.0 |
| `nano:kv` (default KV) | ✅ Complete | `import { kv } from 'nano:kv'` — get/set/delete/list, getJSON/setJSON, hostname-namespaced |
| `nano:kv` (named namespace) | ✅ Complete | `openKV('name')` — isolated named KV store per app |
| `localStorage` shim | ✅ Available | Userland shim — see `examples/localStorage-shim.js` |

---

## Framework Compatibility

| Framework | Status | Notes |
|-----------|--------|-------|
| Hono.js | ✅ Supported | Full WinterTC compatibility |
| Next.js (static export) | ✅ Supported | Static assets + JS execution |
| Astro (static build) | ✅ Supported | Islands architecture |
| Cloudflare Workers | ⚠️ Mostly Compatible | Standard patterns work; KV, DO not available |
| Express.js | ❌ Not Compatible | Requires Node.js http module |
| Fastify | ❌ Not Compatible | Requires Node.js core modules |
| Nuxt (static) | ⚠️ Static only | Static generation works |
| Gatsby | ✅ Good | Static sites work perfectly |
| SvelteKit | ⚠️ Adapter needed | Use adapter-static or custom adapter |
| Remix | ⚠️ Limited | Edge adapter support needed |
| Fresh | ⚠️ Partial | Deno-specific, may need polyfills |

---

## Production Multi-Tenancy (v1.5.0)

| Feature | Status | Notes |
|---------|--------|-------|
| CPU Time Tracking | ✅ Implemented | Microsecond precision per request |
| CPU Time Limits | ✅ Implemented | 50ms default (Cloudflare-style) |
| Timer-based Termination | ✅ Implemented | Linux timer_create + V8 terminate |
| Memory Monitoring | ✅ Implemented | 4-tier pressure levels |
| Soft Eviction | ✅ Implemented | Graceful isolate draining |
| LRU Eviction | ✅ Implemented | Least Recently Used policy |
| Per-Tenant Metrics | ✅ Implemented | Auto-collected per hostname |
| Prometheus Export | ✅ Implemented | /admin/metrics endpoint |
| WASM Support | ✅ Implemented | Load, compile, execute |
| WASM JS API | ✅ Implemented | WebAssembly.* full API |
| WASM Sliver Support | ✅ Implemented | Cached compiled modules |

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ Complete | Fully implemented and tested |
| ⚠️ Partial | Works for common cases, limitations documented |
| 🚧 In Progress | Implementation underway |
| ❌ Not Implemented | Not available (may be planned or out of scope) |

---

## Test Coverage Summary

| Category | Tests | Passing | Percentage |
|----------|-------|---------|------------|
| API Compatibility | 26 | 26 | 100% |
| Comprehensive Suite | 27 | 27 | 100% |
| CRUD Operations | 6 | 6 | 100% |
| Cloudflare Worker | 6 | 6 | 100% |
| Production Multi-Tenancy | 91 | 91 | 100% |
| **Total** | **981** | **981** | **100%** |

*Last test run: 2026-05-02*

---

## Compatibility Claims vs Reality

### What "100%" Means

When we say "100% Complete" for WinterTC APIs, we mean:
- All core APIs are implemented
- All tests pass
- Full specification compliance

### What "55% Node.js Compatibility" Means

When we say ~55% for Node.js compatibility, we mean:
- Common APIs (Buffer, timers) are polyfilled
- Many Node.js modules are intentionally NOT supported (http, net, os)
- NANO is NOT a Node.js replacement

### Design Philosophy

NANO targets **WinterTC first, Node.js convenience second**:

1. Use `fetch()` instead of `http` module
2. Use `URL` instead of `path` module  
3. Use WebCrypto instead of Node.js `crypto`
4. Bundle your app with dependencies (no npm resolution)

---

## Migration from Node.js

See [Node.js Compatibility and Migration Guide](NODEJS_COMPAT.md) for detailed migration patterns.

Quick reference:

| Node.js Pattern | NANO Equivalent |
|-----------------|----------------|
| `http.createServer()` | `export default { fetch }` |
| `process.env.VAR` | Request headers or VFS config |
| `fs.readFileSync()` | `await Nano.fs.readFile()` |
| `crypto.createHash()` | `crypto.subtle.digest()` |
| `path.join()` | `new URL()` |

---

## Upcoming in v2.0

| Feature | Status |
|---------|--------|
| WebSocket Server | Planned |
| RSA/ECDSA Algorithms | Planned |
| Compression Streams | Planned |
| Inter-Isolate Messaging | Planned |
| Full VFS Directory Operations | Planned |

See [ROADMAP](../.planning/ROADMAP.md) for full details.

---

## See Also

- [API Reference](API.md) — All JavaScript APIs with examples
- [Node.js Migration Guide](NODEJS_COMPAT.md) — Detailed migration patterns
- [WinterTC Spec](https://wintertc.org/) — Standard APIs NANO implements
- [Architecture Decision Records](ADR/) — Design decisions behind compatibility choices

---

*Last updated: 2026-05-02*
