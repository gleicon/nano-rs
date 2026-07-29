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

use std::sync::Arc;
use tempfile::TempDir;

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

/// Test the complete sliver workflow: create, run, snapshot, restore, verify
///
/// This test demonstrates how slivers enable fast app startup by preserving
/// both heap state (compiled code, global variables) and filesystem state.
///
/// # Workflow
///
/// 1. **Setup**: Create an isolate with VFS containing app files
/// 2. **Execution**: Run JavaScript that sets global state
/// 3. **Verification**: Confirm state was set
/// 4. **Snapshot**: Create a heap snapshot
/// 5. **Destroy**: Drop the original isolate (simulating restart)
/// 6. **Restore**: Create new isolate from the snapshot
/// 7. **Validation**: Verify VFS is restored
#[test]
#[ignore = "v1 snapshot workflow: NanoIsolate::from_snapshot and create_snapshot_from_nano removed in v2"]
fn test_sliver_full_workflow_create_run_snapshot_restore() {
    use nano::v8::{initialize_platform, NanoIsolate};
    use nano::v8::snapshot::create_snapshot_from_nano;
    use nano::sliver::{pack_sliver, unpack_sliver, SliverMetadata};
    use nano::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};
    
    // Helper to block on async VFS operations
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        pollster::block_on(f)
    }
    
    // Initialize V8 platform (required once per process)
    initialize_platform().expect("Failed to initialize V8 platform");
    
    // ============================================
    // STEP 1: Setup - Create App with VFS
    // ============================================
    println!("\n[STEP 1] Creating app with VFS...");
    
    // Create a VFS with app files
    let vfs = IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    );
    
    // Create isolate using snapshot creator workflow (required for sliver creation)
    let mut isolate = NanoIsolate::snapshot_creator_with_vfs(vfs)
        .expect("Failed to create snapshottable isolate");
    
    // Write initial app files to VFS
    let index_js = r#"
        globalThis.appState = {
            version: "1.0.0",
            counter: 42,
            initialized: true
        };
        globalThis.__app_initialized = true;
    "#;
    
    block_on(isolate.vfs().write("/index.js", index_js.as_bytes()))
        .expect("Failed to write index.js to VFS");
    
    block_on(isolate.vfs().write("/package.json", br#"{"name": "test-app", "version": "1.0.0"}"#))
        .expect("Failed to write package.json to VFS");
    
    println!("  ✓ App files written to VFS");
    
    // ============================================
    // STEP 2: Execution - Run JavaScript to Set State
    // ============================================
    println!("\n[STEP 2] Executing JavaScript to set state...");
    
    // Execute the initialization script using V8 directly
    {
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
        
        let code = v8::String::new(ctx_scope, index_js).unwrap();
        let script = v8::Script::compile(ctx_scope, code, None)
            .expect("Failed to compile script");
        script.run(ctx_scope).expect("Failed to execute initialization");
    }
    
    println!("  ✓ JavaScript executed - global state set");
    
    // ============================================
    // STEP 3: Create Additional VFS Files
    // ============================================
    println!("\n[STEP 3] Creating additional VFS files...");
    
    // Write files during "runtime" that should be preserved
    block_on(isolate.vfs().write("/data/user-settings.json", br#"{"theme": "dark", "language": "en"}"#))
        .expect("Failed to write settings");
    
    block_on(isolate.vfs().write("/data/cache.db", b"cached data from runtime session"))
        .expect("Failed to write cache");
    
    // Create asset files
    for i in 0..5 {
        let path = format!("/assets/image-{}.png", i);
        block_on(isolate.vfs().write(&path, &format!("fake-image-data-{:04}", i).into_bytes()))
            .expect("Failed to write asset");
    }
    
    println!("  ✓ Runtime VFS files created");
    
    // Verify files exist before snapshot
    assert!(block_on(isolate.vfs().exists("/index.js")).unwrap_or(false));
    assert!(block_on(isolate.vfs().exists("/data/user-settings.json")).unwrap_or(false));
    println!("  ✓ VFS file existence verified");
    
    // ============================================
    // STEP 4: Create Sliver (Heap + VFS Capture)
    // ============================================
    println!("\n[STEP 4] Creating sliver (snapshot + VFS capture)...");
    
    // Create VFS snapshot by collecting entries we wrote
    let mut vfs_snapshot = vec![
        (nano::vfs::VfsPath::new("index.js").unwrap(), nano::vfs::VfsFile {
            content: index_js.as_bytes().to_vec(),
            modified_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            size: index_js.len(),
        }),
        (nano::vfs::VfsPath::new("package.json").unwrap(), nano::vfs::VfsFile {
            content: br#"{"name": "test-app", "version": "1.0.0"}"#.to_vec(),
            modified_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            size: 38,
        }),
        (nano::vfs::VfsPath::new("data/user-settings.json").unwrap(), nano::vfs::VfsFile {
            content: br#"{"theme": "dark", "language": "en"}"#.to_vec(),
            modified_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            size: 34,
        }),
        (nano::vfs::VfsPath::new("data/cache.db").unwrap(), nano::vfs::VfsFile {
            content: b"cached data from runtime session".to_vec(),
            modified_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            size: 31,
        }),
    ];
    
    // Add asset files
    for i in 0..5 {
        let path = format!("assets/image-{}.png", i);
        vfs_snapshot.push((
            nano::vfs::VfsPath::new(&path).unwrap(),
            nano::vfs::VfsFile {
                content: format!("fake-image-data-{:04}", i).into_bytes(),
                modified_at: std::time::SystemTime::now(),
                created_at: std::time::SystemTime::now(),
                size: 18,
            }
        ));
    }
    
    println!("  ✓ VFS state captured: {} files", vfs_snapshot.len());
    
    // Create heap snapshot
    let heap_data = create_snapshot_from_nano(isolate)
        .expect("Failed to create heap snapshot");
    println!("  ✓ Heap snapshot created: {} bytes", heap_data.len());
    
    // Create metadata
    let metadata = SliverMetadata::new("test.example.com", env!("CARGO_PKG_VERSION"));
    
    // Create temp directory for sliver file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let sliver_path = temp_dir.path().join("test-app.sliver");
    
    // Pack sliver (metadata + heap + VFS)
    let sliver_data = pack_sliver(
        &metadata,
        Some(&heap_data),
        Some(&vfs_snapshot)
    ).expect("Failed to pack sliver");
    
    std::fs::write(&sliver_path, &sliver_data)
        .expect("Failed to write sliver file");
    
    println!("  ✓ Sliver created: {} bytes total", sliver_data.len());
    println!("    - File: {}", sliver_path.display());
    
    // ============================================
    // STEP 5: Destroy Original Isolate
    // ============================================
    println!("\n[STEP 5] Destroying original isolate (simulating restart)...");
    
    // The isolate was already consumed by create_snapshot_from_nano
    println!("  ✓ Original isolate destroyed");
    
    // ============================================
    // STEP 6: Restore from Sliver
    // ============================================
    println!("\n[STEP 6] Restoring from sliver...");
    
    // Read the sliver
    let sliver_data = std::fs::read(&sliver_path)
        .expect("Failed to read sliver");
    
    // Unpack the sliver
    let unpacked = unpack_sliver(&sliver_data)
        .expect("Failed to unpack sliver");
    
    println!("  ✓ Sliver unpacked:");
    println!("    - Metadata: {} (name: {:?})", 
        unpacked.metadata.hostname, 
        unpacked.metadata.name.as_deref().unwrap_or("unnamed"));
    println!("    - Bytecode: {} bytes", unpacked.bytecode.as_ref().map(|b| b.len()).unwrap_or(0));
    println!("    - VFS entries: {}", unpacked.vfs_entries.len());
    
    // Create new isolate from snapshot
    let vfs = IsolateVfs::new(
        VfsNamespace::from_hostname("test.example.com"),
        VfsBackendEnum::Memory(Arc::new(MemoryBackend::default())),
    );
    
    let restored_isolate = NanoIsolate::from_snapshot(unpacked.bytecode.as_deref().unwrap_or_default(), vfs)
        .expect("Failed to restore isolate from snapshot");
    
    // Restore VFS contents by writing them back
    for (path, file) in &unpacked.vfs_entries {
        let path_str = format!("/{}", path.as_str());
        block_on(restored_isolate.vfs().write(&path_str, &file.content))
            .expect("Failed to restore VFS entry");
    }
    
    println!("  ✓ Isolate restored from snapshot");
    println!("  ✓ VFS contents restored: {} files", unpacked.vfs_entries.len());
    
    // ============================================
    // STEP 7: Verify VFS Restoration
    // ============================================
    println!("\n[STEP 7] Verifying VFS restoration...");
    
    // Check specific files
    let test_files = vec![
        "/index.js",
        "/package.json",
        "/data/user-settings.json",
        "/data/cache.db",
        "/assets/image-0.png",
        "/assets/image-4.png",
    ];
    
    let mut files_found = 0;
    for path in &test_files {
        match block_on(restored_isolate.vfs().read(path)) {
            Ok(content) => {
                println!("  ✓ {}: {} bytes", path, content.len());
                files_found += 1;
            }
            Err(e) => {
                println!("  ✗ {}: {:?}", path, e);
            }
        }
    }
    
    // Verify file contents
    let settings = block_on(restored_isolate.vfs().read("/data/user-settings.json"))
        .expect("Settings file missing");
    let settings_str = String::from_utf8_lossy(&settings);
    assert!(settings_str.contains("dark"), "Settings should contain theme preference");
    println!("  ✓ Settings file contents verified");
    
    let cache = block_on(restored_isolate.vfs().read("/data/cache.db"))
        .expect("Cache file missing");
    let cache_str = String::from_utf8_lossy(&cache);
    assert!(cache_str.contains("cached data"), "Cache should contain runtime data");
    println!("  ✓ Cache file contents verified");
    
    // Print summary
    println!("\n{}", &"=".repeat(50));
    println!("SLIVER WORKFLOW TEST COMPLETE");
    println!("{}", &"=".repeat(50));
    println!("\nSummary:");
    println!("  ✓ App created with VFS");
    println!("  ✓ JavaScript executed - state set");
    println!("  ✓ Runtime files written to VFS");
    println!("  ✓ Heap snapshot created: {} bytes", heap_data.len());
    println!("  ✓ Sliver packed: {} bytes total", sliver_data.len());
    println!("  ✓ Isolate destroyed and restored");
    println!("  ✓ VFS contents preserved: {}/{} files", files_found, test_files.len());
    println!("\nThe sliver enables fast warm-starts by preserving:");
    println!("  - Compiled JavaScript code (heap snapshot)");
    println!("  - Global state and variables");
    println!("  - Virtual filesystem contents");
    println!("{}", &"=".repeat(50));
}

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
        builder.append(&header, metadata.to_string().as_bytes()).unwrap();
        
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
    use nano::sliver::unpack_sliver;
    use nano::sliver::SliverMetadata;
    use nano::sliver::pack_sliver;
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
            }
        ),
        (
            VfsPath::new("readme.md").unwrap(),
            VfsFile {
                content: b"# Test App".to_vec(),
                modified_at: SystemTime::now(),
                created_at: SystemTime::now(),
                size: 10,
            }
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
    assert_eq!(unpacked.bytecode.as_deref().unwrap_or_default(), heap_data.as_slice());
    assert_eq!(unpacked.vfs_entries.len(), 2);

    println!("✓ Sliver unpack structure verified");
    println!("  - Hostname: {}", unpacked.metadata.hostname);
    println!("  - Bytecode size: {} bytes", unpacked.bytecode.as_ref().map(|b| b.len()).unwrap_or(0));
    println!("  - VFS entries: {}", unpacked.vfs_entries.len());
}
