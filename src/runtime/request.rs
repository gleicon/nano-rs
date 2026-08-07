//! Request object helpers for WinterTC compatibility
//!
//! This module provides Request prototype methods (text, json, arrayBuffer)
//! for reading request bodies. These methods decode base64-encoded bodies
//! from the serialized request object.

use anyhow::Result;
use base64::Engine;

/// Binds Request constructor and prototype methods to the V8 context
pub fn bind_request_api(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create Request constructor if it doesn't exist
    let request_key = v8::String::new(&mut ctx_scope, "Request").unwrap();
    let request_ctor = if let Some(existing) = global.get(&mut ctx_scope, request_key.into()) {
        if existing.is_function() {
            existing
        } else {
            // Create new Request constructor
            let ctor = v8::Function::new(&mut ctx_scope, request_constructor_callback).unwrap();
            global.set(&mut ctx_scope, request_key.into(), ctor.into());
            ctor.into()
        }
    } else {
        // Create Request constructor
        let ctor = v8::Function::new(&mut ctx_scope, request_constructor_callback).unwrap();
        global.set(&mut ctx_scope, request_key.into(), ctor.into());
        ctor.into()
    };

    // Add methods to Request.prototype
    if let Some(request_obj) = request_ctor.to_object(&mut ctx_scope) {
        let prototype_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        // Get or create prototype
        let prototype =
            if let Some(existing_proto) = request_obj.get(&mut ctx_scope, prototype_key.into()) {
                if existing_proto.is_object() || existing_proto.is_function() {
                    existing_proto
                } else {
                    let new_proto = v8::Object::new(&mut ctx_scope);
                    request_obj.set(&mut ctx_scope, prototype_key.into(), new_proto.into());
                    new_proto.into()
                }
            } else {
                let new_proto = v8::Object::new(&mut ctx_scope);
                request_obj.set(&mut ctx_scope, prototype_key.into(), new_proto.into());
                new_proto.into()
            };

        if let Some(proto_obj) = prototype.to_object(&mut ctx_scope) {
            bind_request_method(&mut ctx_scope, proto_obj, "text", request_text_callback);
            bind_request_method(&mut ctx_scope, proto_obj, "json", request_json_callback);
            bind_request_method(
                &mut ctx_scope,
                proto_obj,
                "arrayBuffer",
                request_arraybuffer_callback,
            );
        }
    }
}

/// Request constructor callback - creates a new Request instance
fn request_constructor_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // When called with 'new', V8 provides 'this' as a new object with the correct prototype
    // Just use 'this' directly - it already has Request.prototype in its chain!
    let this = args.this();
    let instance = if let Some(obj) = this.to_object(scope) {
        obj
    } else {
        v8::Object::new(scope)
    };

    // Extract URL from first argument
    let url = if args.length() > 0 {
        let arg = args.get(0);
        if let Some(s) = arg.to_string(scope) {
            s.to_rust_string_lossy(scope)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Set url property
    let url_key = v8::String::new(scope, "url").unwrap();
    let url_val = v8::String::new(scope, &url).unwrap();
    instance.set(scope, url_key.into(), url_val.into());

    // Extract properties from init object
    let (method, body_val) = if args.length() > 1 {
        let init = args.get(1);
        if let Some(obj) = init.to_object(scope) {
            // Get method
            let method_key = v8::String::new(scope, "method").unwrap();
            let method = if let Some(method_val) = obj.get(scope, method_key.into()) {
                if let Some(s) = method_val.to_string(scope) {
                    s.to_rust_string_lossy(scope)
                } else {
                    "GET".to_string()
                }
            } else {
                "GET".to_string()
            };

            // Get body
            let body_key = v8::String::new(scope, "body").unwrap();
            let body_val = obj.get(scope, body_key.into());

            (method, body_val)
        } else {
            ("GET".to_string(), None)
        }
    } else {
        ("GET".to_string(), None)
    };

    let method_key = v8::String::new(scope, "method").unwrap();
    let method_val_str = v8::String::new(scope, &method).unwrap();
    instance.set(scope, method_key.into(), method_val_str.into());

    // Set headers property - use headers from init if provided, otherwise create new
    let headers_key = v8::String::new(scope, "headers").unwrap();
    if args.length() > 1 {
        let init = args.get(1);
        if let Some(obj) = init.to_object(scope) {
            // Check if init has headers
            if let Some(headers_val) = obj.get(scope, headers_key.into()) {
                if !headers_val.is_null() && !headers_val.is_undefined() {
                    // Use headers from init
                    instance.set(scope, headers_key.into(), headers_val.into());
                } else {
                    // Create new plain headers object
                    let headers_obj = v8::Object::new(scope);
                    instance.set(scope, headers_key.into(), headers_obj.into());
                }
            } else {
                // Create new plain headers object
                let headers_obj = v8::Object::new(scope);
                instance.set(scope, headers_key.into(), headers_obj.into());
            }
        } else {
            // Create new plain headers object
            let headers_obj = v8::Object::new(scope);
            instance.set(scope, headers_key.into(), headers_obj.into());
        }
    } else {
        // Create new plain headers object
        let headers_obj = v8::Object::new(scope);
        instance.set(scope, headers_key.into(), headers_obj.into());
    }

    // Set body and bodyUsed
    let body_key = v8::String::new(scope, "body").unwrap();
    // Check if body_val exists AND is not undefined/null
    if let Some(body) = body_val {
        if !body.is_null() && !body.is_undefined() {
            instance.set(scope, body_key.into(), body.into());
        } else {
            let null_val = v8::null(scope);
            instance.set(scope, body_key.into(), null_val.into());
        }
    } else {
        let null_val = v8::null(scope);
        instance.set(scope, body_key.into(), null_val.into());
    }

    let body_used_key = v8::String::new(scope, "bodyUsed").unwrap();
    let body_used_val = v8::Boolean::new(scope, false);
    instance.set(scope, body_used_key.into(), body_used_val.into());

    retval.set(instance.into());
}

fn bind_request_method(
    scope: &mut v8::ContextScope<'_, '_, v8::HandleScope>,
    prototype: v8::Local<v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let name = v8::String::new(scope, name).unwrap();
    let func = v8::Function::new(scope, callback).unwrap();
    let _ = prototype.set(scope, name.into(), func.into());
}

/// Callback for Request.prototype.text()
/// Returns the decoded body as a string
fn request_text_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get 'this' (the request object)
    let this = args.this();

    // Extract body from the request object (base64 encoded in body property)
    let body_key = v8::String::new(scope, "body").unwrap();
    if let Some(body_val) = this.get(scope, body_key.into()) {
        if !body_val.is_null() && !body_val.is_undefined() {
            if let Some(body_str) = body_val.to_string(scope) {
                let base64_body = body_str.to_rust_string_lossy(scope);
                // Decode base64 and return as string
                if let Ok(decoded) = base64_decode(&base64_body) {
                    let text = String::from_utf8_lossy(&decoded);
                    let result = v8::String::new(scope, &text).unwrap();
                    retval.set(result.into());
                    return;
                }
            }
        }
    }

    // Return empty string if no body
    tracing::debug!("request.text(): returning empty string");
    let empty = v8::String::new(scope, "").unwrap();
    retval.set(empty.into());
}

/// Callback for Request.prototype.json()
/// Parses the body as JSON and returns the object
fn request_json_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let body_key = v8::String::new(scope, "body").unwrap();

    if let Some(body_val) = this.get(scope, body_key.into()) {
        if !body_val.is_null() && !body_val.is_undefined() {
            if let Some(body_str) = body_val.to_string(scope) {
                let base64_body = body_str.to_rust_string_lossy(scope);
                if let Ok(decoded) = base64_decode(&base64_body) {
                    let text = String::from_utf8_lossy(&decoded);

                    // Parse JSON using JSON.parse
                    let global = scope.get_current_context().global(scope);
                    let json_key = v8::String::new(scope, "JSON").unwrap();
                    if let Some(json_val) = global.get(scope, json_key.into()) {
                        if let Some(json_obj) = json_val.to_object(scope) {
                            let parse_key = v8::String::new(scope, "parse").unwrap();
                            if let Some(parse_fn) = json_obj.get(scope, parse_key.into()) {
                                if parse_fn.is_function() {
                                    let parse = parse_fn.cast::<v8::Function>();
                                    let text_str = v8::String::new(scope, &text).unwrap();
                                    if let Some(result) =
                                        parse.call(scope, json_val.into(), &[text_str.into()])
                                    {
                                        retval.set(result);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Return null on error
    retval.set(v8::null(scope).into());
}

/// Callback for Request.prototype.arrayBuffer()
/// Returns the decoded body as an ArrayBuffer
fn request_arraybuffer_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let body_key = v8::String::new(scope, "body").unwrap();

    if let Some(body_val) = this.get(scope, body_key.into()) {
        if !body_val.is_null() && !body_val.is_undefined() {
            if let Some(body_str) = body_val.to_string(scope) {
                let base64_body = body_str.to_rust_string_lossy(scope);
                if let Ok(decoded) = base64_decode(&base64_body) {
                    // Create ArrayBuffer from decoded bytes
                    let buffer = v8::ArrayBuffer::new(scope, decoded.len());
                    let store = buffer.get_backing_store();

                    // Copy bytes into ArrayBuffer
                    for (i, byte) in decoded.iter().enumerate() {
                        if let Some(cell) = store.get(i) {
                            cell.set(*byte);
                        }
                    }

                    retval.set(buffer.into());
                    return;
                }
            }
        }
    }

    // Return empty ArrayBuffer if no body
    let empty = v8::ArrayBuffer::new(scope, 0);
    retval.set(empty.into());
}

/// Decode base64 string to bytes
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|_| ())
}
