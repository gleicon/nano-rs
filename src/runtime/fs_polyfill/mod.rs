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
    static EVENTS_MODULE: RefCell<Option<v8::Global<v8::Object>>> = RefCell::new(None);
}

const EVENTS_MODULE_SRC: &str = r#"
(function() {
  function EventEmitter() { this._events = Object.create(null); }
  EventEmitter.prototype.on = function(ev, fn) {
    if (!this._events[ev]) this._events[ev] = [];
    this._events[ev].push(fn);
    return this;
  };
  EventEmitter.prototype.addListener = EventEmitter.prototype.on;
  EventEmitter.prototype.once = function(ev, fn) {
    var self = this;
    function wrapper() { fn.apply(this, arguments); self.off(ev, wrapper); }
    wrapper._fn = fn;
    return this.on(ev, wrapper);
  };
  EventEmitter.prototype.off = function(ev, fn) {
    if (this._events[ev]) {
      this._events[ev] = this._events[ev].filter(function(l) {
        return l !== fn && l._fn !== fn;
      });
    }
    return this;
  };
  EventEmitter.prototype.removeListener = EventEmitter.prototype.off;
  EventEmitter.prototype.emit = function(ev) {
    var listeners = this._events[ev] || [];
    var args = Array.prototype.slice.call(arguments, 1);
    listeners.slice().forEach(function(fn) { fn.apply(null, args); });
    return listeners.length > 0;
  };
  EventEmitter.prototype.removeAllListeners = function(ev) {
    if (ev) delete this._events[ev];
    else this._events = Object.create(null);
    return this;
  };
  EventEmitter.prototype.listeners = function(ev) {
    return (this._events[ev] || []).slice();
  };
  return { EventEmitter: EventEmitter };
})()
"#;

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
    arg.to_string(scope).map(|s| s.to_rust_string_lossy(scope))
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

    // Try plain Array (e.g. Buffer.from([0xde, 0xad]))
    if arg.is_array() {
        let arr = arg.cast::<v8::Array>();
        let len = arr.length() as usize;
        let mut vec = Vec::with_capacity(len);
        for i in 0..len {
            if let Some(val) = arr.get_index(scope, i as u32) {
                if let Some(n) = val.to_integer(scope) {
                    vec.push((n.value() & 0xFF) as u8);
                }
            }
        }
        return Some(vec);
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


mod assert;
mod buffer;
mod fs;
mod path;

use assert::*;
use buffer::*;
use fs::*;
use path::*;

/// Create and bind the fs polyfill module to a V8 context
///
/// This creates a module-like object that exposes Node.js fs API
/// and binds it to the global scope as both:
/// - A global `require` function that can resolve 'fs'
/// - Direct access via global._nano_fs for internal use
pub fn bind_fs_polyfill(
    scope: &mut v8::PinnedRef<v8::HandleScope<()>>,
    context: v8::Local<v8::Context>,
) {
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
                if let (Some(f), Some(k)) = (
                    v8::Function::new(&mut ctx_scope, $cb),
                    v8::String::new(&mut ctx_scope, $name),
                ) {
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
        if let (Some(k), Some(v)) = (
            v8::String::new(&mut ctx_scope, "sep"),
            v8::String::new(&mut ctx_scope, "/"),
        ) {
            obj.set(&mut ctx_scope, k.into(), v.into());
        }
        if let (Some(k), Some(v)) = (
            v8::String::new(&mut ctx_scope, "delimiter"),
            v8::String::new(&mut ctx_scope, ":"),
        ) {
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
                if let (Some(f), Some(k)) = (
                    v8::Function::new(&mut ctx_scope, $cb),
                    v8::String::new(&mut ctx_scope, $name),
                ) {
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
                if let (Some(f), Some(k)) = (
                    v8::Function::new(&mut ctx_scope, $cb),
                    v8::String::new(&mut ctx_scope, $name),
                ) {
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

    // === events module (pure-JS EventEmitter) ===
    if let Some(src) = v8::String::new(&mut ctx_scope, EVENTS_MODULE_SRC) {
        if let Some(script) = v8::Script::compile(&mut ctx_scope, src, None) {
            if let Some(result) = script.run(&mut ctx_scope) {
                if let Some(obj) = result.to_object(&mut ctx_scope) {
                    let global_mod = v8::Global::new(&mut ctx_scope, obj);
                    EVENTS_MODULE.with(|cell| *cell.borrow_mut() = Some(global_mod));
                }
            }
        }
    }

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

        // process.version — reported Node.js compat version (current LTS)
        if let (Some(k), Some(v)) = (
            v8::String::new(&mut ctx_scope, "version"),
            v8::String::new(&mut ctx_scope, "v22.11.0"),
        ) {
            process.set(&mut ctx_scope, k.into(), v.into());
        }
        if let (Some(k), Some(v)) = (
            v8::String::new(&mut ctx_scope, "platform"),
            v8::String::new(&mut ctx_scope, "linux"),
        ) {
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
        "events" | "node:events" => {
            EVENTS_MODULE.with(|cell| {
                if let Some(ref global_mod) = *cell.borrow() {
                    let local = v8::Local::new(scope, global_mod);
                    retval.set(local.into());
                } else {
                    let msg = v8::String::new(scope, "events module not initialized").unwrap();
                    let error = v8::Exception::error(scope, msg);
                    scope.throw_exception(error);
                }
            });
        }
        _ => {
            let msg =
                v8::String::new(scope, &format!("Module '{}' not found", module_name)).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;
    use crate::vfs::{MemoryBackend, VfsNamespace};

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
        let require_fn = global
            .get(ctx_scope, require_key.into())
            .expect("require not found");
        assert!(require_fn.is_function());

        // Check _nano_fs exists
        let fs_key = v8::String::new(ctx_scope, "_nano_fs").unwrap();
        let fs_module = global
            .get(ctx_scope, fs_key.into())
            .expect("_nano_fs not found");
        assert!(!fs_module.is_undefined());

        // Check fs module has expected methods
        let fs_obj = fs_module.to_object(ctx_scope).expect("fs is not an object");

        let read_sync_key = v8::String::new(ctx_scope, "readFileSync").unwrap();
        let read_sync_fn = fs_obj
            .get(ctx_scope, read_sync_key.into())
            .expect("readFileSync not found");
        assert!(read_sync_fn.is_function());

        let write_sync_key = v8::String::new(ctx_scope, "writeFileSync").unwrap();
        let write_sync_fn = fs_obj
            .get(ctx_scope, write_sync_key.into())
            .expect("writeFileSync not found");
        assert!(write_sync_fn.is_function());

        let exists_sync_key = v8::String::new(ctx_scope, "existsSync").unwrap();
        let exists_sync_fn = fs_obj
            .get(ctx_scope, exists_sync_key.into())
            .expect("existsSync not found");
        assert!(exists_sync_fn.is_function());
    }
}
