# NANO Examples

Complete, copy-paste ready examples for common NANO use cases.

---

## 1. Simple JavaScript App (5 minutes)

A minimal HTTP handler using the WinterTC fetch API.

### Step 1: Create the JavaScript file

Create `app.js`:

```javascript
export default {
  async fetch(request) {
    const url = new URL(request.url);
    
    if (url.pathname === '/') {
      return new Response('Hello from NANO!', {
        headers: { 'Content-Type': 'text/plain' }
      });
    }
    
    if (url.pathname === '/json') {
      // Option 1: Return a plain object (simplest, always works)
      return {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          message: 'Hello from NANO!',
          runtime: 'nano-rs',
          time: Date.now()
        })
      };
      
      // Option 2: Use Response.json() static method (WinterTC standard)
      // return Response.json({
      //   message: 'Hello from NANO!',
      //   runtime: 'nano-rs',
      //   time: Date.now()
      // });
    }
    
    return new Response('Not Found', { status: 404 });
  }
};
```

### Step 2: Create the config

Create `nano.json`:

```json
{
  "server": {
    "port": 8080
  },
  "apps": [
    {
      "hostname": "localhost",
      "entrypoint": "./app.js",
      "limits": {
        "workers": 2,
        "memory_mb": 64
      }
    }
  ]
}
```

### Step 3: Run

```bash
# Terminal 1: Start the server
nano-rs run --config nano.json

# Terminal 2: Test
 curl http://localhost:8080/
# Output: Hello from NANO!

curl http://localhost:8080/json
# Output: {"message":"Hello from NANO!","runtime":"nano-rs","time":1234567890}
```

---

## 2. WebAssembly App (10 minutes)

Compile Rust to WASM and call it from JavaScript.

### Step 1: Create the Rust WASM module

Create `wasm/Cargo.toml`:

```toml
[package]
name = "calc"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wasm-bindgen = "0.2"

[profile.release]
opt-level = 3
lto = true
```

Create `wasm/src/lib.rs`:

```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

#[wasm_bindgen]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

### Step 2: Compile to WASM

```bash
cd wasm
wasm-pack build --target web --out-dir ../pkg
```

### Step 3: Create the JavaScript handler

Create `wasm-app.js`:

```javascript
import wasmModule from './pkg/calc.js';

let wasm;

async function initWasm() {
  wasm = await wasmModule();
}

// Initialize on first request
let initPromise = initWasm();

export default {
  async fetch(request) {
    await initPromise;
    
    const url = new URL(request.url);
    
    if (url.pathname === '/add') {
      const a = parseInt(url.searchParams.get('a') || '0');
      const b = parseInt(url.searchParams.get('b') || '0');
      const result = wasm.add(a, b);
      return Response.json({ a, b, result });
    }
    
    if (url.pathname === '/fib') {
      const n = parseInt(url.searchParams.get('n') || '10');
      const result = wasm.fibonacci(n);
      return Response.json({ n, result });
    }
    
    return new Response('Usage: /add?a=5&b=3 or /fib?n=20', {
      headers: { 'Content-Type': 'text/plain' }
    });
  }
};
```

### Step 4: Create config

Create `nano-wasm.json`:

```json
{
  "server": {
    "port": 8080
  },
  "apps": [
    {
      "hostname": "localhost",
      "entrypoint": "./wasm-app.js",
      "limits": {
        "workers": 2,
        "memory_mb": 128
      }
    }
  ]
}
```

### Step 5: Run

```bash
# Terminal 1: Start
nano-rs run --config nano-wasm.json

# Terminal 2: Test WASM calls
curl "http://localhost:8080/add?a=5&b=3"
# Output: {"a":5,"b":3,"result":8}

curl "http://localhost:8080/fib?n=20"
# Output: {"n":20,"result":6765}
```

---

## 3. Slivers: Create and Deploy (5 minutes)

Slivers are portable snapshots with ~267µs cold starts.

### Step 1: Create your app

Create `sliver-demo.js`:

```javascript
export default {
  async fetch(request) {
    const url = new URL(request.url);
    
    if (url.pathname === '/') {
      return new Response('Hello from Sliver!', {
        headers: { 'Content-Type': 'text/plain' }
      });
    }
    
    if (url.pathname === '/version') {
      return Response.json({ 
        version: '1.0.0',
        deployed: new Date().toISOString()
      });
    }
    
    return new Response('Not Found', { status: 404 });
  }
};
```

### Step 2: Run the app (for testing)

Create `dev.json`:

```json
{
  "server": { "port": 8080 },
  "apps": [{
    "hostname": "localhost",
    "entrypoint": "./sliver-demo.js",
    "limits": { "workers": 2 }
  }]
}
```

```bash
# Terminal 1: Run for testing
nano-rs run --config dev.json

# Terminal 2: Verify it works
curl http://localhost:8080/
# Output: Hello from Sliver!
```

### Step 3: Create a sliver

```bash
# In another terminal - create from the running app
nano-rs sliver create localhost --output my-app-v1.sliver

# Verify it was created
ls -lh my-app-v1.sliver
```

### Step 4: Stop the dev server

Press `Ctrl+C` in Terminal 1 to stop the dev server.

### Step 5: Run from sliver

```bash
# Terminal 1: Run directly from sliver (no config needed!)
nano-rs run --sliver my-app-v1.sliver --port 8080

# Terminal 2: Test - should have identical behavior
curl http://localhost:8080/
# Output: Hello from Sliver!

curl http://localhost:8080/version
# Output: {"version":"1.0.0","deployed":"2026-01-01T12:00:00Z"}
```

### Production: Use sliver in config

Create `production.json`:

```json
{
  "server": { "port": 80 },
  "apps": [{
    "hostname": "api.example.com",
    "sliver": "./my-app-v1.sliver",
    "limits": {
      "workers": 8,
      "memory_mb": 128
    }
  }]
}
```

```bash
# Production run
nano-rs run --config production.json
```

---

## Quick Reference

### Commands

```bash
# Run with config
nano-rs run --config nano.json

# Run directly from JS
nano-rs run --sliver app.sliver

# Create sliver from running app
nano-rs sliver create <hostname> --output <name>.sliver

# List all slivers
nano-rs sliver list
```

### Config Structure

```json
{
  "server": {
    "port": 8080,
    "host": "0.0.0.0"
  },
  "apps": [{
    "hostname": "localhost",
    "entrypoint": "./app.js",
    "sliver": null,
    "limits": {
      "workers": 4,
      "memory_mb": 128,
      "timeout_secs": 30,
      "cpu_time_ms": 50
    }
  }]
}
```

### Testing

```bash
# Basic test
curl http://localhost:8080/

# With Host header (for multi-tenant)
curl -H "Host: api.example.com" http://localhost:8080/

# JSON endpoint
curl http://localhost:8080/json
```

---

## Understanding Workers and Request Routing

### What is a Worker?

A **worker** is a dedicated OS thread that:
- Owns one V8 isolate (JavaScript execution environment)
- Processes requests sequentially in a loop
- Cannot share isolates with other workers (V8 requirement)

### How Requests Are Distributed

NANO uses **round-robin routing** to distribute requests:

```
Request 1 → Worker 0
Request 2 → Worker 1
Request 3 → Worker 2
Request 4 → Worker 0 (wraps around)
```

Each worker has its own isolate, so:
- **No shared state** between workers (isolation)
- **Same hostname** can hit different workers
- **Memory is per-isolate** (4 workers = 4× memory usage)

### Worker Logs Explained

When you run NANO, you'll see two types of logs:

**HTTP Access Logs** (from the HTTP layer, includes which worker processed the request):
```
HTTP GET / → 200 in 5.23ms (worker: 0)
HTTP GET /json → 200 in 3.45ms (worker: 1)
HTTP GET /notfound → 404 in 1.20ms (worker: 2)
```

**Worker Processing Logs** (from inside the worker thread):
```
Worker 0 processed GET / → 200 in 5ms (isolate: worker_0_localhost)
Worker 1 processed GET /json → 200 in 3ms (isolate: worker_1_localhost)
```

**Log Fields:**
- `worker_id` - Which worker thread (0, 1, 2...)
- `isolate_id` - V8 isolate identifier
- `hostname` - Virtual host/tenant
- `method` - HTTP method (GET, POST, etc.)
- `path` - Request path
- `status` - HTTP status code (200, 404, 500...)
- `duration_ms` - Processing time

### Configuring Workers

Set workers per app in your config:

```json
{
  "apps": [{
    "hostname": "localhost",
    "entrypoint": "./app.js",
    "limits": {
      "workers": 4
    }
  }]
}
```

**Worker Guidelines:**
- **Low traffic:** 1-2 workers
- **Standard:** 4 workers (default)
- **High traffic:** 8-16 workers
- **Max:** 32 workers per app

**Memory considerations:**
- Each worker = 1 isolate = memory_limit_mb RAM
- 4 workers × 128MB = ~512MB total

---

## Troubleshooting

### "Failed to parse config JSON: unknown field `port`"

Your config has `port` at the wrong level. Put it in `server`:

```json
{
  "server": { "port": 8080 },  // ✓ Correct
  "apps": [...]
}
```

NOT:

```json
{
  "port": 8080,  // ✗ Wrong
  "apps": [...]
}
```

### "Cannot find module"

Make sure your `entrypoint` path is correct. Use `./` for relative paths:

```json
"entrypoint": "./app.js"  // ✓ Correct
```

### Sliver not found

If using `--sliver`, check the file exists:

```bash
ls -lh my-app.sliver
nano-rs run --sliver ./my-app.sliver
```

### Port already in use

```bash
# Find and kill the process
lsof -ti:8080 | xargs kill -9
```

---

## Understanding NANO Logs

NANO uses structured JSON logging with request tracing across the system.

### Log Format

All logs are JSON with consistent fields:

```json
{
  "ts": "2026-05-03T12:34:56.789Z",
  "level": "INFO",
  "message": "HTTP GET / - 200 in 5.23ms (worker: 0, isolate: iso_a3f7b2d8_00000001)",
  "request_id": "req_a3f7b2d8",
  "worker_id": 0,
  "isolate_id": "iso_a3f7b2d8_00000001",
  "hostname": "localhost",
  "event": "src/http/router.rs:123",
  "fields": {
    "method": "GET",
    "path": "/",
    "status": 200,
    "duration_ms": "5.23",
    "worker_id": 0,
    "isolate_id": "iso_a3f7b2d8_00000001"
  }
}
```

### Key Fields

| Field | Description |
|-------|-------------|
| `ts` | ISO 8601 timestamp |
| `level` | Log level: DEBUG, INFO, WARN, ERROR |
| `message` | Human-readable summary |
| `request_id` | Unique hash tracking request: `req_{uuid_first_8}` |
| `worker_id` | Which worker thread processed the request (0, 1, 2...) |
| `isolate_id` | Unique hash for V8 isolate instance: `iso_{uuid}_{counter}` |
| `hostname` | Virtual host the request was routed to |
| `event` | Source code location |
| `fields` | Event-specific key-value pairs |

### Workers vs Isolates

NANO uses separate concepts for **workers** and **isolates**:

- **Worker**: A dedicated OS thread (identified by number: 0, 1, 2...)
  - Lives for the entire process lifetime
  - Handles many requests over time
  
- **Isolate**: A V8 JavaScript sandbox (identified by unique hash: `iso_a3f7b2d8_00000001`)
  - Created fresh when a worker starts
  - Replaced after OOM or memory pressure
  - Each isolate instance has a unique hash, even on the same worker

**Round-robin routing**: Requests cycle through workers (0, 1, 2, ..., 0, 1...)
**Thread affinity**: Each isolate stays on its assigned thread
**Context reset**: ~5ms between requests (isolate reused, context fresh)

Configure workers per app in your config:

```json
{
  "apps": [{
    "hostname": "localhost",
    "limits": { "workers": 4 }
  }]
}
```

### Request Lifecycle in Logs

Each request generates multiple log entries:

1. **HTTP Access Log** (from router):
   - Shows method, path, status, duration
   - Includes `worker_id` and `isolate_id` when available
   - Example: `HTTP GET / - 200 in 5.23ms (worker: 0, isolate: iso_a3f7b2d8_00000001)`

2. **Worker Processing Log** (from worker thread):
   - Shows the same request from the worker's perspective
   - Includes all context: `request_id`, `worker_id`, `isolate_id`, `hostname`
   - Example: `Worker 0 processed request req_a3f7b2d8: GET / - 200 in 5ms (isolate: iso_a3f7b2d8_00000001)`

3. **Isolate Execution Log** (from V8 runtime):
   - Shows JavaScript handler execution
   - Includes memory usage and any JS errors

### Tracing Requests: request_id + worker_id + isolate_id

Use the three-part combo to trace any request through the system:

| Combo | Purpose | Example |
|-------|---------|---------|
| `request_id` | Track a single HTTP request end-to-end | `req_a3f7b2d8` |
| `worker_id` | See which thread handled it | `0` |
| `isolate_id` | Identify the exact V8 instance | `iso_a3f7b2d8_00000001` |

**OOM Recovery Example** - Notice the isolate_id changes after memory pressure:
```json
// Request 1 - Normal execution
{"request_id":"req_111","worker_id":0,"isolate_id":"iso_a3f7b2d8_00000001","message":"HTTP GET / - 200 in 5ms"}

// Request 2 - Same worker, NEW isolate after OOM
{"request_id":"req_222","worker_id":0,"isolate_id":"iso_b8e4c9f1_00000002","message":"HTTP GET / - 200 in 8ms"}
```

### Reading the Logs

```bash
# Pretty-print JSON logs
nano-rs run -c config.toml | jq .

# Filter for specific worker
nano-rs run -c config.toml | jq 'select(.worker_id == 0)'

# Filter for specific isolate (e.g., after OOM to see replacement)
nano-rs run -c config.toml | jq 'select(.isolate_id | contains("00000002"))'

# Filter for slow requests (>100ms)
nano-rs run -c config.toml | jq 'select(.fields.duration_ms | tonumber > 100)'

# Follow a single request by ID
nano-rs run -c config.toml | jq 'select(.request_id == "req_a3f7b2d8")'

# See all requests that hit a specific isolate
nano-rs run -c config.toml | jq 'select(.isolate_id == "iso_a3f7b2d8_00000001")'

# Show isolate age on every request
RUST_LOG=debug nano-rs run -c config.toml 2>&1 | grep "received request"

# Monitor memory pressure and OOM events
RUST_LOG=info nano-rs run -c config.toml 2>&1 | grep -E "(memory pressure|OOM|evicting)"
```

---

## Debugging and Profiling

For comprehensive debugging and profiling guidance, see:

**[Debugging and Profiling NANO Isolates](docs/DEBUGGING_ISOLATES.md)**

This guide covers:
- Understanding isolate lifecycle and the three-part tracing combo (`request_id` + `worker_id` + `isolate_id`)
- Debugging isolate age, OOM recovery, and memory pressure
- Profiling context reset timing and memory usage
- Common debugging scenarios and solutions
- jq queries for log analysis

---

## 4. Persistent KV Storage (`nano:kv`)

NANO includes a built-in EdgeStore-backed key-value store accessible as `nano:kv`.
Keys are automatically namespaced by hostname — no cross-tenant leakage.

Full example: [`examples/kv-counter.js`](examples/kv-counter.js)

```javascript
import { kv } from 'nano:kv';

export default {
  async fetch(request) {
    const raw = await kv.get('hits');
    const hits = raw ? parseInt(new TextDecoder().decode(raw), 10) : 0;
    const next = hits + 1;
    await kv.set('hits', new TextEncoder().encode(String(next)));

    return new Response(JSON.stringify({ hits: next }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  },
};
```

```bash
curl http://localhost:8080/
# {"hits":1}
curl http://localhost:8080/
# {"hits":2}
```

### Multiple KV namespaces with JSON helpers

Full example: [`examples/kv-namespaced.js`](examples/kv-namespaced.js)

```javascript
import { kv, openKV } from 'nano:kv';

const cache = openKV('cache');
const sessions = openKV('sessions');

export default {
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === '/set' && request.method === 'POST') {
      const body = await request.json();
      await cache.setJSON(body.key, body.value);
      return new Response(JSON.stringify({ ok: true }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/get') {
      const key = url.searchParams.get('key');
      const value = await cache.getJSON(key);
      return new Response(JSON.stringify({ key, value }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/list') {
      const prefix = url.searchParams.get('prefix') ?? '';
      const entries = await cache.list(prefix);
      const result = entries.map(([k, v]) => ({
        key: k,
        value: new TextDecoder().decode(v),
      }));
      return new Response(JSON.stringify(result), {
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response('Not Found', { status: 404 });
  },
};
```

```bash
curl -X POST http://localhost:8080/set -d '{"key":"x","value":42}' -H 'content-type: application/json'
curl "http://localhost:8080/get?key=x"
# {"key":"x","value":42}
```

---

## 5. Node.js Compatibility (`require`, `process.env`)

NANO polyfills the most common Node.js APIs so npm packages that use `path`,
`buffer`, or `assert` work without modification.

Full example: [`examples/node-compat.js`](examples/node-compat.js)

```javascript
const path = require('path');
const { from: bufferFrom, isBuffer, concat } = require('buffer');
const assert = require('assert');

export default {
  async fetch(request) {
    const url = new URL(request.url);

    if (url.pathname === '/path') {
      const joined = path.join('/var', 'app', 'config.json');
      const dir = path.dirname(joined);
      const base = path.basename(joined);
      const ext = path.extname(joined);
      return new Response(JSON.stringify({ joined, dir, base, ext }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/buffer') {
      const a = bufferFrom('hello ');
      const b = bufferFrom('world');
      const combined = concat([a, b]);
      const text = new TextDecoder().decode(combined);
      return new Response(JSON.stringify({ text, isBuffer: isBuffer(a) }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (url.pathname === '/env') {
      const node_env = process.env.NODE_ENV ?? 'not set';
      const version = process.version;
      return new Response(JSON.stringify({ node_env, version }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    return new Response('Not Found', { status: 404 });
  },
};
```

Set `NODE_ENV` via the app config:

```json
{
  "apps": [{
    "hostname": "api.example.com",
    "entrypoint": "./node-compat.js",
    "env_vars": { "NODE_ENV": "production" }
  }]
}
```

```bash
curl http://localhost:8080/path
# {"joined":"/var/app/config.json","dir":"/var/app","base":"config.json","ext":".json"}
curl http://localhost:8080/env
# {"node_env":"production","version":"v18.0.0"}
```

---

## 6. `localStorage` Shim (built on `nano:kv`)

A drop-in browser `localStorage` API backed by `nano:kv` — useful for running
front-end code that reads from `localStorage` unchanged.

Full example: [`examples/localStorage-shim.js`](examples/localStorage-shim.js)

```javascript
import { openKV } from 'nano:kv';

const store = openKV('localStorage');

const localStorage = {
  async getItem(key) {
    const bytes = await store.get(String(key));
    return bytes ? new TextDecoder().decode(bytes) : null;
  },

  async setItem(key, value) {
    await store.set(String(key), new TextEncoder().encode(String(value)));
  },

  async removeItem(key) {
    await store.delete(String(key));
  },

  async clear() {
    const entries = await store.list('');
    await Promise.all(entries.map(([k]) => store.delete(k)));
  },
};

globalThis.localStorage = localStorage;

export { localStorage };
```

```bash
# Import the shim at the top of your app, then use localStorage normally
curl "http://localhost:8080/set?key=theme&value=dark"
curl "http://localhost:8080/get?key=theme"
# {"key":"theme","value":"dark"}
```

---

## See Also

- [API Reference](docs/API.md) - Complete WinterTC API documentation
- [CLI Reference](docs/CLI.md) - All CLI commands
- [Config Reference](docs/CONFIG.md) - Full configuration schema
- [Node.js Compatibility Guide](docs/NODEJS_COMPAT.md) - Migration patterns
- [Compatibility Matrix](docs/COMPATIBILITY.md) - Full API surface coverage
