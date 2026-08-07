//! JavaScript script execution with console.log binding
//!
//! This module provides script compilation and execution within V8 contexts,
//! including the console.log binding that redirects JavaScript console output
//! to Rust's stdout.
//!
//! # HandleScope Nesting Pattern (D-04) - V147 API
//!
//! Critical pattern for memory safety during script execution:
//! 1. Create HandleScope for the isolate (using pin! + init)
//! 2. Create context within that scope
//! 3. Create ContextScope to enter the context (ContextScope::new - NO init needed!)
//! 4. Bind console.log to global object
//! 5. Create nested HandleScope for script compilation (using pin! + init)
//! 6. Compile and execute script
//! 7. Convert result to Rust String
//!
//! Each nested scope ensures temporary handles are freed after use.

use anyhow::{anyhow, Result};

/// Execute a JavaScript script within the given isolate
///
/// This function compiles and executes JavaScript code, returning the result
/// as a Rust String. It handles the full HandleScope nesting pattern and
/// binds console.log to stdout.
///
/// # Arguments
/// * `isolate` - The V8 isolate to execute in
/// * `code` - The JavaScript code to execute
///
/// # Returns
/// * `Ok(String)` - The script result as a string
/// * `Err(anyhow::Error)` - If compilation or execution fails
///
/// # Example
/// ```
/// use nano::v8::{initialize_platform, NanoIsolate, execute_script};
///
/// initialize_platform().unwrap();
/// let mut isolate = NanoIsolate::new().unwrap();
/// let result = execute_script(&mut isolate, "1 + 1").unwrap();
/// assert_eq!(result, "2");
/// ```
pub fn execute_script(isolate: &mut crate::v8::isolate::NanoIsolate, code: &str) -> Result<String> {
    // Scope 1: HandleScope for the operation (v147 API: pin! + init)
    let scope = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
    let mut scope = scope.init();

    // Create context within the scope
    let context = v8::Context::new(&scope, Default::default());

    // Scope 2: ContextScope to enter the context
    // v147 API: ContextScope does NOT need init() - use directly
    let mut context_scope = v8::ContextScope::new(&mut scope, context);

    // Bind console.log to the global object
    bind_console_log(&mut context_scope, context);

    // Scope 3: Compile and execute script (temporary nested scope, v147 API)
    let result_string = {
        let nested_scope = std::pin::pin!(v8::HandleScope::new(&mut context_scope));
        let nested_scope = nested_scope.init();

        let code_string = v8::String::new(&nested_scope, code)
            .ok_or_else(|| anyhow!("Failed to create code string"))?;
        let script = v8::Script::compile(&nested_scope, code_string, None)
            .ok_or_else(|| anyhow!("Script compilation failed"))?;

        match script.run(&nested_scope) {
            Some(value) => {
                // Convert to string within this scope
                value
                    .to_string(&nested_scope)
                    .map(|s| s.to_rust_string_lossy(&nested_scope))
            }
            None => None,
        }
    };

    // Return result or error
    match result_string {
        Some(s) => Ok(s),
        None => Err(anyhow!("Script execution failed or returned None")),
    }
}

/// Bind console.log to the global object
///
/// This creates a global `console` object with a `log` method that
/// redirects JavaScript console.log calls to Rust's stdout via println!.
///
/// # V147 API Note
/// Uses &**scope to dereference ContextScope to PinnedRef for V8 APIs.
fn bind_console_log(
    scope: &mut v8::ContextScope<'_, '_, v8::HandleScope<'_, v8::Context>>,
    context: v8::Local<'_, v8::Context>,
) {
    // Get the global object
    // v147 API: Dereference ContextScope to get PinnedRef
    let global = context.global(&**scope);

    // Create the console object
    let console = v8::Object::new(&**scope);

    // Create the log function
    // v147 API: Function::new expects &mut PinnedRef and a callback with matching signature
    let log_fn = v8::Function::new(scope, console_log_callback);

    if let Some(log_fn) = log_fn {
        // Set console.log = log_fn
        let log_key = v8::String::new(&**scope, "log").unwrap();
        console.set(&**scope, log_key.into(), log_fn.into());

        // Set global.console = console
        let console_key = v8::String::new(&**scope, "console").unwrap();
        global.set(&**scope, console_key.into(), console.into());
    }
}

/// V8 function callback for console.log
///
/// This callback is invoked when JavaScript code calls console.log().
/// It extracts all arguments, converts them to strings, and prints them
/// to stdout via println!.
///
/// # V147 API Note
/// Callback signature uses PinnedRef<HandleScope<Context>> for v147 compatibility.
fn console_log_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Collect all arguments as strings
    let mut output = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        // v147 API: to_string and to_rust_string_lossy expect &PinnedRef
        // PinnedRef derefs to HandleScope which derefs to Isolate
        if let Some(arg_str) = arg.to_string(scope) {
            output.push(arg_str.to_rust_string_lossy(&**scope));
        }
    }

    // Print to stdout
    if !output.is_empty() {
        println!("{}", output.join(" "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    /// Helper to ensure platform is initialized for tests
    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    /// Test 1: Basic script execution ("1 + 1" returns "2")
    #[test]
    fn test_basic_execution() {
        init_platform();

        let mut isolate = crate::v8::isolate::NanoIsolate::new().expect("Failed to create isolate");
        let result = execute_script(&mut isolate, "1 + 1").expect("Script execution failed");
        assert_eq!(result, "2");
    }

    /// Test 2: console.log prints to stdout
    #[test]
    fn test_console_output() {
        init_platform();

        let mut isolate = crate::v8::isolate::NanoIsolate::new().expect("Failed to create isolate");

        // Execute script with console.log - should not panic
        // Output will be visible in test output
        execute_script(&mut isolate, r#"console.log("test output")"#)
            .expect("Script with console.log failed");
    }

    /// Test 3: Multiple console.log calls
    #[test]
    fn test_multiple_console_calls() {
        init_platform();

        let mut isolate = crate::v8::isolate::NanoIsolate::new().expect("Failed to create isolate");

        execute_script(
            &mut isolate,
            r#"
            console.log("line 1");
            console.log("line 2");
            console.log("line 3");
        "#,
        )
        .expect("Multiple console.log calls failed");
    }

    /// Test 4: Script with syntax error returns error
    #[test]
    fn test_syntax_error() {
        init_platform();

        let mut isolate = crate::v8::isolate::NanoIsolate::new().expect("Failed to create isolate");

        let result = execute_script(&mut isolate, "{ invalid syntax");
        assert!(result.is_err(), "Expected error for invalid syntax");
    }
}
