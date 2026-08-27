# NANO Runtime Compatibility Matrix


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
| crypto.subtle.digest | ✅ Complete | SHA-256, SHA-384, SHA-512 | |
| crypto.subtle.generateKey | ✅ Complete | AES-GCM, HMAC, RSA-OAEP, RSA-PSS, RSASSA-PKCS1-v1_5, ECDSA, ECDH | |
| crypto.subtle.importKey | ✅ Complete | AES-GCM, HMAC, RSA-*, ECDSA, ECDH, PBKDF2 | |
| crypto.subtle.exportKey | ✅ Complete | JWK format | AES-GCM, HMAC |
| crypto.subtle.encrypt | ✅ Complete | AES-GCM, RSA-OAEP | |
| crypto.subtle.decrypt | ✅ Complete | AES-GCM, RSA-OAEP | |
| crypto.subtle.sign | ✅ Complete | HMAC, RSA-PSS, RSASSA-PKCS1-v1_5, ECDSA | |
| crypto.subtle.verify | ✅ Complete | HMAC, RSA-PSS, RSASSA-PKCS1-v1_5, ECDSA | |
| crypto.subtle.deriveBits | ✅ Complete | PBKDF2 (HMAC-SHA-256/384/512), ECDH | length: positive multiple of 8, ≤ 8192 bits |
| crypto.subtle.deriveKey | ✅ Complete | PBKDF2, ECDH | |

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

**Implemented via `require()` (added in v2.2.2):**

See full working example: [`examples/node-compat.js`](../examples/node-compat.js)

```javascript
// require('path') — join, dirname, basename, extname, normalize, isAbsolute
const path = require('path');
path.join('/var', 'app', 'config.json'); // → "/var/app/config.json"
path.dirname('/var/app/config.json');    // → "/var/app"
path.basename('/var/app/config.json');   // → "config.json"
path.extname('/var/app/config.json');    // → ".json"
path.isAbsolute('/abs');                 // → true
path.normalize('/var//app/../app/f');    // → "/var/app/f"
```

```javascript
// require('buffer') — Buffer.from, alloc, isBuffer, concat (returns Uint8Array)
const { from, alloc, isBuffer, concat } = require('buffer');
const a = from('hello ');
const b = from('world');
new TextDecoder().decode(concat([a, b])); // → "hello world"
isBuffer(a);                              // → true
```

```javascript
// require('assert') — ok, equal, strictEqual, notEqual
const assert = require('assert');
assert.ok(true);
assert.equal(1 + 1, 2);
assert.strictEqual('a', 'a');
assert.notEqual(1, 2);
```

```javascript
// process.env — app env vars set via env_vars config; process.version; process.platform
// Config: { "apps": [{ "env_vars": { "NODE_ENV": "production" } }] }
process.env.NODE_ENV;  // → "production"
process.version;       // → "v22.11.0"
process.platform;      // → "linux"
```

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
| Nano.fs.mkdir | ❌ Not Implemented | Not supported (flat VFS namespace) |
| `nano:kv` (default KV) | ✅ Complete | `import { kv } from 'nano:kv'` — get/set/delete/list, getJSON/setJSON, hostname-namespaced |
| `nano:kv` (named namespace) | ✅ Complete | `openKV('name')` — isolated named KV store per app |
| `localStorage` shim | ✅ Available | Userland shim — see [`examples/localStorage-shim.js`](../examples/localStorage-shim.js) |

### `nano:kv` — Persistent Key-Value Store (v2.2.2+)

Keys are stored in EdgeStore and automatically namespaced by hostname, so two apps
on the same process never see each other's data.

See full examples: [`examples/kv-counter.js`](../examples/kv-counter.js), [`examples/kv-namespaced.js`](../examples/kv-namespaced.js)

```javascript
import { kv, openKV } from 'nano:kv';

// Default namespace — scoped to current app hostname
await kv.set('hits', new TextEncoder().encode('42'));
const raw = await kv.get('hits');
new TextDecoder().decode(raw); // → "42"
await kv.delete('hits');

// JSON helpers — serialize/deserialize automatically
await kv.setJSON('config', { version: 3, enabled: true });
const cfg = await kv.getJSON('config'); // → { version: 3, enabled: true }

// Named namespaces — isolated slices within one app
const cache    = openKV('cache');
const sessions = openKV('sessions');
await cache.set('user:1', new TextEncoder().encode('Alice'));
await sessions.setJSON('tok:abc', { uid: 1, exp: Date.now() + 3600000 });

// List keys by prefix
const entries = await cache.list('user:');
// entries: [['user:1', Uint8Array], ...]
```

### `localStorage` Shim (userland, built on `nano:kv`)

See full example: [`examples/localStorage-shim.js`](../examples/localStorage-shim.js)

```javascript
import { openKV } from 'nano:kv';

const store = openKV('localStorage');

globalThis.localStorage = {
  async getItem(key)       { const b = await store.get(String(key)); return b ? new TextDecoder().decode(b) : null; },
  async setItem(key, val)  { await store.set(String(key), new TextEncoder().encode(String(val))); },
  async removeItem(key)    { await store.delete(String(key)); },
  async clear()            { const e = await store.list(''); await Promise.all(e.map(([k]) => store.delete(k))); },
};
```

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

## Production Multi-Tenancy

| Feature | Status | Notes |
|---------|--------|-------|
| CPU Time Tracking | ✅ Implemented | Microsecond precision per request |
| CPU Time Limits | ✅ Implemented | 50ms default (Cloudflare-style) |
| Timer-based Termination | ✅ Implemented | Linux timer_create + V8 terminate |
| Per-isolate heap cap + OOM | ✅ Implemented | V8 heap limit, terminate on approach, recycle after N requests |
| Soft eviction (memory pressure) | ✅ Implemented | Per-worker `MemoryMonitor` recycles the isolate at Critical/Emergency pressure |
| Per-Tenant Metrics | ✅ Implemented | Auto-collected per hostname |
| Prometheus Export | ✅ Implemented | /admin/metrics endpoint |
| WASM load / compile / instantiate | ✅ Implemented | `WebAssembly.*` API; synchronous exports execute |
| WASM async execution | ⚠️ Partial | Prefer synchronous exports; async-heavy WASM flows may not resolve to completion |

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ Complete | Fully implemented and tested |
| ⚠️ Partial | Works for common cases, limitations documented |
| ❌ Not Implemented | Not available |

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
| `process.env.VAR` | `process.env.VAR` (set via `env_vars` in config) |
| `fs.readFileSync()` | `await Nano.fs.readFile()` |
| `crypto.createHash()` | `crypto.subtle.digest()` |
| `path.join()` | `new URL()` |

---

See [CHANGELOG](../CHANGELOG.md) for shipped features.

---

## See Also

- [API Reference](API.md) — All JavaScript APIs with examples
- [Node.js Migration Guide](NODEJS_COMPAT.md) — Detailed migration patterns
- [WinterTC Spec](https://wintertc.org/) — Standard APIs NANO implements
- [Architecture Decision Records](ADR/) — Design decisions behind compatibility choices

---

*Last updated: 2026-08-08*
