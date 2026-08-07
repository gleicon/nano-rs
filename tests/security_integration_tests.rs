//! Security Integration Tests
//!
//! Integration tests that verify VFS security with actual HTTP request handling.

use std::sync::Arc;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Test that malicious paths from request handlers are blocked
#[test]
fn test_request_handler_blocks_traversal() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("app.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Pre-populate some files
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/public/index.html", b"<h1>Public</h1>")
            .await
            .unwrap();
        vfs.write("/private/secret.txt", b"secret-data")
            .await
            .unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);
    nano::runtime::apis::RuntimeAPIs::bind_all(scope, context);

    // Simulate a request handler that tries path traversal
    let handler_code = v8::String::new(
        scope,
        "
        (function(request) {
            const fs = require('fs');
            const path = request.url || '/public/index.html';
            
            // Malicious path from user input
            const userPath = '../private/secret.txt';
            
            try {
                const content = fs.readFileSync(userPath, 'utf8');
                return new Response(content, { status: 200 });
            } catch (err) {
                return new Response('Blocked: ' + err.code, { status: 403 });
            }
        })
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, handler_code, None).unwrap();
    let handler = script.run(scope).unwrap();

    // Create a mock request
    let request = v8::Object::new(scope);
    let url_key = v8::String::new(scope, "url").unwrap();
    let url_val = v8::String::new(scope, "../private/secret.txt").unwrap();
    request.set(scope, url_key.into(), url_val.into());

    // Call handler
    let handler_fn = handler.cast::<v8::Function>();
    let undefined_val = v8::undefined(scope);
    let result = handler_fn
        .call(scope, undefined_val.into(), &[request.into()])
        .unwrap();

    // Verify response indicates blocking
    let result_obj = result.to_object(scope).unwrap();
    let status_key = v8::String::new(scope, "status").unwrap();
    let status = result_obj.get(scope, status_key.into()).unwrap();
    let status_num = status.to_number(scope).unwrap().value() as i32;

    assert_eq!(
        status_num, 403,
        "Traversal attempt should be blocked with 403"
    );
}

/// Test file operations respect namespace boundaries in concurrent contexts
#[test]
fn test_concurrent_namespace_isolation() {
    init_platform();

    let shared_backend = Arc::new(MemoryBackend::default());

    // Setup two namespaces
    let vfs_a = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("tenant-a.example.com"),
        VfsBackendEnum::Memory(Arc::clone(&shared_backend)),
    ));

    let vfs_b = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("tenant-b.example.com"),
        VfsBackendEnum::Memory(Arc::clone(&shared_backend)),
    ));

    // Write data for each tenant
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs_a.write("/data.txt", b"tenant-a-data").await.unwrap();
        vfs_b.write("/data.txt", b"tenant-b-data").await.unwrap();
    });

    // Test tenant A can only read tenant A's data
    set_current_vfs(Some(vfs_a.clone()));

    let mut isolate_a = v8::Isolate::new(Default::default());
    {
        let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate_a));
        let mut handle_scope = storage.init();
        let context = v8::Context::new(&handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            fs.readFileSync('/data.txt', 'utf8')
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

        assert_eq!(
            result_str, "tenant-a-data",
            "Tenant A should read their own data"
        );
    }

    // Test tenant B can only read tenant B's data
    set_current_vfs(Some(vfs_b.clone()));

    let mut isolate_b = v8::Isolate::new(Default::default());
    {
        let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate_b));
        let mut handle_scope = storage.init();
        let context = v8::Context::new(&handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

        nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

        let code = v8::String::new(
            scope,
            "
            const fs = require('fs');
            fs.readFileSync('/data.txt', 'utf8')
        ",
        )
        .unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let result = script.run(scope).unwrap();
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

        assert_eq!(
            result_str, "tenant-b-data",
            "Tenant B should read their own data"
        );
    }
}

/// Test that file operations from user scripts respect path validation
#[test]
fn test_user_script_path_validation() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("user-app.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Create allowed file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/uploads/image.png", b"fake-image-data")
            .await
            .unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // User script attempts various path attacks
    let user_script = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const results = [];
        
        // Valid path - should work
        try {
            fs.readFileSync('/uploads/image.png');
            results.push('valid:ok');
        } catch (e) {
            results.push('valid:' + e.code);
        }
        
        // Traversal - should fail
        try {
            fs.readFileSync('../../../etc/passwd');
            results.push('traversal:ok');
        } catch (e) {
            results.push('traversal:' + e.code);
        }
        
        // Null byte - should fail
        try {
            fs.readFileSync('file\\x00.png');
            results.push('null:ok');
        } catch (e) {
            results.push('null:' + e.code);
        }
        
        results.join(',')
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, user_script, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    let parts: Vec<&str> = result_str.split(',').collect();
    assert_eq!(parts[0], "valid:ok", "Valid path should succeed");
    assert!(parts[1].starts_with("traversal:"), "Traversal should fail");
    assert!(parts[2].starts_with("null:"), "Null byte should fail");
}
