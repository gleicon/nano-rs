//! Handler compilation: turn JS source into a callable V8 fetch handler.
//!
//! Extracted from pool.rs — ESM and classic (WinterTC addEventListener) paths.

use anyhow::{anyhow, Result};

use crate::vfs::IsolateVfs;

/// Read source code from VFS first, fall back to disk.
/// VFS path like `/index.js` is tried against the isolate's VFS;
/// on miss (or non-VFS entrypoints like absolute disk paths), falls back to `read_code_cached`.
pub(crate) fn read_code_vfs_or_disk(entrypoint: &str, vfs: &IsolateVfs) -> Result<std::sync::Arc<str>> {
    let vfs_result = crate::data_plane::with_worker_runtime(|h| h.block_on(vfs.read(entrypoint)));
    if let Some(Ok(bytes)) = vfs_result {
        if let Ok(s) = String::from_utf8(bytes) {
            return Ok(s.into());
        }
    }
    crate::data_plane::read_code_cached(entrypoint)
}

pub(crate) fn compile_esm_handler(
    ctx_scope: &mut v8::ContextScope<'_, '_, v8::HandleScope<'_, v8::Context>>,
    entrypoint: &str,
    code: &str,
    vfs: IsolateVfs,
) -> Result<v8::Global<v8::Function>> {
    use crate::v8::module::{module_resolve_callback, set_current_loader, ModuleLoader};
    let ep_v8 =
        v8::String::new(ctx_scope, entrypoint).ok_or_else(|| anyhow!("OOM: module origin"))?;
    let origin = v8::ScriptOrigin::new(
        ctx_scope,
        ep_v8.into(),
        0,
        0,
        true,
        -1,
        None,
        false,
        false,
        true,
        None,
    );
    let code_v8 = v8::String::new(ctx_scope, code).ok_or_else(|| anyhow!("OOM: module source"))?;
    let mut esm_source = v8::script_compiler::Source::new(code_v8, Some(&origin));
    let esm_module = v8::script_compiler::compile_module(ctx_scope, &mut esm_source)
        .ok_or_else(|| anyhow!("ESM compile failed: {}", entrypoint))?;

    let mut loader = ModuleLoader::new(vfs);
    // SAFETY: loader lives until instantiate_module returns.
    unsafe {
        set_current_loader(Some(&mut loader as *mut _));
    }
    let inst_ok = esm_module
        .instantiate_module(ctx_scope, module_resolve_callback)
        .is_some();
    unsafe {
        set_current_loader(None);
    }
    if !inst_ok {
        return Err(anyhow!("ESM instantiate failed: {}", entrypoint));
    }

    esm_module
        .evaluate(ctx_scope)
        .ok_or_else(|| anyhow!("ESM evaluate failed: {}", entrypoint))?;

    let ns = esm_module
        .get_module_namespace()
        .to_object(ctx_scope)
        .ok_or_else(|| anyhow!("ESM namespace not object: {}", entrypoint))?;

    // Try `export function fetch` first, then `export default { fetch }`.
    let fk = v8::String::new(ctx_scope, "fetch");
    let fk_val = fk
        .and_then(|k| ns.get(ctx_scope, k.into()))
        .filter(|v| v.is_function());
    let handler_val = match fk_val {
        Some(v) => v,
        None => {
            let dk = v8::String::new(ctx_scope, "default");
            let default_obj = dk
                .and_then(|k| ns.get(ctx_scope, k.into()))
                .and_then(|d| d.to_object(ctx_scope));
            let fk2 = v8::String::new(ctx_scope, "fetch");
            match default_obj
                .and_then(|o| fk2.and_then(|k| o.get(ctx_scope, k.into())))
                .filter(|v| v.is_function())
            {
                Some(v) => v,
                None => {
                    return Err(anyhow!(
                        "No 'fetch' export in '{}'. Use: export function fetch(req){{...}}",
                        entrypoint
                    ))
                }
            }
        }
    };
    Ok(v8::Global::new(
        ctx_scope,
        handler_val.cast::<v8::Function>(),
    ))
}

/// WinterTC addEventListener shim — prefix+suffix wrapping the user script.
///
/// Prefix: defines `addEventListener` that stores the user callback in a
/// module-level var (`__nano_fetch_listener`), avoiding closure-over-parameter
/// issues with V8's TryCatch scope in production.
///
/// Suffix: after user code has run and called `addEventListener`, builds
/// `__nano_user_fetch` — the function pool.rs looks for — as a plain module-
/// level FetchEvent adapter that returns `respondWith`'s argument (or the
/// handler's return value as fallback).
const WINTERTC_PREFIX: &str = "\
var __nano_user_fetch;\
\nvar __nano_fetch_listener = null;\
\nglobalThis.addEventListener = function(type, fn) {\
\n  if (type === 'fetch') { __nano_fetch_listener = fn; }\
\n};\n";

const WINTERTC_SUFFIX: &str = "\
\nif (typeof __nano_fetch_listener === 'function') {\
\n  var __nano_fl = __nano_fetch_listener;\
\n  globalThis.__nano_user_fetch = function(request) {\
\n    var captured;\
\n    var event = { request: request, respondWith: function(r) { captured = r; } };\
\n    var ret = __nano_fl(event);\
\n    return captured !== undefined ? captured : ret;\
\n  };\
\n}\n";

pub(crate) fn compile_classic_handler(
    ctx_scope: &mut v8::ContextScope<'_, '_, v8::HandleScope<'_, v8::Context>>,
    entrypoint: &str,
    code: &str,
    context: v8::Local<'_, v8::Context>,
    cache_key: &str,
) -> Result<v8::Global<v8::Function>> {
    let shimmed = format!("{}{}{}", WINTERTC_PREFIX, code, WINTERTC_SUFFIX);
    let code_v8 =
        v8::String::new(ctx_scope, &shimmed).ok_or_else(|| anyhow!("V8 string alloc failed"))?;

    let unbound = if let Some(cached_bytes) = crate::data_plane::get_bytecode_cache(cache_key) {
        let cached_data = v8::script_compiler::CachedData::new(&cached_bytes);
        let mut source =
            v8::script_compiler::Source::new_with_cached_data(code_v8, None, cached_data);
        v8::script_compiler::compile_unbound_script(
            ctx_scope,
            &mut source,
            v8::script_compiler::CompileOptions::ConsumeCodeCache,
            v8::script_compiler::NoCacheReason::NoReason,
        )
    } else {
        let mut source = v8::script_compiler::Source::new(code_v8, None);
        let unbound = v8::script_compiler::compile_unbound_script(
            ctx_scope,
            &mut source,
            v8::script_compiler::CompileOptions::NoCompileOptions,
            v8::script_compiler::NoCacheReason::NoReason,
        );
        if let Some(ref u) = unbound {
            if let Some(cache) = u.create_code_cache() {
                let bytes: std::sync::Arc<[u8]> = (&**cache).into();
                crate::data_plane::set_bytecode_cache(cache_key, bytes);
            }
        }
        unbound
    };

    let script = unbound
        .ok_or_else(|| anyhow!("Script compile failed for '{}'", entrypoint))?
        .bind_to_current_context(ctx_scope);
    script
        .run(ctx_scope)
        .ok_or_else(|| anyhow!("Script execution failed for '{}'", entrypoint))?;

    let global_obj = context.global(ctx_scope);
    let nano_k = v8::String::new(ctx_scope, "__nano_user_fetch")
        .ok_or_else(|| anyhow!("V8 OOM allocating key"))?;
    let fetch_k =
        v8::String::new(ctx_scope, "fetch").ok_or_else(|| anyhow!("V8 OOM allocating key"))?;
    global_obj
        .get(ctx_scope, nano_k.into())
        .filter(|v| v.is_function())
        .or_else(|| {
            global_obj
                .get(ctx_scope, fetch_k.into())
                .filter(|v| v.is_function())
        })
        .map(|f| v8::Global::new(ctx_scope, f.cast::<v8::Function>()))
        .ok_or_else(|| {
            anyhow!(
                "No fetch handler found in '{}'. Export a 'fetch' function.",
                entrypoint
            )
        })
}
