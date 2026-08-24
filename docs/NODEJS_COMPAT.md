# Node.js Compatibility and Migration Guide

**Version:** 2.7.0  
**Last Updated:** 2026-05-02

---

## Overview

NANO provides partial Node.js compatibility for common patterns, but is **NOT a Node.js replacement**. It targets WinterTC (Web-interoperable Runtimes Community Group) APIs first, with Node.js polyfills for convenience.

**Key Differences:**
- NANO is multi-tenant by design (one process, many isolated apps)
- No npm resolution (apps must be bundled)
- No Node.js internal modules (http, net, os, etc.)
- Cold start optimized for edge deployment (precompiled bytecode)

**Target Use Case:**
- Edge functions, serverless workloads
- Static sites with light dynamic processing
- Microservices that fit WinterTC APIs
- Cloudflare Workers migration

**NOT Suitable For:**
- Traditional Node.js servers with heavy dependencies
- Apps using native modules (C++ addons)
- Long-running processes with stateful connections

---

## Compatibility Matrix

### Core Modules

| Module | Status | Coverage | Notes |
|--------|--------|----------|-------|
| **buffer** | ⚠️ Partial | ~60% | Buffer.from, alloc, toString. Limited encodings. |
| **fs** | ⚠️ Partial | ~30% | Async methods + Sync methods via VFS. Limited API surface. |
| **crypto** | ⚠️ Partial | ~20% | WebCrypto only. No Node.js crypto module. |
| **http** | ❌ Not Supported | 0% | Use WinterTC `fetch()` instead |
| **https** | ❌ Not Supported | 0% | Use WinterTC `fetch()` instead |
| **net** | ❌ Not Supported | 0% | Raw sockets not available |
| **os** | ❌ Not Supported | 0% | No system information access |
| **path** | ✅ Supported | 100% | `require('path')` — join, dirname, basename, extname, resolve, isAbsolute, normalize |
| **process** | ⚠️ Partial | ~40% | `process.env` (app env vars), `process.version`, `process.platform` — no `process.exit` |
| **stream** | ⚠️ Partial | ~40% | WinterTC ReadableStream/WritableStream only |
| **url** | ⚠️ Partial | ~80% | URL, URLSearchParams supported |
| **util** | ❌ Not Supported | 0% | Not implemented |
| **events** | ❌ Not Supported | 0% | EventEmitter not available |

**Overall Node.js Compatibility: ~25%**

### Globals

| Global | Status | Alternative |
|--------|--------|-------------|
| `Buffer` | ⚠️ Partial | Use `Uint8Array` for new code |
| `console` | ✅ Full | Same API |
| `setTimeout` | ✅ Full | Same API |
| `setInterval` | ✅ Full | Same API |
| `clearTimeout` | ✅ Full | Same API |
| `clearInterval` | ✅ Full | Same API |
| `fetch` | ✅ Full | Native WinterTC implementation |
| `TextEncoder` | ✅ Full | Same API |
| `TextDecoder` | ✅ Full | Same API |
| `crypto` | ⚠️ Partial | WebCrypto only (no Node.js crypto) |
| `URL` | ✅ Full | Same API |
| `URLSearchParams` | ✅ Full | Same API |
| `process` | ❌ Not Supported | Use request headers or config |
| `global` | ❌ Not Supported | Not applicable in isolate model |
| `__dirname` | ❌ Not Supported | Use relative paths |
| `__filename` | ❌ Not Supported | Use relative paths |
| `require` | ⚠️ Partial | Limited to built-in modules |

---

## Migration Guide

### 1. Replace `http` module with `fetch()`

**Node.js (Old):**
```javascript
const http = require('http');

const server = http.createServer((req, res) => {
  res.writeHead(200, { 'Content-Type': 'text/plain' });
  res.end('Hello');
});

server.listen(3000);
```

**NANO (New):**
```javascript
export default {
  async fetch(request) {
    return new Response('Hello', {
      headers: { 'Content-Type': 'text/plain' }
    });
  }
};
```

**Key Changes:**
- No server creation — NANO manages HTTP
- Handler receives WinterTC `Request` object
- Return `Response` object
- Async by default

---

### 2. `process.env` — now supported via app env vars

`process.env` is available directly. Set environment variables in the app config `env_vars` section; they are injected per-isolate and namespaced to the app.

```javascript
// Works as-is — values come from the app's env_vars config
const dbUrl = process.env.DATABASE_URL;
```

**Config:**
```json
{
  "apps": [{
    "hostname": "api.example.com",
    "env_vars": { "DATABASE_URL": "postgres://..." }
  }]
}
```

If you need values NOT in `env_vars`, the fallback options still work:

**Option 1: Request headers:**
```javascript
export default {
  async fetch(request) {
    const dbUrl = request.headers.get('X-Database-URL');
    // ...
  }
};
```

**Option 2: VFS config file:**
```javascript
export default {
  async fetch(request) {
    const config = JSON.parse(
      await Nano.fs.readFile('/data/config.json', { encoding: 'utf-8' })
    );
    const dbUrl = config.database_url;
    // ...
  }
};
```

**Option 3: Build-time bundling:**
```javascript
// config injected at build time by bundler
import config from './config.json';
const dbUrl = config.database_url;
```

---

### 3. Replace `fs` with `Nano.fs` or bundled code

**Node.js (Old):**
```javascript
const fs = require('fs');
const data = fs.readFileSync('./data.json', 'utf8');
```

**NANO (New) — Option 1: Use Nano.fs (VFS):**
```javascript
const data = await Nano.fs.readFile('/data/config.json', { encoding: 'utf-8' });
```

**Option 2: Bundle data at build time:**
```javascript
// Bundler embeds JSON content
import config from './config.json';
```

---

### 4. Replace `path` with `URL` API

**Node.js (Old):**
```javascript
const path = require('path');
const fullPath = path.join(__dirname, 'config.json');
```

**NANO (New):**
```javascript
const url = new URL('config.json', import.meta.url);
const response = await fetch(url);
const config = await response.json();
```

Or use relative paths with `Nano.fs`:
```javascript
const config = await Nano.fs.readFile('/data/config.json', { encoding: 'utf-8' });
```

---

### 5. Replace `crypto` with WebCrypto

**Node.js (Old):**
```javascript
const crypto = require('crypto');
const hash = crypto.createHash('sha256').update(data).digest('hex');
```

**NANO (New):**
```javascript
const encoder = new TextEncoder();
const data = encoder.encode('Hello');
const hashBuffer = await crypto.subtle.digest('SHA-256', data);
const hashArray = Array.from(new Uint8Array(hashBuffer));
const hashHex = hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
```

**Note:** WebCrypto is async, returns ArrayBuffers, uses different API shape.

See [API Reference](API.md) for full WebCrypto documentation.

---

### 6. Replace `setTimeout` callback patterns with async/await

**Node.js (Old):**
```javascript
function delay(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}
```

**NANO (New):**
```javascript
// Same pattern works in NANO
function delay(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// Usage
await delay(1000);
```

**Note:** Timer duration counts toward request CPU time limits.

---

## Common Patterns

### Express.js-style routing

NANO doesn't support Express directly, but you can use Hono.js (Express-like):

```javascript
import { Hono } from 'hono';

const app = new Hono();

app.get('/', (c) => c.text('Hello'));
app.get('/users/:id', (c) => {
  const id = c.req.param('id');
  return c.json({ id });
});

export default app;
```

**Bundle with Hono:**
```bash
npm install hono
npx esbuild src/index.js --bundle --outfile=dist/app.js --format=esm
```

---

### Environment-specific configuration

**Node.js (Old):**
```javascript
const config = require(`./config.${process.env.NODE_ENV}.json`);
```

**NANO (New) — Build-time bundling:**
```javascript
// Build-time environment substitution
import config from './config.json';
```

**With esbuild:**
```bash
NODE_ENV=production npx esbuild src/index.js --bundle --define:process.env.NODE_ENV=\"production\"
```

**Runtime lookup:**
```javascript
const env = request.headers.get('X-Environment') || 'production';
const config = await Nano.fs.readFile(`/data/config.${env}.json`, { encoding: 'utf-8' });
```

---

### Database connections

**Node.js (Old):**
```javascript
const { Client } = require('pg');
const client = new Client({ connectionString: process.env.DATABASE_URL });
await client.connect();
```

**NANO (New):**
```javascript
// Use fetch-based HTTP database client (if available)
// Or WebSocket-based client
// Or external database proxy

// For edge deployment, prefer:
// - Durable connection pools outside NANO
// - Connectionless protocols (HTTP-based DBs)
// - Short-lived connections per request

export default {
  async fetch(request) {
    // HTTP-based database query
    const response = await fetch('https://db-proxy.internal/query', {
      method: 'POST',
      headers: { 'Authorization': 'Bearer token' },
      body: JSON.stringify({ query: 'SELECT * FROM users' })
    });
    return response;
  }
};
```

---

## Package Compatibility

| Package Category | Compatibility | Notes |
|----------------|---------------|-------|
| **Pure ESM packages** | ✅ Excellent | Any package using standard Web APIs |
| **Node.js-specific packages** | ❌ Poor | Packages using http, net, fs directly |
| **Bundled applications** | ✅ Good | Webpack, Rollup, esbuild output |
| **Hono.js** | ✅ Excellent | Designed for WinterTC |
| **Next.js (static export)** | ✅ Good | Static HTML/JS output works |
| **Astro (static build)** | ✅ Good | Islands architecture preserved |
| **React/Vue/Svelte** | ✅ Good | Client-side bundles work |
| **Database ORMs** | ⚠️ Partial | Need HTTP-based drivers |

---

## Testing Compatibility

**Node.js (Old):**
```bash
npm test  # Uses Jest, Mocha, etc.
```

**NANO (New):**
```bash
# Test with WinterTC-compatible test runner
# Or integration tests against running NANO

# Example: Use Vitest with happy-dom
npm install -D vitest
npx vitest

# Integration test
curl http://localhost:8080/test-endpoint
```

---

## Debugging

**Node.js (Old):**
```bash
node --inspect app.js
```

**NANO (New):**
```bash
# Use logging and admin API
nano-rs run --config config.json --verbose

# Monitor via admin API
curl -H "X-API-Key: secret" http://localhost:8889/isolates
```

---

## When NOT to Migrate

NANO is NOT suitable for:
- Apps requiring Node.js native modules
- Long-running WebSocket servers (support planned in v2.0)
- Apps using extensive Node.js built-in modules
- Traditional server-side rendering with Node.js streams

**Consider staying with Node.js if:**
- You use native dependencies (C++ addons)
- You need extensive file system operations
- You rely on Node.js-specific behavior
- Your app doesn't fit WinterTC patterns

---

## Framework Compatibility

| Framework | Compatibility | Notes |
|-----------|---------------|-------|
| **Next.js** | ⚠️ Static only | Static export (`next export`) works. SSR not supported. |
| **Nuxt** | ⚠️ Static only | Static generation works. Server features not supported. |
| **Gatsby** | ✅ Good | Static sites work perfectly |
| **Astro** | ✅ Excellent | Static and islands architecture fully supported |
| **SvelteKit** | ⚠️ Adapter needed | Use adapter-static or custom adapter |
| **Remix** | ⚠️ Limited | Edge adapter support needed |
| **Hono.js** | ✅ Excellent | Native WinterTC support |
| **Fresh** | ⚠️ Partial | Deno-specific, may need polyfills |
| **Express.js** | ❌ Not supported | Requires Node.js http module |
| **Fastify** | ❌ Not supported | Requires Node.js core modules |

---

## Build Configuration

### esbuild

```bash
npx esbuild src/index.js \
  --bundle \
  --outfile=dist/app.js \
  --format=esm \
  --platform=neutral \
  --target=es2022
```

### Rollup

```javascript
// rollup.config.js
export default {
  input: 'src/index.js',
  output: {
    file: 'dist/app.js',
    format: 'esm'
  },
  external: [] // Bundle everything
};
```

### Webpack

```javascript
// webpack.config.js
module.exports = {
  entry: './src/index.js',
  output: {
    filename: 'app.js',
    library: { type: 'module' }
  },
  experiments: { outputModule: true }
};
```

---

## See Also

- [API Reference](API.md) — All available JavaScript APIs
- [WinterTC Spec](https://wintertc.org/) — Standard APIs NANO implements
- [Compatibility Matrix](COMPATIBILITY.md) — Full feature compatibility
- [CLI Reference](CLI.md) — Commands for running apps

---

*Last updated: 2026-05-02*
