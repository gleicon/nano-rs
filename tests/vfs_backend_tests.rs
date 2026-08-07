//! VFS Backend Integration Tests
//!
//! Cross-backend verification tests to ensure all storage backends
//! (Memory, Disk, S3) behave consistently.

use nano::config::{VfsBackendType, VfsDiskConfig};
use nano::vfs::{BackendFactory, DiskBackend, MemoryBackend, VfsBackend, VfsBackendEnum, VfsPath};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a memory backend for testing
fn create_memory_backend() -> VfsBackendEnum {
    VfsBackendEnum::Memory(Arc::new(MemoryBackend::default()))
}

/// Helper to create a disk backend for testing
async fn create_disk_backend() -> (Arc<dyn VfsBackend>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let backend = DiskBackend::new(temp_dir.path()).await.unwrap();
    (Arc::new(backend), temp_dir)
}

#[tokio::test]
async fn test_all_backends_basic_roundtrip() {
    // Test that all backends can write and read back data
    let factories = [("memory", create_memory_backend())];

    for (name, backend) in factories.iter().cloned() {
        let path = VfsPath::new("test::data.txt").unwrap();
        let data = b"roundtrip test data";

        // Write
        backend.write(&path, data).await.unwrap();

        // Read back
        let read_data = backend.read(&path).await.unwrap();
        assert_eq!(
            read_data, data,
            "{} backend should read back written data",
            name
        );

        // Verify exists
        assert!(
            backend.exists(&path).await.unwrap(),
            "{} backend should report file exists",
            name
        );

        // Delete
        backend.delete(&path).await.unwrap();
        assert!(
            !backend.exists(&path).await.unwrap(),
            "{} backend should report file deleted",
            name
        );
    }

    // Test disk backend separately (requires async setup)
    let (disk_backend, _temp) = create_disk_backend().await;
    let path = VfsPath::new("test::disk.txt").unwrap();
    let data = b"disk roundtrip data";

    disk_backend.write(&path, data).await.unwrap();
    let read_data = disk_backend.read(&path).await.unwrap();
    assert_eq!(
        read_data, data,
        "disk backend should read back written data"
    );
}

#[tokio::test]
async fn test_disk_backend_persists_across_instances() {
    // Create a temp directory that persists
    let temp_dir = TempDir::new().unwrap();
    let base_path = temp_dir.path().to_path_buf();

    // Write with first backend instance
    {
        let backend = DiskBackend::new(&base_path).await.unwrap();
        let path = VfsPath::new("test::persist.txt").unwrap();
        backend.write(&path, b"persistent").await.unwrap();
    }

    // Read with second backend instance (same directory)
    {
        let backend = DiskBackend::new(&base_path).await.unwrap();
        let path = VfsPath::new("test::persist.txt").unwrap();
        let data = backend.read(&path).await.unwrap();
        assert_eq!(data, b"persistent");
    }
}

#[tokio::test]
async fn test_backend_factory_creates_correct_types() {
    let factory = BackendFactory::new();

    // Memory backend
    let mem = factory
        .create_backend(VfsBackendType::Memory, None, None)
        .await
        .unwrap();

    let path = VfsPath::new("factory::test.txt").unwrap();
    mem.write(&path, b"factory test").await.unwrap();
    assert_eq!(mem.read(&path).await.unwrap(), b"factory test");

    // Disk backend
    let temp_dir = TempDir::new().unwrap();
    let disk_config = VfsDiskConfig {
        base_path: temp_dir.path().to_str().unwrap().to_string(),
    };

    let disk = factory
        .create_backend(VfsBackendType::Disk, Some(&disk_config), None)
        .await
        .unwrap();

    disk.write(&path, b"disk factory").await.unwrap();
    assert_eq!(disk.read(&path).await.unwrap(), b"disk factory");
}

#[tokio::test]
async fn test_namespace_isolation_across_backends() {
    // Test that namespaces are isolated within the same backend
    let temp_dir = TempDir::new().unwrap();
    let disk_config = VfsDiskConfig {
        base_path: temp_dir.path().to_str().unwrap().to_string(),
    };
    let factory = BackendFactory::new();

    let backend = factory
        .create_backend(VfsBackendType::Disk, Some(&disk_config), None)
        .await
        .unwrap();

    // Write to different namespaces
    let path1 = VfsPath::new("app1::secret.txt").unwrap();
    let path2 = VfsPath::new("app2::secret.txt").unwrap();

    backend.write(&path1, b"app1 data").await.unwrap();
    backend.write(&path2, b"app2 data").await.unwrap();

    // Verify isolation
    assert_eq!(backend.read(&path1).await.unwrap(), b"app1 data");
    assert_eq!(backend.read(&path2).await.unwrap(), b"app2 data");
}

#[tokio::test]
async fn test_disk_backend_quota_limits() {
    use nano::vfs::ResourceLimits;

    let temp_dir = TempDir::new().unwrap();
    let limits = ResourceLimits::test_limits(); // Small limits for testing
    let disk_config = VfsDiskConfig {
        base_path: temp_dir.path().to_str().unwrap().to_string(),
    };
    let factory = BackendFactory::new();

    let backend = factory
        .create_backend_with_limits(VfsBackendType::Disk, Some(&disk_config), None, limits)
        .await
        .unwrap();

    // Try to write file larger than limit (100 bytes)
    let path = VfsPath::new("test::large.txt").unwrap();
    let large_data = vec![0u8; 200];

    let result = backend.write(&path, &large_data).await;
    assert!(result.is_err(), "Should fail with quota exceeded");
}

#[tokio::test]
async fn test_all_backends_handle_empty_files() {
    // Test empty file handling across backends
    let temp_dir = TempDir::new().unwrap();
    let disk_config = VfsDiskConfig {
        base_path: temp_dir.path().to_str().unwrap().to_string(),
    };
    let factory = BackendFactory::new();

    // Test disk backend
    let disk = factory
        .create_backend(VfsBackendType::Disk, Some(&disk_config), None)
        .await
        .unwrap();

    let path = VfsPath::new("test::empty.txt").unwrap();
    disk.write(&path, b"").await.unwrap();
    let data = disk.read(&path).await.unwrap();
    assert!(data.is_empty(), "Empty file should read back as empty");

    // Test memory backend
    let mem = factory
        .create_backend(VfsBackendType::Memory, None, None)
        .await
        .unwrap();

    mem.write(&path, b"").await.unwrap();
    let data = mem.read(&path).await.unwrap();
    assert!(
        data.is_empty(),
        "Empty file in memory should read back as empty"
    );
}
