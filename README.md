# NANO Runtime Technical Documentation

Version: v2.7.0  
Last Updated: 2026-08-24

## Executive Summary

NANO is a multi-tenant JavaScript and WASM edge runtime based on V8 isolates. One OS process hosts multiple isolated apps with:

- **Precompiled-bytecode cold start** — a sliver can carry V8 bytecode built at
  pack time, so serving skips JavaScript parse + compile
- **Per-request isolation** — a fresh context per request without a new isolate
- **One-time process boot** on server start

See [Cold Start Guide](docs/COLD_START.md) for detailed performance characteristics.

## Architecture

### Core Components

1. **V8 Platform** - Shared V8 instance, one isolate per worker thread
2. **Worker Pool** - Per-app worker pools with configurable size (default: 4 workers)
3. **VFS (Virtual File System)** - Per-isolate filesystem with memory/disk/S3 backends
4. **HTTP Router** - Virtual host routing by Host header
5. **Sliver System** - Portable app bundles: `meta.json` + `vfs/` + optional
   precompiled `bytecode.v8bc` (see [Slivers](#sliver-snapshot-and-encapsulation-system))

### Request Flow

1. HTTP request arrives with a registered Host header
2. Router matches hostname to app configuration
3. Request dispatched to app's worker pool
4. Worker executes handler in V8 isolate context
5. Response returned through HTTP layer

## Implemented APIs

### [WinterTC](https://wintertc.org/) Common APIs

Core WinterTC-compatible APIs are fully implemented and tested.

See [API Reference](docs/API.md) for detailed documentation with examples.

| API | Status | Notes |
|-----|--------|-------|
| fetch() | Implemented | Full HTTP client with request/response handling |
| Request | Implemented | Constructor with method, headers, body support |
| Response | Implemented | Constructor with status, headers, body support |
| Headers | Implemented | Map-like interface for HTTP headers |
| URL | Implemented | Full URL parsing with pathname, search, hash |
| URLSearchParams | Implemented | Query string manipulation |
| TextEncoder | Implemented | UTF-8 encoding to Uint8Array |
| TextDecoder | Implemented | UTF-8 decoding from Uint8Array |
| console | Implemented | log, error, warn methods |

### WebCrypto Implementation

WebCrypto implementation via Rust crypto crates.

See [API Reference](docs/API.md) for detailed crypto documentation.

| API | Status | Algorithms |
|-----|--------|------------|
| crypto.getRandomValues | Implemented | All TypedArray types |
| crypto.subtle.digest | Implemented | SHA-256, SHA-512 |
| crypto.subtle.generateKey | Implemented | AES-GCM, HMAC |
| crypto.subtle.importKey | Implemented | JWK format |
| crypto.subtle.exportKey | Implemented | JWK format |
| crypto.subtle.encrypt | Implemented | AES-GCM |
| crypto.subtle.decrypt | Implemented | AES-GCM |
| crypto.subtle.sign | Implemented | HMAC |
| crypto.subtle.verify | Implemented | HMAC |

### Node.js API Polyfills

Limited Node.js compatibility polyfills for common patterns

See [Compatibility Matrix](docs/COMPATIBILITY.md) for detailed status and [Node.js Migration Guide](docs/NODEJS_COMPAT.md) for migration patterns.

| API | Status | Notes |
|-----|--------|-------|
| Buffer.from() | Implemented | From string, array, hex/base64 |
| Buffer.alloc() | Implemented | Allocate with size and fill value |
| Buffer.toString() | Implemented | Decodes to UTF-8 string |
| TextEncoder | Implemented | Standard encoding |
| TextDecoder | Implemented | Standard decoding |
| setTimeout | Implemented | Basic timer support |
| setInterval | Implemented | Basic timer support |
| clearTimeout | Implemented | Timer cancellation |
| clearInterval | Implemented | Timer cancellation |
| require('fs') | Implemented | Node.js fs polyfill via VFS |
| require('events') | Implemented | EventEmitter (on/once/off/emit/removeAllListeners) |
| Nano.fs.* | Implemented | Direct VFS API |

### HTTP Features

Full HTTP server and client implementation:

| Feature | Status | Notes |
|---------|--------|-------|
| HTTP/1.1 server | Implemented | Configurable host/port |
| Virtual host routing | Implemented | By Host header |
| Multi-tenant isolation | Implemented | Per-app worker pools |
| Worker pool | Implemented | Configurable size and limits |
| Context reset | Implemented | ~5ms between requests |
| Outbound HTTP fetch | Implemented | reqwest client with connection pooling |
| Timeout handling | Implemented | Configurable per-request |
| Redirect handling | Implemented | Configurable max redirects |
| Response body limits | Implemented | 100MB default, configurable |
| WebSocket upgrade | Implemented | Phase 23 — v2.1.x |
| WebSocketPair API | Implemented | Cloudflare Workers compatible; accept/send/close/addEventListener |

### Sliver bundle and encapsulation system

A sliver is a portable tar bundle — `meta.json` + `vfs/` (app files) + an optional
precompiled `bytecode.v8bc`. It carries **no heap snapshot**; an app runs from its
VFS source, using the embedded bytecode to skip parse+compile when the bytecode's
V8 version tag matches the running V8. See [Slivers](docs/SLIVER_WORKFLOW.md).

| Feature | Status | Notes |
|---------|--------|-------|
| Sliver creation (`nano sliver create`) | Implemented | Bundles VFS + compiles bytecode |
| Precompiled bytecode | Implemented | Skips parse+compile; V8-version gated |
| VFS state capture | Implemented | App files included |
| Tar-based format | Implemented | Portable format |
| Cross-instance migration | Implemented | Slivers portable |
| Zero-downtime hot-swap | Implemented | Blue-green pool swap on SIGHUP; same host |
| Sliver listing / inspection / deletion | Implemented | CLI commands |
| Heap-snapshot create/restore primitives | Implemented | Tested; serving integration is [roadmap](docs/ROADMAP.md) |

### Production Multi-Tenancy

| Feature | Status | Test Score | Notes |
|---------|--------|------------|-------|
| CPU Time Tracking | Implemented | 75% | Microsecond precision per request |
| CPU Time Limits | Working | 75% | Prevents infinite loops |
| Timer-based Termination | Implemented | 100% | Linux timer_create + V8 terminate |
| Per-isolate heap cap | Implemented | 100% | V8 heap limit + OOM termination per isolate |
| Isolate recycling | Implemented | 100% | Fresh isolate after N requests |
| Soft eviction (memory pressure) | Implemented | 100% | Per-worker `MemoryMonitor`: recycles the isolate at Critical/Emergency pressure |
| Cross-isolate LRU eviction | Not wired | — | `EvictionManager` implemented + tested, not connected to the serving path — see [ROADMAP](docs/ROADMAP.md) |
| Per-Tenant Metrics | Implemented | 100% | Auto-collected per hostname |
| Prometheus Export | Implemented | 100% | /admin/metrics endpoint |
| Admin Metrics API | Implemented | 100% | JSON endpoints for all metrics |
| WASM Support | Working | 25%* | Load, compile, execute |
| WASM JS API | Implemented | 100% | WebAssembly.* full API |
| Adversarial Security | Protected | 78% | 7/9 attack vectors blocked |
| VFS Security | Verified | 100% | Traversal/path protection working |

\* WASM file loading requires VFS configuration. See [VFS Guide](docs/VFS.md).

## Architectural limitations

The following are intentionally not supported for WinterTC compatibility:

- Node.js http module — Use WinterTC fetch() instead
- Node.js net module — Raw sockets not supported
- process.env global — Use request headers or config
- Node.js path module — Use URL API instead

## Cloudflare Worker Compatibility

Standard Cloudflare Workers run with minimal modifications:

- fetch(), Request, Response, Headers — Fully compatible
- URL, URLSearchParams — Fully compatible
- TextEncoder, TextDecoder — Fully compatible
- ReadableStream, WritableStream — Fully compatible
- WebCrypto (SHA-256, AES-GCM, HMAC) — Fully compatible

Cloudflare-specific APIs (KV, Durable Objects) are not supported.

## Migration from Cloudflare Workers

Existing Cloudflare Workers can run on nano-rs with these changes:

1. Replace env bindings with direct configuration
2. Use standard WinterTC APIs
3. No changes needed for fetch/Response/Request patterns
4. Store state in VFS or external database (no KV)

## Performance Characteristics

- Precompiled bytecode: a sliver's `bytecode.v8bc` lets the first request skip JS
  parse + compile when its V8 version tag matches the running V8
- One isolate per worker thread; a fresh context per request for isolation
- Process boot is a one-time cost on server start
- Max response body size: 100MB (configurable)
- Default timeout: 30 seconds (configurable)

Heap-snapshot serving (restoring an isolate from a baked heap blob) is a
[roadmap](docs/ROADMAP.md) item, not the current cold-start path. See
[Performance Documentation](docs/PERFORMANCE.md) for the tuning guide.

## Architecture

- One OS process hosts many isolated JavaScript apps
- Each app runs in a separate V8 isolate
- Worker pool handles requests with configurable size
- Context reset between requests for isolation
- VFS provides per-isolate filesystem namespaces
- Slivers bundle app files + precompiled bytecode for fast, portable deploys
- Zero-downtime hot-swap: SIGHUP blue-green-swaps a sliver app on the same host
- **CPU time limits prevent runaway scripts (50ms default)**
- **Per-isolate memory cap: V8 heap limit + OOM termination, recycle after N requests, and soft eviction (recycle under memory pressure)**
- **Per-tenant metrics with Prometheus export**
- **WASM module support for compute-heavy workloads**

## Security Model

- Per-isolate VFS namespaces prevent filesystem escape
- Path traversal blocked (".." sequences rejected)
- SSRF prevention blocks private/internal IPs (RFC1918, loopback, link-local, IPv4-mapped IPv6) and internal hostnames
- Dangerous headers filtered (Content-Length, Host, etc.)
- URL scheme restricted to http/https only
- Request timeouts enforced per-isolate
- Memory limits enforced per-isolate

## Documentation

- **[API Reference](docs/API.md)** — JavaScript globals, WebCrypto, WinterTC APIs
- **[WebSocket](docs/WEBSOCKET.md)** — WebSocket upgrade flow, WebSocketPair API, limits
- **[CLI Reference](docs/CLI.md)** — Command-line interface and commands
- **[Configuration](docs/CONFIG.md)** — Configuration schema and options
- **[Admin API](docs/ADMIN_API.md)** — Admin HTTP endpoints for monitoring
- **[Node.js Compatibility](docs/NODEJS_COMPAT.md)** — Migration guide from Node.js
- **[Cold Start Guide](docs/COLD_START.md)** — Performance characteristics
- **[Compatibility Matrix](docs/COMPATIBILITY.md)** — Full API compatibility status
- **[Architecture Decision Records](docs/ADR/)** — Key design decisions
- **[Formal methods](formal/README.md)** — TLA+ and loom model-checks of the core concurrency protocols: request lifecycle, hot-swap, and shutdown drain (`make tla`, `make loom`)

## Building from Source

Requirements:
- Rust 1.70+ 
- LLVM/Clang (for V8 build)
- 8GB RAM minimum for V8 compilation

Build:
```bash
cargo build --release
```

The binary is at `target/release/nano-rs`.

## Running Tests

```bash
# API compatibility tests
cd /path/to/test-suite
NANO_BINARY=/path/to/nano-rs node scripts/fast-compatibility-matrix.js

# Comprehensive test suite
NANO_BINARY=/path/to/nano-rs node scripts/run-tests.js
```

## License

MIT License - See LICENSE file for details.
