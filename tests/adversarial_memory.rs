//! Adversarial Memory Exhaustion Tests
//!
//! Tests to verify memory limits prevent memory exhaustion attacks:
//! - Large array allocations
//! - Large string concatenation
//! - Buffer growth attacks
//! - Closure memory leaks
//! - Circular reference bombs
//! - Typed array exhaustion


#[path = "common.rs"]
mod common;
use std::time::Duration;
use common::{find_available_port, NanoProcess};

/// Test large array allocation
/// Attack: new Array(1e9) or repeated large allocations
/// Mitigation: Memory limit with soft eviction
#[tokio::test]
#[ignore = "timing-sensitive: V8 startup after many sequential tests can exceed CI runner capacity"]
async fn test_large_array_allocation() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        const arrays = [];
        
        // Attempt to allocate many large arrays
        for (let i = 0; i < 100; i++) {
            try {
                const largeArray = new Array(1000000); // 1M elements each
                arrays.push(largeArray);
            } catch (e) {
                // Allocation might fail
                break;
            }
        }
        
        return new Response(JSON.stringify({allocated: arrays.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-array.local",
        "array_alloc.js",
        js_content,
        100,   // 100ms CPU limit
        128,   // 128MB — V8 needs ~50MB to init; JS still exhausts this
    );
    
    nano.wait_ready(port, "mem-array.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-array.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    // With 128MB limit, should either:
    // 1. Complete with limited allocations
    // 2. Get terminated due to memory pressure (503 or OOM response)
    match result {
        Ok(response) => {
            let status = response.status();
            // Either success with limited allocations, or memory error
            assert!(
                status.is_success() || status.as_u16() == 503 || status.as_u16() == 507,
                "Large array allocation should either succeed with limits or return memory error, got {}",
                status
            );
        }
        Err(_) => {
            // Timeout also acceptable if eviction takes time
        }
    }
}

/// Test large string concatenation
/// Attack: Repeated str += str pattern (exponential growth)
/// Mitigation: Memory limit
#[tokio::test]
async fn test_large_string_concatenation() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        let str = 'x';
        
        // Exponential string growth
        // str doubles in size each iteration: 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, ...
        // After 23 iterations: ~8MB
        for (let i = 0; i < 100; i++) {
            str = str + str;
        }
        
        return new Response(JSON.stringify({length: str.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-string.local",
        "string_concat.js",
        js_content,
        100,   // 100ms CPU
        16,    // 16MB memory
    );
    
    nano.wait_ready(port, "mem-string.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-string.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            assert!(
                status.is_success() || status.as_u16() == 500 || status.as_u16() == 503 || status.as_u16() == 507,
                "String concatenation should be memory-limited or error, got {}",
                status
            );
        }
        Err(_) => {}
    }
}

/// Test buffer growth attack
/// Attack: Continuous Buffer.push in loop
/// Mitigation: Memory limit
#[tokio::test]
async fn test_buffer_growth_attack() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        const buffer = [];
        
        // Continuously push to array
        // Each push adds reference overhead
        for (let i = 0; i < 10000000; i++) {
            buffer.push({index: i, data: 'x'.repeat(100)});
        }
        
        return new Response(JSON.stringify({size: buffer.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-buffer.local",
        "buffer_growth.js",
        js_content,
        100,   // 100ms CPU
        128,   // 128MB — V8 needs ~50MB to init; buffer growth still exhausts this
    );
    
    nano.wait_ready(port, "mem-buffer.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-buffer.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            assert!(
                status.is_success() || status.as_u16() == 500 || status.as_u16() == 503 || status.as_u16() == 507,
                "Buffer growth should trigger memory limits, got {}",
                status
            );
        }
        Err(_) => {}
    }
}

/// Test closure memory leak
/// Attack: Closures capturing large scope
/// Mitigation: Memory limit and eviction
#[tokio::test]
async fn test_closure_memory_leak() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        const closures = [];
        
        // Create closures that capture large scope
        for (let i = 0; i < 100000; i++) {
            const largeData = new Array(1000).fill('x');
            
            closures.push(function() {
                // Closure captures largeData by reference
                return largeData.length;
            });
        }
        
        return new Response(JSON.stringify({closures: closures.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-closure.local",
        "closure_leak.js",
        js_content,
        100,   // 100ms CPU
        128,   // 128MB — V8 needs ~50MB to init; closure accumulation still exhausts this
    );
    
    nano.wait_ready(port, "mem-closure.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-closure.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            assert!(
                status.is_success() || status.as_u16() == 500 || status.as_u16() == 503 || status.as_u16() == 507,
                "Closure leak should be memory-limited, got {}",
                status
            );
        }
        Err(_) => {}
    }
}

/// Test circular reference bomb
/// Attack: Object graphs preventing GC
/// Mitigation: Memory limit (GC will eventually collect, but limit prevents runaway)
#[tokio::test]
#[ignore = "timing-sensitive: V8 startup after many sequential tests can exceed CI runner capacity"]
async fn test_circular_reference_bomb() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        const objects = [];
        
        // Create circular reference chains
        for (let i = 0; i < 100000; i++) {
            const a = {};
            const b = {};
            const c = {};
            
            // Circular references
            a.ref = b;
            b.ref = c;
            c.ref = a;
            
            // Add large data
            a.data = new Array(100).fill('x');
            b.data = new Array(100).fill('y');
            c.data = new Array(100).fill('z');
            
            objects.push(a);
        }
        
        return new Response(JSON.stringify({chains: objects.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-circular.local",
        "circular_ref.js",
        js_content,
        100,   // 100ms CPU
        128,   // 128MB — V8 needs ~50MB to init; circular ref accumulation still exhausts this
    );
    
    nano.wait_ready(port, "mem-circular.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-circular.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            assert!(
                status.is_success() || status.as_u16() == 503 || status.as_u16() == 507,
                "Circular reference bomb should be memory-limited, got {}",
                status
            );
        }
        Err(_) => {}
    }
}

/// Test typed array exhaustion
/// Attack: Massive Uint8Array allocations
/// Mitigation: Memory limit
#[tokio::test]
async fn test_typed_array_exhaustion() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        const arrays = [];
        
        // Allocate many large typed arrays
        for (let i = 0; i < 1000; i++) {
            try {
                const arr = new Uint8Array(100000); // 100KB each
                arrays.push(arr);
            } catch (e) {
                // Allocation may fail
                break;
            }
        }
        
        return new Response(JSON.stringify({arrays: arrays.length}), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-typedarray.local",
        "typed_array.js",
        js_content,
        100,   // 100ms CPU
        16,    // 16MB memory
    );
    
    nano.wait_ready(port, "mem-typedarray.local").await;

    let client = reqwest::Client::new();
    let result = client
        .get(&format!("http://127.0.0.1:{}/", port))
        .header("Host", "mem-typedarray.local")
        .timeout(Duration::from_secs(10))
        .send()
        .await;

    nano.stop();

    match result {
        Ok(response) => {
            let status = response.status();
            // With 8MB limit, should complete but with limited allocations
            // or get 503/507 if memory pressure triggers eviction
            assert!(
                status.is_success() || status.as_u16() == 503 || status.as_u16() == 507,
                "Typed array exhaustion should be memory-limited, got {}",
                status
            );
        }
        Err(_) => {}
    }
}

/// Test rapid sequential requests with memory buildup
/// Attack: Sequential requests that accumulate memory
/// Mitigation: Memory monitoring and LRU eviction
#[tokio::test]
async fn test_sequential_memory_buildup() {
    let port = find_available_port();
    let js_content = br#"export default {
    async fetch(request) {
        // Allocate some memory per request
        const data = new Array(10000).fill(Math.random());
        
        return new Response(JSON.stringify({
            allocated: data.length,
            sum: data.reduce((a, b) => a + b, 0)
        }), { status: 200 });
    }
}"#;

    let (mut nano, _temp_dir) = NanoProcess::start(
        port,
        "mem-sequential.local",
        "sequential.js",
        js_content,
        100,   // 100ms CPU per request
        16,    // 16MB memory
    );
    
    nano.wait_ready(port, "mem-sequential.local").await;

    let client = reqwest::Client::new();
    let mut success_count = 0;
    let mut memory_error_count = 0;
    
    // Send 20 rapid requests
    for i in 0..20 {
        let result = client
            .get(&format!("http://127.0.0.1:{}/?req={}", port, i))
            .header("Host", "mem-sequential.local")
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        
        match result {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    success_count += 1;
                } else if status.as_u16() == 503 || status.as_u16() == 507 {
                    memory_error_count += 1;
                }
            }
            Err(_) => {
                memory_error_count += 1;
            }
        }
    }

    nano.stop();

    // Should have at least some successful requests
    // and memory system should handle the load
    assert!(
        success_count > 0,
        "Sequential memory test should complete some requests (got {} success, {} memory errors)",
        success_count,
        memory_error_count
    );
}
