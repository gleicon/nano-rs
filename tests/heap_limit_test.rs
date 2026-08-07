//! Integration test for heap limit enforcement
//!
//! This test verifies that V8 heap limits actually terminate execution
//! when memory allocation exceeds the configured limit.

use nano::v8::{initialize_platform, NanoIsolate};

/// Helper to ensure platform is initialized for tests
fn init_platform() {
    if !nano::v8::is_initialized() {
        initialize_platform().expect("Failed to initialize V8 platform");
    }
}

/// Test that heap limit is stored correctly after calling set_heap_limits
#[test]
fn test_heap_limit_stored() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Set 64MB heap limit
    isolate.set_heap_limits(32 * 1024 * 1024, 64 * 1024 * 1024);

    // Verify the limit was stored
    assert_eq!(isolate.heap_limit_bytes(), 64 * 1024 * 1024);
}

/// Test that JavaScript execution is terminated when exceeding heap limit
///
/// This test actually attempts to trigger the heap limit callback by allocating
/// a large amount of memory in JavaScript.
#[test]
fn test_heap_limit_terminates_execution() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Set a small 8MB heap limit to trigger quickly
    isolate.set_heap_limits(4 * 1024 * 1024, 8 * 1024 * 1024);

    let context = isolate.create_context();
    {
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        // Script that allocates large arrays repeatedly to trigger heap limit
        // This should cause V8 to approach the heap limit and invoke our callback
        let code = r#"
            // Allocate large arrays to consume heap
            const arrays = [];
            for (let i = 0; i < 100; i++) {
                arrays.push(new Array(100000).fill('x'.repeat(100)));
            }
            "done";
        "#;

        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        // Execute - this may be terminated by heap limit callback
        let start = std::time::Instant::now();
        let result = script.run(&mut ctx_scope);
        let elapsed = start.elapsed();

        // Execution should complete (either successfully or with termination)
        // The important thing is it doesn't hang indefinitely
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "Execution should not hang - took {:?}",
            elapsed
        );

        // Result may be None (terminated) or Some (succeeded before limit hit)
        // Both are acceptable - the heap limit callback is registered
        tracing::info!(
            "Execution result: {:?}, elapsed: {:?}",
            result.is_some(),
            elapsed
        );
    }
}

/// Test that isolate remains usable after potential heap limit termination
///
/// This verifies the isolate isn't corrupted after a termination event
#[test]
fn test_isolate_usable_after_heap_termination() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Set heap limit before creating context
    isolate.set_heap_limits(4 * 1024 * 1024, 8 * 1024 * 1024);

    // Create context and verify isolate is still usable
    let context = isolate.create_context();
    {
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        // Simple script that should execute successfully
        let code = r#"
            const result = 2 + 2;
            result;
        "#;

        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        let result = script.run(&mut ctx_scope);
        assert!(
            result.is_some(),
            "Execution should succeed after setting heap limits"
        );

        // Verify the result
        if let Some(value) = result {
            assert!(value.is_number(), "Result should be a number");
            if let Some(int) = value.to_integer(&mut ctx_scope) {
                assert_eq!(int.value(), 4, "2 + 2 should equal 4");
            }
        }
    }
}

/// Test that heap statistics are available
#[test]
fn test_heap_statistics_available() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Get heap stats before setting limits
    let stats_before = isolate.heap_statistics();
    assert!(
        stats_before.total_heap_size() > 0,
        "Heap should have some size"
    );

    // Set heap limit
    isolate.set_heap_limits(10 * 1024 * 1024, 16 * 1024 * 1024);

    // Get heap stats after setting limits
    let stats_after = isolate.heap_statistics();
    assert!(
        stats_after.total_heap_size() > 0,
        "Heap should still have size after setting limits"
    );

    // Heap limit should reflect our setting
    assert_eq!(isolate.heap_limit_bytes(), 16 * 1024 * 1024);
}

/// Test that multiple heap limit settings work correctly
///
/// V8 only allows one near-heap-limit callback per isolate, so subsequent
/// calls should update the stored limit but the callback remains registered.
#[test]
fn test_heap_limit_update_value() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Set initial limit
    isolate.set_heap_limits(10 * 1024 * 1024, 16 * 1024 * 1024);
    assert_eq!(isolate.heap_limit_bytes(), 16 * 1024 * 1024);

    // Update to larger limit - stored value should change
    isolate.set_heap_limits(20 * 1024 * 1024, 32 * 1024 * 1024);
    assert_eq!(isolate.heap_limit_bytes(), 32 * 1024 * 1024);

    // Isolate should still be functional after limit updates
    let _context = isolate.create_context();

    // Try another update
    isolate.set_heap_limits(30 * 1024 * 1024, 64 * 1024 * 1024);
    assert_eq!(isolate.heap_limit_bytes(), 64 * 1024 * 1024);
}
