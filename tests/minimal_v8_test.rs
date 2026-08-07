//! Minimal V8 v147 Test
//!
//! Tests basic V8 operations to verify the CORRECT scope pattern works.
//!
//! CRITICAL: Using the pattern from v8 crate's own tests:
//! v8::scope!(let scope, isolate);

use nano::v8::initialize_platform;
use nano::v8::NanoIsolate;

/// Test that we can create an isolate and a basic scope
#[test]
fn test_minimal_isolate_creation() {
    let _ = initialize_platform();

    let isolate = NanoIsolate::new().expect("Failed to create isolate");

    // Just create and drop - no scope operations
    drop(isolate);

    println!("✅ Isolate created and dropped successfully");
}

/// Test basic V8 scope creation using the manual pattern
#[test]
fn test_basic_scope() {
    let _ = initialize_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // CORRECT PATTERN from v8 crate (manual version of scope! macro):
    let mut scope_storage = v8::HandleScope::new(isolate_ptr);
    let mut scope = {
        let scope_pinned = unsafe { std::pin::Pin::new_unchecked(&mut scope_storage) };
        scope_pinned.init()
    };
    let scope = &mut scope;

    // Now we have `scope` which is &mut PinnedRef<HandleScope>
    // Use it to create a context
    let context = v8::Context::new(scope, Default::default());
    let _ctx_scope = &mut v8::ContextScope::new(scope, context);

    println!("✅ Basic scope created and dropped successfully");
}

/// Test creating a simple V8 string using the correct pattern
#[test]
fn test_create_v8_string() {
    let _ = initialize_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // Use the correct pattern
    let mut scope_storage = v8::HandleScope::new(isolate_ptr);
    let mut scope = {
        let scope_pinned = unsafe { std::pin::Pin::new_unchecked(&mut scope_storage) };
        scope_pinned.init()
    };
    let scope = &mut scope;

    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    // Create a simple string
    let v8_str = v8::String::new(ctx_scope, "hello");
    assert!(v8_str.is_some(), "Should be able to create a V8 string");

    println!("✅ V8 string created successfully");
}

/// Test creating an ArrayBuffer using the correct pattern
#[test]
fn test_create_arraybuffer() {
    let _ = initialize_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // Use the correct pattern
    let mut scope_storage = v8::HandleScope::new(isolate_ptr);
    let mut scope = {
        let scope_pinned = unsafe { std::pin::Pin::new_unchecked(&mut scope_storage) };
        scope_pinned.init()
    };
    let scope = &mut scope;

    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    // Create an ArrayBuffer - this previously crashed with the wrong pattern
    let ab = v8::ArrayBuffer::new(ctx_scope, 10);

    println!(
        "✅ ArrayBuffer created successfully: {} bytes",
        ab.byte_length()
    );
}

/// Test the pattern that matches v8::scope! macro exactly
#[test]
fn test_manual_scope_pattern() {
    let _ = initialize_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    let isolate_ptr = isolate.isolate();

    // PATTERN from v8::scope! macro:
    let mut scope = v8::HandleScope::new(isolate_ptr);
    // SAFETY: we are shadowing the original binding, so it can't be accessed ever again
    let mut scope = {
        let scope_pinned = unsafe { std::pin::Pin::new_unchecked(&mut scope) };
        scope_pinned.init()
    };
    let scope = &mut scope;

    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    // Create something
    let v8_str = v8::String::new(ctx_scope, "manual pattern works!");
    assert!(v8_str.is_some());

    println!("✅ Manual scope pattern works!");
}
