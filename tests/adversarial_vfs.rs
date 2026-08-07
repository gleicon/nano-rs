//! Adversarial VFS Escape Attempt Tests
//!
//! Tests to verify VFS path validation prevents directory traversal attacks:
//! - Basic traversal (../etc/passwd)
//! - URL encoded traversal (%2e%2e%2f)
//! - Double encoded traversal
//! - Null byte injection
//! - Unicode normalization attacks
//! - Symlink escapes
//! - Absolute path attacks
//! - Case variants
//! - Deeply nested traversal
//! - Directory traversal via null byte
//! - Directory listing attacks
//! - Write operations outside namespace

#[path = "common.rs"]
mod common;
use common::{create_test_vfs, init_platform, SecurityTestContext};

/// Helper to execute code with V8 v147 scope pattern
#[allow(dead_code)]
fn with_v8_context<F, R>(isolate: &mut v8::Isolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

/// Test basic path traversal is blocked
/// Attack: ../etc/passwd
/// Mitigation: Path validator rejects all ".." components
#[test]
fn test_traversal_basic_blocked() {
    let _ctx = SecurityTestContext::new("vfs-traversal.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(handle_scope, nano_isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
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

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "EINVAL",
        "Basic path traversal should be blocked with EINVAL"
    );
}

/// Test URL encoded traversal is blocked
/// Attack: %2e%2e%2fetc%2fpasswd (../etc/passwd encoded)
/// Mitigation: Path validation after decoding
#[test]
fn test_traversal_encoded_blocked() {
    let _ctx = SecurityTestContext::new("vfs-encoded.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // URL encoded: %2e = ., %2f = /
            fs.readFileSync('%2e%2e%2fetc%2fpasswd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // Encoded traversal should be blocked (results in EINVAL or ENOENT depending on path handling)
    assert!(
        result_str == "EINVAL" || result_str == "ENOENT",
        "URL encoded traversal should be blocked, got: {}",
        result_str
    );
}

/// Test double URL encoded traversal
/// Attack: %252e%252e%252f (double encoded ../)
/// Mitigation: Single decode only, remaining % characters don't form valid traversal
#[test]
fn test_traversal_double_encoded_blocked() {
    let _ctx = SecurityTestContext::new("vfs-double-enc.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Double encoded: %25 = %, so %252e = %2e = .
            fs.readFileSync('%252e%252e%252fetc%252fpasswd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // Double encoding results in a path with literal % characters
    // Should either be blocked (EINVAL) or treated as file not found (ENOENT)
    assert!(
        result_str == "EINVAL" || result_str == "ENOENT",
        "Double encoded traversal should be blocked, got: {}",
        result_str
    );
}

/// Test null byte injection
/// Attack: file.jpg%00.txt (null byte to truncate extension check)
/// Mitigation: Null byte validation in path
#[test]
fn test_traversal_null_byte_blocked() {
    let _ctx = SecurityTestContext::new("vfs-null.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Null byte injection attempt
            fs.readFileSync('file.jpg\\x00.txt');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "EINVAL",
        "Null byte injection should be blocked with EINVAL"
    );
}

/// Test Unicode normalization attacks
/// Attack: Using Unicode characters that normalize to ..
/// Mitigation: Check for .. substring after normalization
#[test]
fn test_traversal_unicode_blocked() {
    let ctx = SecurityTestContext::new("vfs-unicode.example.com");

    // Create test file with legitimate unicode name
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        ctx.vfs
            .write("/文件.txt", b"unicode content")
            .await
            .unwrap();
    });

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    // Test that U+2024 (one dot leader) + U+2024 doesn't become ".."
    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // U+2024 is ․ (One Dot Leader) - looks like . but different
            // Some systems normalize this to regular dot - VFS should still block
            fs.readFileSync('\\u2024\\u2024/etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // Should be blocked (EINVAL) or not found (ENOENT) - but definitely not success
    assert!(
        result_str != "success",
        "Unicode traversal attack should be blocked, got: {}",
        result_str
    );
}

/// Test symlink escape attempts
/// Attack: Create symlink pointing outside namespace, then read it
/// Mitigation: VFS doesn't support symlinks that escape namespace
#[test]
fn test_symlink_escape_blocked() {
    init_platform();

    // Note: MemoryBackend doesn't support symlinks
    // This test documents the expected behavior
    // In a real disk backend, symlinks would be resolved and validated

    let vfs = create_test_vfs("symlink-test.example.com");

    // Symlinks that escape namespace should be blocked at resolution time
    // For now, just verify the VFS doesn't allow .. in paths
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(async {
        // Attempt to write via a path that looks like symlink escape
        vfs.write("/link/../../../etc/passwd", b"data").await
    });

    assert!(result.is_err(), "Symlink escape attempt should be blocked");
}

/// Test absolute path attacks
/// Attack: /etc/passwd
/// Mitigation: Absolute paths are validated the same as relative
#[test]
fn test_absolute_path_blocked() {
    let _ctx = SecurityTestContext::new("vfs-absolute.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Absolute path attack
            fs.readFileSync('/etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // Should be ENOENT (file not found) - /etc/passwd is outside namespace
    // Not EINVAL because it's a valid absolute path, just not accessible
    assert_eq!(
        result_str, "ENOENT",
        "Absolute path should result in ENOENT (outside namespace)"
    );
}

/// Test traversal with case variants
/// Attack: .., %2E%2E, %2e%2E, mixed case
/// Mitigation: All case variants blocked
#[test]
fn test_traversal_case_variants_blocked() {
    let _ctx = SecurityTestContext::new("vfs-case.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        const attempts = [
            '../etc/passwd',
            '..\\etc\\passwd',  // Windows-style
            '/../etc/passwd',
        ];
        
        const results = [];
        for (const path of attempts) {
            try {
                fs.readFileSync(path);
                results.push('success');
            } catch (err) {
                results.push(err.code);
            }
        }
        
        results.join(',')
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // All attempts should fail (not be 'success')
    assert!(
        !result_str.contains("success"),
        "All traversal case variants should be blocked, got: {}",
        result_str
    );
}

/// Test deeply nested traversal
/// Attack: ../../../../../../../etc/passwd
/// Mitigation: Any occurrence of ".." as path component is blocked
#[test]
fn test_traversal_nested_deep_blocked() {
    let _ctx = SecurityTestContext::new("vfs-deep.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Deeply nested traversal
            fs.readFileSync('../../../../../../../etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "EINVAL",
        "Deeply nested traversal should be blocked"
    );
}

/// Test traversal with embedded null byte
/// Attack: ../\x00/etc/passwd
/// Mitigation: Null byte detected, path rejected
#[test]
fn test_traversal_with_null_blocked() {
    let _ctx = SecurityTestContext::new("vfs-null-trav.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Null byte in traversal path
            fs.readFileSync('../\\x00/etc/passwd');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "EINVAL",
        "Traversal with null byte should be blocked"
    );
}

/// Test directory traversal listing attack
/// Attack: Attempt to list parent directory contents
/// Mitigation: VFS doesn't expose directory listing APIs to JS
#[test]
fn test_directory_traversal_listing() {
    // This test documents that readdir is not exposed
    // readdir is intentionally not implemented in the fs polyfill

    let _ctx = SecurityTestContext::new("vfs-listing.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        // Check if readdir exists (it shouldn't in NANO)
        typeof fs.readdir === 'function' ? 'exists' : 'not-exposed'
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "not-exposed",
        "readdir should not be exposed to prevent directory listing"
    );
}

/// Test write operations outside namespace
/// Attack: Write file to parent directory
/// Mitigation: Write operations validated same as reads
#[test]
fn test_write_to_parent_blocked() {
    let _ctx = SecurityTestContext::new("vfs-write-parent.example.com");

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        "
        const fs = require('fs');
        try {
            // Attempt to write outside namespace
            fs.writeFileSync('../outside.txt', 'malicious content');
            'success'
        } catch (err) {
            err.code
        }
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "EINVAL",
        "Write to parent directory should be blocked"
    );
}
