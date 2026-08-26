# Debugging and Profiling NANO Isolates

This guide covers how to trace, debug, and profile V8 isolates in NANO for performance analysis and troubleshooting.

---

## Understanding Isolate Lifecycle

### What is an Isolate?

A **V8 Isolate** is a JavaScript execution sandbox that contains:
- Heap memory (JavaScript objects, compiled code)
- Contexts (global scope)
- Runtime state

### Isolate vs Worker

| Concept | Identifier | Lifetime | Changes When |
|---------|-----------|----------|--------------|
| **Worker** | `worker_id: 0, 1, 2...` | Process lifetime | Never (OS thread) |
| **Isolate** | `isolate_id: iso_abc123_00000001` | Thousands of requests | OOM, eviction, manual restart |

### Key Principle

**Workers are long-lived, Isolates are replaced on memory pressure.**

```
Worker 0 (thread) ───────────────────────────────────────►
   │
   ├─ Isolate iso_abc123_00000001 ───────────┐ OOM! Dispose
   │   Request 1 ✓
   │   Request 2 ✓
   │   Request 3 ✗ (OOM triggered)
   │
   ├─ Isolate iso_def456_00000002 ───────────┐ OOM! Dispose
   │   Request 4 ✓
   │   Request 5 ✓
   │   Request 6 ✗ (OOM triggered)
   │
   └─ Isolate iso_ghi789_00000003 ──────────► Current
       Request 7 ✓
       Request 8 ✓
       ...
```

---

## Request Tracing: The Three-Part Combo

Every request in NANO carries three identifiers for complete tracing:

| ID | Format | Purpose | Changes? |
|----|--------|---------|----------|
| `request_id` | `req_abc12345` | Track single HTTP request end-to-end | No (request-scoped) |
| `worker_id` | `0, 1, 2...` | Which OS thread handled it | No (thread-scoped) |
| `isolate_id` | `iso_abc12345_00000001` | Exact V8 isolate instance | **YES** on OOM/eviction |

### Example Log Flow

```json
// HTTP Access Log (from router)
{
  "ts": "2026-05-03T12:34:56.789Z",
  "level": "INFO",
  "message": "HTTP GET / - 200 in 5.23ms (worker: 0, isolate: app.local:0)",
  "request_id": "req_a3f7b2d8",
  "worker_id": 0,
  "isolate_id": "app.local:0",
  "hostname": "localhost",
  "fields": {
    "method": "GET",
    "path": "/",
    "status": 200,
    "duration_ms": "5.23",
    "worker_id": 0,
    "isolate_id": "app.local:0"
  }
}

// Worker Processing Log (from worker thread)
{
  "ts": "2026-05-03T12:34:56.785Z",
  "level": "INFO",
  "message": "Worker 0 processed request req_a3f7b2d8: GET / - 200 in 5ms (isolate: app.local:0)",
  "request_id": "req_a3f7b2d8",
  "worker_id": 0,
  "isolate_id": "app.local:0",
  "hostname": "localhost",
  "fields": {
    "duration_ms": 5,
    "status": 200
  }
}
```

---

## Debugging Isolate Lifecycle

### Enable Debug Logging

```bash
# Show isolate age on every request
RUST_LOG=debug nano-rs run -c config.toml 2>&1 | grep "received request"

# Example output:
# DEBUG Worker 0 received request req_a3f7b2d8 (isolate: app.local:0, age: 45s)
# DEBUG Worker 0 received request req_b8e4c9f1 (isolate: app.local:0, age: 47s)
# DEBUG Worker 1 received request req_c5d6e7f8 (isolate: app.local:0, age: 3s) <- OOM recovery
```

### Show Isolate Creation/Destruction

```bash
# See all isolate lifecycle events
RUST_LOG=info nano-rs run -c config.toml 2>&1 | grep -E "(initialized|fresh isolate|shutting down)"

# Example output:
# INFO Worker 0 initialized with context and memory monitoring (isolate_id: app.local:0, initial_age: 0s)
# INFO Worker 0 created fresh isolate after OOM (isolate_id: app.local:0)
# INFO Worker 0 shutting down (served: N, isolate_id: telemetry-app.local:0)
```

### Track Specific Isolate

```bash
# Follow all requests handled by a specific isolate
nano-rs run -c config.toml 2>&1 | jq 'select(.isolate_id == "app.local:0")'

# Follow all requests on a specific worker
nano-rs run -c config.toml 2>&1 | jq 'select(.worker_id == 0)'

# Follow a single request across the system
nano-rs run -c config.toml 2>&1 | jq 'select(.request_id == "req_a3f7b2d8")'
```

---

## Profiling Isolate Performance

### Context Reset Timing

A fresh context is created for each request in the same isolate:

```bash
# Show slow context resets (>10ms target)
RUST_LOG=warn nano-rs run -c config.toml 2>&1 | grep "context reset took"

# Example output:
# WARN Worker 0 context reset took 15.3ms (target <10ms)
```

### Memory Monitoring

```bash
# Show memory pressure events
RUST_LOG=info nano-rs run -c config.toml 2>&1 | grep -E "(memory pressure|OOM|evicting)"

# Example output:
# WARN Worker 0 memory pressure detected (85%), initiating soft eviction
# ERROR Worker 0 OOM detected after request execution (oom_count: 1)
# WARN Worker 0 disposing isolate due to OOM
# INFO Worker 0 created fresh isolate after OOM (isolate_id: app.local:0)
```

### End-to-End Request Timing

```bash
# Show slow requests (>100ms)
nano-rs run -c config.toml 2>&1 | jq 'select(.fields.duration_ms | tonumber > 100)'

# Show worker/isolate breakdown for slow requests
nano-rs run -c config.toml 2>&1 | jq -c 'select(.fields.duration_ms | tonumber > 100) | {request_id, worker_id, isolate_id, duration_ms: .fields.duration_ms}'
```

---

## When Does isolate_id Change?

| Event | isolate_id Changes? | Log Indicator |
|-------|---------------------|---------------|
| Normal request completion | **NO** | `HTTP GET / - 200 in 5ms (worker: 0, isolate: iso_abc123)` |
| 30/60 seconds pass | **NO** | Age increases: `(isolate: iso_abc123, age: 45s)` |
| Long-running request | **NO** | CPU timeout kills request, not isolate |
| **OOM detected** | **YES** | `created fresh isolate after OOM (new isolate_id: iso_def456)` |
| **Memory pressure eviction** | **YES** | `evicting isolate, creating fresh (new isolate_id: iso_ghi789)` |
| **Context reset failure** | **YES** | `context reset failed, creating fresh isolate` |

### OOM Recovery Pattern

```
Request N:     isolate_id=iso_abc123 (age: 30s) → OK
Request N+1:   isolate_id=iso_abc123 (age: 32s) → OK
Request N+2:   ALLOCATE TOO MUCH MEMORY → OOM TRIGGERED
               ↓
               Dispose isolate iso_abc123
               Create new isolate → iso_def456
               ↓
Request N+3:   isolate_id=iso_def456 (age: 0s) → OK
```

---

## Common Debugging Scenarios

### Scenario 1: Requests Getting Slower Over Time

```bash
# Check if isolate age correlates with latency
nano-rs run -c config.toml 2>&1 | jq -c 'select(.message | contains("HTTP")) | {ts, isolate_id, duration_ms: .fields.duration_ms, age_ms: (.ts | fromdateiso8601 | now - .)}'

# If older isolates are slower → Possible memory fragmentation
# Solution: Lower memory limits to trigger earlier eviction
```

### Scenario 2: Memory Usage Growing Continuously

```bash
# Monitor memory pressure levels
RUST_LOG=debug nano-rs run -c config.toml 2>&1 | grep "pressure_level"

# Check eviction actions
RUST_LOG=info nano-rs run -c config.toml 2>&1 | grep -E "(SoftEvict|HardEvict)"

# If memory grows without eviction → Memory limits may be too high
# Solution: Reduce `memory_mb` in config or enable stricter OOM threshold
```

### Scenario 3: Isolate Churn (Too Many OOMs)

```bash
# Count OOM events per worker
nano-rs run -c config.toml 2>&1 | jq -c 'select(.message | contains("OOM")) | {worker_id, message}' | sort | uniq -c

# If high OOM count → Isolate memory limit too low or memory leak in JS
# Solution: Increase `memory_mb` or fix memory leak in JavaScript code
```

### Scenario 4: Tracing a Request Through the System

```bash
# Capture all logs for a specific request_id
REQUEST_ID="req_a3f7b2d8"
nano-rs run -c config.toml 2>&1 | jq -c "select(.request_id == \"$REQUEST_ID\")" | jq -s 'sort_by(.ts) | .[]'

# Example output (chronological):
# {"ts":"...","event":"router.rs:673","message":"Request received for host: localhost","request_id":"req_a3f7b2d8",...}
# {"ts":"...","event":"queue.rs:264","message":"Worker 0 executing task for ./app.js","request_id":"req_a3f7b2d8",...}
# {"ts":"...","event":"pool.rs:783","message":"Worker 0 received request req_a3f7b2d8 (isolate: app.local:0, age: 45s)",...}
# {"ts":"...","event":"pool.rs:817","message":"Worker 0 processed request req_a3f7b2d8: GET / - 200 in 5ms (isolate: app.local:0)",...}
# {"ts":"...","event":"router.rs:976","message":"HTTP GET / - 200 in 5.23ms (worker: 0, isolate: app.local:0)",...}
```

---

## Configuration for Debugging

### Enable Detailed Logging

```toml
[server]
port = 8080

[logging]
# Enable all worker/isolate debug logs
level = "debug"

[limits]
# Lower memory to trigger OOM sooner (for testing)
memory_mb = 32

# Aggressive OOM detection (10% threshold)
oom_threshold = 0.1

# Fewer workers to simplify tracing
workers = 2
```

### JSON Log Analysis

```bash
# Pretty-print with jq
nano-rs run -c config.toml 2>&1 | jq .

# Extract key fields only
nano-rs run -c config.toml 2>&1 | jq -c '{ts: .ts[11:19], level, worker_id, isolate_id, message}'

# Count requests per isolate
nano-rs run -c config.toml 2>&1 | jq -r 'select(.isolate_id != null) | .isolate_id' | sort | uniq -c | sort -rn

# Find isolates with most OOMs
nano-rs run -c config.toml 2>&1 | jq -r 'select(.message | contains("OOM")) | .isolate_id' | sort | uniq -c | sort -rn
```

---

## Testing Isolate Behavior

### Integration Tests

The `tests/isolate_id_oom_test.rs` file contains tests that verify:

1. **Isolate ID changes after OOM**: Confirms new isolate gets new ID
2. **Isolate age tracking**: Verifies age increases over time
3. **Request tracing combo**: Tests request_id + worker_id + isolate_id

Run the tests:

```bash
cargo test --test isolate_id_oom_test -- --nocapture
```

### Manual Testing

Create a memory-heavy handler to trigger OOM:

```javascript
// app.js
export default {
  async fetch(request) {
    const url = new URL(request.url);
    
    if (url.pathname === '/oom') {
      // Allocate lots of memory to trigger OOM
      const arrays = [];
      for (let i = 0; i < 1000; i++) {
        arrays.push(new Array(100000).fill(i)); // ~800MB total
      }
      return new Response('Should not reach here');
    }
    
    return new Response('Hello');
  }
};
```

Then watch the logs:

```bash
# Terminal 1
RUST_LOG=info nano-rs run -c config.toml

# Terminal 2
curl http://localhost:8080/        # Normal request
curl http://localhost:8080/oom    # OOM trigger
curl http://localhost:8080/        # Request after OOM (new isolate_id)
```

---

## Summary

| Goal | Command |
|------|---------|
| See isolate age | `RUST_LOG=debug nano-rs run -c config.toml \| grep "received request"` |
| Track OOM events | `RUST_LOG=info nano-rs run -c config.toml \| grep "OOM"` |
| Follow one request | `nano-rs run -c config.toml \| jq 'select(.request_id == "req_abc123")'` |
| Find slow requests | `nano-rs run -c config.toml \| jq 'select(.fields.duration_ms \| tonumber > 100)'` |
| Count isolates | `nano-rs run -c config.toml \| jq -r '.isolate_id' \| sort \| uniq -c` |

Remember: **Worker = thread (never changes), Isolate = V8 sandbox (changes on OOM)**. Use the three-part combo (`request_id` + `worker_id` + `isolate_id`) to trace any request through its complete lifecycle.
