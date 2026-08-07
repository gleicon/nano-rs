//! Test utilities for V8 v147 API
//!
//! This module provides helper functions for creating V8 scopes
//! in tests using the v147 API patterns.
#![allow(dead_code)]

/// Creates a test V8 isolate with default settings
pub fn create_test_isolate() -> v8::OwnedIsolate {
    v8::Isolate::new(Default::default())
}

/// Initialize V8 platform for tests
pub fn init_v8_platform() {
    v8::V8::initialize_platform(v8::new_default_platform(0, false).make_shared());
    v8::V8::initialize();
}

/// Execute JavaScript code in a V8 isolate and return the result as a string
///
/// This helper encapsulates the entire scope setup and teardown for simple tests.
/// Uses the v8::scope! macro pattern from V8 v147.
pub fn execute_js(isolate: &mut v8::Isolate, code: &str) -> Option<String> {
    // Create scopes using the v147 v8::scope! macro pattern
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    // Execute the code
    let code_str = v8::String::new(ctx_scope, code)?;
    let script = v8::Script::compile(ctx_scope, code_str, None)?;
    let result = script.run(ctx_scope)?;

    // Convert to string
    let result_str = result.to_string(ctx_scope)?;
    Some(result_str.to_rust_string_lossy(ctx_scope))
}

/// A helper macro to execute code within V8 scopes
///
/// Usage:
/// ```rust
/// v8_test!(&mut isolate, |scope, context| {
///     // Your test code here
///     v8::String::new(scope, "1 + 1").unwrap()
/// });
/// ```
#[macro_export]
macro_rules! v8_test {
    ($isolate:expr, |$scope:ident, $context:ident| $body:expr) => {{
        v8::scope!(handle_scope, $isolate);
        let $context = v8::Context::new(handle_scope, Default::default());
        let $scope = &mut v8::ContextScope::new(handle_scope, $context);
        $body
    }};
}

/// Run a function within a V8 context
///
/// This is a more explicit version of the macro for complex test cases.
pub fn with_v8_context<F, R>(isolate: &mut v8::Isolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}
