//! JavaScript handler execution for WinterTC requests
//!
//! This module provides the core interface for executing JavaScript handlers
//! that receive WinterTC Request objects and return Response objects.

use anyhow::{anyhow, Result};
use bytes::Bytes;
use std::fs;

use crate::http::{NanoHeaders, NanoRequest, NanoResponse};
use crate::http::v8_bridge::serialize_request_to_json;
use crate::runtime::async_support;
use crate::runtime::apis::RuntimeAPIs;

/// Context for executing a JavaScript handler
#[derive(Debug, Clone)]
pub struct HandlerContext {
    /// Path to the JavaScript entrypoint file
    pub entrypoint: String,
    /// The incoming HTTP request
    pub request: NanoRequest,
    /// Memory limit per request in MB (0 = use default 16MB)
    pub memory_limit_mb: u32,
    /// Hostname (tenant identifier) for logging and metrics
    pub hostname: String,
}

/// Execute a JavaScript handler in a V8 isolate
///
/// # Note for Test Authors
/// This function is SYNCHRONOUS to ensure V8 scope tracking works correctly.
/// In async tests, use `tokio::task::spawn_blocking` or call from a synchronous context.
/// The function has no internal async await points - file I/O is synchronous.
pub fn execute_handler(
    isolate: &mut crate::v8::NanoIsolate,
    context: HandlerContext,
) -> Result<NanoResponse> {
    // Read the entrypoint file (synchronous - tests don't need async here)
    let code = fs::read_to_string(&context.entrypoint)
        .map_err(|e| anyhow!("Failed to read entrypoint '{}': {}", context.entrypoint, e))?;

    // Serialize the request to JSON
    let request_json = serialize_request_to_json(&context.request);

    // Execute in V8 context - synchronous, no async/await
    execute_in_v8(isolate, &code, &request_json)
}

/// Execute a JavaScript handler with an explicit V8 context
///
/// This variant is used by WorkerPool to execute handlers with a pre-existing
/// V8 context (for context reset optimization).
///
/// # Arguments
///
/// * `isolate` - The V8 isolate
/// * `v8_context` - The V8 context to execute in
/// * `context` - The handler context with entrypoint and request
///
/// # Returns
///
/// Result containing NanoResponse or an error
pub fn execute_handler_with_context(
    isolate: &mut crate::v8::NanoIsolate,
    v8_context: v8::Local<v8::Context>,
    context: HandlerContext,
) -> Result<NanoResponse> {
    use crate::runtime::vfs_bindings;
    use crate::v8::module::{is_esm_module, transform_module_code};
    
    // Read the entrypoint file
    let code = fs::read_to_string(&context.entrypoint)
        .map_err(|e| anyhow!("Failed to read entrypoint '{}': {}", context.entrypoint, e))?;

    // Check if this is an ESM module before consuming code
    let is_esm = is_esm_module(&code);
    
    // Transform ES6 module syntax only if this is an ESM module
    let transformed_code = if is_esm {
        transform_module_code(&code)
    } else {
        code
    };

    // Set up VFS context for Nano.fs API (must be before HandleScope borrows isolate)
    let vfs_ref = std::sync::Arc::new(isolate.vfs().clone());
    vfs_bindings::set_current_vfs(Some(vfs_ref));
    // Clear CURRENT_ENV so this context does not inherit stale env from a prior
    // isolate on the same thread (handler path has no app-specific env vars).
    vfs_bindings::set_current_env(std::collections::HashMap::new());

    // v147 API: HandleScope::new() returns ScopeStorage, need pin! + init
    let handle_scope = v8::HandleScope::new(isolate.isolate());
    let pinned_scope = std::pin::pin!(handle_scope);
    let mut pinned_ref = pinned_scope.init();

    // Disable eval/new Function in this context (matches worker-pool hardening)
    v8_context.set_allow_generation_from_strings(false);

    // Bind APIs first (before entering context scope)
    // v147: bind_all now accepts PinnedRef<HandleScope>
    RuntimeAPIs::bind_all(&mut pinned_ref, v8_context);

    // Enter the provided context with ContextScope
    let mut ctx_scope = v8::ContextScope::new(&mut pinned_ref, v8_context);

    // Get global object
    let global = v8_context.global(&ctx_scope);

    // Compile and execute the script (use ctx_scope for V8 operations)
    let code_string = v8::String::new(&ctx_scope, &transformed_code)
        .ok_or_else(|| anyhow!("Failed to create code string"))?;
    let script = v8::Script::compile(&ctx_scope, code_string, None)
        .ok_or_else(|| anyhow!("Script compilation failed"))?;

    // Execute script to define the fetch function
    script.run(&ctx_scope);

    // Look for the fetch function on global scope
    // For ESM modules, check __nano_user_fetch first (set by transform_module_code)
    let fetch_val = if is_esm {
        let fetch_key = v8::String::new(&ctx_scope, "__nano_user_fetch").unwrap();
        global.get(&ctx_scope, fetch_key.into())
            .filter(|val| !val.is_undefined() && !val.is_null())
    } else {
        None
    };

    let fetch_val = match fetch_val {
        Some(val) => val,
        None => {
            // Fall back to checking for global fetch function
            let fetch_key = v8::String::new(&ctx_scope, "fetch").unwrap();
            match global.get(&ctx_scope, fetch_key.into()) {
                Some(val) if !val.is_undefined() && !val.is_null() => val,
                _ => {
                    // Return a default response for now - handler doesn't define fetch
                    return Ok(NanoResponse::ok()
                        .with_header("Content-Type", "text/plain")
                        .with_body("Handler executed (no fetch function defined)"));
                }
            }
        }
    };

    // Verify it's actually a function
    if !fetch_val.is_function() {
        return Ok(NanoResponse::ok()
            .with_header("Content-Type", "text/plain")
            .with_body("Handler executed (fetch is not a function)"));
    }

    let fetch_fn = fetch_val.cast::<v8::Function>();

    // Serialize request to JSON and parse in V8
    let request_json = serialize_request_to_json(&context.request);

    // Get JSON.parse function
    let json_key = v8::String::new(&ctx_scope, "JSON").unwrap();
    let json_val = match global.get(&ctx_scope, json_key.into()) {
        Some(val) => val,
        None => return Err(anyhow!("JSON not found in global")),
    };

    let json_obj = match json_val.to_object(&ctx_scope) {
        Some(obj) => obj,
        None => return Err(anyhow!("JSON is not an object")),
    };

    let parse_key = v8::String::new(&ctx_scope, "parse").unwrap();
    let parse_fn_val = match json_obj.get(&ctx_scope, parse_key.into()) {
        Some(val) if val.is_function() => val,
        _ => return Err(anyhow!("JSON.parse not found or not a function")),
    };

    let parse_fn = parse_fn_val.cast::<v8::Function>();

    // Create the JSON string and parse it
    let json_str = match v8::String::new(&ctx_scope, &request_json) {
        Some(s) => s,
        None => return Err(anyhow!("Failed to create JSON string")),
    };

    let js_request = match parse_fn.call(&ctx_scope, json_val.into(), &[json_str.into()]) {
        Some(req) => req,
        None => return Err(anyhow!("Failed to parse request JSON")),
    };

    // Call the fetch handler with the Request
    let result = fetch_fn.call(&ctx_scope, global.into(), &[js_request]);

    // Extract the response
    match result {
        Some(response) => extract_js_response(&mut ctx_scope, response),
        None => Err(anyhow!("Handler returned None")),
    }
}

/// Internal function to execute handler in V8
fn execute_in_v8(
    isolate: &mut crate::v8::NanoIsolate,
    code: &str,
    request_json: &str,
) -> Result<NanoResponse> {
    use crate::runtime::apis::RuntimeAPIs;
    use crate::runtime::vfs_bindings;
    use crate::v8::module::{is_esm_module, transform_module_code};

    // Check if this is an ESM module first
    let is_esm = is_esm_module(code);

    // Transform ES6 module syntax to V8-compatible code if needed
    let transformed_code = if is_esm {
        transform_module_code(code)
    } else {
        code.to_string()
    };

    // Set up VFS context for Nano.fs API
    let vfs_ref = std::sync::Arc::new(isolate.vfs().clone());
    vfs_bindings::set_current_vfs(Some(vfs_ref));
    vfs_bindings::set_current_env(std::collections::HashMap::new());

    // v147 API: Create HandleScope using pin! + init pattern
    // SAFETY: We transmute to erase lifetime constraints. This is sound because:
    // 1. The HandleScope borrows from the isolate
    // 2. The isolate lives for the entire function
    // 3. All V8 operations complete before returning
    // 4. Scopes are dropped in reverse order of creation
    //
    // CRITICAL FIX: scope_storage must outlive all V8 operations.
    // We use explicit drops at the end to ensure correct drop order.
    let isolate_ref = isolate.isolate();
    
    // Create scope storage first - it must live the longest
    let mut scope_storage = unsafe {
        v8::HandleScope::new(std::mem::transmute::<_, &'static mut v8::Isolate>(isolate_ref))
    };
    
    // Use a macro-like block to capture the result, ensuring all V8 handles
    // are converted to owned data before the scopes are dropped
    let result: Result<NanoResponse> = 'v8_block: {
        let scope_pin = unsafe { std::pin::Pin::new_unchecked(&mut scope_storage) };
        let mut pinned_ref: v8::PinnedRef<v8::HandleScope> = unsafe {
            std::mem::transmute(scope_pin.init())
        };

        // Create context within the scope
        let v8_context = v8::Context::new(&mut pinned_ref, Default::default());
        v8_context.set_allow_generation_from_strings(false);

        // Bind runtime APIs first (before entering context scope)
        RuntimeAPIs::bind_all(&mut pinned_ref, v8_context);

        // Enter the context with ContextScope (v147 API)
        let mut ctx_scope = v8::ContextScope::new(&mut pinned_ref, v8_context);

        // Get global object
        let global = v8_context.global(&ctx_scope);

        // Compile and execute the script
        let code_string = v8::String::new(&ctx_scope, &transformed_code)
            .ok_or_else(|| anyhow!("Failed to create code string"))?;
        let script = v8::Script::compile(&ctx_scope, code_string, None)
            .ok_or_else(|| anyhow!("Script compilation failed"))?;

        // Execute script to define the fetch function
        script.run(&ctx_scope);

        // Look for the fetch function on global scope
        // For ESM modules, check __nano_user_fetch first (set by transform_module_code)
        let fetch_val = if is_esm {
            let fetch_key = v8::String::new(&ctx_scope, "__nano_user_fetch").unwrap();
            global.get(&ctx_scope, fetch_key.into())
                .filter(|val| !val.is_undefined() && !val.is_null())
        } else {
            None
        };

        let fetch_val = match fetch_val {
            Some(val) => val,
            None => {
                // Fall back to checking for global fetch function
                let fetch_key = v8::String::new(&ctx_scope, "fetch").unwrap();
                match global.get(&ctx_scope, fetch_key.into()) {
                    Some(val) if !val.is_undefined() && !val.is_null() => val,
                    _ => {
                        // Return a default response for now - handler doesn't define fetch
                        break 'v8_block Ok(NanoResponse::ok()
                            .with_header("Content-Type", "text/plain")
                            .with_body("Handler executed (no fetch function defined)"));
                    }
                }
            }
        };

        // Verify it's actually a function
        if !fetch_val.is_function() {
            break 'v8_block Ok(NanoResponse::ok()
                .with_header("Content-Type", "text/plain")
                .with_body("Handler executed (fetch is not a function)"));
        }

        let fetch_fn = fetch_val.cast::<v8::Function>();

        // Get JSON.parse function to create the request object
        let json_key = v8::String::new(&ctx_scope, "JSON").unwrap();
        let json_val = match global.get(&ctx_scope, json_key.into()) {
            Some(val) => val,
            None => break 'v8_block Err(anyhow!("JSON not found in global")),
        };

        let json_obj = match json_val.to_object(&ctx_scope) {
            Some(obj) => obj,
            None => break 'v8_block Err(anyhow!("JSON is not an object")),
        };

        let parse_key = v8::String::new(&ctx_scope, "parse").unwrap();
        let parse_fn_val = match json_obj.get(&ctx_scope, parse_key.into()) {
            Some(val) if val.is_function() => val,
            _ => break 'v8_block Err(anyhow!("JSON.parse not found or not a function")),
        };

        let parse_fn = parse_fn_val.cast::<v8::Function>();

        // Create the JSON string and parse it
        let json_str = match v8::String::new(&ctx_scope, request_json) {
            Some(s) => s,
            None => break 'v8_block Err(anyhow!("Failed to create JSON string")),
        };

        let js_request = match parse_fn.call(&ctx_scope, json_val.into(), &[json_str.into()]) {
            Some(req) => req,
            None => break 'v8_block Err(anyhow!("Failed to parse request JSON")),
        };

    // Convert plain headers object to Headers instance
    // Get the Headers constructor
    let headers_key = v8::String::new(&ctx_scope, "Headers").unwrap();
    if let Some(headers_ctor) = global.get(&ctx_scope, headers_key.into()) {
        if headers_ctor.is_function() {
            let headers_ctor_fn = headers_ctor.cast::<v8::Function>();

            // Get the headers from the request
            let req_headers_key = v8::String::new(&ctx_scope, "headers").unwrap();
            if let Some(req_headers) = js_request.to_object(&ctx_scope).and_then(|o| o.get(&ctx_scope, req_headers_key.into())) {
                if !req_headers.is_null() && !req_headers.is_undefined() {
                    // Create new Headers(headers)
                    if let Some(new_headers) = headers_ctor_fn.call(&ctx_scope, headers_ctor.into(), &[req_headers]) {
                        if let Some(req_obj) = js_request.to_object(&ctx_scope) {
                            let _ = req_obj.set(&ctx_scope, req_headers_key.into(), new_headers);
                        }
                    }
                }
            }
        }
    }

    // Call the fetch handler with the Request
    let result = fetch_fn.call(&ctx_scope, global.into(), &[js_request]);

    // Extract the response (may be a Promise, so resolve it)
    let final_result = match result {
        Some(response) => {
            // Check if response is a Promise and resolve if needed
            // Resolve using async event loop for Promises
            let resolved = if response.is_promise() {
                let promise = response.cast::<v8::Promise>();
                match async_support::resolve_promise_with_async(&mut ctx_scope, promise) {
                    Ok(value) => Some(value),
                    Err(e) => break 'v8_block Err(e),
                }
            } else {
                Some(response)
            };

            match resolved {
                Some(response) => extract_js_response(&mut ctx_scope, response),
                None => Err(anyhow!("Handler returned None")),
            }
        }
        None => Err(anyhow!("Handler returned None")),
    };
    
    break 'v8_block final_result;
};

// At this point, all V8 scopes have been dropped in the correct order
// (ctx_scope, then pinned_ref, then scope_storage)
result
}

/// Extract a NanoResponse from a V8 JavaScript Response object
pub fn extract_js_response(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    js_response: v8::Local<v8::Value>,
) -> Result<NanoResponse> {
    // Verify the response is an object
    let obj = match js_response.to_object(scope) {
        Some(o) => o,
        None => return Err(anyhow!("Response is not an object")),
    };

    // Extract status property (default to 200)
    let status_key = v8::String::new(scope, "status").unwrap();
    let status_val_opt = obj.get(scope, status_key.into());
    let status = match status_val_opt {
        Some(val) if !val.is_null() && !val.is_undefined() => {
            tracing::debug!("Status value found: is_number={}, is_int32={}, to_integer={:?}",
                val.is_number(), val.is_int32(), val.to_integer(scope).map(|i| i.value()));
            match val.to_integer(scope) {
                Some(int) => {
                    let s = int.value() as u16;
                    tracing::debug!("Status extracted from integer: {}", s);
                    s
                }
                None => {
                    // Try to_number as fallback
                    match val.to_number(scope) {
                        Some(num) => {
                            let s = num.value() as u16;
                            tracing::debug!("Status extracted from number: {}", s);
                            s
                        }
                        None => {
                            tracing::warn!("Failed to convert status to number, defaulting to 200");
                            200
                        }
                    }
                }
            }
        }
        Some(val) if val.is_null() => {
            tracing::debug!("Status value is null, defaulting to 200");
            200
        }
        Some(val) if val.is_undefined() => {
            tracing::debug!("Status value is undefined, defaulting to 200");
            200
        }
        _ => {
            tracing::debug!("Status property not found, defaulting to 200");
            200
        }
    };

    // Extract headers property
    let mut nano_headers = NanoHeaders::new();
    let headers_key = v8::String::new(scope, "headers").unwrap();

    if let Some(headers_val) = obj.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            // Headers may be stored internally in __headers__ property (for Headers class instances)
            // or directly on the object (for plain objects used by Response)
            let internal_headers_key = v8::String::new(scope, "__headers__").unwrap();
            let headers_source = headers_obj.get(scope, internal_headers_key.into())
                .and_then(|v| v.to_object(scope))
                .unwrap_or(headers_obj);

            // Get all property names
            if let Some(names) = headers_source.get_own_property_names(scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            let key_name = key_str.to_rust_string_lossy(scope);
                            // Skip internal properties and methods (functions)
                            if key_name.starts_with("__") || key_name == "set" || key_name == "get" || key_name == "forEach" {
                                continue;
                            }
                            if let Some(value) = headers_source.get(scope, key.into()) {
                                // Only include string values (not functions)
                                if !value.is_function() {
                                    if let Some(value_str) = value.to_string(scope) {
                                        let value_string = value_str.to_rust_string_lossy(scope);
                                        nano_headers.set(&key_name, &value_string);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract body property
    let body_key = v8::String::new(scope, "body").unwrap();
    let body = match obj.get(scope, body_key.into()) {
        Some(val) if !val.is_null() && !val.is_undefined() => {
            tracing::debug!("Response body value: type check - is_string={}, is_object={}, is_array={}",
                val.is_string(), val.is_object(), val.is_array());
            match val.to_string(scope) {
                Some(s) => {
                    let body_str = s.to_rust_string_lossy(scope);
                    tracing::debug!("Extracted response body: {} bytes", body_str.len());
                    Some(Bytes::from(body_str))
                }
                None => {
                    tracing::warn!("Failed to convert response body to string");
                    None
                }
            }
        }
        Some(val) if val.is_null() => {
            tracing::debug!("Response body is null");
            None
        }
        Some(val) if val.is_undefined() => {
            tracing::debug!("Response body is undefined");
            None
        }
        _ => {
            tracing::debug!("Response body property not found or not accessible");
            None
        }
    };

    tracing::debug!("Final NanoResponse: status={}, has_body={}, body_len={}",
        status, body.is_some(), body.as_ref().map(|b| b.len()).unwrap_or(0));
    
    Ok(NanoResponse::new(status, nano_headers, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoUrl, NanoHeaders};
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_handler_context_creation() {
        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let request = NanoRequest::new(
            "GET".to_string(),
            url,
            NanoHeaders::new(),
            None,
        );

        let context = HandlerContext {
            entrypoint: "/app/index.js".to_string(),
            request,
            memory_limit_mb: 0,
            hostname: String::new(),
        };

        assert_eq!(context.entrypoint, "/app/index.js");
        assert_eq!(context.request.method(), "GET");
    }

    #[test]
    fn test_extract_js_response_basic() {
        init_platform();

        let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

        let mut isolate = crate::v8::NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Create a simple response object in JavaScript with dynamic body
        let code = format!(
            r#"({{ status: 200, headers: {{ "Content-Type": "text/plain" }}, body: "{}" }})"#,
            dynamic_token
        );
        let code_str = v8::String::new(ctx_scope, &code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_str, None).unwrap();
        let result = script.run(ctx_scope).expect("Script execution failed");

        let response = extract_js_response(ctx_scope, result);
        assert!(response.is_ok(), "Failed to extract response: {:?}", response.err());

        let nano_response = response.unwrap();
        assert_eq!(nano_response.status(), 200);
        assert_eq!(nano_response.headers().get("Content-Type"), Some("text/plain".to_string()));
        assert!(
            nano_response.body().is_some(),
            "Response should have a body"
        );
        let body_text = String::from_utf8_lossy(nano_response.body().unwrap());
        assert!(
            body_text.contains(&dynamic_token),
            "Response body must contain dynamic token '{}', got: {}",
            dynamic_token,
            body_text
        );
    }

    #[test]
    fn test_extract_js_response_no_body() {
        init_platform();

        let mut isolate = crate::v8::NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Create a response without body
        let code = r#"({ status: 204, headers: {} })"#;
        let code_str = v8::String::new(ctx_scope, code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_str, None).unwrap();
        let result = script.run(ctx_scope).expect("Script execution failed");

        let response = extract_js_response(ctx_scope, result);
        assert!(response.is_ok());

        let nano_response = response.unwrap();
        assert_eq!(nano_response.status(), 204);
        assert!(nano_response.body().is_none());
    }

    #[test]
    fn test_extract_js_response_default_status() {
        init_platform();

        let dynamic_token = format!("nanotest-{}", uuid::Uuid::new_v4());

        let mut isolate = crate::v8::NanoIsolate::new().expect("Failed to create isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Create a response without explicit status (should default to 200)
        let code = format!(r#"({{ headers: {{}}, body: "{}" }})"#, dynamic_token);
        let code_str = v8::String::new(ctx_scope, &code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_str, None).unwrap();
        let result = script.run(ctx_scope).expect("Script execution failed");

        let response = extract_js_response(ctx_scope, result);
        assert!(response.is_ok());

        let nano_response = response.unwrap();
        assert_eq!(nano_response.status(), 200);
        assert!(
            nano_response.body().is_some(),
            "Response should have a body"
        );
        let body_text = String::from_utf8_lossy(nano_response.body().unwrap());
        assert!(
            body_text.contains(&dynamic_token),
            "Response body must contain dynamic token '{}', got: {}",
            dynamic_token,
            body_text
        );
    }
}
