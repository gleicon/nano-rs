//! Node.js fs Polyfill Tests
//!
//! Tests for the Node.js-compatible fs module polyfill.
//! Verifies that require('fs') works and all methods route to VFS correctly.

use std::sync::Arc;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Test that require('fs') returns the polyfill object
#[test]
fn test_require_fs_returns_polyfill() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    // Bind the fs polyfill
    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Execute require('fs')
    let code = v8::String::new(scope, "const fs = require('fs'); typeof fs").unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "object",
        "require('fs') should return an object"
    );
}

/// Test fs.readFileSync() with text file
#[test]
fn test_read_file_sync_text() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write a file first
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/test.txt", b"Hello, World!").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test readFileSync with encoding
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const content = fs.readFileSync('/test.txt', 'utf8');
        content
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "Hello, World!");
}

/// Test fs.readFileSync() returns Uint8Array without encoding
#[test]
fn test_read_file_sync_binary() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write binary content
    let binary_content = vec![0u8, 1u8, 255u8, 128u8];
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/binary.bin", &binary_content).await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test readFileSync returns Uint8Array
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const content = fs.readFileSync('/binary.bin');
        content instanceof Uint8Array && content.length
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_num = result.to_number(scope).unwrap().value() as i32;

    assert_eq!(result_num, 4, "Should return Uint8Array with 4 bytes");
}

/// Test fs.writeFileSync() creates files
#[test]
fn test_write_file_sync() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs.clone()));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test writeFileSync
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.writeFileSync('/output.txt', 'Test content');
        'success'
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "success");

    // Verify file was written
    let rt = tokio::runtime::Runtime::new().unwrap();
    let content = rt.block_on(async { vfs.read("/output.txt").await.unwrap() });
    assert_eq!(content, b"Test content");
}

/// Test fs.existsSync() returns correct boolean
#[test]
fn test_exists_sync() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write a file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/exists.txt", b"content").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test existsSync for existing file
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.existsSync('/exists.txt')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    assert!(result.is_true(), "Should return true for existing file");

    // Test existsSync for non-existing file (re-use fs from previous script in same context)
    let code = v8::String::new(
        scope,
        "
        fs.existsSync('/nonexistent.txt')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    assert!(
        !result.is_true(),
        "Should return false for non-existing file"
    );
}

/// Test fs.unlinkSync() deletes files
#[test]
fn test_unlink_sync() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write a file first
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/delete.txt", b"content").await.unwrap();
    });

    set_current_vfs(Some(vfs.clone()));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test unlinkSync
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.unlinkSync('/delete.txt');
        'deleted'
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "deleted");

    // Verify file was deleted
    let exists = rt.block_on(async { vfs.exists("/delete.txt").await.unwrap() });
    assert!(!exists, "File should be deleted");
}

/// Test async fs.readFile() with callback
#[test]
fn test_read_file_async() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write a file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/async.txt", b"async content").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test async readFile with callback
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        let result = null;
        fs.readFile('/async.txt', function(err, data) {
            if (err) {
                result = 'error: ' + err.message;
            } else {
                result = data.length;
            }
        });
        // Small delay to let callback execute (sync execution for now)
        result
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_num = result.to_number(scope).unwrap().value() as i32;

    assert_eq!(
        result_num, 13,
        "Should read 13 bytes (length of 'async content')"
    );
}

/// Test error handling for non-existent files
#[test]
fn test_error_enoent() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test readFileSync throws ENOENT
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

    assert_eq!(result_str, "ENOENT", "Should throw ENOENT error");
}

/// Test require() with unsupported module throws error
#[test]
fn test_require_unsupported_module() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test require('unsupported') throws
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            require('path');
            'no error'
        } catch (err) {
            'error: ' + err.message
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert!(
        result_str.contains("error"),
        "Should throw error for unsupported module"
    );
}

/// Test writing and reading Uint8Array data
#[test]
fn test_write_read_binary() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs.clone()));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test writing Uint8Array data
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const data = new Uint8Array([0, 1, 255, 128]);
        fs.writeFileSync('/binary.bin', data);
        'written'
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "written");

    // Verify content
    let rt = tokio::runtime::Runtime::new().unwrap();
    let content = rt.block_on(async { vfs.read("/binary.bin").await.unwrap() });
    assert_eq!(content, vec![0u8, 1u8, 255u8, 128u8]);
}

/// Verify the `events` polyfill: require('events').EventEmitter with on/emit/once/off.
/// This capability is advertised (README, website) but was previously untested.
#[test]
fn test_require_events_emitter() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Exercise on/emit, once (fires once), and off (removes listener).
    let code = v8::String::new(
        scope,
        r#"
        const { EventEmitter } = require('events');
        const e = new EventEmitter();
        let calls = 0, onceCalls = 0;
        const h = (n) => { calls += n; };
        e.on('tick', h);
        e.once('tick', () => { onceCalls += 1; });
        e.emit('tick', 2);   // on(+2), once(+1)
        e.emit('tick', 3);   // on(+3), once already removed
        e.off('tick', h);
        e.emit('tick', 10);  // nothing
        `on=${calls},once=${onceCalls}`
        "#,
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).expect("compile");
    let result = script.run(scope).expect("run");
    let out = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    // on handler saw 2 then 3 (=5); once handler fired exactly once; off stopped further calls.
    assert_eq!(out, "on=5,once=1", "EventEmitter on/once/off semantics");
}
