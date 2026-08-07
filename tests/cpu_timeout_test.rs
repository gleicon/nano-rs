//! Integration test for CPU timeout enforcement
//!
//! This test verifies that CPU timeout actually terminates execution
//! when a script runs longer than the configured limit.

use nano::v8::{initialize_platform, NanoIsolate};

/// Helper to ensure platform is initialized for tests
fn init_platform() {
    if !nano::v8::is_initialized() {
        initialize_platform().expect("Failed to initialize V8 platform");
    }
}

/// Test that CPU timeout terminates an infinite loop within expected time
#[test]
fn test_cpu_timeout_terminates_infinite_loop() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let context = isolate.create_context();

    // Start timing before creating the timeout guard
    let start = std::time::Instant::now();

    {
        let iso_ptr: *mut v8::Isolate = &mut **isolate.isolate();
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        // Create a CpuTimeoutGuard with a 50ms timeout
        // This will spawn a timer thread that calls terminate_execution()
        let _guard = nano::data_plane::CpuTimeoutGuard::new(
            unsafe { &mut *iso_ptr },
            50, // 50ms timeout
        );

        // Script that runs an infinite loop
        let code = "while(true) {}";

        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        // Execute - this should be terminated by the CPU timeout
        let result = script.run(&mut ctx_scope);
        let elapsed = start.elapsed();

        // Execution should complete (either successfully or with termination)
        // The important thing is it doesn't hang indefinitely
        // With 50ms timeout + overhead, should complete within 500ms
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "Execution should not hang indefinitely with CPU timeout - took {:?}",
            elapsed
        );

        // Result may be None (terminated) or Some (succeeded before timeout)
        // In this case with infinite loop, it should be None (terminated)
        tracing::info!(
            "Execution result: {:?}, elapsed: {:?}",
            result.is_some(),
            elapsed
        );

        // With a tight infinite loop and 50ms timeout, we expect termination
        // (result should be None due to terminate_execution)
        assert!(
            result.is_none(),
            "Infinite loop should be terminated (result should be None)"
        );
    }

    // Guard should be dropped here, joining the timer thread
}

/// Test that normal scripts complete successfully before timeout
#[test]
fn test_cpu_timeout_allows_normal_execution() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let context = isolate.create_context();

    {
        let iso_ptr: *mut v8::Isolate = &mut **isolate.isolate();
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        // Create a CpuTimeoutGuard with a long 1000ms timeout
        let _guard = nano::data_plane::CpuTimeoutGuard::new(
            unsafe { &mut *iso_ptr },
            1000, // 1 second timeout - plenty for a simple script
        );

        // Simple script that completes quickly
        let code = "2 + 2";

        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        // Execute - this should complete normally
        let result = script.run(&mut ctx_scope);

        // Script should succeed before timeout
        assert!(
            result.is_some(),
            "Simple script should complete successfully before timeout"
        );

        // Verify the result is 4
        if let Some(value) = result {
            assert!(value.is_number(), "Result should be a number");
            if let Some(int) = value.to_integer(&mut ctx_scope) {
                assert_eq!(int.value(), 4, "2 + 2 should equal 4");
            }
        }
    }
}

/// Test that isolate remains usable after CPU timeout termination
#[test]
fn test_isolate_usable_after_cpu_timeout() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    // First execution: timeout
    {
        let context = isolate.create_context();
        let iso_ptr: *mut v8::Isolate = &mut **isolate.isolate();
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        let _guard = nano::data_plane::CpuTimeoutGuard::new(
            unsafe { &mut *iso_ptr },
            50, // 50ms timeout
        );

        let code = "while(true) {}";
        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        let result = script.run(&mut ctx_scope);
        assert!(result.is_none(), "Infinite loop should be terminated");
    }

    // Second execution: should still work
    {
        let context = isolate.create_context();
        let iso_ptr: *mut v8::Isolate = &mut **isolate.isolate();
        let scope_storage = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
        let mut scope = scope_storage.init();
        let local_context = v8::Local::new(&mut scope, &context);
        let mut ctx_scope = v8::ContextScope::new(&mut scope, local_context);

        let _guard = nano::data_plane::CpuTimeoutGuard::new(
            unsafe { &mut *iso_ptr },
            1000, // 1 second timeout
        );

        let code = "'hello'";
        let code_str = v8::String::new(&mut ctx_scope, code).expect("Failed to create code string");
        let script =
            v8::Script::compile(&mut ctx_scope, code_str, None).expect("Failed to compile script");

        let result = script.run(&mut ctx_scope);
        assert!(
            result.is_some(),
            "Isolate should be usable after previous timeout"
        );
    }
}
