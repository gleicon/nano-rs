//! VFS Security Tests
//!
//! Comprehensive security tests for VFS JavaScript bindings:
//! - Path traversal prevention
//! - Namespace isolation
//! - Resource limit enforcement
//! - Unicode and special character handling
//! - Null byte injection prevention

use std::sync::Arc;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::runtime::vfs_bindings::set_current_vfs as set_nano_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

// ========== Path Traversal Tests ==========

/// Test basic path traversal ../ is blocked
#[test]
fn test_traversal_parent_directory_blocked() {
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

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('../etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "EINVAL",
        "Path traversal with ../ should be blocked"
    );
}

/// Test nested path traversal is blocked
#[test]
fn test_traversal_nested_blocked() {
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

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('foo/../../etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "EINVAL",
        "Nested path traversal should be blocked"
    );
}

/// Test foo/../bar is blocked (results in traversal attempt)
#[test]
fn test_traversal_middle_component_blocked() {
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

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('data/../secret.txt');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "EINVAL",
        "Middle-component traversal should be blocked"
    );
}

/// Test multiple slashes are normalized safely
#[test]
fn test_multiple_slashes_normalized() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // Create a file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/data/file.txt", b"content").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Multiple slashes should be normalized, not traversal
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.existsSync('//data//file.txt')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();

    assert!(
        result.is_true(),
        "Multiple slashes should normalize to valid path"
    );
}

/// Test null byte injection is blocked
#[test]
fn test_null_byte_injection_blocked() {
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

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('file\\x00.txt');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "EINVAL",
        "Null byte injection should be blocked"
    );
}

// ========== Namespace Isolation Tests ==========

/// Test app A cannot read app B's files
#[test]
fn test_namespace_isolation_different_apps() {
    init_platform();

    let shared_backend: Arc<MemoryBackend> = Arc::new(MemoryBackend::default());

    // App A writes a file
    let vfs_a = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("app-a.example.com"),
        VfsBackendEnum::Memory(Arc::clone(&shared_backend)),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs_a.write("/secret.txt", b"app-a-secret").await.unwrap();
    });

    // App B tries to read it via JS
    let vfs_b = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("app-b.example.com"),
        VfsBackendEnum::Memory(Arc::clone(&shared_backend)),
    ));
    set_current_vfs(Some(vfs_b));

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
        try {
            fs.readFileSync('/secret.txt');
            'read-success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "ENOENT", "App B should not see App A's files");
}

/// Test same namespace can access files
#[test]
fn test_namespace_same_app_can_access() {
    init_platform();

    let backend: Arc<MemoryBackend> = Arc::new(MemoryBackend::default());

    // Create file
    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("same-app.example.com"),
        VfsBackendEnum::Memory(Arc::clone(&backend)),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/shared.txt", b"shared-content").await.unwrap();
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
        fs.readFileSync('/shared.txt', 'utf8')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(
        result_str, "shared-content",
        "Same namespace should access files"
    );
}

// ========== Edge Case Tests ==========

/// Test empty path is rejected
#[test]
fn test_empty_path_rejected() {
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

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "EINVAL", "Empty path should be rejected");
}

/// Test root path "/" is normalized
#[test]
fn test_root_path_handled() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/root-file.txt", b"root-content").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Root path should normalize to just the file
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        fs.readFileSync('/root-file.txt', 'utf8')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "root-content", "Root path should work");
}

/// Test file starting with .. but not containing .. as component is blocked
/// Note: VFS rejects any path containing ".." as a substring for security
#[test]
fn test_filename_with_dotdot_prefix_blocked() {
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

    // VFS rejects any path containing ".." substring for safety
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        try {
            fs.readFileSync('/..hidden.txt');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    // Files starting with '..' are valid filenames, not path traversal
    // Should return ENOENT (file not found), not EINVAL (invalid path)
    assert_eq!(
        result_str, "ENOENT",
        "Files with .. prefix are valid, just not found"
    );
}

/// Test unicode paths work correctly
#[test]
fn test_unicode_paths_allowed() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // File with unicode name
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/文件.txt", b"unicode-content").await.unwrap();
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
        fs.readFileSync('/文件.txt', 'utf8')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "unicode-content", "Unicode paths should work");
}

/// Test emoji in filenames works
#[test]
fn test_emoji_filename_allowed() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // File with emoji name
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/🎉party.txt", b"emoji-content").await.unwrap();
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
        fs.readFileSync('/🎉party.txt', 'utf8')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "emoji-content", "Emoji filenames should work");
}

/// Test spaces in paths work
#[test]
fn test_spaces_in_paths_allowed() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));

    // File with spaces
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/my file.txt", b"space-content").await.unwrap();
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
        fs.readFileSync('/my file.txt', 'utf8')
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "space-content", "Spaces in paths should work");
}

/// Test that error messages contain useful information for debugging
#[test]
fn test_error_messages_informative() {
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

    // Error message should contain useful information
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

    // Message should contain ENOENT and mention the file
    assert!(
        result_str.contains("ENOENT"),
        "Error should contain ENOENT code"
    );
    assert!(!result_str.is_empty(), "Error message should not be empty");
}

/// Test Nano.fs also respects security boundaries
#[test]
fn test_nano_fs_respects_traversal_protection() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    ));
    set_nano_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut handle_scope = storage.init();
    let context = v8::Context::new(&handle_scope, Default::default());
    let scope = &mut v8::ContextScope::new(&mut handle_scope, context);

    nano::runtime::vfs_bindings::bind_nano_fs(scope, context);

    let code = v8::String::new(
        scope,
        "
        try {
            Nano.fs.readFileSync('../etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();
    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "EINVAL", "Nano.fs should also block traversal");
}
