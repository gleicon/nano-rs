//! Integration Tests for Sliver Edge Cases
//!
//! Tests error paths, concurrent operations, and large file handling.

use std::io::Write;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// Helper: Create a minimal valid sliver
fn create_test_sliver(path: &std::path::Path, hostname: &str) -> Vec<u8> {
    use tar::{Builder, Header};

    let mut builder = Builder::new(Vec::new());

    // Metadata - use correct filename from sliver format
    let metadata = serde_json::json!({
        "format_version": "1.0",
        "hostname": hostname,
        "name": hostname.replace(".", "-"),
        "created_at": "2026-04-20T00:00:00Z",
        "nano_version": "1.1.0"
    });

    let mut header = Header::new_gnu();
    header.set_path("meta.json").unwrap();
    header.set_size(metadata.to_string().len() as u64);
    header.set_cksum();
    builder
        .append(&header, metadata.to_string().as_bytes())
        .unwrap();

    // Heap
    let heap = vec![0u8; 1024];
    let mut header = Header::new_gnu();
    header.set_path("heap.bin").unwrap();
    header.set_size(heap.len() as u64);
    header.set_cksum();
    builder.append(&header, heap.as_slice()).unwrap();

    let data = builder.into_inner().unwrap();
    std::fs::write(path, &data).unwrap();
    data
}

/// Helper: Create a corrupted sliver
fn create_corrupted_sliver(path: &std::path::Path, corruption_type: &str) {
    use tar::{Builder, Header};

    match corruption_type {
        "invalid_tar" => {
            std::fs::write(path, "not a tar file").unwrap();
        }
        "missing_metadata" => {
            let mut builder = Builder::new(Vec::new());
            let heap = vec![0u8; 1024];
            let mut header = Header::new_gnu();
            header.set_path("heap.bin").unwrap();
            header.set_size(heap.len() as u64);
            header.set_cksum();
            builder.append(&header, heap.as_slice()).unwrap();
            let data = builder.into_inner().unwrap();
            std::fs::write(path, data).unwrap();
        }
        "missing_heap" => {
            let mut builder = Builder::new(Vec::new());
            let metadata = r#"{"format_version":"1.0","hostname":"test.example.com","name":"test","created_at":"2026-04-20T00:00:00Z","nano_version":"1.1.0"}"#;
            let mut header = Header::new_gnu();
            header.set_path("meta.json").unwrap(); // Use correct filename
            header.set_size(metadata.len() as u64);
            header.set_cksum();
            builder.append(&header, metadata.as_bytes()).unwrap();
            let data = builder.into_inner().unwrap();
            std::fs::write(path, data).unwrap();
        }
        "invalid_json" => {
            let mut builder = Builder::new(Vec::new());
            let bad_json = "not valid json";
            let mut header = Header::new_gnu();
            header.set_path("meta.json").unwrap(); // Use correct filename
            header.set_size(bad_json.len() as u64);
            header.set_cksum();
            builder.append(&header, bad_json.as_bytes()).unwrap();
            let heap = vec![0u8; 1024];
            let mut header = Header::new_gnu();
            header.set_path("heap.bin").unwrap();
            header.set_size(heap.len() as u64);
            header.set_cksum();
            builder.append(&header, heap.as_slice()).unwrap();
            let data = builder.into_inner().unwrap();
            std::fs::write(path, data).unwrap();
        }
        _ => panic!("Unknown corruption type: {}", corruption_type),
    }
}

mod tests {
    use super::*;

    #[test]
    fn test_sliver_not_found_gives_helpful_error() {
        let temp_dir = TempDir::new().unwrap();
        let nonexistent = temp_dir.path().join("nonexistent.sliver");

        // Attempt to validate nonexistent file
        let result = nano::sliver::validate_sliver_integrity(&nonexistent);

        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        assert!(err_str.contains("not found") || err_str.contains("No such file"));
    }

    #[test]
    fn test_corrupted_sliver_detection() {
        let temp_dir = TempDir::new().unwrap();
        let bad_sliver = temp_dir.path().join("bad.sliver");

        // Create invalid file (not a tar)
        std::fs::write(&bad_sliver, "not a valid sliver").unwrap();

        let result = nano::sliver::validate_sliver_integrity(&bad_sliver);
        assert!(result.is_err());
        // Should fail due to invalid tar structure
    }

    #[test]
    fn test_missing_metadata_detection() {
        let temp_dir = TempDir::new().unwrap();
        let bad_sliver = temp_dir.path().join("no-meta.sliver");

        create_corrupted_sliver(&bad_sliver, "missing_metadata");

        let result = nano::sliver::validate_sliver_integrity(&bad_sliver);
        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        assert!(err_str.contains("metadata") || err_str.contains("Missing"));
    }

    #[test]
    fn test_missing_heap_detection() {
        let temp_dir = TempDir::new().unwrap();
        let bad_sliver = temp_dir.path().join("no-heap.sliver");

        create_corrupted_sliver(&bad_sliver, "missing_heap");

        let result = nano::sliver::validate_sliver_integrity(&bad_sliver);
        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        assert!(err_str.contains("heap") || err_str.contains("Missing"));
    }

    #[test]
    fn test_invalid_json_detection() {
        let temp_dir = TempDir::new().unwrap();
        let bad_sliver = temp_dir.path().join("bad-json.sliver");

        create_corrupted_sliver(&bad_sliver, "invalid_json");

        let result = nano::sliver::validate_sliver_integrity(&bad_sliver);
        assert!(result.is_err());
        let err_str = format!("{}", result.unwrap_err());
        // Should detect invalid JSON
    }

    #[test]
    fn test_find_sliver_file_by_name() {
        let temp_dir = TempDir::new().unwrap();
        let sliver_path = temp_dir.path().join("my-sliver.sliver");
        create_test_sliver(&sliver_path, "test.example.com");

        // Change to temp dir and search by name
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(&temp_dir).unwrap();

        let found = nano::sliver::find_sliver_file("my-sliver");

        std::env::set_current_dir(original).unwrap();

        assert!(found.is_some());
    }

    #[test]
    fn test_find_sliver_file_direct_path() {
        let temp_dir = TempDir::new().unwrap();
        let sliver_path = temp_dir.path().join("direct.sliver");
        create_test_sliver(&sliver_path, "direct.example.com");

        let found = nano::sliver::find_sliver_file(sliver_path.to_str().unwrap());

        assert_eq!(found, Some(sliver_path));
    }

    #[test]
    fn test_concurrent_sliver_read() {
        let temp_dir = TempDir::new().unwrap();
        let sliver_path = Arc::new(temp_dir.path().join("shared.sliver"));

        // Create one sliver
        create_test_sliver(&sliver_path, "shared.example.com");

        let mut handles = vec![];

        // Spawn 10 threads reading the same sliver
        for _ in 0..10 {
            let path = Arc::clone(&sliver_path);
            let handle = thread::spawn(move || {
                let data = std::fs::read(&*path).unwrap();
                assert!(!data.is_empty());
                thread::sleep(Duration::from_millis(10));
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_sliver_creation() {
        let temp_dir = Arc::new(TempDir::new().unwrap());
        let mut handles = vec![];

        // Spawn 5 threads creating slivers simultaneously
        for i in 0..5 {
            let temp = Arc::clone(&temp_dir);
            let handle = thread::spawn(move || {
                let sliver_path = temp.path().join(format!("concurrent-{}.sliver", i));
                create_test_sliver(&sliver_path, &format!("app{}.example.com", i));
                assert!(sliver_path.exists());
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all slivers exist
        for i in 0..5 {
            let path = temp_dir.path().join(format!("concurrent-{}.sliver", i));
            assert!(path.exists(), "Sliver {} should exist", i);
        }
    }

    #[test]
    fn test_large_sliver_many_files() {
        use tar::{Builder, Header};

        let temp_dir = TempDir::new().unwrap();
        let sliver_path = temp_dir.path().join("many-files.sliver");

        let mut builder = Builder::new(Vec::new());

        // Metadata - use correct filename
        let metadata = serde_json::json!({
            "format_version": "1.0",
            "hostname": "many.example.com",
            "name": "many-files",
            "created_at": "2026-04-20T00:00:00Z",
            "nano_version": "1.1.0"
        });

        let mut header = Header::new_gnu();
        header.set_path("meta.json").unwrap();
        header.set_size(metadata.to_string().len() as u64);
        header.set_cksum();
        builder
            .append(&header, metadata.to_string().as_bytes())
            .unwrap();

        // Heap
        let heap = vec![0u8; 1024];
        let mut header = Header::new_gnu();
        header.set_path("heap.bin").unwrap();
        header.set_size(heap.len() as u64);
        header.set_cksum();
        builder.append(&header, heap.as_slice()).unwrap();

        // Add 100 small files
        for i in 0..100 {
            let content = format!("File {} content", i);
            let mut header = Header::new_gnu();
            header.set_path(format!("vfs/file-{:03}.txt", i)).unwrap();
            header.set_size(content.len() as u64);
            header.set_cksum();
            builder.append(&header, content.as_bytes()).unwrap();
        }

        let data = builder.into_inner().unwrap();
        std::fs::write(&sliver_path, data).unwrap();

        // Verify it validates correctly
        assert!(nano::sliver::validate_sliver_integrity(&sliver_path).is_ok());

        // Count entries
        let file = std::fs::File::open(&sliver_path).unwrap();
        let mut archive = tar::Archive::new(file);
        let count = archive.entries().unwrap().count();
        assert_eq!(count, 102); // meta.json + heap.bin + 100 files
    }

    #[test]
    fn test_version_compatibility_check() {
        let metadata = nano::sliver::SliverMetadata::new("test.example.com", "1.1.0");

        // Same version should be OK
        let result = nano::sliver::check_version_compatibility(&metadata, "1.1.0");
        assert!(result.is_ok());

        // Different minor version should still be OK
        let result = nano::sliver::check_version_compatibility(&metadata, "1.2.0");
        assert!(result.is_ok());

        // Different major version - should still pass but may warn
        let result = nano::sliver::check_version_compatibility(&metadata, "2.0.0");
        assert!(result.is_ok());
    }
}
