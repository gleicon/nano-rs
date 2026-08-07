//! Unit tests for MemoryBackend — extracted from src/vfs/memory.rs
use nano::vfs::{
    MemoryBackend, ResourceLimits, VfsBackend, VfsBackendEnum, VfsError, VfsFile, VfsPath,
};

#[tokio::test]
async fn test_memory_backend_basic() {
    let backend = MemoryBackend::default();
    let path = VfsPath::new("test.txt").unwrap();

    backend.write(&path, b"hello world").await.unwrap();

    let content = backend.read(&path).await.unwrap();
    assert_eq!(content, b"hello world");

    assert!(backend.exists(&path).await.unwrap());

    backend.delete(&path).await.unwrap();
    assert!(!backend.exists(&path).await.unwrap());
}

#[tokio::test]
async fn test_memory_backend_empty_file() {
    let backend = MemoryBackend::default();
    let path = VfsPath::new("empty.txt").unwrap();

    backend.write(&path, b"").await.unwrap();

    let content = backend.read(&path).await.unwrap();
    assert!(content.is_empty());

    let meta = backend.metadata(&path).await.unwrap();
    assert_eq!(meta.size, 0);
}

#[tokio::test]
async fn test_memory_backend_not_found() {
    let backend = MemoryBackend::default();
    let path = VfsPath::new("nonexistent.txt").unwrap();

    let result = backend.read(&path).await;
    assert!(matches!(result, Err(VfsError::NotFound { .. })));
    assert_eq!(result.unwrap_err().code(), "ENOENT");
}

#[tokio::test]
async fn test_memory_backend_update() {
    let backend = MemoryBackend::default();
    let path = VfsPath::new("update.txt").unwrap();

    backend.write(&path, b"first").await.unwrap();
    let meta1 = backend.metadata(&path).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    backend.write(&path, b"second version").await.unwrap();
    let meta2 = backend.metadata(&path).await.unwrap();

    assert_eq!(meta1.created_at, meta2.created_at);
    assert!(meta2.modified_at > meta1.modified_at);
    assert_eq!(meta2.content, b"second version");
}

#[tokio::test]
async fn test_memory_backend_quota_file_size() {
    let limits = ResourceLimits {
        file_size_bytes_max: 100,
        ..Default::default()
    };
    let backend = MemoryBackend::with_limits(limits);
    let path = VfsPath::new("large.txt").unwrap();

    backend.write(&path, &[0u8; 50]).await.unwrap();

    let result = backend.write(&path, &[0u8; 101]).await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "file_size")
    );
    assert_eq!(result.unwrap_err().code(), "EQUOTA");
}

#[tokio::test]
async fn test_memory_backend_quota_file_count() {
    let limits = ResourceLimits {
        files_count_max: 3,
        ..Default::default()
    };
    let backend = MemoryBackend::with_limits(limits);

    for i in 0..3 {
        let path = VfsPath::new(&format!("file{}.txt", i)).unwrap();
        backend.write(&path, b"content").await.unwrap();
    }

    let path = VfsPath::new("file3.txt").unwrap();
    let result = backend.write(&path, b"content").await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "file_count")
    );
}

#[tokio::test]
async fn test_memory_backend_quota_total_storage() {
    let limits = ResourceLimits {
        total_storage_bytes_max: 200,
        file_size_bytes_max: 100,
        files_count_max: 10,
    };
    let backend = MemoryBackend::with_limits(limits);

    backend
        .write(&VfsPath::new("file1.txt").unwrap(), &[0u8; 100])
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("file2.txt").unwrap(), &[0u8; 100])
        .await
        .unwrap();

    let result = backend
        .write(&VfsPath::new("file3.txt").unwrap(), &[0u8; 10])
        .await;
    assert!(
        matches!(result, Err(VfsError::QuotaExceeded { ref resource, .. }) if resource == "total_storage")
    );
}

#[tokio::test]
async fn test_memory_backend_counters() {
    let backend = MemoryBackend::default();
    let path1 = VfsPath::new("file1.txt").unwrap();
    let path2 = VfsPath::new("file2.txt").unwrap();

    assert_eq!(backend.len(), 0);
    assert!(backend.is_empty());
    assert_eq!(backend.current_usage(), (0, 0));

    backend.write(&path1, &[0u8; 100]).await.unwrap();
    assert_eq!(backend.len(), 1);
    assert_eq!(backend.current_usage(), (1, 100));

    backend.write(&path2, &[0u8; 50]).await.unwrap();
    assert_eq!(backend.len(), 2);
    assert_eq!(backend.current_usage(), (2, 150));

    backend.write(&path1, &[0u8; 80]).await.unwrap();
    assert_eq!(backend.current_usage(), (2, 130));

    backend.delete(&path2).await.unwrap();
    assert_eq!(backend.len(), 1);
    assert_eq!(backend.current_usage(), (1, 80));

    backend.clear();
    assert_eq!(backend.len(), 0);
    assert!(backend.is_empty());
    assert_eq!(backend.current_usage(), (0, 0));
}

#[tokio::test]
async fn test_memory_backend_concurrent_writes() {
    let backend = VfsBackendEnum::memory(MemoryBackend::default());
    let mut handles = vec![];

    for i in 0..10 {
        let backend = backend.clone();
        let handle = tokio::spawn(async move {
            let path = VfsPath::new(&format!("file{}.txt", i)).unwrap();
            backend.write(&path, &[i as u8; 100]).await.unwrap();
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    assert_eq!(backend.len(), 10);
    for i in 0..10 {
        let path = VfsPath::new(&format!("file{}.txt", i)).unwrap();
        assert!(backend.exists(&path).await.unwrap());
    }
}

#[tokio::test]
async fn test_snapshot_entries() {
    let backend = MemoryBackend::default();

    backend
        .write(&VfsPath::new("file1.txt").unwrap(), b"content1")
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("dir/file2.txt").unwrap(), b"content2")
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("empty.txt").unwrap(), b"")
        .await
        .unwrap();

    let entries = backend.snapshot_entries();
    assert_eq!(entries.len(), 3);

    let paths: Vec<_> = entries
        .iter()
        .map(|(p, _)| p.as_str().to_string())
        .collect();
    assert!(paths.contains(&"file1.txt".to_string()));
    assert!(paths.contains(&"dir/file2.txt".to_string()));
    assert!(paths.contains(&"empty.txt".to_string()));

    let file1 = entries
        .iter()
        .find(|(p, _)| p.as_str() == "file1.txt")
        .unwrap();
    assert_eq!(file1.1.content, b"content1");
}

#[tokio::test]
async fn test_restore_from_snapshot() {
    let backend = MemoryBackend::default();

    backend
        .write(&VfsPath::new("old.txt").unwrap(), b"old content")
        .await
        .unwrap();
    assert_eq!(backend.len(), 1);

    let entries = vec![
        (
            VfsPath::new("new1.txt").unwrap(),
            VfsFile::new(b"new content 1".to_vec()),
        ),
        (
            VfsPath::new("new2.txt").unwrap(),
            VfsFile::new(b"new content 2".to_vec()),
        ),
    ];

    backend.restore_from_snapshot(&entries);

    assert!(!backend
        .exists(&VfsPath::new("old.txt").unwrap())
        .await
        .unwrap());
    assert_eq!(backend.len(), 2);

    let content1 = backend
        .read(&VfsPath::new("new1.txt").unwrap())
        .await
        .unwrap();
    assert_eq!(content1, b"new content 1");
    assert_eq!(backend.current_usage(), (2, 26));
}

#[tokio::test]
async fn test_snapshot_roundtrip() {
    let backend = MemoryBackend::default();

    backend
        .write(
            &VfsPath::new("config.json").unwrap(),
            b"{\"key\": \"value\"}",
        )
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("data/users.txt").unwrap(), b"user1\nuser2")
        .await
        .unwrap();

    let entries = backend.snapshot_entries();

    let new_backend = MemoryBackend::default();
    new_backend.restore_from_snapshot(&entries);

    assert_eq!(new_backend.len(), 2);

    let config = new_backend
        .read(&VfsPath::new("config.json").unwrap())
        .await
        .unwrap();
    assert_eq!(config, b"{\"key\": \"value\"}");

    let users = new_backend
        .read(&VfsPath::new("data/users.txt").unwrap())
        .await
        .unwrap();
    assert_eq!(users, b"user1\nuser2");
}

#[tokio::test]
async fn test_list_dir_returns_immediate_children_only() {
    let backend = MemoryBackend::default();

    backend
        .write(&VfsPath::new("a/b/c/deep.txt").unwrap(), b"deep")
        .await
        .unwrap();

    let a_dir = VfsPath::new("a").unwrap();
    let a_entries = backend.list_dir(&a_dir).await.unwrap();
    assert_eq!(a_entries.len(), 1);
    assert!(a_entries[0].as_str().contains("b"));

    let b_dir = VfsPath::new("a/b").unwrap();
    let b_entries = backend.list_dir(&b_dir).await.unwrap();
    assert_eq!(b_entries.len(), 1);
    assert!(b_entries[0].as_str().contains("c"));
}

#[tokio::test]
async fn test_list_dir_nested_directory() {
    let backend = MemoryBackend::default();

    backend
        .write(&VfsPath::new("data/file1.txt").unwrap(), b"content1")
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("data/file2.txt").unwrap(), b"content2")
        .await
        .unwrap();
    backend
        .write(&VfsPath::new("data/subdir/nested.txt").unwrap(), b"nested")
        .await
        .unwrap();

    let data_dir = VfsPath::new("data").unwrap();
    let entries = backend.list_dir(&data_dir).await.unwrap();

    assert_eq!(entries.len(), 3);
}
