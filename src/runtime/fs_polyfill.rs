//! Node.js fs Module Polyfill
//!
//! This module provides a Node.js-compatible fs module that routes
//! operations to the VFS backend. This allows existing Node.js applications
//! to use `require('fs')` and have their file operations transparently
//! directed to the NANO VFS.
//!
//! # API Reference
//!
//! ```javascript
//! const fs = require('fs');
//! fs.readFileSync('/data/config.json');  // Returns Buffer
//! fs.writeFileSync('/data/output.txt', 'Hello'); // Writes file
//! fs.existsSync('/data/config.json');    // Returns boolean
//! fs.unlinkSync('/data/temp.txt');       // Deletes file
//! ```

use std::cell::RefCell;
use std::sync::Arc;

use crate::vfs::{IsolateVfs, VfsError};

thread_local! {
    static FS_POLYFILL: RefCell<Option<v8::Global<v8::Object>>> = RefCell::new(None);
    static CURRENT_VFS: RefCell<Option<Arc<IsolateVfs>>> = RefCell::new(None);
    // Virtual module objects built once per isolate context.
    static PATH_MODULE: RefCell<Option<v8::Global<v8::Object>>> = RefCell::new(None);
    static BUFFER_MODULE: RefCell<Option<v8::Global<v8::Object>>> = RefCell::new(None);
    static ASSERT_MODULE: RefCell<Option<v8::Global<v8::Object>>> = RefCell::new(None);
}

/// Set the fs polyfill module for the current context
pub fn set_fs_polyfill(polyfill: Option<v8::Global<v8::Object>>) {
    FS_POLYFILL.with(|cell| {
        *cell.borrow_mut() = polyfill;
    });
}

/// Set the current VFS context for JS callbacks
pub fn set_current_vfs(vfs: Option<Arc<IsolateVfs>>) {
    CURRENT_VFS.with(|cell| {
        *cell.borrow_mut() = vfs;
    });
}

/// Get the current VFS context if available
fn with_current_vfs<F, R>(f: F) -> R
where
    F: FnOnce(Option<&IsolateVfs>) -> R,
{
    CURRENT_VFS.with(|cell| {
        let vfs = cell.borrow();
        f(vfs.as_ref().map(|arc| arc.as_ref()))
    })
}

/// Run an async VFS operation synchronously from a V8 callback.
fn vfs_block_on<F, Fut, R>(make_fut: F) -> R
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    let handle = tokio::runtime::Handle::try_current()
        .ok()
        .or_else(|| crate::data_plane::with_worker_runtime(|h| h.clone()));
    let _cpu_wait = crate::data_plane::AsyncWaitGuard::begin();
    if let Some(handle) = handle {
        handle.block_on(make_fut())
    } else {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(make_fut())
    }
}

/// Helper to extract string argument from V8 callback
fn extract_string_arg(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: &v8::FunctionCallbackArguments,
    index: i32,
) -> Option<String> {
    if args.length() <= index {
        return None;
    }
    let arg = args.get(index);
    arg.to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
}

/// Helper to extract bytes from V8 argument
fn extract_bytes_arg(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: &v8::FunctionCallbackArguments,
    index: i32,
) -> Option<Vec<u8>> {
    if args.length() <= index {
        return None;
    }
    let arg = args.get(index);

    // Try Uint8Array first (before string, since Uint8Array.toString() returns array representation)
    if let Ok(uint8array) = arg.try_cast::<v8::Uint8Array>() {
        let length = uint8array.byte_length();
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            if let Some(val) = uint8array.get_index(scope, i as u32) {
                if let Some(int) = val.to_integer(scope) {
                    vec.push(int.value() as u8);
                }
            }
        }
        return Some(vec);
    }

    // Try ArrayBuffer
    if let Ok(arraybuffer) = arg.try_cast::<v8::ArrayBuffer>() {
        let store = arraybuffer.get_backing_store();
        let length = arraybuffer.byte_length();
        let bytes: Vec<u8> = (0..length)
            .filter_map(|i| store.get(i).map(|cell| cell.get()))
            .collect();
        return Some(bytes);
    }

    // Try string last (for text data)
    if let Some(s) = arg.to_string(scope) {
        return Some(s.to_rust_string_lossy(scope).into_bytes());
    }

    None
}

/// Convert VfsError to V8 Error object and throw it
fn throw_fs_error(scope: &mut v8::PinnedRef<v8::HandleScope>, error: &VfsError) {
    let message = format!("{}", error);
    let message_str = v8::String::new(scope, &message).unwrap();
    let error_obj = v8::Exception::error(scope, message_str);

    // Add code property
    if let Some(err_obj) = error_obj.to_object(scope) {
        let code_key = v8::String::new(scope, "code").unwrap();
        let code_str = v8::String::new(scope, error.code()).unwrap();
        err_obj.set(scope, code_key.into(), code_str.into());

        // Add path property if available
        if let Some(path) = error.path() {
            let path_key = v8::String::new(scope, "path").unwrap();
            let path_str = v8::String::new(scope, path).unwrap();
            err_obj.set(scope, path_key.into(), path_str.into());
        }
    }

    scope.throw_exception(error_obj);
}

/// Create error object properties for callbacks
/// 
/// Note: This macro-like pattern avoids lifetime issues with returning Local handles
macro_rules! create_error_obj {
    ($scope:expr, $error:expr) => {{
        let message = format!("{}", $error);
        let message_str = v8::String::new($scope, &message).unwrap();
        let error_obj = v8::Exception::error($scope, message_str);

        // Add code property
        if let Some(err_obj) = error_obj.to_object($scope) {
            let code_key = v8::String::new($scope, "code").unwrap();
            let code_str = v8::String::new($scope, $error.code()).unwrap();
            err_obj.set($scope, code_key.into(), code_str.into());

            // Add path property if available
            if let Some(path) = $error.path() {
                let path_key = v8::String::new($scope, "path").unwrap();
                let path_str = v8::String::new($scope, path).unwrap();
                err_obj.set($scope, path_key.into(), path_str.into());
            }
        }

        error_obj
    }};
}

// ============== Path normalization (pure Rust) ==============

fn normalize_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => { parts.pop(); }
            c => parts.push(c),
        }
    }
    let joined = parts.join("/");
    if is_absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

// ============== Path module callbacks ==============

fn path_join(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..args.length() {
        if let Some(s) = args.get(i).to_string(scope) {
            let s = s.to_rust_string_lossy(scope);
            if !s.is_empty() {
                parts.push(s);
            }
        }
    }
    let result = normalize_path(&parts.join("/"));
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

fn path_dirname(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let result = match p.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
        None => ".".to_string(),
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

fn path_basename(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let ext = extract_string_arg(scope, &args, 1);
    let base = p.split('/').last().unwrap_or("").to_string();
    let result = if let Some(ref e) = ext {
        if base.ends_with(e.as_str()) {
            base[..base.len() - e.len()].to_string()
        } else {
            base
        }
    } else {
        base
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

fn path_extname(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let base = p.split('/').last().unwrap_or("");
    let result = match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_string(),
        _ => String::new(),
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

fn path_resolve(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..args.length() {
        if let Some(s) = args.get(i).to_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    let joined = parts.join("/");
    let normalized = normalize_path(&joined);
    let result = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{}", normalized)
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

fn path_is_absolute(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    retval.set(v8::Boolean::new(scope, p.starts_with('/')).into());
}

fn path_normalize_fn(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let result = normalize_path(&p);
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

// ============== Buffer module callbacks ==============

fn buffer_from(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let bytes = if args.length() > 0 {
        extract_bytes_arg(scope, &args, 0).unwrap_or_default()
    } else {
        Vec::new()
    };
    let ab = v8::ArrayBuffer::new(scope, bytes.len());
    {
        let store = ab.get_backing_store();
        for (i, &byte) in bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) { cell.set(byte); }
        }
    }
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
        retval.set(arr.into());
    }
}

fn buffer_alloc(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let size = args.get(0).to_integer(scope).map(|n| n.value() as usize).unwrap_or(0);
    let ab = v8::ArrayBuffer::new(scope, size);
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, size) {
        retval.set(arr.into());
    }
}

fn buffer_is_buffer(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let is_buf = args.length() > 0 && args.get(0).is_uint8_array();
    retval.set(v8::Boolean::new(scope, is_buf).into());
}

fn buffer_concat(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut all_bytes: Vec<u8> = Vec::new();
    if args.length() > 0 {
        let arr_val = args.get(0);
        if arr_val.is_array() {
            let arr = arr_val.cast::<v8::Array>();
            for i in 0..arr.length() {
                if let Some(item) = arr.get_index(scope, i) {
                    if let Ok(uint8arr) = item.try_cast::<v8::Uint8Array>() {
                        for j in 0..uint8arr.byte_length() {
                            if let Some(v) = uint8arr.get_index(scope, j as u32) {
                                if let Some(n) = v.to_integer(scope) {
                                    all_bytes.push(n.value() as u8);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let ab = v8::ArrayBuffer::new(scope, all_bytes.len());
    {
        let store = ab.get_backing_store();
        for (i, &byte) in all_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) { cell.set(byte); }
        }
    }
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, all_bytes.len()) {
        retval.set(arr.into());
    }
}

// ============== Assert module callbacks ==============

fn assert_ok(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let passed = args.length() > 0 && {
        let val = args.get(0);
        // Truthy check: reject null, undefined, false, 0, ""
        !val.is_null_or_undefined()
            && !val.is_false()
            && val.to_string(scope).map(|s| !s.to_rust_string_lossy(scope).is_empty()).unwrap_or(true)
            && val.to_number(scope).map(|n| n.value() != 0.0).unwrap_or(true)
    };
    if !passed {
        let msg = extract_string_arg(scope, &args, 1)
            .unwrap_or_else(|| "Assertion failed".to_string());
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

fn assert_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 { return; }
    let a = args.get(0).to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default();
    let b = args.get(1).to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default();
    if a != b {
        let msg = extract_string_arg(scope, &args, 2)
            .unwrap_or_else(|| format!("{} != {}", a, b));
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

fn assert_strict_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 { return; }
    if !args.get(0).strict_equals(args.get(1)) {
        let msg = extract_string_arg(scope, &args, 2)
            .unwrap_or_else(|| "Strict equality assertion failed".to_string());
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

fn assert_not_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 { return; }
    let a = args.get(0).to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default();
    let b = args.get(1).to_string(scope).map(|s| s.to_rust_string_lossy(scope)).unwrap_or_default();
    if a == b {
        let msg = extract_string_arg(scope, &args, 2)
            .unwrap_or_else(|| format!("{} == {}", a, b));
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

/// Create and bind the fs polyfill module to a V8 context
///
/// This creates a module-like object that exposes Node.js fs API
/// and binds it to the global scope as both:
/// - A global `require` function that can resolve 'fs'
/// - Direct access via global._nano_fs for internal use
pub fn bind_fs_polyfill(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create the fs module object and immediately convert to Global to avoid lifetime issues
    let fs_module = {
        let fs = v8::Object::new(&mut ctx_scope);

        // Synchronous methods
        if let Some(fn_read_sync) = v8::Function::new(&mut ctx_scope, fs_read_file_sync) {
            let key = v8::String::new(&mut ctx_scope, "readFileSync").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_read_sync.into());
        }

        if let Some(fn_write_sync) = v8::Function::new(&mut ctx_scope, fs_write_file_sync) {
            let key = v8::String::new(&mut ctx_scope, "writeFileSync").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_write_sync.into());
        }

        if let Some(fn_exists_sync) = v8::Function::new(&mut ctx_scope, fs_exists_sync) {
            let key = v8::String::new(&mut ctx_scope, "existsSync").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_exists_sync.into());
        }

        if let Some(fn_unlink_sync) = v8::Function::new(&mut ctx_scope, fs_unlink_sync) {
            let key = v8::String::new(&mut ctx_scope, "unlinkSync").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_unlink_sync.into());
        }

        // Alias deleteSync to unlinkSync for compatibility
        if let Some(fn_delete_sync) = v8::Function::new(&mut ctx_scope, fs_unlink_sync) {
            let key = v8::String::new(&mut ctx_scope, "deleteSync").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_delete_sync.into());
        }

        // Asynchronous methods (callbacks)
        if let Some(fn_read) = v8::Function::new(&mut ctx_scope, fs_read_file) {
            let key = v8::String::new(&mut ctx_scope, "readFile").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_read.into());
        }

        if let Some(fn_write) = v8::Function::new(&mut ctx_scope, fs_write_file) {
            let key = v8::String::new(&mut ctx_scope, "writeFile").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_write.into());
        }

        if let Some(fn_exists) = v8::Function::new(&mut ctx_scope, fs_exists) {
            let key = v8::String::new(&mut ctx_scope, "exists").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_exists.into());
        }

        if let Some(fn_unlink) = v8::Function::new(&mut ctx_scope, fs_unlink) {
            let key = v8::String::new(&mut ctx_scope, "unlink").unwrap();
            fs.set(&mut ctx_scope, key.into(), fn_unlink.into());
        }

        v8::Global::new(&mut ctx_scope, fs)
    };

    // Convert back to Local for setting on global
    let fs_module_local = v8::Local::new(&mut ctx_scope, fs_module.clone());

    // Store in global._nano_fs for internal reference
    let internal_key = v8::String::new(&mut ctx_scope, "_nano_fs").unwrap();
    global.set(&mut ctx_scope, internal_key.into(), fs_module_local.into());

    // Create require function
    let require_fn = v8::Function::new(&mut ctx_scope, require_callback);
    if let Some(require_fn) = require_fn {
        let require_key = v8::String::new(&mut ctx_scope, "require").unwrap();
        global.set(&mut ctx_scope, require_key.into(), require_fn.into());
    }

    // Store the polyfill globally for this thread
    set_fs_polyfill(Some(fs_module));

    // === path module ===
    let path_mod = {
        let obj = v8::Object::new(&mut ctx_scope);
        macro_rules! path_fn {
            ($name:expr, $cb:expr) => {
                if let (Some(f), Some(k)) = (v8::Function::new(&mut ctx_scope, $cb), v8::String::new(&mut ctx_scope, $name)) {
                    obj.set(&mut ctx_scope, k.into(), f.into());
                }
            };
        }
        path_fn!("join", path_join);
        path_fn!("dirname", path_dirname);
        path_fn!("basename", path_basename);
        path_fn!("extname", path_extname);
        path_fn!("resolve", path_resolve);
        path_fn!("isAbsolute", path_is_absolute);
        path_fn!("normalize", path_normalize_fn);
        if let (Some(k), Some(v)) = (v8::String::new(&mut ctx_scope, "sep"), v8::String::new(&mut ctx_scope, "/")) {
            obj.set(&mut ctx_scope, k.into(), v.into());
        }
        if let (Some(k), Some(v)) = (v8::String::new(&mut ctx_scope, "delimiter"), v8::String::new(&mut ctx_scope, ":")) {
            obj.set(&mut ctx_scope, k.into(), v.into());
        }
        v8::Global::new(&mut ctx_scope, obj)
    };
    PATH_MODULE.with(|cell| *cell.borrow_mut() = Some(path_mod));

    // === buffer module ===
    let buffer_mod = {
        let obj = v8::Object::new(&mut ctx_scope);
        macro_rules! buf_fn {
            ($name:expr, $cb:expr) => {
                if let (Some(f), Some(k)) = (v8::Function::new(&mut ctx_scope, $cb), v8::String::new(&mut ctx_scope, $name)) {
                    obj.set(&mut ctx_scope, k.into(), f.into());
                }
            };
        }
        buf_fn!("from", buffer_from);
        buf_fn!("alloc", buffer_alloc);
        buf_fn!("isBuffer", buffer_is_buffer);
        buf_fn!("concat", buffer_concat);
        buf_fn!("Buffer", buffer_from);
        v8::Global::new(&mut ctx_scope, obj)
    };
    BUFFER_MODULE.with(|cell| *cell.borrow_mut() = Some(buffer_mod));

    // === assert module ===
    let assert_mod = {
        let obj = v8::Object::new(&mut ctx_scope);
        macro_rules! assert_fn {
            ($name:expr, $cb:expr) => {
                if let (Some(f), Some(k)) = (v8::Function::new(&mut ctx_scope, $cb), v8::String::new(&mut ctx_scope, $name)) {
                    obj.set(&mut ctx_scope, k.into(), f.into());
                }
            };
        }
        assert_fn!("ok", assert_ok);
        assert_fn!("equal", assert_equal);
        assert_fn!("strictEqual", assert_strict_equal);
        assert_fn!("notEqual", assert_not_equal);
        assert_fn!("assert", assert_ok);
        v8::Global::new(&mut ctx_scope, obj)
    };
    ASSERT_MODULE.with(|cell| *cell.borrow_mut() = Some(assert_mod));

    // === process global ===
    {
        let process = v8::Object::new(&mut ctx_scope);

        // process.env — uses the per-app operator-configured allowlist, NOT std::env::vars().
        // std::env::vars() would leak host secrets (API keys, DB passwords) into sandboxed JS.
        let env_obj = v8::Object::new(&mut ctx_scope);
        let app_env = crate::runtime::vfs_bindings::current_env();
        for (key, val) in &app_env {
            if let (Some(k), Some(v)) = (
                v8::String::new(&mut ctx_scope, key.as_str()),
                v8::String::new(&mut ctx_scope, val.as_str()),
            ) {
                env_obj.set(&mut ctx_scope, k.into(), v.into());
            }
        }
        if let Some(env_key) = v8::String::new(&mut ctx_scope, "env") {
            process.set(&mut ctx_scope, env_key.into(), env_obj.into());
        }

        // process.version (mock Node.js compat version)
        if let (Some(k), Some(v)) = (v8::String::new(&mut ctx_scope, "version"), v8::String::new(&mut ctx_scope, "v18.0.0")) {
            process.set(&mut ctx_scope, k.into(), v.into());
        }
        if let (Some(k), Some(v)) = (v8::String::new(&mut ctx_scope, "platform"), v8::String::new(&mut ctx_scope, "linux")) {
            process.set(&mut ctx_scope, k.into(), v.into());
        }

        if let Some(process_key) = v8::String::new(&mut ctx_scope, "process") {
            global.set(&mut ctx_scope, process_key.into(), process.into());
        }
    }
}

/// require() function implementation
///
/// Currently only supports 'fs' module. Returns the fs polyfill.
fn require_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() == 0 {
        let msg = v8::String::new(scope, "require() requires a module name").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    }

    let module_name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    match module_name.as_str() {
        "fs" => {
            let global = scope.get_current_context().global(scope);
            let fs_key = v8::String::new(scope, "_nano_fs").unwrap();
            if let Some(fs_module) = global.get(scope, fs_key.into()) {
                retval.set(fs_module);
            } else {
                let msg = v8::String::new(scope, "fs module not available").unwrap();
                let error = v8::Exception::error(scope, msg);
                scope.throw_exception(error);
            }
        }
        "path" => {
            PATH_MODULE.with(|cell| {
                if let Some(ref global_mod) = *cell.borrow() {
                    let local = v8::Local::new(scope, global_mod);
                    retval.set(local.into());
                } else {
                    let msg = v8::String::new(scope, "path module not initialized").unwrap();
                    let error = v8::Exception::error(scope, msg);
                    scope.throw_exception(error);
                }
            });
        }
        "buffer" | "node:buffer" => {
            BUFFER_MODULE.with(|cell| {
                if let Some(ref global_mod) = *cell.borrow() {
                    let local = v8::Local::new(scope, global_mod);
                    retval.set(local.into());
                } else {
                    let msg = v8::String::new(scope, "buffer module not initialized").unwrap();
                    let error = v8::Exception::error(scope, msg);
                    scope.throw_exception(error);
                }
            });
        }
        "assert" | "node:assert" => {
            ASSERT_MODULE.with(|cell| {
                if let Some(ref global_mod) = *cell.borrow() {
                    let local = v8::Local::new(scope, global_mod);
                    retval.set(local.into());
                } else {
                    let msg = v8::String::new(scope, "assert module not initialized").unwrap();
                    let error = v8::Exception::error(scope, msg);
                    scope.throw_exception(error);
                }
            });
        }
        _ => {
            let msg = v8::String::new(scope, &format!("Module '{}' not found", module_name)).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}

// ============== Synchronous Methods ==============

/// fs.readFileSync(path[, options])
///
/// Reads the entire contents of a file synchronously.
/// Returns a Uint8Array (Buffer-like) containing the file contents.
fn fs_read_file_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "readFileSync requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Check for encoding option (second argument)
    let encoding = if args.length() > 1 {
        extract_string_arg(scope, &args, 1)
            .or_else(|| {
                // Try to get encoding from options object
                args.get(1).to_object(scope).and_then(|obj| {
                    let enc_key = v8::String::new(scope, "encoding").unwrap();
                    obj.get(scope, enc_key.into())
                        .and_then(|v| v.to_string(scope))
                        .map(|s| s.to_rust_string_lossy(scope))
                })
            })
    } else {
        None
    };

    // Perform read
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.read(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    match result {
        Ok(bytes) => {
            // If encoding specified, return string; otherwise return Uint8Array
            if let Some(_enc) = encoding {
                let text = String::from_utf8_lossy(&bytes);
                if let Some(s) = v8::String::new(scope, &text) {
                    retval.set(s.into());
                }
            } else {
                // Return as Uint8Array (Buffer-like)
                let ab = v8::ArrayBuffer::new(scope, bytes.len());
                let store = ab.get_backing_store();
                for (i, byte) in bytes.iter().enumerate() {
                    if let Some(cell) = store.get(i) {
                        cell.set(*byte);
                    }
                }
                if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
                    retval.set(uint8array.into());
                } else {
                    retval.set(ab.into());
                }
            }
        }
        Err(e) => {
            throw_fs_error(scope, &e);
        }
    }
}

/// fs.writeFileSync(path, data[, options])
///
/// Writes data to a file synchronously.
fn fs_write_file_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "writeFileSync requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let data = match extract_bytes_arg(scope, &args, 1) {
        Some(d) => d,
        None => {
            let msg = v8::String::new(scope, "writeFileSync requires data argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.write(&path, &data).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    if let Err(e) = result {
        throw_fs_error(scope, &e);
    }
}

/// fs.existsSync(path)
///
/// Returns true if the file exists, false otherwise.
fn fs_exists_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            retval.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.exists(&path).await })
        } else {
            Ok(false)
        }
    });

    match result {
        Ok(exists) => {
            retval.set(v8::Boolean::new(scope, exists).into());
        }
        Err(_) => {
            retval.set(v8::Boolean::new(scope, false).into());
        }
    }
}

/// fs.unlinkSync(path)
///
/// Deletes a file synchronously.
fn fs_unlink_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "unlinkSync requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.delete(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    if let Err(e) = result {
        throw_fs_error(scope, &e);
    }
}

// ============== Asynchronous Methods (Callbacks) ==============

/// fs.readFile(path[, options], callback)
///
/// Asynchronously reads the entire contents of a file.
fn fs_read_file(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // For simplicity, we're using sync implementation in async wrapper
    // In production, this should be properly async
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "readFile requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Get callback (last argument)
    let callback = if args.length() >= 2 {
        let last_idx = args.length() - 1;
        let cb = args.get(last_idx);
        if cb.is_function() {
            Some(cb.cast::<v8::Function>())
        } else {
            None
        }
    } else {
        None
    };

    // Perform read (sync for now)
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.read(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    // Call callback with result
    if let Some(cb) = callback {
        let global = scope.get_current_context().global(scope);
        match result {
            Ok(bytes) => {
                // Create Uint8Array
                let ab = v8::ArrayBuffer::new(scope, bytes.len());
                let store = ab.get_backing_store();
                for (i, byte) in bytes.iter().enumerate() {
                    if let Some(cell) = store.get(i) {
                        cell.set(*byte);
                    }
                }
                let data = if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
                    uint8array.into()
                } else {
                    ab.into()
                };
                let null_val = v8::null(scope);
                let _ = cb.call(scope, global.into(), &[null_val.into(), data]);
            }
            Err(e) => {
                let err_obj = create_error_obj!(scope, &e);
                let undefined = v8::undefined(scope);
                let _ = cb.call(scope, global.into(), &[err_obj, undefined.into()]);
            }
        }
    }
}

/// fs.writeFile(path, data[, options], callback)
///
/// Asynchronously writes data to a file.
fn fs_write_file(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "writeFile requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    let data = match extract_bytes_arg(scope, &args, 1) {
        Some(d) => d,
        None => {
            let msg = v8::String::new(scope, "writeFile requires data argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Get callback (last argument)
    let callback = if args.length() >= 3 {
        let last_idx = args.length() - 1;
        let cb = args.get(last_idx);
        if cb.is_function() {
            Some(cb.cast::<v8::Function>())
        } else {
            None
        }
    } else {
        None
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.write(&path, &data).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    if let Some(cb) = callback {
        let global = scope.get_current_context().global(scope);
        match result {
            Ok(()) => {
                let null_val = v8::null(scope);
                let _ = cb.call(scope, global.into(), &[null_val.into()]);
            }
            Err(e) => {
                let err_obj = create_error_obj!(scope, &e);
                let _ = cb.call(scope, global.into(), &[err_obj]);
            }
        }
    }
}

/// fs.exists(path, callback)
///
/// Asynchronously test whether a file exists.
fn fs_exists(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            retval.set(v8::Boolean::new(scope, false).into());
            return;
        }
    };

    // Get callback (second argument)
    let callback = if args.length() >= 2 {
        let cb = args.get(1);
        if cb.is_function() {
            Some(cb.cast::<v8::Function>())
        } else {
            None
        }
    } else {
        None
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.exists(&path).await })
        } else {
            Ok(false)
        }
    });

    if let Some(cb) = callback {
        let global = scope.get_current_context().global(scope);
        match result {
            Ok(exists) => {
                let exists_val = v8::Boolean::new(scope, exists);
                let _ = cb.call(scope, global.into(), &[exists_val.into()]);
            }
            Err(_) => {
                let false_val = v8::Boolean::new(scope, false);
                let _ = cb.call(scope, global.into(), &[false_val.into()]);
            }
        }
    }
}

/// fs.unlink(path, callback)
///
/// Asynchronously delete a file.
fn fs_unlink(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "unlink requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Get callback (second argument)
    let callback = if args.length() >= 2 {
        let cb = args.get(1);
        if cb.is_function() {
            Some(cb.cast::<v8::Function>())
        } else {
            None
        }
    } else {
        None
    };

    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.delete(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available".to_string()))
        }
    });

    if let Some(cb) = callback {
        let global = scope.get_current_context().global(scope);
        match result {
            Ok(()) => {
                let null_val = v8::null(scope);
                let _ = cb.call(scope, global.into(), &[null_val.into()]);
            }
            Err(e) => {
                let err_obj = create_error_obj!(scope, &e);
                let _ = cb.call(scope, global.into(), &[err_obj]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{MemoryBackend, VfsNamespace};
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    /// Test that fs module is created correctly
    #[test]
    fn test_fs_polyfill_created() {
        init_platform();

        let vfs = Arc::new(IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        ));
        set_current_vfs(Some(vfs));

        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(handle_scope, &mut isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

        bind_fs_polyfill(ctx_scope, context);

        // Check require function exists
        let global = context.global(ctx_scope);
        let require_key = v8::String::new(ctx_scope, "require").unwrap();
        let require_fn = global.get(ctx_scope, require_key.into()).expect("require not found");
        assert!(require_fn.is_function());

        // Check _nano_fs exists
        let fs_key = v8::String::new(ctx_scope, "_nano_fs").unwrap();
        let fs_module = global.get(ctx_scope, fs_key.into()).expect("_nano_fs not found");
        assert!(!fs_module.is_undefined());

        // Check fs module has expected methods
        let fs_obj = fs_module.to_object(ctx_scope).expect("fs is not an object");

        let read_sync_key = v8::String::new(ctx_scope, "readFileSync").unwrap();
        let read_sync_fn = fs_obj.get(ctx_scope, read_sync_key.into()).expect("readFileSync not found");
        assert!(read_sync_fn.is_function());

        let write_sync_key = v8::String::new(ctx_scope, "writeFileSync").unwrap();
        let write_sync_fn = fs_obj.get(ctx_scope, write_sync_key.into()).expect("writeFileSync not found");
        assert!(write_sync_fn.is_function());

        let exists_sync_key = v8::String::new(ctx_scope, "existsSync").unwrap();
        let exists_sync_fn = fs_obj.get(ctx_scope, exists_sync_key.into()).expect("existsSync not found");
        assert!(exists_sync_fn.is_function());
    }
}
