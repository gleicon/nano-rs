//! VFS Integration Tests
//!
//! Comprehensive test coverage for the Virtual File System including:
//! - Basic CRUD operations
//! - Cross-namespace isolation
//! - Path traversal security
//! - Resource limit enforcement
//! - Concurrent access
//! - Edge cases (empty files, unicode paths)
//! - Error code verification

use nano::vfs::*;
use std::sync::Arc;

/// Helper to create a VFS backend with proper type annotation
fn create_backend() -> VfsBackendEnum {
    VfsBackendEnum::Memory(Arc::new(MemoryBackend::default()))
}

/// Helper to create a VFS backend with custom limits
fn create_backend_with_limits(limits: ResourceLimits) -> VfsBackendEnum {
    VfsBackendEnum::Memory(Arc::new(MemoryBackend::with_limits(limits)))
}

// ============================================================================
// Basic Operations
// ============================================================================

#[tokio::test]
async fn test_basic_read_write() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Write
    vfs.write("/config.json", b"{\"key\": \"value\"}")
        .await
        .unwrap();

    // Read
    let content = vfs.read("/config.json").await.unwrap();
    assert_eq!(content, b"{\"key\": \"value\"}");

    // Exists
    assert!(vfs.exists("/config.json").await.unwrap());
    assert!(!vfs.exists("/missing.txt").await.unwrap());
}

#[tokio::test]
async fn test_basic_delete() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create and delete
    vfs.write("/temp.txt", b"temporary").await.unwrap();
    assert!(vfs.exists("/temp.txt").await.unwrap());

    vfs.delete("/temp.txt").await.unwrap();
    assert!(!vfs.exists("/temp.txt").await.unwrap());

    // Delete non-existent returns error
    let result = vfs.delete("/nonexistent.txt").await;
    assert!(matches!(result, Err(VfsError::NotFound { .. })));
}

#[tokio::test]
async fn test_file_metadata() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Write file
    vfs.write("/data.txt", b"metadata test").await.unwrap();

    // Get metadata
    let meta = vfs.metadata("/data.txt").await.unwrap();
    assert_eq!(meta.size, 13);
    assert_eq!(meta.content, b"metadata test");

    // Verify timestamps exist
    let now = std::time::SystemTime::now();
    assert!(meta.created_at <= now);
    assert!(meta.modified_at <= now);
}

// ============================================================================
// Cross-Namespace Isolation
// ============================================================================

#[tokio::test]
async fn test_cross_namespace_isolation() {
    let shared_backend = create_backend();

    let vfs_a = IsolateVfs::new(
        VfsNamespace::from_hostname("app-a.example.com"),
        shared_backend.clone(),
    );

    let vfs_b = IsolateVfs::new(
        VfsNamespace::from_hostname("app-b.example.com"),
        shared_backend.clone(),
    );

    // Write in app A
    vfs_a.write("/secret.txt", b"app-a-secret").await.unwrap();

    // App B cannot read
    let result = vfs_b.read("/secret.txt").await;
    assert!(matches!(result, Err(VfsError::NotFound { .. })));

    // App A can read
    let content = vfs_a.read("/secret.txt").await.unwrap();
    assert_eq!(content, b"app-a-secret");
}

#[tokio::test]
async fn test_same_namespace_shares_files() {
    let shared_backend = create_backend();

    // Two isolates with SAME namespace
    let vfs_1 = IsolateVfs::new(
        VfsNamespace::from_hostname("shared.example.com"),
        shared_backend.clone(),
    );

    let vfs_2 = IsolateVfs::new(
        VfsNamespace::from_hostname("shared.example.com"),
        shared_backend.clone(),
    );

    // Write via vfs_1
    vfs_1.write("/shared.txt", b"shared data").await.unwrap();

    // Read via vfs_2 (same namespace)
    let content = vfs_2.read("/shared.txt").await.unwrap();
    assert_eq!(content, b"shared data");
}

// ============================================================================
// Path Traversal Security
// ============================================================================

#[tokio::test]
async fn test_path_traversal_blocked() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create file
    vfs.write("/data/file.txt", b"content").await.unwrap();

    // Try traversal - all should be blocked
    let result = vfs.read("../data/file.txt").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));

    let result = vfs.read("data/../../etc/passwd").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));

    let result = vfs.read("/etc/../passwd").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));

    let result = vfs.write("/../../../etc/passwd", b"hacked").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));
}

#[tokio::test]
async fn test_null_byte_injection_blocked() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Null byte in path should be rejected
    let result = vfs.read("file\0.txt").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));

    let result = vfs.write("/path\0/.txt", b"content").await;
    assert!(matches!(result, Err(VfsError::InvalidPath { .. })));
}

#[tokio::test]
async fn test_traversal_with_namespace_prefix() {
    // Even if path validation is bypassed, namespace prefix prevents escape
    let backend = create_backend();
    let vfs_a = IsolateVfs::new(
        VfsNamespace::from_hostname("app-a.example.com"),
        backend.clone(),
    );
    let vfs_b = IsolateVfs::new(
        VfsNamespace::from_hostname("app-b.example.com"),
        backend.clone(),
    );

    // Write in app A
    vfs_a.write("/file.txt", b"secret").await.unwrap();

    // App B tries to access with crafted path containing app-a's namespace
    // This should fail because the namespace is prepended to the path,
    // resulting in: app_b_example_com::app_a_example_com::/file.txt
    // Which doesn't exist
    let result = vfs_b.read("/../app_a_example_com::/file.txt").await;
    // Should fail with InvalidPath due to ".." or NotFound if somehow bypassed
    assert!(result.is_err());
}

// ============================================================================
// Resource Limits
// ============================================================================

#[tokio::test]
async fn test_quota_file_size() {
    let limits = ResourceLimits {
        file_size_bytes_max: 100,
        ..Default::default()
    };
    let backend = create_backend_with_limits(limits);
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Small file OK
    vfs.write("/small.txt", &[0u8; 50]).await.unwrap();

    // Large file rejected
    let result = vfs.write("/large.txt", &[0u8; 101]).await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "file_size")
    );
}

#[tokio::test]
async fn test_quota_total_storage() {
    let limits = ResourceLimits {
        total_storage_bytes_max: 200,
        files_count_max: 10,
        ..Default::default()
    };
    let backend = create_backend_with_limits(limits);
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // First file OK (100 bytes)
    vfs.write("/file1.txt", &[0u8; 100]).await.unwrap();

    // Second file OK (100 bytes = 200 total)
    vfs.write("/file2.txt", &[0u8; 100]).await.unwrap();

    // Third file rejected (would exceed 200)
    let result = vfs.write("/file3.txt", &[0u8; 10]).await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "total_storage")
    );
}

#[tokio::test]
async fn test_quota_file_count() {
    let limits = ResourceLimits {
        files_count_max: 3,
        file_size_bytes_max: 1000,
        total_storage_bytes_max: 10000,
    };
    let backend = create_backend_with_limits(limits);
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create 3 files
    for i in 0..3 {
        vfs.write(&format!("/file{}.txt", i), b"content")
            .await
            .unwrap();
    }

    // 4th file rejected
    let result = vfs.write("/file3.txt", b"content").await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "file_count")
    );
}

#[tokio::test]
async fn test_quota_update_respected() {
    // Updating a file should respect quota (can't grow beyond limit)
    let limits = ResourceLimits {
        total_storage_bytes_max: 100,
        ..Default::default()
    };
    let backend = create_backend_with_limits(limits);
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create file with 50 bytes
    vfs.write("/file.txt", &[0u8; 50]).await.unwrap();

    // Update to 100 bytes (at limit)
    vfs.write("/file.txt", &[0u8; 100]).await.unwrap();

    // Try to update to 101 bytes (would exceed limit)
    let result = vfs.write("/file.txt", &[0u8; 101]).await;
    assert!(matches!(result, Err(VfsError::QuotaExceeded { .. })));

    // Original file should still be 100 bytes
    let content = vfs.read("/file.txt").await.unwrap();
    assert_eq!(content.len(), 100);
}

// ============================================================================
// Concurrent Access
// ============================================================================

#[tokio::test]
async fn test_concurrent_writes() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        backend.clone(),
    );

    let mut handles = vec![];

    // Spawn 10 concurrent writes
    for i in 0..10 {
        let vfs_clone = Arc::new(IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            backend.clone(),
        ));
        handles.push(tokio::spawn(async move {
            vfs_clone
                .write(&format!("/file{}.txt", i), &[i as u8; 100])
                .await
        }));
    }

    // All should succeed
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    // Verify all files exist
    for i in 0..10 {
        assert!(vfs.exists(&format!("/file{}.txt", i)).await.unwrap());
    }
}

#[tokio::test]
async fn test_concurrent_read_write() {
    let backend = create_backend();
    let vfs_write = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        backend.clone(),
    ));
    let vfs_read = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        backend.clone(),
    ));

    // Write initial content
    vfs_write.write("/shared.txt", b"initial").await.unwrap();

    // Concurrent read while writing
    let write_handle = tokio::spawn(async move {
        for i in 0..10 {
            vfs_write
                .write("/shared.txt", &format!("update{}", i).into_bytes())
                .await
                .unwrap();
            tokio::task::yield_now().await;
        }
    });

    let read_handle = tokio::spawn(async move {
        for _ in 0..10 {
            // Should never fail, just might get old or new data
            let _ = vfs_read.read("/shared.txt").await;
            tokio::task::yield_now().await;
        }
    });

    let (write_result, read_result) = tokio::join!(write_handle, read_handle);
    write_result.unwrap();
    read_result.unwrap();
}

// ============================================================================
// Edge Cases
// ============================================================================

#[tokio::test]
async fn test_empty_file() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create empty file
    vfs.write("/empty.txt", b"").await.unwrap();

    // Read back
    let content = vfs.read("/empty.txt").await.unwrap();
    assert!(content.is_empty());

    // Metadata
    let meta = vfs.metadata("/empty.txt").await.unwrap();
    assert_eq!(meta.size, 0);
}

#[tokio::test]
async fn test_unicode_paths() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Various unicode paths
    vfs.write("/文件.txt", b"chinese").await.unwrap();
    vfs.write("/📁emoji.txt", b"emoji").await.unwrap();
    vfs.write("/naïve café.txt", b"accents").await.unwrap();
    vfs.write("/日本語/ファイル.txt", b"japanese")
        .await
        .unwrap();

    // Read back
    assert_eq!(vfs.read("/文件.txt").await.unwrap(), b"chinese");
    assert_eq!(vfs.read("/📁emoji.txt").await.unwrap(), b"emoji");
    assert_eq!(vfs.read("/naïve café.txt").await.unwrap(), b"accents");
    assert_eq!(vfs.read("/日本語/ファイル.txt").await.unwrap(), b"japanese");
}

#[tokio::test]
async fn test_deeply_nested_paths() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // Create deeply nested structure
    vfs.write("/a/b/c/d/e/f/deep.txt", b"deep content")
        .await
        .unwrap();

    // Read back
    let content = vfs.read("/a/b/c/d/e/f/deep.txt").await.unwrap();
    assert_eq!(content, b"deep content");

    // Verify exists at each level
    assert!(vfs.exists("/a/b/c/d/e/f/deep.txt").await.unwrap());
}

#[tokio::test]
async fn test_large_file_content() {
    let limits = ResourceLimits {
        file_size_bytes_max: 1024 * 1024, // 1MB
        total_storage_bytes_max: 10 * 1024 * 1024,
        files_count_max: 100,
    };
    let backend = create_backend_with_limits(limits);
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // 1MB file
    let large_content = vec![0u8; 1024 * 1024];
    vfs.write("/large.bin", &large_content).await.unwrap();

    // Read back
    let content = vfs.read("/large.bin").await.unwrap();
    assert_eq!(content.len(), 1024 * 1024);
}

// ============================================================================
// Error Codes
// ============================================================================

#[tokio::test]
async fn test_error_codes_match_nodejs() {
    let backend = create_backend();
    let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

    // ENOENT - file not found
    let err = vfs.read("/missing.txt").await.unwrap_err();
    assert_eq!(err.code(), "ENOENT");

    // EINVAL - invalid path (traversal)
    let err = vfs.read("/../invalid").await.unwrap_err();
    assert_eq!(err.code(), "EINVAL");

    // EQUOTA - quota exceeded
    let limits = ResourceLimits::test_limits();
    let quota_backend = VfsBackendEnum::Memory(Arc::new(MemoryBackend::with_limits(limits)));
    let quota_vfs = IsolateVfs::new(VfsNamespace::from_hostname("quota.test.com"), quota_backend);

    // Fill up quota
    for i in 0..5 {
        quota_vfs
            .write(&format!("/file{}.txt", i), &[0u8; 90])
            .await
            .unwrap();
    }

    // Next file should fail with EQUOTA
    let err = quota_vfs.write("/overflow.txt", b"x").await.unwrap_err();
    assert_eq!(err.code(), "EQUOTA");
}

// ============================================================================
// Hostname Sanitization
// ============================================================================

#[tokio::test]
async fn test_hostname_sanitization() {
    // Test various hostname formats
    let ns1 = VfsNamespace::from_hostname("api.example.com");
    assert_eq!(ns1.as_str(), "api_example_com");

    let ns2 = VfsNamespace::from_hostname("MY-APP.EXAMPLE.COM");
    assert_eq!(ns2.as_str(), "my_app_example_com");

    let ns3 = VfsNamespace::from_hostname("sub.domain.example.co.uk");
    assert_eq!(ns3.as_str(), "sub_domain_example_co_uk");
}

// ============================================================================
// Integration with NanoIsolate (v8 module)
// ============================================================================

#[tokio::test]
async fn test_vfs_through_nano_isolate() {
    use nano::v8::{initialize_platform, NanoIsolate};

    // Initialize V8 platform
    initialize_platform().expect("Failed to initialize V8 platform");

    // Create isolate with custom VFS
    let vfs = IsolateVfs::new(
        VfsNamespace::from_hostname("isolate.test.com"),
        create_backend(),
    );
    let mut isolate = NanoIsolate::new_with_vfs(vfs).expect("Failed to create isolate");

    // Use VFS through isolate
    isolate
        .vfs_mut()
        .write("/test.txt", b"via isolate")
        .await
        .unwrap();
    let content = isolate.vfs().read("/test.txt").await.unwrap();

    assert_eq!(content, b"via isolate");
}
