# Cloudflare Workers Compatibility Mode

**Date:** 2026-05-17  
**Version:** v2.0a  
**Status:** ✅ IMPLEMENTED

---

## Overview

NANO-RS now supports a **Cloudflare Workers compatible mode** where global state persists between requests on the same isolate. This matches the behavior of Cloudflare Workers regular (stateless) workers.

---

## The Difference

### Default Mode (Security-Focused)

```rust
let pool = WorkerPool::new(hostname, workers, memory_limit_mb);
// or
let queue = WorkQueue::new(workers_per_pool);
```

**Behavior:**
- V8 context is reset before each request
- All global variables are cleared
- Maximum security isolation between requests
- No state leakage possible

**Use case:** Stateless APIs, maximum security requirements

### Cloudflare Workers Compatible Mode

```rust
let pool = WorkerPool::with_backend_and_reset_mode(
    hostname,
    workers,
    memory_limit_mb,
    backend,
    true // skip_context_reset - Cloudflare compatible
);

// or with WorkQueue:
let queue = WorkQueue::new(workers_per_pool)
    .with_cloudflare_compatibility();
```

**Behavior:**
- V8 context is NOT reset between requests
- Global `const` variables persist between requests
- Module-level state survives across requests
- State persists until isolate eviction (memory pressure or inactivity)

**Use case:** Cloudflare Workers compatible code, stateful request handling

---

## Example: Stateful Request Handler

```javascript
// This works in Cloudflare Workers compatible mode
// Global state persists between requests on the same isolate

const storage = new Map();
let nextId = 1;

export default {
    async fetch(request) {
        const url = new URL(request.url);
        
        if (request.method === 'POST' && url.pathname === '/items') {
            const body = await request.json();
            const id = nextId++;
            const item = { id, ...body, created: Date.now() };
            storage.set(id, item);
            
            return new Response(JSON.stringify(item), {
                status: 201,
                headers: { 'Content-Type': 'application/json' }
            });
        }
        
        if (request.method === 'GET' && url.pathname.startsWith('/items/')) {
            const id = parseInt(url.pathname.split('/')[2]);
            const item = storage.get(id);
            
            if (item) {
                return new Response(JSON.stringify(item), {
                    status: 200,
                    headers: { 'Content-Type': 'application/json' }
                });
            }
            return new Response(JSON.stringify({ error: 'Not found' }), { 
                status: 404 
            });
        }
        
        return new Response('Not Found', { status: 404 });
    }
};
```

---

## Security Considerations

### Default Mode (Recommended for Production)

✅ **Pros:**
- Guaranteed no state leakage between requests
- Each request starts with a clean slate
- No risk of one tenant seeing another's data

❌ **Cons:**
- Cannot use in-memory state between requests
- Requires external storage for persistence

### Cloudflare Compatible Mode

✅ **Pros:**
- Matches Cloudflare Workers behavior
- Allows fast in-memory state access
- No external database needed for simple state

❌ **Cons:**
- Global state persists between requests (security risk)
- State only persists until isolate eviction
- Risk of state leakage if not careful

**Security Warning:** When using Cloudflare compatible mode, ensure:
1. No sensitive data is stored in global variables
2. Proper tenant isolation is maintained
3. You understand state is ephemeral (cleared on eviction)

---

## Comparison with Cloudflare

| Feature | Cloudflare Workers | NANO-RS Default | NANO-RS Cloudflare Mode |
|---------|-------------------|-----------------|-------------------------|
| **Global state** | Persists until eviction | Cleared each request | Persists until eviction |
| **Context reset** | No | Yes (security) | No |
| **Isolate reuse** | Yes | Yes | Yes |
| **State guarantee** | Ephemeral | None | Ephemeral |
| **Durable Objects** | ✅ Available | ❌ Not implemented | ❌ Not implemented |

---

## What This Is NOT

This is **NOT** Durable Objects. For true state persistence that survives:
- Process restarts
- Machine failures
- Eviction

You still need:
1. **External database** (PostgreSQL, Redis, etc.)
2. **Durable Object equivalent** (future feature)

See [Durable Objects Analysis](./DURABLE_OBJECTS_ANALYSIS.md) for what's needed.

---

## API Reference

### ContextManager

```rust
// Default: security mode (context reset per request)
let manager = ContextManager::new(isolate);

// Cloudflare compatible: skip context reset
let manager = ContextManager::with_skip_context_reset(isolate, true);

// Check current mode
if manager.is_skip_context_reset() {
    println!("Cloudflare compatible mode enabled");
}

// Change mode dynamically
manager.set_skip_context_reset(true);
```

### WorkerPool

```rust
// Default: security mode
let pool = WorkerPool::new(hostname, workers, memory_limit);

// Cloudflare compatible mode
let pool = WorkerPool::with_backend_and_reset_mode(
    hostname,
    workers,
    memory_limit,
    backend,
    true, // skip_context_reset
);
```

### WorkQueue

```rust
// Default: security mode
let queue = WorkQueue::new(workers_per_pool);

// Cloudflare compatible mode
let queue = WorkQueue::new(workers_per_pool)
    .with_cloudflare_compatibility();
```

---

## Migration Guide

### From Cloudflare Workers

Your existing Cloudflare Workers code should work with minimal changes:

1. Enable Cloudflare compatible mode
2. Ensure global variables are properly initialized
3. Handle state eviction gracefully

### To Durable Objects (Future)

For true durability, you'll need to migrate to the Durable Objects equivalent when available:

```javascript
// Current: Cloudflare compatible (ephemeral state)
const storage = new Map(); // Lost on eviction

// Future: Durable Objects equivalent (persistent)
// await ctx.storage.put('key', value); // Survives eviction
```

---

## Implementation Details

The `skip_context_reset` flag controls whether `ContextManager::reset_context()` actually performs a context reset:

```rust
pub fn reset_context(&mut self) -> Result<Duration> {
    // Cloudflare Workers compatibility
    if self.skip_context_reset {
        return Ok(Duration::ZERO); // Skip reset
    }
    
    // Perform actual context reset
    // ... clears all global state
}
```

When skipped:
- V8 context is reused
- Global scope persists
- Module code not re-executed
- ~0ms overhead

When reset (default):
- New V8 context created
- Global scope cleared
- Module bindings re-applied
- context reset overhead (not benchmarked)

---

## Testing

All CRUD tests now use Cloudflare compatible mode:

```rust
#[tokio::test]
async fn test_crud_create() {
    let mut queue = WorkQueue::new(1)
        .with_cloudflare_compatibility();
    
    // Global storage Map persists between requests
    let js_code = r#"
        const storage = new Map();
        let nextId = 1;
        
        export default {
            async fetch(request) {
                // storage and nextId persist across requests
                // ... CRUD operations
            }
        };
    "#;
    
    // Test stateful operations...
}
```

---

## WebSocket Compatibility

NANO-RS WebSocket follows the Cloudflare Workers WebSocket API (`WebSocketPair`, `server.accept()`, `addEventListener`).

See [WebSocket Guide](WEBSOCKET.md) for:
- Upgrade flow and architecture
- `WebSocketPair` API reference
- Per-connection limits (`ws_max_connections`, `ws_max_message_bytes`, `ws_idle_timeout_ms`)

**Status:** Implemented — server-side `WebSocketPair` (Phase 23). The outbound
`new WebSocket(url)` client remains a WinterCG compatibility stub.

---

## See Also

- [WebSocket Guide](WEBSOCKET.md) — WebSocket upgrade, API, limits
- [API Reference](API.md) — Full JavaScript API docs

---

**Last Updated:** 2026-05-17  
**Status:** ✅ Production Ready
