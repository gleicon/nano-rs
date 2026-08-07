//! Complete Phase 3 Runtime API integration test
//!
//! This test verifies all WinterTC runtime APIs work together in a single handler:
//! - console API
//! - TextEncoder/TextDecoder
//! - crypto.getRandomValues
//! - performance.now
//! - structuredClone
//! - DOMException
//! - Blob (basic)
//! - FormData (basic)

use nano::runtime::RuntimeAPIs;
use nano::v8::initialize_platform;
use nano::v8::NanoIsolate;

/// Helper to execute code with V8 v147 scope pattern
fn with_v8_context<F, R>(isolate: &mut v8::Isolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

/// Test all APIs together in a single context
#[test]
fn test_all_apis_together() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind all APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    // Test all APIs in sequence
    let code = r#"
        // 1. Console API
        console.log("Testing all Phase 3 APIs");

        // 2. TextEncoder/TextDecoder
        const encoder = new TextEncoder();
        const decoder = new TextDecoder();
        const text = "Hello, World!";
        const encoded = encoder.encode(text);
        const decoded = decoder.decode(encoded);
        const encoding_ok = decoded === text;
        console.log("Encoding roundtrip:", encoding_ok);

        // 3. crypto.getRandomValues
        const randomBytes = new Uint8Array(8);
        crypto.getRandomValues(randomBytes);
        const crypto_ok = randomBytes.length === 8;
        console.log("Random bytes:", crypto_ok);

        // 4. performance.now
        const start = performance.now();
        // (do some work)
        const end = performance.now();
        const perf_ok = typeof start === 'number' && start >= 0 && end >= start;
        console.log("Performance timing:", perf_ok);

        // 5. structuredClone
        const original = { a: 1, b: [2, 3] };
        const cloned = structuredClone(original);
        cloned.a = 999;
        const clone_ok = original.a === 1 && cloned.a === 999;
        console.log("Clone independence:", clone_ok);

        // 6. DOMException
        const error = new DOMException("Something went wrong", "AbortError");
        const exception_ok = error.name === "AbortError" && error.message === "Something went wrong";
        console.log("DOMException:", exception_ok);

        // 7. Blob (basic)
        const blob = new Blob(["test content"]);
        const blob_ok = blob.size === 12;
        console.log("Blob size:", blob_ok);

        // 8. FormData (basic)
        const form = new FormData();
        const form_ok = typeof form === 'object';
        console.log("FormData exists:", form_ok);

        // Return overall result
        encoding_ok && crypto_ok && perf_ok && clone_ok && exception_ok && blob_ok && form_ok
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "true", "All Phase 3 APIs should work together");
}

/// Test crypto with different TypedArray types
#[test]
fn test_crypto_various_typed_arrays() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        // Test Uint8Array
        const u8 = new Uint8Array(4);
        crypto.getRandomValues(u8);
        const u8_ok = u8.length === 4;

        // Test Uint16Array
        const u16 = new Uint16Array(4);
        crypto.getRandomValues(u16);
        const u16_ok = u16.length === 4;

        // Test Uint32Array
        const u32 = new Uint32Array(4);
        crypto.getRandomValues(u32);
        const u32_ok = u32.length === 4;

        u8_ok && u16_ok && u32_ok
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "true",
        "crypto.getRandomValues should work with all TypedArray types"
    );
}

/// Test performance.now() monotonic behavior
#[test]
fn test_performance_monotonic() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        const times = [];
        for (let i = 0; i < 10; i++) {
            times.push(performance.now());
        }

        // Check monotonic increase
        let monotonic = true;
        for (let i = 1; i < times.length; i++) {
            if (times[i] < times[i-1]) {
                monotonic = false;
            }
        }

        // Check all are numbers
        const allNumbers = times.every(t => typeof t === 'number' && !isNaN(t));

        monotonic && allNumbers
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "true", "performance.now() should be monotonic");
}

/// Test structuredClone with complex objects
#[test]
fn test_structured_clone_complex() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        // Test nested objects
        const nested = { a: { b: { c: 1 } } };
        const cloned = structuredClone(nested);
        cloned.a.b.c = 999;
        const nested_ok = nested.a.b.c === 1 && cloned.a.b.c === 999;

        // Test arrays
        const arr = { items: [1, 2, 3] };
        const arr_cloned = structuredClone(arr);
        arr_cloned.items.push(4);
        const arr_ok = arr.items.length === 3 && arr_cloned.items.length === 4;

        // Test null and undefined handling
        const withNull = { a: null, b: undefined };
        const null_cloned = structuredClone(withNull);
        const null_ok = null_cloned.a === null;

        nested_ok && arr_ok && null_ok
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "true",
        "structuredClone should handle complex objects"
    );
}

/// Test DOMException with various error names
#[test]
fn test_dom_exception_various_names() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        const abortError = new DOMException("Aborted", "AbortError");
        const typeError = new DOMException("Invalid type", "TypeError");
        const notFound = new DOMException("Not found", "NotFoundError");
        const defaultError = new DOMException();

        const abort_ok = abortError.name === "AbortError";
        const type_ok = typeError.name === "TypeError";
        const notfound_ok = notFound.name === "NotFoundError";
        const default_ok = defaultError.name === "Error";

        abort_ok && type_ok && notfound_ok && default_ok
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "true",
        "DOMException should support various error names"
    );
}

/// Test Blob with type option
#[test]
fn test_blob_with_type() {
    initialize_platform().expect("Failed to initialize V8 platform");

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Bind APIs
    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        const textBlob = new Blob(["hello"], { type: "text/plain" });
        const jsonBlob = new Blob(['{"key": "value"}'], { type: "application/json" });
        const defaultBlob = new Blob(["test"]);

        const text_ok = textBlob.type === "text/plain";
        const json_ok = jsonBlob.type === "application/json";
        const default_ok = defaultBlob.type === "";

        text_ok && json_ok && default_ok
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "true", "Blob should support type option");
}
