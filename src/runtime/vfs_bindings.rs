//! VFS JavaScript Bindings — Nano.fs.* API
//!
//! This module provides JavaScript bindings for the VFS operations,
//! exposing `Nano.fs.*` API to JavaScript code running in isolates.
//!
//! # API Reference
//!
//! ```javascript
//! Nano.fs.readFileSync('/data/config.json');     // Returns Uint8Array
//! Nano.fs.writeFileSync('/data/output.txt', 'Hello'); // Returns void
//! Nano.fs.existsSync('/data/config.json');       // Returns boolean
//! Nano.fs.deleteSync('/data/temp.txt');          // Returns void
//! ```

use std::cell::RefCell;
use std::sync::Arc;

use crate::vfs::{IsolateVfs, VfsError};

// Thread-local storage for the current isolate's VFS during JS execution
thread_local! {
    static CURRENT_VFS: RefCell<Option<Arc<IsolateVfs>>> = RefCell::new(None);
}

/// Set the current VFS context for JS callbacks
///
/// This should be called before executing JS that uses Nano.fs
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
///
/// Tries (in order): the current tokio handle, the worker-thread runtime registered via
/// `data_plane::set_worker_runtime`, then creates a temporary runtime as a last resort.
fn vfs_block_on<F, Fut, R>(make_fut: F) -> R
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    let handle = tokio::runtime::Handle::try_current()
        .ok()
        .or_else(|| crate::data_plane::with_worker_runtime(|h| h.clone()));
    if let Some(handle) = handle {
        handle.block_on(make_fut())
    } else {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(make_fut())
    }
}

/// Bind the Nano.fs API to the V8 global scope
///
/// This creates the `Nano` global object with an `fs` property containing
/// all VFS methods. Each method returns a Promise for async operations.
pub fn bind_nano_fs(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create Nano object
    let nano = v8::Object::new(&mut ctx_scope);

    // Create fs object
    let fs = v8::Object::new(&mut ctx_scope);

    // Bind readFileSync method
    if let Some(read_fn) = v8::Function::new(&mut ctx_scope, nano_fs_read_file_sync) {
        let key = v8::String::new(&mut ctx_scope, "readFileSync").unwrap();
        fs.set(&mut ctx_scope, key.into(), read_fn.into());
    }

    // Bind readFile (async) method
    if let Some(read_fn) = v8::Function::new(&mut ctx_scope, nano_fs_read_file) {
        let key = v8::String::new(&mut ctx_scope, "readFile").unwrap();
        fs.set(&mut ctx_scope, key.into(), read_fn.into());
    }

    // Bind writeFileSync method
    if let Some(write_fn) = v8::Function::new(&mut ctx_scope, nano_fs_write_file_sync) {
        let key = v8::String::new(&mut ctx_scope, "writeFileSync").unwrap();
        fs.set(&mut ctx_scope, key.into(), write_fn.into());
    }

    // Bind writeFile (async) method
    if let Some(write_fn) = v8::Function::new(&mut ctx_scope, nano_fs_write_file) {
        let key = v8::String::new(&mut ctx_scope, "writeFile").unwrap();
        fs.set(&mut ctx_scope, key.into(), write_fn.into());
    }

    // Bind existsSync method
    if let Some(exists_fn) = v8::Function::new(&mut ctx_scope, nano_fs_exists_sync) {
        let key = v8::String::new(&mut ctx_scope, "existsSync").unwrap();
        fs.set(&mut ctx_scope, key.into(), exists_fn.into());
    }

    // Bind deleteSync method
    if let Some(delete_fn) = v8::Function::new(&mut ctx_scope, nano_fs_delete_sync) {
        let key = v8::String::new(&mut ctx_scope, "deleteSync").unwrap();
        fs.set(&mut ctx_scope, key.into(), delete_fn.into());
    }

    // Attach fs to Nano
    let fs_key = v8::String::new(&mut ctx_scope, "fs").unwrap();
    nano.set(&mut ctx_scope, fs_key.into(), fs.into());

    // Attach Nano to global
    let nano_key = v8::String::new(&mut ctx_scope, "Nano").unwrap();
    global.set(&mut ctx_scope, nano_key.into(), nano.into());
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

/// Helper to extract bytes from V8 argument (string, Uint8Array, or ArrayBuffer)
fn extract_bytes_arg(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: &v8::FunctionCallbackArguments,
    index: i32,
) -> Option<Vec<u8>> {
    if args.length() <= index {
        return None;
    }
    let arg = args.get(index);

    // Try string first
    if let Some(s) = arg.to_string(scope) {
        return Some(s.to_rust_string_lossy(scope).into_bytes());
    }

    // Try Uint8Array
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

    None
}

/// Convert VfsError to V8 Error object and throw it
fn throw_vfs_error(scope: &mut v8::PinnedRef<v8::HandleScope>, error: &VfsError) {
    let message = format!("{}", error);
    let message_str = v8::String::new(scope, &message).unwrap();
    let exception = v8::Exception::error(scope, message_str);

    // Add code property to the error object
    if let Some(error_obj) = exception.to_object(scope) {
        let code_key = v8::String::new(scope, "code").unwrap();
        let code_str = v8::String::new(scope, error.code()).unwrap();
        error_obj.set(scope, code_key.into(), code_str.into());

        // Add path property if available
        if let Some(path) = error.path() {
            let path_key = v8::String::new(scope, "path").unwrap();
            let path_str = v8::String::new(scope, path).unwrap();
            error_obj.set(scope, path_key.into(), path_str.into());
        }
    }

    scope.throw_exception(exception);
}

/// Nano.fs.readFileSync(path, encoding?) implementation
///
/// Returns Uint8Array containing file contents (or string if encoding specified), or throws on error
fn nano_fs_read_file_sync(
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

    // Check for optional encoding parameter
    let encoding = extract_string_arg(scope, &args, 1);
    let return_string = encoding.as_ref().map(|e| e == "utf8" || e == "utf-8").unwrap_or(false);

    // Perform synchronous read using block_on
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.read(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    match result {
        Ok(bytes) => {
            if return_string {
                // Return as UTF-8 string
                let content = String::from_utf8_lossy(&bytes);
                let str_val = v8::String::new(scope, &content).unwrap();
                retval.set(str_val.into());
            } else {
                // Create Uint8Array from bytes (default behavior)
                let ab = v8::ArrayBuffer::new(scope, bytes.len());
                if !bytes.is_empty() {
                    let store = ab.get_backing_store();
                    // SAFETY: V8 allocates `bytes.len()` bytes for this buffer.
                    // `ab` has no JS references yet, so write access is exclusive.
                    // `data()` is non-null for non-empty ArrayBuffers.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            store.data().unwrap().as_ptr() as *mut u8,
                            bytes.len(),
                        );
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
            throw_vfs_error(scope, &e);
        }
    }
}

/// Nano.fs.writeFileSync(path, data) implementation
///
/// Writes data to file, throws on error
fn nano_fs_write_file_sync(
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

    // Perform synchronous write
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.write(&path, &data).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    if let Err(e) = result {
        throw_vfs_error(scope, &e);
    }
}

/// Nano.fs.existsSync(path) implementation
///
/// Returns boolean indicating if file exists
fn nano_fs_exists_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "existsSync requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Perform synchronous check
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.exists(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    match result {
        Ok(exists) => {
            retval.set(v8::Boolean::new(scope, exists).into());
        }
        Err(e) => {
            throw_vfs_error(scope, &e);
        }
    }
}

/// Nano.fs.deleteSync(path) implementation
///
/// Deletes file, throws on error
fn nano_fs_delete_sync(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "deleteSync requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Perform synchronous delete
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.delete(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    if let Err(e) = result {
        throw_vfs_error(scope, &e);
    }
}

/// Nano.fs.readFile(path) implementation - async version
///
/// Returns a Promise that resolves to Uint8Array containing file contents
fn nano_fs_read_file(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let path = match extract_string_arg(scope, &args, 0) {
        Some(p) => p,
        None => {
            let msg = v8::String::new(scope, "readFile requires a path argument").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            scope.throw_exception(error);
            return;
        }
    };

    // Check for optional encoding parameter
    let encoding = extract_string_arg(scope, &args, 1);
    let return_string = encoding.as_ref().map(|e| e == "utf8" || e == "utf-8").unwrap_or(false);

    // Perform read synchronously
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.read(&path).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    match result {
        Ok(bytes) => {
            let data: v8::Local<v8::Value> = if return_string {
                // Return as UTF-8 string
                let content = String::from_utf8_lossy(&bytes);
                v8::String::new(scope, &content).unwrap().into()
            } else {
                // Create Uint8Array from bytes (default behavior)
                let ab = v8::ArrayBuffer::new(scope, bytes.len());
                if !bytes.is_empty() {
                    let store = ab.get_backing_store();
                    // SAFETY: V8 allocates `bytes.len()` bytes for this buffer.
                    // `ab` has no JS references yet, so write access is exclusive.
                    // `data()` is non-null for non-empty ArrayBuffers.
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            store.data().unwrap().as_ptr() as *mut u8,
                            bytes.len(),
                        );
                    }
                }
                if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
                    uint8array.into()
                } else {
                    ab.into()
                }
            };

            // Return resolved Promise: Promise.resolve(data)
            let global = scope.get_current_context().global(scope);
            let promise_key = v8::String::new(scope, "Promise").unwrap();
            let resolve_key = v8::String::new(scope, "resolve").unwrap();
            
            if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
                if let Some(promise_obj) = promise_ctor.to_object(scope) {
                    if let Some(resolve_fn) = promise_obj.get(scope, resolve_key.into()) {
                        if resolve_fn.is_function() {
                            let resolve = resolve_fn.cast::<v8::Function>();
                            if let Some(resolved_promise) = resolve.call(scope, promise_ctor, &[data]) {
                                retval.set(resolved_promise);
                                return;
                            }
                        }
                    }
                }
            }
            
            // Fallback: return the data directly
            retval.set(data);
        }
        Err(e) => {
            throw_vfs_error(scope, &e);
        }
    }
}

/// Nano.fs.writeFile(path, data) implementation - async version
///
/// Returns a Promise that resolves when write completes
fn nano_fs_write_file(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
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

    // Perform write synchronously
    let result = with_current_vfs(|vfs_opt| {
        if let Some(vfs) = vfs_opt {
            vfs_block_on(|| async { vfs.write(&path, &data).await })
        } else {
            Err(VfsError::IoError("No VFS available for this isolate".to_string()))
        }
    });

    // Return Promise.resolve() on success or throw on error
    match result {
        Ok(()) => {
            // Return Promise.resolve()
            let global = scope.get_current_context().global(scope);
            let promise_key = v8::String::new(scope, "Promise").unwrap();
            let resolve_key = v8::String::new(scope, "resolve").unwrap();
            
            if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
                if let Some(promise_obj) = promise_ctor.to_object(scope) {
                    if let Some(resolve_fn) = promise_obj.get(scope, resolve_key.into()) {
                        if resolve_fn.is_function() {
                            let resolve = resolve_fn.cast::<v8::Function>();
                            let undefined_val = v8::undefined(scope);
                            if let Some(resolved_promise) = resolve.call(scope, promise_ctor, &[undefined_val.into()]) {
                                retval.set(resolved_promise);
                                return;
                            }
                        }
                    }
                }
            }
            
            // Fallback: return undefined
            retval.set_undefined();
        }
        Err(e) => {
            throw_vfs_error(scope, &e);
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

    /// Test that Nano.fs object is created correctly
    #[test]
    fn test_nano_fs_binding_created() {
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

        bind_nano_fs(ctx_scope, context);

        // Check Nano exists
        let global = context.global(ctx_scope);
        let nano_key = v8::String::new(ctx_scope, "Nano").unwrap();
        let nano = global.get(ctx_scope, nano_key.into()).expect("Nano not found");
        assert!(!nano.is_undefined());

        // Check Nano.fs exists
        let nano_obj = nano.to_object(ctx_scope).expect("Nano is not an object");
        let fs_key = v8::String::new(ctx_scope, "fs").unwrap();
        let fs = nano_obj.get(ctx_scope, fs_key.into()).expect("fs not found");
        assert!(!fs.is_undefined());

        // Check readFileSync exists
        let fs_obj = fs.to_object(ctx_scope).expect("fs is not an object");
        let read_key = v8::String::new(ctx_scope, "readFileSync").unwrap();
        let read_fn = fs_obj.get(ctx_scope, read_key.into()).expect("readFileSync not found");
        assert!(read_fn.is_function());

        // Check writeFileSync exists
        let write_key = v8::String::new(ctx_scope, "writeFileSync").unwrap();
        let write_fn = fs_obj.get(ctx_scope, write_key.into()).expect("writeFileSync not found");
        assert!(write_fn.is_function());

        // Check existsSync exists
        let exists_key = v8::String::new(ctx_scope, "existsSync").unwrap();
        let exists_fn = fs_obj.get(ctx_scope, exists_key.into()).expect("existsSync not found");
        assert!(exists_fn.is_function());

        // Check deleteSync exists
        let delete_key = v8::String::new(ctx_scope, "deleteSync").unwrap();
        let delete_fn = fs_obj.get(ctx_scope, delete_key.into()).expect("deleteSync not found");
        assert!(delete_fn.is_function());
    }
}
