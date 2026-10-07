//! Node `fs` module callbacks — route file operations to the VFS backend.

use super::{
    extract_bytes_arg, extract_string_arg, throw_fs_error, vfs_block_on, with_current_vfs,
};
use crate::vfs::VfsError;

// ============== Synchronous Methods ==============

/// fs.readFileSync(path[, options])
///
/// Reads the entire contents of a file synchronously.
/// Returns a Uint8Array (Buffer-like) containing the file contents.
pub(super) fn fs_read_file_sync(
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
        extract_string_arg(scope, &args, 1).or_else(|| {
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
pub(super) fn fs_write_file_sync(
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
pub(super) fn fs_exists_sync(
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
pub(super) fn fs_unlink_sync(
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
pub(super) fn fs_read_file(
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
                let data = if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len())
                {
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
pub(super) fn fs_write_file(
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
pub(super) fn fs_exists(
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
pub(super) fn fs_unlink(
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

