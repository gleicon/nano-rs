# Production Multi-Tenancy

## Overview

NANO implements production-grade multi-tenancy features for NANO, providing Cloudflare Workers-level resource management and observability for self-hosted deployments.

### Features Delivered

1. CPU Time Tracking and Limits - Microsecond-precision per-request CPU tracking with configurable limits
2. Memory Monitoring and Eviction - 4-tier pressure levels with soft and LRU eviction
3. Per-Tenant Metrics - Automatic metrics collection with Prometheus export
4. WASM Support - WebAssembly module loading and execution with sliver integration

## CPU Time Tracking and Termination

### Architecture

Request → CPU Timer Start → JS Execution → CPU Check → Response
                              |
                         Timeout? → V8 TerminateExecution

### Configuration

```json
{
  "apps": [{
    "hostname": "api.example.com",
    "entrypoint": "./app.js",
    "limits": {
      "cpu_time_ms": 50,
      "cpu_time_enabled": true
    }
  }]
}
```

### Implementation Details

Platform Support:
- Linux: timer_create(CLOCK_THREAD_CPUTIME_ID) for microsecond precision
- macOS: getrusage(RUSAGE_THREAD) fallback

Safety:
- Signal handler sets atomic flag (never calls V8 directly)
- Main thread checks flag and calls isolate.terminate_execution()
- Prevents SIGSEGV from signal context V8 calls

Default: 50ms CPU time limit (matches Cloudflare Workers)

### Metrics Exposed

- nano_tenant_cpu_seconds_total - Cumulative CPU time per tenant
- nano_tenant_cpu_time_per_request_seconds - Histogram of per-request CPU time

## Memory Monitoring and Eviction

### Pressure Levels

Level       Threshold   Action
Normal      <70%        Continue normal operation
Warning     70-85%      Log warning, may throttle
Critical    85-95%      Soft eviction - drain stateless isolates
Emergency   >95%        Hard eviction - immediate termination

### Eviction Policies

- LRU (default): Least Recently Used isolates evicted first
- LFU: Least Frequently Used
- Random: Random selection for stateless isolates
- LargestFirst: Largest memory footprint first

### Soft Eviction Flow

Memory Pressure Detected (85%)
         |
Set Isolate to "Draining" State
         |
Allow Current Requests to Complete
         |
Reject New Requests
         |
Dispose Isolate, Free Memory

### Configuration

```json
{
  "apps": [{
    "hostname": "api.example.com",
    "limits": {
      "memory_mb": 128
    }
  }]
}
```

Soft limits (80%) and critical limits (95%) are calculated automatically from memory_mb.

## Per-Tenant Metrics

### Automatic Collection

Every request automatically records:

- Request counts (total, success, error, timeout)
- CPU time consumed (microseconds)
- Memory usage (heap + external)
- Request latency (milliseconds)
- Context reset count

### Prometheus Endpoint

curl http://localhost:8889/admin/metrics

Example Output:

    HELP nano_tenant_requests_total Total requests per tenant
    TYPE nano_tenant_requests_total counter
    nano_tenant_requests_total{hostname="api.example.com",status="success"} 1523
    nano_tenant_requests_total{hostname="api.example.com",status="error"} 12

    HELP nano_tenant_cpu_seconds_total Cumulative CPU time per tenant
    TYPE nano_tenant_cpu_seconds_total counter
    nano_tenant_cpu_seconds_total{hostname="api.example.com"} 45.23

    HELP nano_tenant_memory_bytes Current memory usage per tenant
    TYPE nano_tenant_memory_bytes gauge
    nano_tenant_memory_bytes{hostname="api.example.com",type="heap"} 67108864

### JSON API Endpoints

Endpoint                              Description
GET /admin/metrics                    Prometheus format (global + tenant metrics)
GET /admin/metrics/tenants            JSON format with all tenant metrics
GET /admin/metrics/isolates          Per-isolate statistics
GET /admin/metrics/apps/:hostname    Specific app metrics (404 if not found)
GET /admin/metrics/summary            High-level overview

### Metrics Structure

```json
{
  "tenants": [{
    "hostname": "api.example.com",
    "requests": {
      "total": 1523,
      "success": 1489,
      "error": 12,
      "timeout": 22,
      "active": 3
    },
    "cpu": {
      "total_seconds": 45.23,
      "avg_per_request_ms": 29.7
    },
    "memory": {
      "current_bytes": 67108864,
      "external_bytes": 2097152,
      "peak_bytes": 83886080
    },
    "latency": {
      "p50_ms": 15,
      "p95_ms": 45,
      "p99_ms": 89
    }
  }]
}
```

## WASM Support

### Loading WASM Modules

```javascript
// From filesystem (VFS or OS)
const wasmBytes = await Nano.fs.readFile('./module.wasm');
const module = await WebAssembly.compile(wasmBytes);
const instance = await WebAssembly.instantiate(module, imports);

// Call exported functions
const result = instance.exports.add(1, 2);
```

### WebAssembly JavaScript API

Full WebAssembly JS API available:

- WebAssembly.compile(bytes) → Promise<Module>
- WebAssembly.instantiate(moduleOrBytes, imports) → Promise<Instance>
- WebAssembly.validate(bytes) → boolean
- WebAssembly.Module - Module constructor
- WebAssembly.Instance - Instance constructor
- WebAssembly.Memory - Memory constructor
- WebAssembly.CompileError - Compilation error type
- WebAssembly.RuntimeError - Runtime error type

### Sliver Integration

WASM modules are automatically discovered and cached during sliver creation:

    Create sliver with WASM modules
    nano-rs sliver create api.example.com --output api.sliver

    WASM files (.wasm) automatically included
    Compiled modules cached for faster cold starts

Sliver WASM Cache Features:
- Automatic .wasm file discovery
- SHA-256 hash verification
- Compiled module serialization
- Integrity checking on restore

### Resource Limits

WASM execution respects the same limits as JavaScript:

- CPU time limits (default 50ms)
- Memory limits (heap + WASM memory total)
- Request timeouts

## Test Results

### Test Summary

Plan    Feature                           Tests   Status
27-01   CPU Time Tracking                  39     Pass
27-02   Memory Monitoring                   31     Pass
27-03   Per-Tenant Metrics                  13     Pass
27-04   WASM Support                         8     Pass
Total   91                                 Pass

### Full Test Suite

    cargo test --all
    Running 35 test suites
    Test result: ok. 981 passed; 3 ignored

## Migration Guide

### From v1.2 to v1.5.0

1. Update configuration to include new CPU limits (optional - defaults applied):

```json
{
  "apps": [{
    "hostname": "api.example.com",
    "limits": {
      "memory_mb": 128,
      "timeout_secs": 30,
      "workers": 4,
      "cpu_time_ms": 50,         // NEW: optional, default 50
      "cpu_time_enabled": true   // NEW: optional, default true
    }
  }]
}
```

2. Access new metrics at /admin/metrics (Prometheus format)

3. Use WASM modules in your JavaScript apps

4. Monitor memory pressure via logs (warn at 85%, critical at 95%)

## Security Considerations

### Threat Model

Threat                  Mitigation
Infinite loops          CPU time limits with V8 termination
Memory exhaustion       4-tier pressure monitoring + eviction
Resource starvation     Per-tenant limits isolate blast radius
WASM escape             V8 sandbox contains WASM execution

### Signal Handler Safety

- SIGALRM handler sets atomic flag only
- V8 terminate_execution() called from main thread
- Prevents undefined behavior from signal context

### Memory Isolation

- Per-isolate heap statistics
- Soft eviction prevents new requests during drain
- Hard eviction terminates immediately in emergencies

## Performance Impact

### CPU Tracking Overhead

Platform    Overhead per request    Measured impact
Linux       ~1-2µs                  <0.1% on 50ms requests
macOS       ~3-5µs                  <0.1% on 50ms requests

### Memory Monitoring Overhead

Operation                       Overhead    Measured impact
Heap check                      ~5µs        <0.2% on typical workloads
Trend calculation               ~1µs        <0.2% on typical workloads

### Metrics Collection Overhead

Operation           Overhead    Measured impact
Per-request         ~2µs        <0.1% with 1000+ tenants
Prometheus export   ~100µs      <0.1% with 1000+ tenants

## API Reference

### Admin Endpoints

Prometheus Metrics
    GET /admin/metrics
    Content-Type: text/plain; version=0.0.4

Tenant Metrics (JSON)
    GET /admin/metrics/tenants
    Content-Type: application/json

Isolate Statistics
    GET /admin/metrics/isolates
    Content-Type: application/json

Specific App Metrics
    GET /admin/metrics/apps/api.example.com
    Content-Type: application/json
    404 - if hostname not found

Metrics Summary
    GET /admin/metrics/summary
    Content-Type: application/json

## See Also

- docs/config-mode.md - Configuration format and limits
- docs/SLIVER_WORKFLOW.md - Creating and managing slivers

Shipped: 2026-05-01
Total implementation: ~3,640 lines of Rust code
Test coverage: 91 new tests, 981 total tests passing
