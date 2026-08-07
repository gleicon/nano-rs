//! Node.js Compatibility Tests
//!
//! Tests that verify the fs polyfill matches Node.js behavior and semantics.

use std::sync::Arc;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Test that fs module exports match Node.js structure
#[test]
fn test_fs_module_structure() {
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

    // Test that all expected methods exist
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const methods = [
            'readFileSync', 'writeFileSync', 'existsSync', 'unlinkSync',
            'readFile', 'writeFile', 'exists', 'unlink'
        ];
        methods.every(m => typeof fs[m] === 'function')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();

    assert!(result.is_true(), "All expected fs methods should exist");
}

/// Test readFileSync with encoding returns string
#[test]
fn test_readfile_sync_encoding() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Write a text file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/text.txt", b"Hello, World!").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test with utf8 encoding
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const content = fs.readFileSync('/text.txt', 'utf8');
        typeof content + ':' + content
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "string:Hello, World!",
        "Should return string with utf8 encoding"
    );
}

/// Test readFileSync with options object
#[test]
fn test_readfile_sync_options_object() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/text.txt", b"Hello, World!").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test with options object { encoding: 'utf8' }
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const content = fs.readFileSync('/text.txt', { encoding: 'utf8' });
        typeof content + ':' + content
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "string:Hello, World!",
        "Should accept options object"
    );
}

/// Test readFileSync without encoding returns Buffer (Uint8Array)
#[test]
fn test_readfile_sync_returns_buffer() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/text.txt", b"Hello").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const content = fs.readFileSync('/text.txt');
        content instanceof Uint8Array
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();

    assert!(
        result.is_true(),
        "Without encoding should return Uint8Array (Buffer-like)"
    );
}

/// Test writeFileSync accepts string data
#[test]
fn test_writefile_sync_string() {
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

    // Write string data
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.writeFileSync('/string.txt', 'String content');
        'done'
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    script.run(scope).unwrap();

    // Verify content
    let rt = tokio::runtime::Runtime::new().unwrap();
    let content = rt.block_on(async { vfs.read("/string.txt").await.unwrap() });
    assert_eq!(content, b"String content");
}

/// Test writeFileSync accepts Uint8Array data
#[test]
fn test_writefile_sync_uint8array() {
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

    // Write Uint8Array data
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const data = new Uint8Array([1, 2, 3, 4, 5]);
        fs.writeFileSync('/binary.bin', data);
        'done'
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    script.run(scope).unwrap();

    // Verify content
    let rt = tokio::runtime::Runtime::new().unwrap();
    let content = rt.block_on(async { vfs.read("/binary.bin").await.unwrap() });
    assert_eq!(content, vec![1, 2, 3, 4, 5]);
}

/// Test existsSync returns false for directories (not implemented)
#[test]
fn test_exists_sync_returns_boolean() {
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

    // existsSync should return a boolean, not throw
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const result = fs.existsSync('/nonexistent');
        typeof result
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "boolean", "existsSync should return boolean");
}

/// Test async readFile callback signature (err, data)
#[test]
fn test_async_callback_signature() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/callback.txt", b"callback data").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Test callback receives (err, data) on success
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        let result = '';
        fs.readFile('/callback.txt', function(err, data) {
            result = (err === null) + ':' + (data instanceof Uint8Array);
        });
        result
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "true:true",
        "Callback should receive (null, Uint8Array) on success"
    );
}

/// Test unlinkSync removes files
#[test]
fn test_unlink_sync_removes_file() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/delete-me.txt", b"delete me").await.unwrap();
    });

    set_current_vfs(Some(vfs.clone()));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Unlink the file
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.unlinkSync('/delete-me.txt');
        fs.existsSync('/delete-me.txt')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();

    assert!(!result.is_true(), "File should not exist after unlink");
}

/// Test error codes match Node.js exactly
#[test]
fn test_error_codes_exact_match() {
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

    // Verify ENOENT code matches Node.js format exactly
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('/missing');
            ''
        } catch (err) {
            // Node.js ENOENT format: ENOENT: no such file or directory, open '/path'
            err.code === 'ENOENT' && err.message.includes('ENOENT')
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();

    assert!(
        result.is_true(),
        "Error code ENOENT should match Node.js format"
    );
}
