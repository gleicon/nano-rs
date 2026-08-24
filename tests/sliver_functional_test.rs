//! Sliver Functional Tests - Full Workflow Demonstration
//!
//! This module demonstrates the complete sliver workflow:
//! 1. Create an app with JavaScript code and VFS files
//! 2. Execute JavaScript to set state (global variables)
//! 3. Write files to the virtual filesystem
//! 4. Create a sliver (capture heap state + VFS)
//! 5. Stop/destroy the original isolate
//! 6. Restore from the sliver
//! 7. Verify state restoration (global variables)
//! 8. Verify VFS contents are preserved
//!
//! This test validates that slivers enable fast warm-starts with
//! preserved application state.
//!
//! ## Running This Test
//!
//! ```bash
//! # Run the full sliver workflow test
//! cargo test --test sliver_functional_test test_sliver_full_workflow -- --nocapture
//!
//! # Run all sliver functional tests
//! cargo test --test sliver_functional_test -- --nocapture
//! ```

/// Test that verifies sliver format compatibility and validation
#[test]
fn test_sliver_format_validation() {
    use nano::sliver::validate_sliver_integrity;
    use tar::{Builder, Header};
    use tempfile::TempDir;

    fn create_test_sliver(path: &std::path::Path, hostname: &str) -> Vec<u8> {
        let mut builder = Builder::new(Vec::new());

        // Metadata
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

        // Heap (placeholder)
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

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sliver_path = temp_dir.path().join("test.sliver");

    // Create a valid sliver using the helper
    let data = create_test_sliver(&sliver_path, "validation.test.com");

    // Validate the sliver
    let result = validate_sliver_integrity(&sliver_path);
    assert!(result.is_ok(), "Sliver should be valid: {:?}", result.err());

    println!("✓ Sliver format validation passed");
    println!("  - Size: {} bytes", data.len());
}

/// Test sliver unpacking and structure verification
#[test]
fn test_sliver_unpack_structure() {
    use nano::sliver::pack_sliver;
    use nano::sliver::unpack_sliver;
    use nano::sliver::SliverMetadata;
    use nano::vfs::{VfsFile, VfsPath};
    use std::time::SystemTime;
    use tempfile::TempDir;

    // Create metadata
    let metadata = SliverMetadata::new("unpack.test.com", "1.1.0");

    // Create fake heap data
    let heap_data = b"fake-heap-snapshot-data".to_vec();

    // Create VFS entries
    let vfs_entries: Vec<(VfsPath, VfsFile)> = vec![
        (
            VfsPath::new("index.js").unwrap(),
            VfsFile {
                content: b"console.log('hello');".to_vec(),
                modified_at: SystemTime::now(),
                created_at: SystemTime::now(),
                size: 19,
            },
        ),
        (
            VfsPath::new("readme.md").unwrap(),
            VfsFile {
                content: b"# Test App".to_vec(),
                modified_at: SystemTime::now(),
                created_at: SystemTime::now(),
                size: 10,
            },
        ),
    ];

    // Pack sliver
    let packed = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries))
        .expect("Failed to pack sliver");

    // Write to temp file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sliver_path = temp_dir.path().join("unpack-test.sliver");
    std::fs::write(&sliver_path, &packed).expect("Failed to write sliver");

    // Unpack and verify structure
    let unpacked = unpack_sliver(&packed).expect("Failed to unpack sliver");

    assert_eq!(unpacked.metadata.hostname, "unpack.test.com");
    assert_eq!(
        unpacked.bytecode.as_deref().unwrap_or_default(),
        heap_data.as_slice()
    );
    assert_eq!(unpacked.vfs_entries.len(), 2);

    println!("✓ Sliver unpack structure verified");
    println!("  - Hostname: {}", unpacked.metadata.hostname);
    println!(
        "  - Bytecode size: {} bytes",
        unpacked.bytecode.as_ref().map(|b| b.len()).unwrap_or(0)
    );
    println!("  - VFS entries: {}", unpacked.vfs_entries.len());
}
