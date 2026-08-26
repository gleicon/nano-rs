# Control Plane / Data Plane Architecture

## Overview

NANO implements a strict separation between **control plane** and **data plane** per TigerStyle principles. This document describes the architecture, responsibilities, and handoff protocol.

## Principles

- **Batch operations**: Group requests for efficient processing
- **Let CPU sprint through large work units**: Minimize branching in hot path
- **Assert in control plane without hurting data plane performance**

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────┐
│  CONTROL PLANE (batches, validates, coordinates)         │
├─────────────────────────────────────────────────────────┤
│  - Request routing & scheduling                         │
│  - Isolate lifecycle management                         │
│  - Health checks & metrics                              │
│  - Assertion validation                                 │
│  - WorkQueue depth monitoring                           │
│  - Request batching by tenant/isolate                   │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼ batch requests
┌─────────────────────────────────────────────────────────┐
│  DATA PLANE (zero-copy, minimal branching, sprint)       │
├─────────────────────────────────────────────────────────┤
│  - V8 execution                                         │
│  - Request deserialization (zero-copy where possible)   │
│  - Response serialization                               │
│  - Context reset (optimized path)                       │
│  - NO assertions (validated by control plane)           │
│  - NO allocations (pre-allocated by control plane)    │
└─────────────────────────────────────────────────────────┘
```

## Control Plane (`src/control_plane.rs`)

### Responsibilities

All validation, assertions, and coordination happen in the control plane:

- **Request validation**: Script size limits, timeout bounds, tenant existence, request format
- **Assertion framework**: 40+ TigerStyle assertions covering positive space, negative space, preconditions, postconditions, invariants, ranges, and resource limits
- **Request batching**: Groups requests by tenant/isolate for efficient dispatch
- **Metrics collection**: Tracks requests submitted, validated, rejected, batches created/flushed
- **Tenant registry**: Per-tenant limits for multi-tenant isolation

### Key Types

- `ControlPlane`: Main controller with validation and batching
- `ValidatedRequest`: Request that has passed all control plane checks
- `RequestBatch`: Group of requests for the same tenant ready for data plane execution
- `ControlMetrics`: Metrics snapshot for observability

### API

```rust
// Submit a request for validation and batching
let id = control_plane.submit_request(task)?;

// Process ready batches (called periodically or when batch is full)
let batches = control_plane.process_batches();

// Validate without consuming ownership
control_plane.validate_request_ref(&task)?;
```

## Data Plane (`src/data_plane.rs`)

### Responsibilities

The data plane is the optimized hot path with zero assertions:

- **V8 execution**: `execute_with_context_manager` handles V8 scope lifecycle
- **Handler execution**: `execute_handler_code` compiles and runs JS handlers
- **Response extraction**: `extract_js_response` converts V8 values to NanoResponse
- **Lookup tables**: HTTP status lines mapped via array index (no branching)
- **Batch execution**: `DataPlane::execute_batch` amortizes context reset overhead

### Key Types

- `DataPlane`: Executor with `execute_single` and `execute_batch` methods
- `CpuTimeoutGuard`: Timer-based CPU limit enforcement
- `lookup_status_line`: O(1) status line lookup via match table

### Invariants

The data plane assumes the control plane has guaranteed:
- All requests pre-validated
- All sizes within limits
- Isolates pre-allocated

### Zero-Assertion Guarantee

`grep -c "assert_" src/data_plane.rs` returns 0. The data plane uses error returns (`Result`) for any exceptional conditions rather than assertions, keeping the hot path minimal.

## Handoff Protocol

### Control → Data Plane

1. **Validation**: Control plane validates request against all limits and assertions
2. **Batching**: Validated request is added to per-tenant batch queue
3. **Flush**: When batch is full or timeout reached, `process_batches()` drains ready batches
4. **Dispatch**: Each batch is handed to data plane for execution
5. **Execution**: Data plane executes sequentially on the same isolate (single context reset)
6. **Response**: Results returned to caller via oneshot channels

### Performance Characteristics

- **Control plane**: per-request validation (script size, timeout, tenant existence)
- **Data plane**: per-request JS execution on the worker hot path
- **Batching**: amortizes context setup across multiple requests
- **Lookup tables**: O(1) status line resolution vs O(n) branching

## Integration Points

### Worker Pool (`src/worker/pool.rs`)

- Constructors use explicit `panic!` for setup validation (not assertion macros)
- Worker threads call `data_plane::execute_with_context_manager()` for assertion-free execution
- Re-exports `execute_with_context_manager`, `with_worker_runtime`, and `CpuTimeoutGuard` for backward compatibility

### Work Queue (`src/worker/queue.rs`)

- Each `WorkQueue` owns an optional `ControlPlane`
- Created by default with `ControlPlane::new()`
- Router can validate requests through the queue's control plane before dispatch

### HTTP Router (`src/http/router.rs`)

- `dispatch_to_worker_pool` validates `HandlerTask` through `ControlPlane::validate_request_ref()`
- Validation failures return HTTP 400 with error details
- Validated tasks are then dispatched to the `WorkQueue`

## Testing

Run verification:

```bash
# Verify separation
grep -c "assert_" src/data_plane.rs    # Should be 0
grep -c "assert_" src/control_plane.rs  # Should be >20

# Verify compilation
cargo check --lib
```

## Decisions

- **D-37-05-01**: Data plane contains zero assertion macros to keep hot path minimal
- **D-37-05-02**: Control plane uses TigerStyle assertion macros (40+) for comprehensive validation
- **D-37-05-03**: Lookup tables replace conditionals for HTTP status line resolution
- **D-37-05-04**: Batch execution amortizes context reset cost across multiple requests
- **D-37-05-05**: `validate_request_ref` provides non-consuming validation for router integration
