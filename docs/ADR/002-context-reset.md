# ADR-002: Context Reset for Request Isolation

**Status:** Accepted  
**Date:** 2026-04-19  
**Deciders:** Core Team  
**Technical Story:** Need per-request isolation without 50-100ms isolate creation overhead

> **Note (2026-08-24):** the "~267µs sliver restore" figures in the comparison below
> refer to a heap-snapshot restore path that was never shipped (see
> [ROADMAP](../ROADMAP.md)). The context-reset decision itself stands; treat those
> specific timings as historical/aspirational, not measured.

---

## Context and Problem Statement

In a multi-tenant edge runtime, each HTTP request must be isolated from previous requests to prevent:
1. **Data leakage** — Global variables from request A visible to request B
2. **State pollution** — Modified prototypes affecting subsequent requests
3. **Security breaches** — Cross-tenant data access

The naive approach is creating a new V8 isolate per request. However, this takes 50-100ms — unacceptable for edge latency requirements (target: <10ms request handling).

We need a mechanism that:
1. Resets JavaScript global state between requests
2. Maintains performance for high-throughput scenarios
3. Ensures security isolation
4. Scales to thousands of requests per second

---

## Decision Drivers

* **Latency** — Target <10ms request handling overhead
* **Isolation** — No data leakage between requests
* **Throughput** — Support 1000+ RPS per core
* **Security** — Strong isolation guarantees
* **Simplicity** — Avoid complex pooling logic

---

## Considered Options

### Option 1: Context Reset

Reset V8 context between requests (~5ms), keep isolate alive.

### Option 2: Fresh Isolate

Create new isolate per request (~50-100ms).

### Option 3: Isolate Pool

Pre-warm isolates, checkout/checkin pattern.

### Option 4: Realm per Request

Use V8 Realms (experimental in rusty_v8).

---

## Decision Outcome

**Chosen option: "Context Reset"**

Context reset (~5ms) clears the JavaScript global object and reinitializes built-in APIs while keeping the V8 isolate (the expensive part). This provides:
- Clean global state per request
- All built-in APIs re-bound
- 10x faster than isolate creation (5ms vs 50-100ms)
- Adequate isolation for edge use case

**Rationale:**
- 10x latency improvement over fresh isolates
- Simpler than pooling (no checkout/checkin logic)
- Deterministic behavior (no pool exhaustion)
- Sufficient isolation (context is the JavaScript global scope)

---

## Implementation Details

### Code Location

`src/worker/context.rs`

### Pattern

```rust
// Per-request context management
impl IsolateWorker {
    fn handle_request(&mut self, request: HttpRequest) -> HttpResponse {
        // 1. Reset context (fast)
        self.context.reset();  // ~5ms
        
        // 2. Re-bind APIs
        self.bind_console();
        self.bind_fetch();
        self.bind_timers();
        
        // 3. Execute handler
        self.execute_handler(request)
    }
}
```

### What Context Reset Does

1. **Clear global object** — New empty global
2. **Remove user-defined properties** — Clean slate
3. **Preserve isolate heap** — No deallocation
4. **Re-bind built-ins** — console, fetch, etc.

### What It Doesn't Do

- **Does NOT clear V8 heap** — Same heap, new context
- **Does NOT fix all security issues** — V8 bugs still possible
- **Does NOT isolate memory completely** — Same isolate memory limits

---

## Positive Consequences

* **10x lower latency** — 5ms vs 50-100ms per request
* **Simpler implementation** — No pooling checkout/checkin
* **Deterministic behavior** — No pool exhaustion issues
* **Adequate isolation** — Global scope reset sufficient for edge use
* **Memory efficiency** — Reuse heap allocation

---

## Negative Consequences

* **Not perfect isolation** — Same isolate, potential V8 bugs
* **Context reset still takes ~5ms** — Overhead for every request
* **Memory usage grows** — With isolate lifetime (mitigated by limits)
* **Prototype pollution possible** — If built-ins are modified (we re-bind)
* **Security model weaker** — Than fresh isolates (trade-off accepted)

---

## Performance Comparison

| Approach | Time | Use Case |
|----------|------|----------|
| Context Reset | ~5ms | Between requests (our approach) |
| Fresh Isolate | ~50-100ms | New app, sliver unavailable |
| Sliver Restore | ~267µs | New worker, snapshot available |

**Key insight:** Combine context reset + sliver restoration for best of both:
- Sliver restore: 267µs (new isolate with state)
- Context reset: 5ms (between requests, same isolate)

---

## Alternatives Rejected

### Option 2: Fresh Isolate — Rejected

**Why:** 10-20x slower (50-100ms vs 5ms). Unacceptable for edge latency. Would limit runtime to <20 RPS per isolate.

### Option 3: Isolate Pool — Rejected

**Why:** Adds complexity (pool sizing, checkout/checkin, deadlock risk). Doesn't solve the per-request isolation problem (just amortizes cost). Still need context reset or fresh isolate per request.

### Option 4: Realm per Request — Rejected

**Why:** V8 Realms experimental in rusty_v8. API unstable, may have same bugs. Context reset is proven, well-understood mechanism.

---

## Security Considerations

### Threat: Cross-Request Data Leakage

**Mitigation:** Context reset clears all global variables. Each request starts with fresh global object.

### Threat: Prototype Pollution

**Mitigation:** We re-bind all built-in APIs (console, fetch, etc.) after reset. Any prototype modifications are discarded.

### Threat: V8 Bugs

**Risk accepted:** If V8 has a bug allowing escape from context, we're vulnerable. This is a trade-off for performance. Threat model: edge runtime (untrusted code), not full sandbox.

### When to Use Fresh Isolates Instead

- **High-security scenarios** — Banking, healthcare (use fresh per request)
- **Untrusted code** — User-uploaded scripts (use fresh)
- **Debugging** — When isolation issues suspected (use fresh for comparison)

---

## Related Decisions

* [ADR-001: EPT Fix](001-ept-fix.md) — Isolate lifecycle safety
* [ADR-003: Thread-Local Isolates](003-thread-local-isolates.md) — When we create isolates
* [ADR-006: Sliver Format](006-sliver-format.md) — Faster alternative via snapshots
* [Cold Start Guide](../COLD_START.md) — Detailed performance characteristics

---

## Code References

- `src/worker/context.rs` — Context reset implementation
- `src/worker/pool.rs` — Worker pool that uses context reset
- `src/v8/context.rs` — V8 context management

---

## Monitoring

Track these metrics in production:

| Metric | Target | Alert If |
|--------|--------|----------|
| Context reset time | < 10 ms | > 15 ms |
| Context reset rate | Stable | Sudden increase |

Sudden increase in context reset time may indicate:
- Global object pollution (memory leak)
- V8 heap pressure (GC during reset)
- Need for worker pool expansion

---

*Last updated: 2026-04-19*
