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

use crate::runtime::subtle_v8::{
    subtle_generate_key, subtle_import_key, subtle_export_key,
    subtle_encrypt, subtle_decrypt,
    subtle_sign, subtle_verify, subtle_digest,
};

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
        Self::bind_kv(scope, context);
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
        if let Some(blocked_fn) = v8::Function::new(&mut ctx_scope, crate::runtime::console_api::function_constructor_blocked) {
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

    fn bind_streams(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::stream::bind_streams(scope, context);
    }

    fn bind_websocket_pair(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::websocket::bind_websocket_pair(scope, context);
    }

    fn bind_request(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::request::bind_request_api(scope, context);
    }

    fn bind_nano_fs(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::vfs_bindings::bind_nano_fs(scope, context);
    }

    fn bind_fs_polyfill(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::fs_polyfill::bind_fs_polyfill(scope, context);
    }

    fn bind_kv(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::kv::bind_kv(scope, context);
    }

    fn bind_fetch(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::fetch::bind_fetch(scope, context);
    }

    fn bind_console(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::console_api::bind_console(scope, context);
    }

    fn bind_text_encoder(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::text_codec_api::bind_text_encoder(scope, context);
    }

    fn bind_text_decoder(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::text_codec_api::bind_text_decoder(scope, context);
    }

    fn bind_crypto(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let crypto = v8::Object::new(&mut &mut ctx_scope);
        if let Some(f) = v8::Function::new(&mut ctx_scope, crate::runtime::web_apis::crypto_get_random_values) {
            let k = v8::String::new(&mut ctx_scope, "getRandomValues").unwrap();
            crypto.set(&mut ctx_scope, k.into(), f.into());
        }
        let subtle = v8::Object::new(&mut &mut ctx_scope);
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_generate_key) {
            let k = v8::String::new(&mut ctx_scope, "generateKey").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_import_key) {
            let k = v8::String::new(&mut ctx_scope, "importKey").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_export_key) {
            let k = v8::String::new(&mut ctx_scope, "exportKey").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_encrypt) {
            let k = v8::String::new(&mut ctx_scope, "encrypt").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_decrypt) {
            let k = v8::String::new(&mut ctx_scope, "decrypt").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_sign) {
            let k = v8::String::new(&mut ctx_scope, "sign").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_verify) {
            let k = v8::String::new(&mut ctx_scope, "verify").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        if let Some(f) = v8::Function::new(&mut ctx_scope, subtle_digest) {
            let k = v8::String::new(&mut ctx_scope, "digest").unwrap();
            subtle.set(&mut ctx_scope, k.into(), f.into());
        }
        let subtle_key = v8::String::new(&mut ctx_scope, "subtle").unwrap();
        crypto.set(&mut ctx_scope, subtle_key.into(), subtle.into());
        let key = v8::String::new(&mut ctx_scope, "crypto").unwrap();
        global.set(&mut ctx_scope, key.into(), crypto.into());
    }

    fn bind_performance(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        crate::runtime::web_apis::PERFORMANCE_BASELINE.with(|cell| {
            if cell.get().is_none() {
                cell.set(Some(std::time::Instant::now()));
            }
        });
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let performance = v8::Object::new(&mut &mut ctx_scope);
        if let Some(now_fn) = v8::Function::new(&mut ctx_scope, crate::runtime::web_apis::performance_now) {
            let key = v8::String::new(&mut ctx_scope, "now").unwrap();
            performance.set(&mut ctx_scope, key.into(), now_fn.into());
        }
        let key = v8::String::new(&mut ctx_scope, "performance").unwrap();
        global.set(&mut ctx_scope, key.into(), performance.into());
    }

    fn bind_structured_clone(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        if let Some(clone_fn) = v8::Function::new(&mut ctx_scope, crate::runtime::web_apis::structured_clone) {
            let key = v8::String::new(&mut ctx_scope, "structuredClone").unwrap();
            global.set(&mut ctx_scope, key.into(), clone_fn.into());
        }
    }

    fn bind_dom_exception(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let template = v8::FunctionTemplate::new(&mut ctx_scope, crate::runtime::web_apis::dom_exception_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();
        let key = v8::String::new(&mut ctx_scope, "DOMException").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    fn bind_blob(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let template = v8::FunctionTemplate::new(&mut ctx_scope, crate::runtime::web_apis::blob_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();
        let key = v8::String::new(&mut ctx_scope, "Blob").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    fn bind_form_data(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let template = v8::FunctionTemplate::new(&mut ctx_scope, crate::runtime::web_apis::form_data_constructor);
        let ctor = template.get_function(&mut &mut ctx_scope).unwrap();
        let key = v8::String::new(&mut ctx_scope, "FormData").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    fn bind_response(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        use crate::runtime::fetch::{response_text_callback, response_json_callback, response_arraybuffer_callback, response_json_static_callback};
        let global = context.global(scope);
        let mut ctx_scope = v8::ContextScope::new(scope, context);
        let template = v8::FunctionTemplate::new(&mut ctx_scope, crate::runtime::web_apis::response_constructor);
        let ctor = template.get_function(&mut ctx_scope).unwrap();
        if let Some(ctor_obj) = ctor.to_object(&mut ctx_scope) {
            let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
            if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
                if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                    if let Some(f) = v8::Function::new(&mut ctx_scope, response_text_callback) {
                        let k = v8::String::new(&mut ctx_scope, "text").unwrap();
                        proto_obj.set(&mut ctx_scope, k.into(), f.into());
                    }
                    if let Some(f) = v8::Function::new(&mut ctx_scope, response_json_callback) {
                        let k = v8::String::new(&mut ctx_scope, "json").unwrap();
                        proto_obj.set(&mut ctx_scope, k.into(), f.into());
                    }
                    if let Some(f) = v8::Function::new(&mut ctx_scope, response_arraybuffer_callback) {
                        let k = v8::String::new(&mut ctx_scope, "arrayBuffer").unwrap();
                        proto_obj.set(&mut ctx_scope, k.into(), f.into());
                    }
                }
            }
            if let Some(f) = v8::Function::new(&mut ctx_scope, response_json_static_callback) {
                let k = v8::String::new(&mut ctx_scope, "json").unwrap();
                ctor_obj.set(&mut ctx_scope, k.into(), f.into());
            }
        }
        let key = v8::String::new(&mut ctx_scope, "Response").unwrap();
        global.set(&mut ctx_scope, key.into(), ctor.into());
    }

    fn bind_url(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::url_api::bind_url(scope, context);
    }

    fn bind_headers(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::url_api::bind_headers(scope, context);
    }

    fn bind_timers(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        super::timers::bind_timers(scope, context);
    }

    fn bind_wasm(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::wasm::WebAssemblyAPI::bind(scope, context);
        tracing::debug!("Bound WebAssembly API");
    }

    fn bind_buffer(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        crate::runtime::buffer_api::bind_buffer(scope, context);
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
