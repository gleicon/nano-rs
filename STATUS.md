# NANO Runtime v2.1.3-alpha Technical Summary

Date: 2026-07-31
Status: Stable — bug-fix release series

## Release Overview

v2.1.x is the active bug-fix and hardening series following v2.0a (WebSocket). The series includes
a V8 engine bump, WASM fixes, CPU timeout correctness, VFS disk backend fixes, security hardening,
and worker architecture cleanup.

See [CHANGELOG.md](CHANGELOG.md) for full history.

### v2.1.3-alpha (current)

- Fixed test infrastructure: `wait_ready()` no longer hangs indefinitely when server startup fails

### v2.1.2-alpha

- rusty_v8 bumped to **150.4.0** (stricter WASM validator)
- **WASM binary fix** — export section length byte corrected in `add.wasm`
- **WASM CPU timeout** — async WASM infinite loops now correctly terminated via `terminate_execution()` + Pending-loop check
- **CPU timeout enforcement** — `cancel_terminate_execution()` removed from poll loop; now enforced correctly for all async handlers
- **VFS disk backend** — `prefix_namespace()` no longer double-prefixes with hostname for DiskBackend
- **Heap limits** — `CreateParams::heap_limits` enforced at isolate creation

### v2.1.1-alpha

- Security: SSRF prevention, constant-time auth, JSON escape, bind address validation
- Worker loop merge, dead code removal
- Sliver v2 format, WebSocket send state check

### v2.1.0-alpha

- eval/new-Function ban in standalone execution path
- Unified worker loop

---

## Key Highlights

- **WASM CPU timeout works** — async handlers running WASM infinite loops are correctly killed at the configured wall-clock limit
- **CPU limit enforcement is now correct** — async handlers that exceed `cpu_time_ms` are killed; this includes those waiting on timers longer than the limit
- **696+ tests passing** (library + adversarial security)
- **Multi-tenant production ready** — CPU limits, memory eviction, per-tenant metrics, WASM support

## API Status

### WinterTC Minimum Common APIs

| API | Status |
|-----|--------|
| fetch() | ✅ Full HTTP client |
| Request / Response / Headers | ✅ |
| URL / URLSearchParams | ✅ |
| TextEncoder / TextDecoder | ✅ |
| console | ✅ |
| Streams | ✅ ReadableStream, WritableStream |
| WebSocket | ✅ Server-side (Phase 23) |
| WASM | ✅ V8 built-in engine |

### WebCrypto

| API | Status |
|-----|--------|
| crypto.getRandomValues | ✅ |
| crypto.subtle.digest | ✅ SHA-256/384/512 |
| crypto.subtle.generateKey | ✅ AES-GCM, HMAC |
| crypto.subtle.importKey / exportKey | ✅ JWK |
| crypto.subtle.encrypt / decrypt | ✅ AES-GCM |
| crypto.subtle.sign / verify | ✅ HMAC |
| RSA-OAEP, RSASSA-PKCS1-v1_5, ECDSA | ✅ P-256, P-384 |

### VFS / Node.js Polyfills

| API | Status |
|-----|--------|
| Nano.fs.* | ✅ Memory, Disk, S3 backends |
| require('fs') | ✅ Polyfill via Nano.fs |
| Buffer | ✅ |
| setTimeout / setInterval / clearTimeout / clearInterval | ✅ |

## Security Model

- Per-isolate VFS namespaces — filesystem escape impossible
- Path traversal blocked — `..` sequences rejected in all VFS operations
- SSRF prevention — private IP ranges blocked in outbound `fetch()`
- eval/new-Function banned — `set_allow_generation_from_strings(false)`
- CPU time limits — wall-clock enforcement via `terminate_execution()`
- Memory limits — `heap_limits` at isolate creation + near-heap callback
- Request timeouts — 30s default, configurable per-app
- Dangerous headers filtered

## Performance

- Cold start from sliver: ~267 µs
- Context reset: ~5 ms
- Fresh isolate creation: 50-100 ms
- WASM CPU timeout response: ~100-110 ms (matches configured limit)

## Documentation

- [CHANGELOG.md](CHANGELOG.md) — Full version history
- [API Reference](docs/API.md) — JavaScript APIs with examples
- [WebSocket Guide](docs/WEBSOCKET.md) — WebSocket architecture
- [CLI Documentation](docs/CLI.md) — Command line interface
- [Configuration](docs/CONFIG.md) — App configuration and limits
- [Admin API](docs/ADMIN_API.md) — Monitoring and management
- [VFS Guide](VFS.md) — Virtual File System
- [Sliver Guide](SLIVER.md) — Portable isolate snapshots
- [Security Gateway](docs/SECURITY_GATEWAY.md) — Adversarial testing
