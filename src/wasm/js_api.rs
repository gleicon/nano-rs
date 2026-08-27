//! WebAssembly JavaScript API implementation
//!
//! Exposes V8's built-in WebAssembly to JavaScript:
//! - WebAssembly.compile(bytes) -> Promise<Module>
//! - WebAssembly.instantiate(moduleOrBytes, imports) -> Promise<Instance>
//! - WebAssembly.validate(bytes) -> boolean
//! - WebAssembly.Module class
//! - WebAssembly.Instance class
//! - WebAssembly.Memory class
//! - WebAssembly.CompileError, RuntimeError
//!
//! ## Cache Architecture (two tiers)
//!
//! **Tier 1 (JS polyfill — active):** `wasm_cache_polyfill()` injected at bind time.
//! Per-isolate `Map` cache keyed by FNV-32 hash of bytes. After first compile on
//! a worker, all subsequent `WebAssembly.compile(sameBytes)` return cached module.
//! Cost: O(n) hash per call (fast integer ops). Benefit: zero V8 recompilation
//! after warmup within each worker.

use crate::wasm::WasmLoader;
use v8;

/// JS polyfill injected once per isolate at bind time.
///
/// Wraps WebAssembly.compile and WebAssembly.instantiate with a per-isolate
/// Map cache. After first compilation of a given WASM module, all subsequent
/// compile calls for the same bytes return Promise.resolve(cached) immediately.
///
/// Caching is done in JS because compiling WASM from inside a Rust
/// `FunctionCallback` was found unreliable; the JS `WebAssembly.compile` path is
/// the compilation primitive and this cache sits on top of it.
///
/// Key: FNV-32 hash of bytes as hex + ':' + byte length (collision-resistant
/// for practical WASM module sizes in edge functions).
/// Limits injected from Rust constants at bind time — keep in sync with `limits::wasm`.
pub const WASM_IMPORT_COUNT_MAX: u32 = crate::limits::wasm::IMPORT_COUNT_MAX;
pub const WASM_EXPORT_COUNT_MAX: u32 = crate::limits::wasm::EXPORT_COUNT_MAX;

/// Build the WASM cache + limit-enforcement polyfill, injecting the Rust limit constants.
///
/// Wraps `WebAssembly.compile` and `WebAssembly.instantiate` with:
/// - Per-isolate module cache (FNV-32 key on bytes)
/// - Import count check: rejects if `WebAssembly.Module.imports(m).length > IMPORT_MAX`
/// - Export count check: rejects if `WebAssembly.Module.exports(m).length > EXPORT_MAX`
///
/// Rejection throws a `RangeError` with the limit and actual count, which the JS
/// caller sees as a rejected Promise. Counters on the Rust side are incremented via
/// `Nano.__wasm_import_rejected()` / `Nano.__wasm_export_rejected()` stubs registered
/// at bind time — see `WebAssemblyAPI::bind`.
pub fn wasm_cache_polyfill(import_max: u32, export_max: u32) -> String {
    format!(
        r#"
(function() {{
    var _wc = new Map();
    var _oc = WebAssembly.compile;
    var _oi = WebAssembly.instantiate;
    var _IMAX = {import_max};
    var _EMAX = {export_max};

    function _h(src) {{
        var u8 = ArrayBuffer.isView(src)
            ? new Uint8Array(src.buffer, src.byteOffset, src.byteLength)
            : new Uint8Array(src);
        var h = 2166136261;
        for (var i = 0; i < u8.length; i++) {{
            h = (Math.imul(h ^ u8[i], 16777619)) >>> 0;
        }}
        return h.toString(16) + ':' + u8.length;
    }}

    function _isBytes(s) {{
        return s instanceof ArrayBuffer || ArrayBuffer.isView(s);
    }}

    function _checkLimits(m) {{
        var ic = WebAssembly.Module.imports(m).length;
        if (ic > _IMAX) {{
            if (typeof Nano !== 'undefined' && Nano.__wasm_import_rejected) Nano.__wasm_import_rejected();
            throw new RangeError('WASM module has ' + ic + ' imports; limit is ' + _IMAX);
        }}
        var ec = WebAssembly.Module.exports(m).length;
        if (ec > _EMAX) {{
            if (typeof Nano !== 'undefined' && Nano.__wasm_export_rejected) Nano.__wasm_export_rejected();
            throw new RangeError('WASM module has ' + ec + ' exports; limit is ' + _EMAX);
        }}
    }}

    WebAssembly.compile = function(source) {{
        if (!_isBytes(source)) return _oc(source);
        var k = _h(source);
        if (_wc.has(k)) return Promise.resolve(_wc.get(k));
        return _oc(source).then(function(m) {{ _checkLimits(m); _wc.set(k, m); return m; }});
    }};

    WebAssembly.instantiate = function(source, imports) {{
        if (!_isBytes(source)) return _oi(source, imports);
        var k = _h(source);
        if (_wc.has(k)) {{
            var m = _wc.get(k);
            return _oi(m, imports).then(function(inst) {{
                return {{ module: m, instance: inst }};
            }});
        }}
        return _oc(source).then(function(m) {{
            _checkLimits(m);
            _wc.set(k, m);
            return _oi(m, imports).then(function(inst) {{
                return {{ module: m, instance: inst }};
            }});
        }});
    }};
}})();
"#
    )
}

/// WebAssembly JavaScript API binder
pub struct WebAssemblyAPI;

impl WebAssemblyAPI {
    /// Bind WebAssembly global to context
    pub fn bind(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
        // V8 already has WebAssembly built-in
        // We just need to ensure it's accessible on the global object
        // The native WebAssembly object is added by V8 automatically
        // We can optionally extend it with our custom functionality here

        let global = context.global(scope);

        // Enter context scope for V8 APIs that require HandleScope<Context>
        let mut ctx_scope = v8::ContextScope::new(scope, context);

        // Check if WebAssembly already exists (it should in modern V8)
        let wasm_key = v8::String::new(&mut ctx_scope, "WebAssembly").unwrap();

        if global.get(&mut ctx_scope, wasm_key.into()).is_none() {
            panic!("V8 WebAssembly is not available. This runtime requires V8 with WebAssembly support.");
        }

        tracing::debug!("WebAssembly API available in V8");

        // Add our validate function which provides additional Rust-side validation
        if let Some(wasm_val) = global.get(&mut ctx_scope, wasm_key.into()) {
            if let Some(wasm_obj) = wasm_val.to_object(&mut ctx_scope) {
                // Create validate function
                let validate_fn = v8::FunctionTemplate::new(&mut ctx_scope, wasm_validate_callback);
                let validate_func = validate_fn.get_function(&mut ctx_scope).unwrap();
                let validate_name = v8::String::new(&mut ctx_scope, "validate").unwrap();
                wasm_obj.set(&mut ctx_scope, validate_name.into(), validate_func.into());

                // WebAssembly.compile / instantiate are wrapped in JS (the polyfill
                // below), not via Rust FunctionCallbacks: compiling from inside a
                // callback proved unreliable, and the JS path works and is tested.
            }
        }

        // Register Nano.__wasm_import_rejected / Nano.__wasm_export_rejected as Rust
        // callbacks so the JS polyfill can increment metrics counters when limits fire.
        {
            let nano_key = v8::String::new(&mut ctx_scope, "Nano").unwrap();
            if let Some(nano_val) = global.get(&mut ctx_scope, nano_key.into()) {
                if let Some(nano_obj) = nano_val.to_object(&mut ctx_scope) {
                    let import_fn =
                        v8::Function::new(&mut ctx_scope, wasm_import_rejected_callback).unwrap();
                    let import_key =
                        v8::String::new(&mut ctx_scope, "__wasm_import_rejected").unwrap();
                    nano_obj.set(&mut ctx_scope, import_key.into(), import_fn.into());

                    let export_fn =
                        v8::Function::new(&mut ctx_scope, wasm_export_rejected_callback).unwrap();
                    let export_key =
                        v8::String::new(&mut ctx_scope, "__wasm_export_rejected").unwrap();
                    nano_obj.set(&mut ctx_scope, export_key.into(), export_fn.into());
                }
            }
        }

        // Inject the compile cache + limit-enforcement polyfill — runs once per isolate.
        // Constants are baked in at bind time so the JS closure holds plain integers.
        let polyfill = wasm_cache_polyfill(WASM_IMPORT_COUNT_MAX, WASM_EXPORT_COUNT_MAX);
        let polyfill_src = v8::String::new(&mut ctx_scope, &polyfill).unwrap();
        if let Some(script) = v8::Script::compile(&mut ctx_scope, polyfill_src, None) {
            script.run(&mut ctx_scope);
        } else {
            tracing::warn!("WebAssembly cache polyfill failed to compile — running uncached");
        }
        tracing::debug!("WebAssembly cache+limits polyfill installed (import_max={WASM_IMPORT_COUNT_MAX}, export_max={WASM_EXPORT_COUNT_MAX})");

        tracing::debug!("Bound WebAssembly API");
    }
}

fn wasm_import_rejected_callback(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    tracing::error!("WASM module rejected: exceeds IMPORT_COUNT_MAX");
    crate::metrics::METRICS.wasm_import_rejected_total.inc();
}

fn wasm_export_rejected_callback(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    tracing::error!("WASM module rejected: exceeds EXPORT_COUNT_MAX");
    crate::metrics::METRICS.wasm_export_rejected_total.inc();
}

/// WebAssembly.validate() callback implementation
fn wasm_validate_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let buffer = args.get(0);

    // Extract bytes from ArrayBuffer or TypedArray
    let bytes = if buffer.is_array_buffer() {
        let ab = buffer.cast::<v8::ArrayBuffer>();
        let store = ab.get_backing_store();
        let len = ab.byte_length();
        let mut vec = Vec::with_capacity(len);
        for i in 0..len {
            if let Some(cell) = store.get(i) {
                vec.push(cell.get());
            }
        }
        vec
    } else if buffer.is_uint8_array() {
        let ta = buffer.cast::<v8::Uint8Array>();
        let len = ta.byte_length();
        let mut vec = Vec::with_capacity(len);
        for i in 0..len {
            if let Some(val) = ta.get_index(scope, i as u32) {
                if let Some(num) = val.to_integer(scope) {
                    vec.push(num.value() as u8);
                }
            }
        }
        vec
    } else {
        // Invalid argument type
        let msg = v8::String::new(
            scope,
            "WebAssembly.validate: argument must be an ArrayBuffer or TypedArray",
        )
        .unwrap();
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    };

    // Validate using our Rust validator
    let valid = WasmLoader::validate(&bytes).is_ok();
    retval.set(v8::Boolean::new(scope, valid).into());
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Full integration tests require V8 isolate setup
    // These are basic unit tests for the validation logic

    #[test]
    fn test_validation_helper() {
        let valid_wasm = b"\0asm\x01\x00\x00\x00";
        assert!(WasmLoader::validate(valid_wasm).is_ok());

        let invalid_wasm = b"invalid";
        assert!(WasmLoader::validate(invalid_wasm).is_err());
    }
}
