# WASM Integration Example for NANO

This example demonstrates WebAssembly support in the nano-rs runtime.

## Files

- `add.wasm` - Pre-built WASM module with `add(a: i32, b: i32) -> i32` function
- `handler.js` - JavaScript that loads and executes the WASM module
- `config.json` - NANO configuration with CPU time limits
- `Cargo.toml` - Rust source configuration (for building from source)
- `src/lib.rs` - Rust source code (optional: for rebuilding WASM)

## Quick Start

1. Copy the example to a test directory:
```bash
cp -r examples/wasm-test /tmp/wasm-demo
cd /tmp/wasm-demo
```

2. Run NANO (from nano-rs repo):
```bash
cd /path/to/nano-rs
cargo run --release -- run --config /tmp/wasm-demo/config.json
```

3. Test in another terminal:
```bash
# Basic add operation
curl "http://localhost:8080/?a=10&b=20"
# Expected: {"operation":"add","inputs":{"a":10,"b":20},"result":30,...}

# Different values
curl "http://localhost:8080/?a=5&b=6"
# Expected: {"operation":"add","inputs":{"a":5,"b":6},"result":11,...}
```

## Building WASM from Source

If you want to modify the WASM module:

1. Install Rust wasm target:
```bash
rustup target add wasm32-unknown-unknown
```

2. Build:
```bash
cd examples/wasm-test
cargo build --target wasm32-unknown-unknown --release
```

3. Copy the built WASM:
```bash
cp target/wasm32-unknown-unknown/release/nano_wasm_example.wasm ./add.wasm
```

## Testing CPU Time Limits

To test CPU timeout with an infinite loop:

1. Create `infinite.js`:
```javascript
export default {
    async fetch(request) {
        while (true) {
            Math.random();
        }
    }
}
```

2. Create `infinite.json`:
```json
{
  "apps": [{
    "hostname": "cpu-test.local",
    "entrypoint": "./infinite.js",
    "limits": {
      "cpu_time_ms": 10,
      "cpu_time_enabled": true,
      "memory_mb": 128,
      "timeout_secs": 30
    }
  }],
  "server": { "port": 8081 }
}
```

3. Run and test:
```bash
./target/release/nano-rs run --config infinite.json &
time curl -H "Host: cpu-test.local" http://localhost:8081/
# Should timeout within ~50-100ms (real time)
```

## Verifying WebAssembly API

The handler uses these WebAssembly APIs:

- `WebAssembly.validate(bytes)` - Returns true for valid WASM
- `WebAssembly.compile(bytes)` - Returns Promise<Module>
- `WebAssembly.instantiate(module, imports)` - Returns Promise<Instance>
- `instance.exports.add(a, b)` - Calls the exported function

## Metrics

Check metrics at the admin endpoint:
```bash
curl http://localhost:8889/admin/metrics
```

Look for:
- `nano_tenant_cpu_seconds_total` - CPU time used
- `nano_tenant_requests_total` - Request counts

## Troubleshooting

### "Cannot find module"
Ensure all files are in the same directory when running NANO.

### "Invalid WASM module"
Check that add.wasm wasn't corrupted. First bytes should be: 00 61 73 6d (hex)

### "WebAssembly is not defined"
Ensure the nano-rs binary is built with WASM support.

### Request hangs
Check CPU time limit - infinite loops should be terminated. Check logs for timeout messages.

## Architecture

```
HTTP Request
    |
    v
Handler.js
    |
    +-- WebAssembly.validate(wasmBytes)
    +-- WebAssembly.compile(wasmBytes)
    +-- WebAssembly.instantiate(module, {})
    +-- instance.exports.add(a, b)
    |
    v
JSON Response
```

The WASM runs in the same V8 isolate as the JavaScript, sharing the event loop and memory constraints.
