//! VFS Error Code Compatibility Tests
//!
//! Tests that verify VFS errors are properly converted to JavaScript Error objects
//! with Node.js-compatible error codes (ENOENT, EACCES, EINVAL, EQUOTA, EIO).

use std::sync::Arc;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::runtime::vfs_bindings::set_current_vfs as set_nano_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};

mod v8_test_utils;

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Helper to create V8 scopes using v147 API
fn with_v8_context<F, R>(isolate: &mut v8::Isolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

/// Test ENOENT error code for non-existent files
#[test]
fn test_error_code_enoent() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/nonexistent.txt');
                'no error'
            } catch (err) {
                err.code
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "ENOENT", "Error code should be ENOENT");
}

/// Test ENOENT error has correct message format
#[test]
fn test_error_message_enoent() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/nonexistent.txt');
                ''
            } catch (err) {
                err.message
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert!(result.contains("ENOENT"), "Message should contain ENOENT");
    assert!(
        result.contains("nonexistent.txt"),
        "Message should contain the filename"
    );
}

/// Test EINVAL error code for invalid paths (path traversal)
#[test]
fn test_error_code_einval() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('../etc/passwd');
                'no error'
            } catch (err) {
                err.code
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(
        result, "EINVAL",
        "Error code should be EINVAL for path traversal"
    );
}

/// Test error.code property is accessible
#[test]
fn test_error_code_property() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/missing.txt');
                null
            } catch (err) {
                typeof err.code
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "string", "err.code should be a string");
}

/// Test error.path property is accessible
#[test]
fn test_error_path_property() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/missing.txt');
                null
            } catch (err) {
                err.path
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert!(
        result.contains("missing.txt"),
        "err.path should contain the path"
    );
}

/// Test async error handling with callbacks
#[test]
fn test_async_error_callback() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            let error_code = null;
            fs.readFile('/nonexistent.txt', function(err, data) {
                if (err) {
                    error_code = err.code;
                }
            });
            error_code
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(
        result, "ENOENT",
        "Async callback should receive ENOENT error"
    );
}

/// Test try/catch works with sync methods
#[test]
fn test_trycatch_sync() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            let caught = false;
            let error_code = '';
            try {
                fs.readFileSync('/missing.txt');
            } catch (err) {
                caught = true;
                error_code = err.code;
            }
            caught + ':' + error_code
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(
        result, "true:ENOENT",
        "try/catch should work and catch ENOENT"
    );
}

/// Test error instanceof Error
#[test]
fn test_error_instanceof_error() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/missing.txt');
                false
            } catch (err) {
                err instanceof Error
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        result.is_true()
    });

    assert!(result, "Error should be instanceof Error");
}

/// Test Nano.fs error codes match Node.js fs
#[test]
fn test_nano_fs_error_codes() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_nano_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::vfs_bindings::bind_nano_fs(scope, context);

        let code = v8::String::new(
            scope,
            "
            try {
                Nano.fs.readFileSync('/nonexistent.txt');
                'no error'
            } catch (err) {
                err.code
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "ENOENT", "Nano.fs should also throw ENOENT");
}

/// Test error stack trace is present
#[test]
fn test_error_has_stack() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());

    let result = with_v8_context(&mut isolate, |scope, context| {
        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            try {
                fs.readFileSync('/missing.txt');
                ''
            } catch (err) {
                typeof err.stack
            }
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "string", "Error should have stack trace");
}
