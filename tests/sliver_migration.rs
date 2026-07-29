//! Sliver Migration Test
//!
//! Tests portability of slivers between NANO instances.
//! Simulates creating a sliver on "Instance A", transferring it,
//! and restoring it on "Instance B".

use nano::sliver::{pack_sliver, SliverMetadata, unpack_sliver, validate_sliver};
use nano::vfs::{VfsFile, VfsPath};

/// Test scenario: Pack on Instance A, transfer to Instance B, verify identical
#[test]
fn test_sliver_migration_portability() {
    // === Instance A: Create sliver ===
    let hostname = "migration-test.example.com";
    let mut metadata = SliverMetadata::new(hostname, "1.1.0");
    metadata.name = Some("migration-test".to_string());
    metadata.description = Some("Test sliver for migration".to_string());
    
    // Create heap data (simulating V8 snapshot)
    let heap_data = vec![0xDEu8, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
    
    // Create VFS entries
    let vfs_entries = vec![
        (VfsPath::new("config.json").unwrap(), VfsFile::new(
            br#"{"app": "migration-test", "version": "1.0.0"}"#.to_vec()
        )),
        (VfsPath::new("data/users.csv").unwrap(), VfsFile::new(
            b"id,name,email\n1,Alice,alice@example.com\n2,Bob,bob@example.com".to_vec()
        )),
        (VfsPath::new("assets/logo.png").unwrap(), VfsFile::new(
            vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] // PNG header
        )),
    ];
    
    // Pack sliver on "Instance A"
    let archive_a = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries))
        .expect("Instance A: Failed to pack sliver");
    
    // Validate the created sliver
    validate_sliver(&archive_a)
        .expect("Instance A: Created sliver failed validation");
    
    println!("Instance A: Created sliver with {} bytes", archive_a.len());
    
    // === Transfer: Serialize to bytes (simulates network transfer) ===
    let transferred_bytes = archive_a.clone();
    println!("Transfer: {} bytes transferred", transferred_bytes.len());
    
    // === Instance B: Receive and unpack ===
    let unpacked_b = unpack_sliver(&transferred_bytes)
        .expect("Instance B: Failed to unpack sliver");
    
    println!("Instance B: Unpacked sliver successfully");
    
    // === Verification: All data must match ===
    
    // 1. Verify metadata matches
    assert_eq!(
        unpacked_b.metadata.hostname, 
        hostname, 
        "Hostname mismatch after migration"
    );
    assert_eq!(
        unpacked_b.metadata.format_version, 
        metadata.format_version,
        "Format version mismatch"
    );
    assert_eq!(
        unpacked_b.metadata.name, 
        metadata.name,
        "Sliver name mismatch"
    );
    assert_eq!(
        unpacked_b.metadata.description, 
        metadata.description,
        "Description mismatch"
    );
    
    // 2. Verify bytecode is identical
    assert_eq!(
        unpacked_b.bytecode.as_deref().unwrap_or_default(),
        heap_data.as_slice(),
        "Bytecode mismatch after migration"
    );
    
    // 3. Verify VFS entries match
    assert_eq!(
        unpacked_b.vfs_entries.len(), 
        vfs_entries.len(),
        "VFS entry count mismatch"
    );
    
    for (i, (expected_path, expected_file)) in vfs_entries.iter().enumerate() {
        let (actual_path, actual_file) = &unpacked_b.vfs_entries[i];
        assert_eq!(
            actual_path.as_str(), 
            expected_path.as_str(),
            "VFS path mismatch at index {}", i
        );
        assert_eq!(
            actual_file.content, 
            expected_file.content,
            "VFS content mismatch at path {}", expected_path.as_str()
        );
    }
    
    // 4. Verify total size is preserved
    assert_eq!(
        unpacked_b.total_size(),
        // Calculate expected total size
        {
            let metadata_size = serde_json::to_vec(&metadata).map(|v| v.len()).unwrap_or(0);
            let bytecode_size = heap_data.len();
            let vfs_size: usize = vfs_entries.iter().map(|(_, f)| f.content.len()).sum();
            metadata_size + bytecode_size + vfs_size
        },
        "Total size mismatch"
    );
    
    println!("✓ All verifications passed - sliver migration successful!");
    println!("  - Metadata: ✓");
    println!("  - Bytecode ({} bytes): ✓", unpacked_b.bytecode.as_ref().map(|b| b.len()).unwrap_or(0));
    println!("  - VFS entries ({} files): ✓", unpacked_b.vfs_entries.len());
    println!("  - Total size: {} bytes: ✓", unpacked_b.total_size());
}

/// Test migration with empty VFS (minimum sliver)
#[test]
fn test_sliver_migration_empty_vfs() {
    let metadata = SliverMetadata::new("empty.example.com", "1.1.0");
    let heap_data = vec![0x00u8; 100];
    
    // Pack with no VFS entries
    let archive = pack_sliver(&metadata, Some(&heap_data), None)
        .expect("Failed to pack empty sliver");

    // Transfer
    let unpacked = unpack_sliver(&archive)
        .expect("Failed to unpack empty sliver");

    // Verify
    assert_eq!(unpacked.metadata.hostname, "empty.example.com");
    assert_eq!(unpacked.bytecode.as_deref().unwrap_or_default(), heap_data.as_slice());
    assert!(unpacked.vfs_entries.is_empty());
}

/// Test migration with large VFS (stress test)
#[test]
fn test_sliver_migration_large_vfs() {
    let metadata = SliverMetadata::new("large.example.com", "1.1.0");
    let heap_data = vec![0xABu8; 1024 * 1024]; // 1MB heap
    
    // Create 100 files with 10KB each = ~1MB VFS data
    let vfs_entries: Vec<(VfsPath, VfsFile)> = (0..100)
        .map(|i| {
            let path = VfsPath::new(&format!("data/file{:03}.txt", i)).unwrap();
            let content = vec![0xCDu8; 10 * 1024]; // 10KB per file
            let file = VfsFile::new(content);
            (path, file)
        })
        .collect();
    
    // Pack
    let archive = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries))
        .expect("Failed to pack large sliver");
    
    println!("Large sliver: {} bytes packed", archive.len());
    
    // Transfer
    let unpacked = unpack_sliver(&archive)
        .expect("Failed to unpack large sliver");
    
    // Verify
    assert_eq!(unpacked.vfs_entries.len(), 100);
    assert_eq!(unpacked.bytecode.as_ref().map(|b| b.len()).unwrap_or(0), 1024 * 1024);
    
    // Verify all files
    for i in 0..100 {
        let expected_path = format!("data/file{:03}.txt", i);
        let entry = unpacked.vfs_entries.iter()
            .find(|(p, _)| p.as_str() == expected_path);
        assert!(entry.is_some(), "Missing file: {}", expected_path);
        assert_eq!(entry.unwrap().1.content.len(), 10 * 1024);
    }
    
    println!("✓ Large sliver migration ({} files, {} bytes) successful!", 
        unpacked.vfs_entries.len(), 
        unpacked.total_size()
    );
}

/// Test that corrupted transfer is detected
#[test]
fn test_sliver_migration_corrupted_data() {
    let metadata = SliverMetadata::new("corrupt.example.com", "1.1.0");
    let heap_data = vec![0x00u8; 100];
    
    // Pack
    let archive = pack_sliver(&metadata, Some(&heap_data), None)
        .expect("Failed to pack");
    
    // Corrupt the archive
    let mut corrupted = archive.clone();
    if !corrupted.is_empty() {
        corrupted[0] ^= 0xFF; // Flip bits in first byte
    }
    
    // Attempt to unpack - should fail
    let result = unpack_sliver(&corrupted);
    assert!(result.is_err(), "Should fail to unpack corrupted data");
}

/// Test cross-platform compatibility (paths with different separators)
#[test]
fn test_sliver_migration_cross_platform_paths() {
    let metadata = SliverMetadata::new("cross-platform.example.com", "1.1.0");
    let heap_data = vec![0x00u8; 100];
    
    // Create entries with nested paths
    let vfs_entries = vec![
        (VfsPath::new("config/app.json").unwrap(), VfsFile::new(b"{}".to_vec())),
        (VfsPath::new("data/2024/01/records.csv").unwrap(), VfsFile::new(b"date,value".to_vec())),
        (VfsPath::new("assets/images/logo.png").unwrap(), VfsFile::new(b"PNG".to_vec())),
    ];
    
    // Pack
    let archive = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries))
        .expect("Failed to pack");
    
    // Transfer
    let unpacked = unpack_sliver(&archive)
        .expect("Failed to unpack");
    
    // Verify all paths preserved correctly
    assert_eq!(unpacked.vfs_entries.len(), 3);
    
    let paths: Vec<&str> = unpacked.vfs_entries.iter()
        .map(|(p, _)| p.as_str())
        .collect();
    
    assert!(paths.contains(&"config/app.json"));
    assert!(paths.contains(&"data/2024/01/records.csv"));
    assert!(paths.contains(&"assets/images/logo.png"));
}
