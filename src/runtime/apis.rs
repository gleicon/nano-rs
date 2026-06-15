//! Runtime JavaScript APIs for WinterTC compatibility
//!
//! This module provides JavaScript API bindings that bridge between V8 and Rust:
//! - console.log/warn/error with structured logging via tracing
//! - TextEncoder/TextDecoder for UTF-8 encoding/decoding
//! - crypto.getRandomValues for cryptographic randomness
//! - performance.now for high-resolution monotonic timing
//! - structuredClone for deep object cloning
//! - DOMException for standard error types
//! - Blob for binary data containers
//! - FormData for multipart form data
//!
//! All APIs are bound to the V8 global scope via RuntimeAPIs::bind_all().

use std::cell::Cell;
use std::time::Instant;
use crate::runtime::subtle_v8::{
    subtle_generate_key, subtle_import_key, subtle_export_key,
    subtle_encrypt, subtle_decrypt,
    subtle_sign, subtle_verify, subtle_digest,
};

thread_local! {
    static PERFORMANCE_BASELINE: Cell<Option<Instant>> = Cell::new(None);
}

pub(crate) use super::timers::{
    fire_pending_intervals, clear_pending_intervals,
    fire_pending_timeouts, clear_pending_timeouts,
};

/// RuntimeAPIs manages all JavaScript API bindings
///
/// This struct provides methods to bind WinterTC-compatible APIs to V8 contexts.
/// Call RuntimeAPIs::bind_all() during context setup to make all APIs available.
pub struct RuntimeAPIs;

impl RuntimeAPIs {
    /// Bind all runtime APIs to the V8 context
    ///
    /// This should be called once per context during handler setup.
    /// Makes all WinterTC APIs available to JavaScript.
    /// v147 API: Accepts PinnedRef<HandleScope<()>> (before context entry)
    pub fn bind_all(
        scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
        context: v8::Local<v8::Context>,
    ) {
        Self::bind_console(scope, context);
        Self::bind_text_encoder(scope, context);
        Self::bind_text_decoder(scope, context);
        Self::bind_crypto(scope, context);
        Self::bind_performance(scope, context);
        Self::bind_structured_clone(scope, context);
        Self::bind_dom_exception(scope, context);
        Self::bind_blob(scope, context);
        Self::bind_form_data(scope, context);
        Self::bind_headers(scope, context);
        Self::bind_url(scope, context);
        Self::bind_response(scope, context);
        Self::bind_request(scope, context);
        Self::bind_fetch(scope, context);
        Self::bind_nano_fs(scope, context);
        Self::bind_fs_polyfill(scope, context);
        Self::bind_timers(scope, context);
        Self::bind_buffer(scope, context);
        Self::bind_streams(scope, context);
        Self::bind_wasm(scope, context);
        Self::bind_websocket_pair(scope, context);
        // Security hardening must run last: removes eval and blocks dynamic code generation
        Self::bind_security_hardening(scope, context);
    }

    /// Security hardening: remove eval and block dynamic code generation via Function constructor
    ///
    /// Must be called after all other binds. Removes `eval` from globalThis and
    /// replaces `Function` with a locked-down stub that throws TypeError.
    /// Function declarations and arrow functions are unaffected (parsed statically by V8).
    fn bind_security_hardening(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Remove eval from global — makes typeof eval !== 'function'
        if let Some(eval_key) = v8::String::new(&mut ctx_scope, "eval") {
            global.delete(&mut ctx_scope, eval_key.into());
        }

        // Replace globalThis.Function with a stub that always throws TypeError.
        // This blocks dynamic code generation attacks while leaving function
        // declarations/expressions unaffected (they're parsed statically by V8).
        if let Some(blocked_fn) = v8::Function::new(&mut ctx_scope, function_constructor_blocked) {
            let fn_key = match v8::String::new(&mut ctx_scope, "Function") {
                Some(k) => k,
                None => return, // V8 OOM during hardening — skip, not fatal
            };
            // writable:false via constructor; configurable and enumerable set separately
            let mut desc = v8::PropertyDescriptor::new_from_value_writable(blocked_fn.into(), false);
            desc.set_configurable(false);
            desc.set_enumerable(false);
            global.define_property(&mut ctx_scope, fn_key.into(), &desc);
        }
    }

    /// Bind Streams API (ReadableStream, WritableStream)
    fn bind_streams(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::stream::bind_streams(scope, context);
    }

    /// Bind WebSocketPair API (Cloudflare Workers WebSocket API)
    fn bind_websocket_pair(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::websocket::bind_websocket_pair(scope, context);
    }

    /// Bind Request API (text, json, arrayBuffer methods)
    fn bind_request(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        // v147 API: ContextScope requires PinnedRef<HandleScope>
        // For now, we enter context scope manually after pinning
        // This is handled by the caller after bind_all completes
        crate::runtime::request::bind_request_api(scope, context);
    }

    /// Bind Nano.fs API for VFS operations
    fn bind_nano_fs(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::vfs_bindings::bind_nano_fs(scope, context);
    }

    /// Bind Node.js fs polyfill for compatibility
    fn bind_fs_polyfill(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::fs_polyfill::bind_fs_polyfill(scope, context);
    }

    /// Bind fetch() API to global scope
    fn bind_fetch(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::fetch::bind_fetch(scope, context);
    }

    /// Bind console API (log/warn/error) to global scope
    fn bind_console(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for operations that need HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        let console = v8::Object::new(&mut &mut ctx_scope);

        // Bind log method
        if let Some(log_fn) = v8::Function::new(&mut ctx_scope, console_log_callback) {
            let key = v8::String::new(&mut ctx_scope, "log").unwrap();
            console.set(&mut ctx_scope, key.into(), log_fn.into());
        }

        // Bind warn method
        if let Some(warn_fn) = v8::Function::new(&mut ctx_scope, console_warn_callback) {
            let key = v8::String::new(&mut ctx_scope, "warn").unwrap();
            console.set(&mut ctx_scope, key.into(), warn_fn.into());
        }

        // Bind error method
        if let Some(error_fn) = v8::Function::new(&mut ctx_scope, console_error_callback) {
            let key = v8::String::new(&mut ctx_scope, "error").unwrap();
            console.set(&mut ctx_scope, key.into(), error_fn.into());
        }

        // Attach console to global
        let console_key = v8::String::new(&mut ctx_scope, "console").unwrap();
        global.set(&mut ctx_scope, console_key.into(), console.into());
    }

    /// Bind TextEncoder API to global scope
    fn bind_text_encoder(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create TextEncoder constructor function
        let encoder_template = v8::FunctionTemplate::new(&mut ctx_scope, text_encoder_constructor);

        // Add encode method to prototype via instance template
        let instance_template = encoder_template.prototype_template(&mut &mut ctx_scope);
        let encode_fn = v8::FunctionTemplate::new(&mut ctx_scope, text_encoder_encode);
        let encode_key = v8::String::new(&mut ctx_scope, "encode").unwrap();
        instance_template.set(encode_key.into(), encode_fn.into());

        let encoder_ctor = encoder_template.get_function(&mut &mut ctx_scope).unwrap();

        // Attach TextEncoder to global
        let key = v8::String::new(&mut ctx_scope, "TextEncoder").unwrap();
        global.set(&mut ctx_scope, key.into(), encoder_ctor.into());
    }

    /// Bind TextDecoder API to global scope
    fn bind_text_decoder(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create TextDecoder constructor function
        let decoder_template = v8::FunctionTemplate::new(&mut ctx_scope, text_decoder_constructor);

        // Add decode method to prototype via instance template
        let instance_template = decoder_template.prototype_template(&mut &mut ctx_scope);
        let decode_fn = v8::FunctionTemplate::new(&mut ctx_scope, text_decoder_decode);
        let decode_key = v8::String::new(&mut ctx_scope, "decode").unwrap();
        instance_template.set(decode_key.into(), decode_fn.into());

        let decoder_ctor = decoder_template.get_function(&mut &mut ctx_scope).unwrap();

        // Attach TextDecoder to global
        let key = v8::String::new(&mut ctx_scope, "TextDecoder").unwrap();
        global.set(&mut ctx_scope, key.into(), decoder_ctor.into());
    }

    /// Bind crypto API with getRandomValues and subtle
    fn bind_crypto(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create crypto object
        let crypto = v8::Object::new(&mut &mut ctx_scope);

        // Bind getRandomValues
        if let Some(grv_fn) = v8::Function::new(&mut ctx_scope, crypto_get_random_values) {
            let key = v8::String::new(&mut ctx_scope, "getRandomValues").unwrap();
            crypto.set(&mut ctx_scope, key.into(), grv_fn.into());
        }

        // Bind subtle object with crypto.subtle methods
        let subtle = v8::Object::new(&mut &mut ctx_scope);
        
        // generateKey method
        if let Some(fn_gen) = v8::Function::new(&mut ctx_scope, subtle_generate_key) {
            let key = v8::String::new(&mut ctx_scope, "generateKey").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_gen.into());
        }
        
        // importKey method
        if let Some(fn_imp) = v8::Function::new(&mut ctx_scope, subtle_import_key) {
            let key = v8::String::new(&mut ctx_scope, "importKey").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_imp.into());
        }
        
        // exportKey method
        if let Some(fn_exp) = v8::Function::new(&mut ctx_scope, subtle_export_key) {
            let key = v8::String::new(&mut ctx_scope, "exportKey").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_exp.into());
        }
        
        // encrypt method
        if let Some(fn_enc) = v8::Function::new(&mut ctx_scope, subtle_encrypt) {
            let key = v8::String::new(&mut ctx_scope, "encrypt").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_enc.into());
        }
        
        // decrypt method
        if let Some(fn_dec) = v8::Function::new(&mut ctx_scope, subtle_decrypt) {
            let key = v8::String::new(&mut ctx_scope, "decrypt").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_dec.into());
        }
        
        // sign method
        if let Some(fn_sign) = v8::Function::new(&mut ctx_scope, subtle_sign) {
            let key = v8::String::new(&mut ctx_scope, "sign").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_sign.into());
        }
        
        // verify method
        if let Some(fn_verify) = v8::Function::new(&mut ctx_scope, subtle_verify) {
            let key = v8::String::new(&mut ctx_scope, "verify").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_verify.into());
        }
        
        // digest method
        if let Some(fn_digest) = v8::Function::new(&mut ctx_scope, subtle_digest) {
            let key = v8::String::new(&mut ctx_scope, "digest").unwrap();
            subtle.set(&mut ctx_scope, key.into(), fn_digest.into());
        }
        
        // Attach subtle to crypto
        let subtle_key = v8::String::new(&mut ctx_scope, "subtle").unwrap();
        crypto.set(&mut ctx_scope, subtle_key.into(), subtle.into());

        // Attach crypto to global
        let key = v8::String::new(&mut ctx_scope, "crypto").unwrap();
        global.set(&mut ctx_scope, key.into(), crypto.into());
    }

    /// Bind performance API with now()
    fn bind_performance(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Initialize baseline on first call
        PERFORMANCE_BASELINE.with(|cell| {
            if cell.get().is_none() {
                cell.set(Some(Instant::now()));
            }
        });

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create performance object
        let performance = v8::Object::new(&mut &mut ctx_scope);

        // Bind now() method
        if let Some(now_fn) = v8::Function::new(&mut ctx_scope, performance_now) {
            let key = v8::String::new(&mut ctx_scope, "now").unwrap();
            performance.set(&mut ctx_scope, key.into(), now_fn.into());
        }

        // Attach performance to global
        let key = v8::String::new(&mut ctx_scope, "performance").unwrap();
        global.set(&mut ctx_scope, key.into(), performance.into());
    }

    /// Bind structuredClone as global function
    fn bind_structured_clone(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        if let Some(clone_fn) = v8::Function::new(&mut ctx_scope, structured_clone) {
            let key = v8::String::new(&mut ctx_scope, "structuredClone").unwrap();
            global.set(&mut ctx_scope, key.into(), clone_fn.into());
        }
    }

    /// Bind DOMException constructor
    fn bind_dom_exception(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create DOMException constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, dom_exception_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "DOMException").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    /// Bind Blob constructor
    fn bind_blob(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create Blob constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, blob_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "Blob").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    /// Bind FormData constructor
    fn bind_form_data(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create FormData constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, form_data_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "FormData").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    /// Bind Response constructor for WinterTC compatibility
    fn bind_response(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        use crate::runtime::fetch::{response_text_callback, response_json_callback, response_arraybuffer_callback, response_json_static_callback};
        
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create Response constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, response_constructor);
        let ctor = template.get_function(&mut ctx_scope).unwrap();
        
        // Add prototype methods to Response (text, json, arrayBuffer)
        if let Some(ctor_obj) = ctor.to_object(&mut ctx_scope) {
            let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
            if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
                if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                    // Bind text() method
                    if let Some(text_fn) = v8::Function::new(&mut ctx_scope, response_text_callback) {
                        let text_key = v8::String::new(&mut ctx_scope, "text").unwrap();
                        proto_obj.set(&mut ctx_scope, text_key.into(), text_fn.into());
                    }
                    // Bind json() method
                    if let Some(json_fn) = v8::Function::new(&mut ctx_scope, response_json_callback) {
                        let json_key = v8::String::new(&mut ctx_scope, "json").unwrap();
                        proto_obj.set(&mut ctx_scope, json_key.into(), json_fn.into());
                    }
                    // Bind arrayBuffer() method
                    if let Some(ab_fn) = v8::Function::new(&mut ctx_scope, response_arraybuffer_callback) {
                        let ab_key = v8::String::new(&mut ctx_scope, "arrayBuffer").unwrap();
                        proto_obj.set(&mut ctx_scope, ab_key.into(), ab_fn.into());
                    }
                }
            }
            
            // Add static Response.json() method
            if let Some(json_static_fn) = v8::Function::new(&mut ctx_scope, response_json_static_callback) {
                let json_key = v8::String::new(&mut ctx_scope, "json").unwrap();
                ctor_obj.set(&mut ctx_scope, json_key.into(), json_static_fn.into());
            }
        }

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "Response").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    /// Bind URL constructor for WinterTC compatibility
    fn bind_url(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create URLSearchParams constructor first (needed by URL)
        let usp_template = v8::FunctionTemplate::new(&mut ctx_scope, url_search_params_constructor);
        let usp_ctor = usp_template.get_function(&mut ctx_scope).unwrap();
        
        // Add prototype methods to URLSearchParams
        if let Some(usp_obj) = usp_ctor.to_object(&mut ctx_scope) {
            let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
            if let Some(proto) = usp_obj.get(&mut ctx_scope, proto_key.into()) {
                if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                    // Bind get method
                    if let Some(get_fn) = v8::Function::new(&mut ctx_scope, usp_get_callback) {
                        let get_key = v8::String::new(&mut ctx_scope, "get").unwrap();
                        proto_obj.set(&mut ctx_scope, get_key.into(), get_fn.into());
                    }
                    // Bind set method
                    if let Some(set_fn) = v8::Function::new(&mut ctx_scope, usp_set_callback) {
                        let set_key = v8::String::new(&mut ctx_scope, "set").unwrap();
                        proto_obj.set(&mut ctx_scope, set_key.into(), set_fn.into());
                    }
                    // Bind has method
                    if let Some(has_fn) = v8::Function::new(&mut ctx_scope, usp_has_callback) {
                        let has_key = v8::String::new(&mut ctx_scope, "has").unwrap();
                        proto_obj.set(&mut ctx_scope, has_key.into(), has_fn.into());
                    }
                    // Bind delete method
                    if let Some(delete_fn) = v8::Function::new(&mut ctx_scope, usp_delete_callback) {
                        let delete_key = v8::String::new(&mut ctx_scope, "delete").unwrap();
                        proto_obj.set(&mut ctx_scope, delete_key.into(), delete_fn.into());
                    }
                    // Bind toString method
                    if let Some(tostring_fn) = v8::Function::new(&mut ctx_scope, usp_tostring_callback) {
                        let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                        proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                    }
                }
            }
        }
        
        // Attach URLSearchParams to global
        let usp_key = v8::String::new(&mut ctx_scope, "URLSearchParams").unwrap();
        global.set(&mut ctx_scope, usp_key.into(), usp_ctor.into());

        // Create URL constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, url_constructor);
        let ctor = template.get_function(&mut ctx_scope).unwrap();

        // Add toString method to URL prototype
        if let Some(ctor_obj) = ctor.to_object(&mut ctx_scope) {
            let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
            if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
                if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                    if let Some(tostring_fn) = v8::Function::new(&mut ctx_scope, url_tostring_callback) {
                        let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                        proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                    }
                    // Also add href getter property if not already set
                    if let Some(href_fn) = v8::Function::new(&mut ctx_scope, url_href_callback) {
                        let href_key = v8::String::new(&mut ctx_scope, "href").unwrap();
                        proto_obj.set(&mut ctx_scope, href_key.into(), href_fn.into());
                    }
                }
            }
        }

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "URL").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    /// Bind Headers constructor for WinterTC compatibility
    fn bind_headers(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create Headers constructor
        let template = v8::FunctionTemplate::new(&mut ctx_scope, headers_constructor);
        let ctor = template.get_function(&mut ctx_scope).unwrap();

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "Headers").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    fn bind_timers(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        super::timers::bind_timers(scope, context);
    }

    /// Bind WebAssembly API to context
    fn bind_wasm(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::wasm::WebAssemblyAPI::bind(scope, context);
        tracing::debug!("Bound WebAssembly API");
    }

    /// Bind Node.js Buffer API
    fn bind_buffer(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Create Buffer constructor function
        let buffer_template = v8::FunctionTemplate::new(&mut ctx_scope, buffer_constructor);
        let buffer_ctor = buffer_template.get_function(&mut ctx_scope).unwrap();

        // Attach static methods
        let from_key = v8::String::new(&mut ctx_scope, "from").unwrap();
        if let Some(from_fn) = v8::Function::new(&mut ctx_scope, buffer_from_callback) {
            buffer_ctor.set(&mut ctx_scope, from_key.into(), from_fn.into());
        }

        let alloc_key = v8::String::new(&mut ctx_scope, "alloc").unwrap();
        if let Some(alloc_fn) = v8::Function::new(&mut ctx_scope, buffer_alloc_callback) {
            buffer_ctor.set(&mut ctx_scope, alloc_key.into(), alloc_fn.into());
        }

        // Add toString method to Buffer prototype for Node.js compatibility
        if let Some(ctor_obj) = buffer_ctor.to_object(&mut ctx_scope) {
            let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
            if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
                if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                    if let Some(tostring_fn) = v8::Function::new(&mut ctx_scope, buffer_tostring_callback) {
                        let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                        proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                    }
                }
            }
        }

        // Attach to global
        let key = v8::String::new(&mut ctx_scope, "Buffer").unwrap();
        global.set(&mut ctx_scope, key.into(), buffer_ctor.into());
    }
}

/// Format console arguments into a single string
fn format_console_args(scope: &mut v8::PinnedRef<v8::HandleScope>, args: v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        if let Some(s) = arg.to_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    parts.join(" ")
}

/// V8 callback that blocks dynamic code generation via the Function constructor
/// Throws TypeError unconditionally. Replaces globalThis.Function in hardened contexts.
fn function_constructor_blocked(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let msg = v8::String::new(scope, "Function constructor is not allowed in this context").unwrap();
    let err = v8::Exception::type_error(scope, msg);
    scope.throw_exception(err);
    retval.set_undefined();
}

/// V8 callback for console.log
fn console_log_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::info!(target: "js_console", "{}", message);
}

/// V8 callback for console.warn
fn console_warn_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::warn!(target: "js_console", "{}", message);
}

/// V8 callback for console.error
fn console_error_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::error!(target: "js_console", "{}", message);
}

/// TextEncoder constructor callback
fn text_encoder_constructor(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Constructor - creates TextEncoder instance
    // No internal state needed for basic UTF-8 encoding
}

/// TextEncoder.encode() implementation
fn text_encoder_encode(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get first argument as string
    if args.length() == 0 {
        // Return empty Uint8Array
        let empty = v8::ArrayBuffer::new(scope, 0);
        if let Some(uint8array) = v8::Uint8Array::new(scope, empty, 0, 0) {
            retval.set(uint8array.into());
        }
        return;
    }

    let arg = args.get(0);
    let text = if let Some(s) = arg.to_string(scope) {
        s.to_rust_string_lossy(scope)
    } else {
        String::new()
    };

    // Encode to UTF-8 bytes
    let bytes = text.into_bytes();

    // Create ArrayBuffer and copy bytes
    let ab = v8::ArrayBuffer::new(scope, bytes.len());
    let store = ab.get_backing_store();

    // Copy bytes into ArrayBuffer
    for (i, byte) in bytes.iter().enumerate() {
        if let Some(cell) = store.get(i) {
            cell.set(*byte);
        }
    }

    // Create Uint8Array view
    if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
        retval.set(uint8array.into());
    } else {
        retval.set(ab.into());
    }
}

/// TextDecoder constructor callback
fn text_decoder_constructor(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Constructor - TextDecoder always uses UTF-8 in WinterTC
    // No internal state needed
}

/// TextDecoder.decode() implementation
fn text_decoder_decode(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get first argument (should be ArrayBuffer or Uint8Array)
    if args.length() == 0 {
        retval.set(v8::String::new(scope, "").unwrap().into());
        return;
    }

    let arg = args.get(0);

    // Try to extract bytes from Uint8Array
    let bytes = if arg.is_uint8_array() {
        let uint8array = arg.cast::<v8::Uint8Array>();
        let length = uint8array.byte_length();
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            if let Some(val) = uint8array.get_index(scope, i as u32) {
                if let Some(int) = val.to_integer(scope) {
                    vec.push(int.value() as u8);
                }
            }
        }
        vec
    } else if arg.is_array_buffer() {
        let arraybuffer = arg.cast::<v8::ArrayBuffer>();
        // Extract bytes from ArrayBuffer
        let store = arraybuffer.get_backing_store();
        let length = arraybuffer.byte_length();
        (0..length)
            .filter_map(|i| store.get(i).map(|cell| cell.get()))
            .collect()
    } else {
        Vec::new()
    };

    // Decode UTF-8 bytes to string (with replacement for invalid sequences)
    let text = String::from_utf8_lossy(&bytes);

    // Return as JS string
    if let Some(s) = v8::String::new(scope, &text) {
        retval.set(s.into());
    }
}

/// crypto.getRandomValues implementation
fn crypto_get_random_values(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get first argument (should be TypedArray)
    if args.length() < 1 {
        retval.set_undefined();
        return;
    }

    let arg = args.get(0);

    // Handle Uint8Array
    if let Some(uint8array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint8Array>().ok())
    {
        let length = uint8array.byte_length();

        if length == 0 {
            retval.set(arg);
            return;
        }

        // Generate random bytes using getrandom
        let mut buffer = vec![0u8; length];
        if getrandom::getrandom(&mut buffer).is_err() {
            retval.set_undefined();
            return;
        }

        // Copy bytes into the TypedArray
        for (i, byte) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *byte as f64);
            uint8array.set(scope, idx.into(), val.into());
        }

        retval.set(arg);
        return;
    }

    // Handle Uint16Array
    if let Some(uint16array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint16Array>().ok())
    {
        let length = uint16array.byte_length() / 2;

        if length == 0 {
            retval.set(arg);
            return;
        }

        let mut buffer = vec![0u16; length];
        let byte_buffer = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 2)
        };

        if getrandom::getrandom(byte_buffer).is_err() {
            retval.set_undefined();
            return;
        }

        for (i, value) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *value as f64);
            uint16array.set(scope, idx.into(), val.into());
        }

        retval.set(arg);
        return;
    }

    // Handle Uint32Array
    if let Some(uint32array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint32Array>().ok())
    {
        let length = uint32array.byte_length() / 4;

        if length == 0 {
            retval.set(arg);
            return;
        }

        let mut buffer = vec![0u32; length];
        let byte_buffer = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 4)
        };

        if getrandom::getrandom(byte_buffer).is_err() {
            retval.set_undefined();
            return;
        }

        for (i, value) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *value as f64);
            uint32array.set(scope, idx.into(), val.into());
        }

        retval.set(arg);
        return;
    }

    // If not a supported TypedArray, return undefined
    retval.set_undefined();
}

/// performance.now() implementation
fn performance_now(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let now = Instant::now();

    let elapsed_ms = PERFORMANCE_BASELINE.with(|baseline| {
        if let Some(base) = baseline.get() {
            now.duration_since(base).as_nanos() as f64 / 1_000_000.0
        } else {
            0.0
        }
    });

    retval.set(v8::Number::new(scope, elapsed_ms).into());
}

/// structuredClone() implementation
fn structured_clone(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 1 {
        retval.set_undefined();
        return;
    }

    let value = args.get(0);

    // Use V8's built-in cloning via JSON serialization as a baseline
    // Convert to JSON string then parse back
    if let Some(json_string) = v8::json::stringify(scope, value) {
        if let Some(json_str) = json_string.to_string(scope) {
            // Parse the JSON back into a value
            if let Some(cloned) = v8::json::parse(scope, json_str.into()) {
                retval.set(cloned);
                return;
            }
        }
    }

    // Fallback: return the original value
    retval.set(value);
}

/// DOMException constructor implementation
fn dom_exception_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Get message argument (defaults to "")
    let message = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Get name argument (defaults to "Error")
    let name = if args.length() > 1 {
        args.get(1)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "Error".to_string())
    } else {
        "Error".to_string()
    };

    // Set message property
    let msg_key = v8::String::new(scope, "message").unwrap();
    let msg_val = v8::String::new(scope, &message).unwrap();
    this.set(scope, msg_key.into(), msg_val.into());

    // Set name property
    let name_key = v8::String::new(scope, "name").unwrap();
    let name_val = v8::String::new(scope, &name).unwrap();
    this.set(scope, name_key.into(), name_val.into());

    // Set stack property (simplified for v1)
    let stack_key = v8::String::new(scope, "stack").unwrap();
    let stack_str = format!("DOMException: {}", message);
    let stack_val = v8::String::new(scope, &stack_str).unwrap();
    this.set(scope, stack_key.into(), stack_val.into());

    retval.set(this.into());
}

/// Blob constructor implementation (simplified v1)
fn blob_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Get parts array (first argument, defaults to empty)
    let mut total_size: usize = 0;
    let mut parts: Vec<String> = Vec::new();

    if args.length() > 0 {
        let arg = args.get(0);
        if let Some(array) = arg.to_object(scope) {
            // Try to iterate over the array
            if let Some(length_key) = v8::String::new(scope, "length") {
                if let Some(length_val) = array.get(scope, length_key.into()) {
                    if let Some(length_num) = length_val.to_number(scope) {
                        let length = length_num.value() as usize;

                        for i in 0..length {
                            let idx = v8::Number::new(scope, i as f64);
                            if let Some(item) = array.get(scope, idx.into()) {
                                // Convert item to string
                                if let Some(item_str) = item.to_string(scope) {
                                    let item_rust = item_str.to_rust_string_lossy(scope);
                                    total_size += item_rust.len();
                                    parts.push(item_rust);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Get type option (second argument with { type: "..." })
    let mut blob_type = String::new();
    if args.length() > 1 {
        let options = args.get(1);
        if let Some(options_obj) = options.to_object(scope) {
            if let Some(type_key) = v8::String::new(scope, "type") {
                if let Some(type_val) = options_obj.get(scope, type_key.into()) {
                    if let Some(type_str) = type_val.to_string(scope) {
                        blob_type = type_str.to_rust_string_lossy(scope);
                    }
                }
            }
        }
    }

    // Store size property
    let size_key = v8::String::new(scope, "size").unwrap();
    let size_val = v8::Number::new(scope, total_size as f64);
    this.set(scope, size_key.into(), size_val.into());

    // Store type property
    let type_key = v8::String::new(scope, "type").unwrap();
    let type_val = v8::String::new(scope, &blob_type).unwrap();
    this.set(scope, type_key.into(), type_val.into());

    // Store parts in internal field (using a unique symbol approach)
    // For v1, we store as a hidden property
    let parts_key = v8::String::new(scope, "__blob_parts__").unwrap();
    let parts_val = v8::String::new(scope, &parts.join("")).unwrap();
    this.set(scope, parts_key.into(), parts_val.into());

    retval.set(this.into());
}

/// FormData constructor implementation (simplified v1)
fn form_data_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Initialize internal data store as a JSON-serializable string for v1
    // In a full implementation, we'd use V8's private properties
    let data_key = v8::String::new(scope, "__form_data__").unwrap();
    let data_val = v8::String::new(scope, "{}").unwrap();
    this.set(scope, data_key.into(), data_val.into());

    retval.set(this.into());
}

/// Response constructor implementation for WinterTC compatibility
fn response_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Get body argument (first argument - string or null)
    let mut body_string = String::new();
    if args.length() > 0 {
        let arg = args.get(0);
        if !arg.is_null() && !arg.is_undefined() {
            if let Some(s) = arg.to_string(scope) {
                body_string = s.to_rust_string_lossy(scope);
            }
        }
    }

    // Get options argument (second argument - { status, headers })
    let mut status = 200;
    let mut headers_obj: Option<v8::Local<v8::Object>> = None;

    if args.length() > 1 {
        let options = args.get(1);
        if let Some(opts) = options.to_object(scope) {
            // Extract status
            let status_key = v8::String::new(scope, "status").unwrap();
            if let Some(status_val) = opts.get(scope, status_key.into()) {
                // Only update status if the value is a valid number (not undefined/null/NaN)
                if !status_val.is_null() && !status_val.is_undefined() {
                    if let Some(num) = status_val.to_number(scope) {
                        let val = num.value();
                        if !val.is_nan() && val > 0.0 {
                            status = val as u16;
                        }
                    }
                }
            }

            // Extract headers
            let headers_key = v8::String::new(scope, "headers").unwrap();
            headers_obj = opts.get(scope, headers_key.into()).and_then(|h| h.to_object(scope));
        }
    }

    // Set status property
    let status_key = v8::String::new(scope, "status").unwrap();
    let status_val = v8::Number::new(scope, status as f64);
    this.set(scope, status_key.into(), status_val.into());

    // Create headers object
    let headers = v8::Object::new(scope);
    
    // Initialize internal headers store for set/get methods
    let internal_headers_key = v8::String::new(scope, "__headers__").unwrap();
    let internal_headers = v8::Object::new(scope);
    headers.set(scope, internal_headers_key.into(), internal_headers.into());
    
    if let Some(hdrs) = headers_obj {
        // Copy headers from options
        if let Some(names) = hdrs.get_own_property_names(scope, Default::default()) {
            let len = names.length();
            for i in 0..len {
                if let Some(key) = names.get_index(scope, i) {
                    if let Some(key_str) = key.to_string(scope) {
                        let key_name = key_str.to_rust_string_lossy(scope);
                        if let Some(value) = hdrs.get(scope, key.into()) {
                            if let Some(value_str) = value.to_string(scope) {
                                let value_string = value_str.to_rust_string_lossy(scope);
                                let hkey = v8::String::new(scope, &key_name).unwrap();
                                let hval = v8::String::new(scope, &value_string).unwrap();
                                headers.set(scope, hkey.into(), hval.into());
                                // Also store in internal headers for extraction
                                internal_headers.set(scope, hkey.into(), hval.into());
                            }
                        }
                    }
                }
            }
        }
    }

    // Set headers property
    let headers_key = v8::String::new(scope, "headers").unwrap();
    this.set(scope, headers_key.into(), headers.into());

    // Set body property
    let body_key = v8::String::new(scope, "body").unwrap();
    let body_val = v8::String::new(scope, &body_string).unwrap();
    this.set(scope, body_key.into(), body_val.into());

    // Add headers.set method for CORS middleware support
    let set_key = v8::String::new(scope, "set").unwrap();
    if let Some(set_fn) = v8::Function::new(scope, headers_set_callback) {
        headers.set(scope, set_key.into(), set_fn.into());
    }

    retval.set(this.into());
}

/// Callback for headers.set() method
fn headers_set_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Get the headers object (this)
    let this = args.this();

    // Get header name and value
    if args.length() >= 2 {
        let name = args.get(0).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();
        let value = args.get(1).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();

        // Store in __headers__ internal object (same as headers_get_callback uses)
        let headers_key = v8::String::new(scope, "__headers__").unwrap();
        if let Some(headers_val) = this.get(scope, headers_key.into()) {
            if let Some(headers_obj) = headers_val.to_object(scope) {
                let key = v8::String::new(scope, &name).unwrap();
                let val = v8::String::new(scope, &value).unwrap();
                headers_obj.set(scope, key.into(), val.into());
            }
        }
    }
}

/// URL constructor implementation (simplified v1)
fn url_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let global = scope.get_current_context().global(scope);

    // Get the URL string argument
    let url_string = if args.length() > 0 {
        args.get(0).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Parse the URL to extract components
    let parsed = url::Url::parse(&url_string).unwrap_or_else(|_| {
        url::Url::parse("http://localhost/").unwrap()
    });

    // Set href property (full URL)
    let href_key = v8::String::new(scope, "href").unwrap();
    let href_val = v8::String::new(scope, parsed.as_str()).unwrap();
    this.set(scope, href_key.into(), href_val.into());

    // Set protocol property
    let protocol_key = v8::String::new(scope, "protocol").unwrap();
    let protocol = format!("{}:", parsed.scheme());
    let protocol_val = v8::String::new(scope, &protocol).unwrap();
    this.set(scope, protocol_key.into(), protocol_val.into());

    // Set host property (hostname:port)
    let host_key = v8::String::new(scope, "host").unwrap();
    let host = if let Some(port) = parsed.port() {
        format!("{}:{}", parsed.host_str().unwrap_or(""), port)
    } else {
        parsed.host_str().unwrap_or("").to_string()
    };
    let host_val = v8::String::new(scope, &host).unwrap();
    this.set(scope, host_key.into(), host_val.into());

    // Set hostname property
    let hostname_key = v8::String::new(scope, "hostname").unwrap();
    let hostname = parsed.host_str().unwrap_or("");
    let hostname_val = v8::String::new(scope, hostname).unwrap();
    this.set(scope, hostname_key.into(), hostname_val.into());

    // Set port property
    let port_key = v8::String::new(scope, "port").unwrap();
    let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
    let port_val = v8::String::new(scope, &port).unwrap();
    this.set(scope, port_key.into(), port_val.into());

    // Set pathname property
    let pathname_key = v8::String::new(scope, "pathname").unwrap();
    let pathname = parsed.path();
    let pathname_val = v8::String::new(scope, pathname).unwrap();
    this.set(scope, pathname_key.into(), pathname_val.into());

    // Set search property (query string with ?)
    let search_key = v8::String::new(scope, "search").unwrap();
    let search = if parsed.query().is_some() {
        format!("?{}", parsed.query().unwrap_or(""))
    } else {
        String::new()
    };
    let search_val = v8::String::new(scope, &search).unwrap();
    this.set(scope, search_key.into(), search_val.into());

    // Set hash property (fragment with #)
    let hash_key = v8::String::new(scope, "hash").unwrap();
    let hash = if let Some(fragment) = parsed.fragment() {
        format!("#{}", fragment)
    } else {
        String::new()
    };
    let hash_val = v8::String::new(scope, &hash).unwrap();
    this.set(scope, hash_key.into(), hash_val.into());

    // Set searchParams property with URLSearchParams instance
    let search_params_key = v8::String::new(scope, "searchParams").unwrap();
    let search_params_ctor_key = v8::String::new(scope, "URLSearchParams").unwrap();
    if let Some(usp_ctor) = global.get(scope, search_params_ctor_key.into()) {
        if usp_ctor.is_function() {
            let usp_fn = usp_ctor.cast::<v8::Function>();
            // Pass the query string (without ?) to URLSearchParams constructor
            let query_str = parsed.query().unwrap_or("");
            let query_val = v8::String::new(scope, query_str).unwrap();
            if let Some(search_params) = usp_fn.new_instance(scope, &[query_val.into()]) {
                this.set(scope, search_params_key.into(), search_params.into());
            }
        }
    }

    retval.set(this.into());
}

/// URL.prototype.toString() callback - returns the href property
fn url_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = _args.this();

    // Get the href property from this URL object
    let href_key = v8::String::new(scope, "href").unwrap();
    if let Some(href_val) = this.get(scope, href_key.into()) {
        if let Some(href_str) = href_val.to_string(scope) {
            retval.set(href_str.into());
            return;
        }
    }

    // Fallback: return empty string
    retval.set(v8::String::new(scope, "").unwrap().into());
}

/// URL.prototype.href getter callback
fn url_href_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = _args.this();

    // Get the href property from this URL object
    let href_key = v8::String::new(scope, "href").unwrap();
    if let Some(href_val) = this.get(scope, href_key.into()) {
        retval.set(href_val);
        return;
    }

    // Fallback: return empty string
    retval.set(v8::String::new(scope, "").unwrap().into());
}

/// URLSearchParams constructor implementation
fn url_search_params_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Initialize internal params store as a plain Object (like Headers does)
    let params_key = v8::String::new(scope, "__params__").unwrap();
    let params_obj = v8::Object::new(scope);
    this.set(scope, params_key.into(), params_obj.into());

    // Parse init argument if provided
    if args.length() > 0 {
        let init = args.get(0);
        if let Some(init_str) = init.to_string(scope) {
            let query_string = init_str.to_rust_string_lossy(scope);
            // Parse query string like "foo=bar&baz=qux"
            for pair in query_string.split('&') {
                if let Some(eq_pos) = pair.find('=') {
                    let key = &pair[..eq_pos];
                    let value = &pair[eq_pos + 1..];
                    let key_val = v8::String::new(scope, key).unwrap();
                    let value_val = v8::String::new(scope, value).unwrap();
                    params_obj.set(scope, key_val.into(), value_val.into());
                } else if !pair.is_empty() {
                    let key_val = v8::String::new(scope, pair).unwrap();
                    let empty_val = v8::String::new(scope, "").unwrap();
                    params_obj.set(scope, key_val.into(), empty_val.into());
                }
            }
        }
    }

    retval.set(this.into());
}

/// URLSearchParams.get() callback
fn usp_get_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set_null();
        return;
    }

    let name = args.get(0).to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    
    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            if let Some(value) = params_obj.get(scope, name_key.into()) {
                if !value.is_null() && !value.is_undefined() {
                    retval.set(value);
                    return;
                }
            }
        }
    }

    retval.set_null();
}

/// URLSearchParams.set() callback
fn usp_set_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 2 {
        retval.set_undefined();
        return;
    }

    let name = args.get(0).to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let value = args.get(1).to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    
    // Create string keys for consistent lookup
    let name_key = v8::String::new(scope, &name).unwrap();
    let value_key = v8::String::new(scope, &value).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            params_obj.set(scope, name_key.into(), value_key.into());
        }
    }

    retval.set_undefined();
}

/// URLSearchParams.has() callback
fn usp_has_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set(v8::Boolean::new(scope, false).into());
        return;
    }

    let name = args.get(0).to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    
    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            // Check if key exists directly in the object
            if let Some(val) = params_obj.get(scope, name_key.into()) {
                if !val.is_null() && !val.is_undefined() {
                    retval.set(v8::Boolean::new(scope, true).into());
                    return;
                }
            }
        }
    }

    retval.set(v8::Boolean::new(scope, false).into());
}

/// URLSearchParams.delete() callback
fn usp_delete_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set_undefined();
        return;
    }

    let name = args.get(0).to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    
    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            // Delete directly from the object
            let _ = params_obj.delete(scope, name_key.into());
        }
    }

    retval.set_undefined();
}

/// URLSearchParams.toString() callback
fn usp_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = _args.this();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    let entries_key = v8::String::new(scope, "entries").unwrap();
    
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_map) = params_val.to_object(scope) {
            // Get entries from the Map
            if let Some(entries_fn) = params_map.get(scope, entries_key.into()) {
                if entries_fn.is_function() {
                    let entries_func = entries_fn.cast::<v8::Function>();
                    if let Some(_iterator) = entries_func.call(scope, params_val, &[]) {
                        // Note: Full implementation would iterate the iterator
                        // and build query string from entries
                        // For now, return empty string as basic implementation
                        let result = v8::String::new(scope, "").unwrap();
                        retval.set(result.into());
                        return;
                    }
                }
            }
        }
    }

    let empty = v8::String::new(scope, "").unwrap();
    retval.set(empty.into());
}

/// Headers constructor implementation (simplified v1)
fn headers_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Initialize internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    let headers_val = v8::Object::new(scope);
    this.set(scope, headers_key.into(), headers_val.into());

    // If an initial headers object is provided, copy its values
    if args.length() > 0 {
        let init = args.get(0);
        if let Some(init_obj) = init.to_object(scope) {
            // Try to iterate over the object
            if let Some(names) = init_obj.get_own_property_names(scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            // Normalize header name to lowercase (per Fetch spec)
                            let key_name = key_str.to_rust_string_lossy(scope).to_lowercase();
                            if let Some(value) = init_obj.get(scope, key.into()) {
                                if let Some(value_str) = value.to_string(scope) {
                                    let value_string = value_str.to_rust_string_lossy(scope);
                                    let hkey = v8::String::new(scope, &key_name).unwrap();
                                    let hval = v8::String::new(scope, &value_string).unwrap();
                                    headers_val.set(scope, hkey.into(), hval.into());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Add get method
    let get_key = v8::String::new(scope, "get").unwrap();
    if let Some(get_fn) = v8::Function::new(scope, headers_get_callback) {
        this.set(scope, get_key.into(), get_fn.into());
    }

    // Add set method
    let set_key = v8::String::new(scope, "set").unwrap();
    if let Some(set_fn) = v8::Function::new(scope, headers_set_callback_v2) {
        this.set(scope, set_key.into(), set_fn.into());
    }

    // Add forEach method
    let foreach_key = v8::String::new(scope, "forEach").unwrap();
    if let Some(foreach_fn) = v8::Function::new(scope, headers_foreach_callback) {
        this.set(scope, foreach_key.into(), foreach_fn.into());
    }

    retval.set(this.into());
}

/// Callback for Headers.get() method
fn headers_get_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Get the header name and normalize to lowercase (per Fetch spec)
    let name = if args.length() > 0 {
        args.get(0).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Get the internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            let name_key = v8::String::new(scope, &name).unwrap();
            if let Some(value) = headers_obj.get(scope, name_key.into()) {
                if !value.is_null() && !value.is_undefined() {
                    retval.set(value);
                    return;
                }
            }
        }
    }

    // Return null if not found
    retval.set_null();
}

/// Callback for Headers.set() method (version for Headers object)
fn headers_set_callback_v2(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() >= 2 {
        // Normalize header name to lowercase (per Fetch spec)
        let name = args.get(0).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default();
        let value = args.get(1).to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();

        // Get the internal headers store
        let headers_key = v8::String::new(scope, "__headers__").unwrap();
        if let Some(headers_val) = this.get(scope, headers_key.into()) {
            if let Some(headers_obj) = headers_val.to_object(scope) {
                let name_key = v8::String::new(scope, &name).unwrap();
                let val_str = v8::String::new(scope, &value).unwrap();
                headers_obj.set(scope, name_key.into(), val_str.into());
            }
        }
    }
}

/// Callback for Headers.forEach() method
fn headers_foreach_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        return;
    }

    let callback = args.get(0);
    if !callback.is_function() {
        return;
    }
    let callback_fn = callback.cast::<v8::Function>();

    // Get the internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            // Iterate over all properties
            if let Some(names) = headers_obj.get_own_property_names(scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            let key_name = key_str.to_rust_string_lossy(scope);
                            if let Some(value) = headers_obj.get(scope, key.into()) {
                                // Call the callback with (value, key, headers)
                                let key_js = v8::String::new(scope, &key_name).unwrap();
                                let _ = callback_fn.call(scope, this.into(), &[value, key_js.into(), this.into()]);
                            }
                        }
                    }
                }
            }
        }
    }
}


/// Timer callback for setTimeout
///
/// Registers the callback in the PENDING_TIMEOUTS thread-local and returns
/// immediately. The callback fires when the pump loop in pool.rs calls
/// `fire_pending_timeouts()` and the deadline has passed.
///
/// Never blocks — avoids the CPU timeout guard firing during a blocking sleep,
/// which caused HTTP 500 for any delay ≥ the CPU limit (typically 50–100ms).
/// Buffer constructor callback
fn buffer_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get size argument
    let size = if args.length() > 0 {
        let arg = args.get(0);
        if let Some(num) = arg.to_number(scope) {
            num.value() as usize
        } else if let Some(str) = arg.to_string(scope) {
            str.to_rust_string_lossy(scope).len()
        } else {
            0
        }
    } else {
        0
    };

    // Create Uint8Array as buffer backing
    let buffer = v8::ArrayBuffer::new(scope, size);
    let uint8_array = v8::Uint8Array::new(scope, buffer, 0, size).unwrap();

    // Add toString method for Buffer compatibility
    add_buffer_tostring_to_instance(scope, uint8_array.into());

    retval.set(uint8_array.into());
}

/// Helper to add toString method to a Uint8Array instance for Buffer compatibility
fn add_buffer_tostring_to_instance(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    obj: v8::Local<v8::Object>,
) {
    // Create the toString function
    if let Some(tostring_fn) = v8::Function::new(scope, buffer_tostring_callback) {
        let tostring_key = v8::String::new(scope, "toString").unwrap();
        // Set as own property (not prototype) to override Uint8Array's toString
        let _ = obj.set(scope, tostring_key.into(), tostring_fn.into());
    }
}

/// Buffer.from() static method
fn buffer_from_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() == 0 {
        let empty = v8::ArrayBuffer::new(scope, 0);
        let arr = v8::Uint8Array::new(scope, empty, 0, 0).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    let arg = args.get(0);

    // Handle ArrayBuffer input
    if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(arg) {
        let store = ab.get_backing_store();
        let len = store.len();
        let out = v8::ArrayBuffer::new(scope, len);
        let out_store = out.get_backing_store();
        for i in 0..len {
            if let (Some(src), Some(dst)) = (store.get(i), out_store.get(i)) {
                dst.set(src.get());
            }
        }
        let arr = v8::Uint8Array::new(scope, out, 0, len).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    // Handle ArrayBufferView (Uint8Array, etc.) input
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(arg) {
        let len = view.byte_length();
        let buffer = v8::ArrayBuffer::new(scope, len);
        let store = buffer.get_backing_store();
        let mut tmp = vec![0u8; len];
        view.copy_contents(&mut tmp);
        for (i, byte) in tmp.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }
        let arr = v8::Uint8Array::new(scope, buffer, 0, len).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    // Handle array-like input (check BEFORE string coercion — to_string() coerces arrays)
    if arg.is_array() {
        if let Some(obj) = arg.to_object(scope) {
            let len_key = v8::String::new(scope, "length").unwrap();
            if let Some(len_val) = obj.get(scope, len_key.into()) {
                if let Some(len_num) = len_val.to_number(scope) {
                    let len = len_num.value() as usize;
                    let buffer = v8::ArrayBuffer::new(scope, len);
                    let store = buffer.get_backing_store();
                    for i in 0..len {
                        let idx = v8::Number::new(scope, i as f64);
                        if let Some(val) = obj.get(scope, idx.into()) {
                            if let Some(num) = val.to_number(scope) {
                                if let Some(cell) = store.get(i) {
                                    cell.set(num.value() as u8);
                                }
                            }
                        }
                    }
                    let arr = v8::Uint8Array::new(scope, buffer, 0, len).unwrap();
                    add_buffer_tostring_to_instance(scope, arr.into());
                    retval.set(arr.into());
                    return;
                }
            }
        }
    }

    // Handle string input (after array check — to_string() would coerce arrays)
    if arg.is_string() {
        if let Some(str_val) = arg.to_string(scope) {
            let text = str_val.to_rust_string_lossy(scope);

            // Check encoding argument (args[1]).
            let encoding = if args.length() > 1 {
                if let Some(enc) = args.get(1).to_string(scope) {
                    enc.to_rust_string_lossy(scope).to_ascii_lowercase()
                } else {
                    "utf8".to_string()
                }
            } else {
                "utf8".to_string()
            };

            let bytes: Vec<u8> = match encoding.as_str() {
                "hex" => {
                    // Decode hex pairs: "68656c6c6f" → [0x68, 0x65, …]
                    // Odd-length or invalid chars produce truncated/best-effort output
                    // (matches Node.js behaviour).
                    (0..text.len())
                        .step_by(2)
                        .filter_map(|i| {
                            text.get(i..i + 2)
                                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                        })
                        .collect()
                }
                // "utf8" | "utf-8" | "ascii" | "latin1" | "binary" | anything else
                _ => text.as_bytes().to_vec(),
            };

            let buffer = v8::ArrayBuffer::new(scope, bytes.len());
            let store = buffer.get_backing_store();
            for (i, byte) in bytes.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }
            let arr = v8::Uint8Array::new(scope, buffer, 0, bytes.len()).unwrap();
            add_buffer_tostring_to_instance(scope, arr.into());
            retval.set(arr.into());
            return;
        }
    }

    // Default: return empty buffer
    let empty = v8::ArrayBuffer::new(scope, 0);
    let arr = v8::Uint8Array::new(scope, empty, 0, 0).unwrap();
    add_buffer_tostring_to_instance(scope, arr.into());
    retval.set(arr.into());
}

/// Buffer.alloc() static method
fn buffer_alloc_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let size = if args.length() > 0 {
        let arg = args.get(0);
        if let Some(num) = arg.to_number(scope) {
            num.value() as usize
        } else {
            0
        }
    } else {
        0
    };

    let fill_value = if args.length() > 1 {
        let arg = args.get(1);
        if let Some(num) = arg.to_number(scope) {
            num.value() as u8
        } else {
            0
        }
    } else {
        0
    };

    let buffer = v8::ArrayBuffer::new(scope, size);
    if fill_value != 0 {
        let store = buffer.get_backing_store();
        for i in 0..size {
            if let Some(cell) = store.get(i) {
                cell.set(fill_value);
            }
        }
    }
    let arr = v8::Uint8Array::new(scope, buffer, 0, size).unwrap();
    add_buffer_tostring_to_instance(scope, arr.into());
    retval.set(arr.into());
}

/// Buffer.prototype.toString() callback - decodes buffer to UTF-8 string
fn buffer_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Extract bytes from the Uint8Array (which is what Buffer is)
    let bytes = if let Some(uint8array) = this
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint8Array>().ok())
    {
        let length = uint8array.byte_length();
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            if let Some(val) = uint8array.get_index(scope, i as u32) {
                if let Some(int) = val.to_integer(scope) {
                    vec.push(int.value() as u8);
                }
            }
        }
        vec
    } else {
        // Fallback: return empty string
        retval.set(v8::String::new(scope, "").unwrap().into());
        return;
    };

    // Decode bytes to UTF-8 string
    let text = String::from_utf8_lossy(&bytes);

    // Return the decoded string
    if let Some(s) = v8::String::new(scope, &text) {
        retval.set(s.into());
    } else {
        retval.set(v8::String::new(scope, "").unwrap().into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::{initialize_platform, NanoIsolate};
    use crate::runtime::timers::pending_timeout_count;

    fn init_platform() {
        initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_text_encoder_basic() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test basic encoding
        let code = r#"
            const encoder = new TextEncoder();
            const text = "Hello, World!";
            const encoded = encoder.encode(text);
            encoded.length === 13 && encoded[0] === 72;
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(
            result_str, "true",
            "TextEncoder should encode 'Hello, World!' correctly"
        );
    }

    #[test]
    fn test_text_encoder_utf8() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test emoji encoding: "🎉" should produce [240, 159, 142, 137]
        let code = r#"
            const encoder = new TextEncoder();
            const bytes = encoder.encode("🎉");
            bytes.length;
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        // Emoji should be 4 bytes in UTF-8
        assert_eq!(result_str, "4");
    }

    #[test]
    fn test_text_decoder_basic() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test basic decoding
        let code = r#"
            const encoder = new TextEncoder();
            const decoder = new TextDecoder();
            const original = "Hello, UTF-8! 🎉";
            const bytes = encoder.encode(original);
            const decoded = decoder.decode(bytes);
            decoded === original ? "PASS" : "FAIL: " + decoded;
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert!(
            result_str.starts_with("PASS"),
            "Roundtrip failed: {}",
            result_str
        );
    }

    #[test]
    fn test_console_exists() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that console object exists and has log/warn/error methods
        let code = r#"
            typeof console === "object" &&
            typeof console.log === "function" &&
            typeof console.warn === "function" &&
            typeof console.error === "function"
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(result_str, "true");
    }

    #[test]
    fn test_console_log_no_crash() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that console.log doesn't crash
        let code = r#"console.log("test message"); "OK";"#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(result_str, "OK");
    }

    #[test]
    fn test_text_decoder_invalid_utf8() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that invalid UTF-8 produces replacement character
        let code = r#"
            const decoder = new TextDecoder();
            // 0xFF is invalid in UTF-8
            const bytes = new Uint8Array([0xFF, 0xFE]);
            decoder.decode(bytes);
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert!(
            result_str.contains("\u{FFFD}"),
            "Invalid UTF-8 should produce replacement character, got: {:?}", result_str
        );
    }

    #[test]
    fn test_crypto_get_random_values() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that we can call getRandomValues
        let code = r#"
            const arr = new Uint8Array(8);
            const result = crypto.getRandomValues(arr);
            result.length === 8 && result === arr
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(
            result_str, "true",
            "crypto.getRandomValues should return the same array"
        );
    }

    #[test]
    fn test_performance_now() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that performance.now() returns a number >= 0
        let code = r#"
            const t1 = performance.now();
            const t2 = performance.now();
            typeof t1 === 'number' && t1 >= 0 && t2 >= t1
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(
            result_str, "true",
            "performance.now() should return monotonic increasing numbers"
        );
    }

    #[test]
    fn test_structured_clone() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test that structuredClone creates independent copies
        let code = r#"
            const original = { a: 1, b: [2, 3] };
            const cloned = structuredClone(original);
            cloned.a = 999;
            original.a === 1 && cloned.a === 999
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(
            result_str, "true",
            "structuredClone should create independent copies"
        );
    }

    #[test]
    fn test_dom_exception() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test DOMException constructor
        let code = r#"
            const err = new DOMException("Something went wrong", "AbortError");
            err.name === "AbortError" && err.message === "Something went wrong"
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(
            result_str, "true",
            "DOMException should have correct name and message"
        );
    }

    #[test]
    fn test_blob() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test Blob constructor
        let code = r#"
            const blob = new Blob(["test content"]);
            blob.size === 12 && blob.type === ""
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(result_str, "true", "Blob should have correct size");
    }

    #[test]
    fn test_fire_pending_timeouts_requeues_on_v8_termination() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        // SAFETY: iso_ptr is valid for the duration of this test (isolate lives on the stack).
        let iso_ptr: *mut v8::Isolate = &mut **isolate.isolate();

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        RuntimeAPIs::bind_all(ctx_scope, context);

        let src = v8::String::new(ctx_scope, "setTimeout(() => {}, 0)").unwrap();
        let script = v8::Script::compile(ctx_scope, src, None).unwrap();
        script.run(ctx_scope).unwrap();

        assert_eq!(
            pending_timeout_count(),
            1,
            "one timeout must be pending before fire"
        );

        unsafe { (*iso_ptr).terminate_execution(); }

        let tc_storage = v8::TryCatch::new(&mut *ctx_scope);
        let tc_pin = std::pin::pin!(tc_storage);
        let mut tc = tc_pin.init();

        fire_pending_timeouts(&mut *tc);

        unsafe { (*iso_ptr).cancel_terminate_execution(); }

        assert_eq!(
            pending_timeout_count(),
            1,
            "terminated timeout entry must be re-queued for the next pump iteration"
        );

        clear_pending_timeouts();
    }

    #[test]
    fn test_form_data() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        // Bind APIs
        RuntimeAPIs::bind_all(ctx_scope, context);

        // Test FormData constructor exists
        let code = r#"
            typeof FormData === 'function'
        "#;

        let code_string = v8::String::new(ctx_scope, code).unwrap();
        let script =
            v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

        let result = script.run(ctx_scope).expect("Script execution failed");
        let result_str = result.to_string(ctx_scope).unwrap().to_rust_string_lossy(ctx_scope);

        assert_eq!(result_str, "true", "FormData should be a function");
    }
}
