//! VFS Security Benchmarks
//!
//! Performance and security stress tests for VFS operations.

use std::sync::Arc;
use std::time::Instant;

use nano::runtime::fs_polyfill::set_current_vfs;
use nano::v8::platform;
use nano::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};

fn init_platform() {
    platform::initialize_platform().expect("Failed to initialize V8 platform");
}

/// Benchmark rapid file operations don't bypass security
#[test]
fn bench_rapid_file_operations() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("bench.example.com"),
        Arc::new(MemoryBackend::default()),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Rapid create/delete operations
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const start = performance.now();
        
        // Perform many file operations
        for (let i = 0; i < 100; i++) {
            fs.writeFileSync('/test-' + i + '.txt', 'data-' + i);
        }
        
        for (let i = 0; i < 100; i++) {
            fs.readFileSync('/test-' + i + '.txt', 'utf8');
        }
        
        for (let i = 0; i < 100; i++) {
            fs.unlinkSync('/test-' + i + '.txt');
        }
        
        performance.now() - start
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let duration_ms = result.to_number(scope).unwrap().value();

    println!("100 write/read/delete cycles took: {:.2}ms", duration_ms);

    // Should complete in reasonable time (< 5 seconds for 100 operations)
    assert!(
        duration_ms < 5000.0,
        "Operations should complete in under 5 seconds"
    );
}

/// Test memory usage under high load
#[test]
fn test_memory_under_load() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("bench.example.com"),
        Arc::new(MemoryBackend::default()),
    ));
    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Create and read many files
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const data = new Uint8Array(1024); // 1KB of zeros
        
        // Write 50 1KB files
        for (let i = 0; i < 50; i++) {
            fs.writeFileSync('/memtest-' + i + '.bin', data);
        }
        
        // Read all files
        for (let i = 0; i < 50; i++) {
            fs.readFileSync('/memtest-' + i + '.bin');
        }
        
        // Clean up
        for (let i = 0; i < 50; i++) {
            fs.unlinkSync('/memtest-' + i + '.bin');
        }
        
        'completed'
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    assert_eq!(result_str, "completed", "Memory test should complete");
}

/// Test traversal attempts are consistently blocked under load
#[test]
fn test_traversal_blocked_under_load() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("bench.example.com"),
        Arc::new(MemoryBackend::default()),
    ));

    // Create a legitimate file
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        vfs.write("/legit.txt", b"legitimate file").await.unwrap();
    });

    set_current_vfs(Some(vfs));

    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    // Multiple traversal attempts
    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const results = [];
        
        // Attempt many traversals - all should fail
        for (let i = 0; i < 50; i++) {
            try {
                fs.readFileSync('../'.repeat(i + 1) + 'etc/passwd');
                results.push('success');
            } catch (e) {
                results.push(e.code);
            }
        }
        
        // Verify legit file still accessible
        const legit = fs.readFileSync('/legit.txt', 'utf8');
        
        results.filter(r => r === 'EINVAL').length + ':' + legit
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);

    let parts: Vec<&str> = result_str.split(':').collect();
    let blocked_count: i32 = parts[0].parse().unwrap();
    let legit_content = parts[1];

    assert_eq!(
        blocked_count, 50,
        "All 50 traversal attempts should be blocked"
    );
    assert_eq!(
        legit_content, "legitimate file",
        "Legitimate file should still be accessible"
    );
}

/// Benchmark security check performance
#[test]
fn bench_security_validation() {
    init_platform();

    let vfs = Arc::new(IsolateVfs::new(
        VfsNamespace::from_hostname("bench.example.com"),
        Arc::new(MemoryBackend::default()),
    ));
    set_current_vfs(Some(vfs));

    let start = Instant::now();

    // Run 1000 path validation checks through JS
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = &mut v8::HandleScope::new(&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    nano::runtime::fs_polyfill::bind_fs_polyfill(scope, context);

    let code = v8::String::new(
        scope,
        "
        const fs = require('fs');
        const start = performance.now();
        
        // Mix of valid and invalid paths
        const paths = [
            '/valid.txt',
            '../traversal',
            'normal/path.txt',
            '../../etc/passwd',
            '/unicode-文件.txt',
            'file\\x00.txt',
            '/spaces in file.txt',
            'foo/../bar'
        ];
        
        for (let i = 0; i < 125; i++) {
            paths.forEach(p => {
                try {
                    fs.existsSync(p);
                } catch (e) {
                    // Expected for some paths
                }
            });
        }
        
        performance.now() - start
    ",
    )
    .unwrap();

    let script = v8::Script::compile(scope, code, None).unwrap();
    let result = script.run(scope).unwrap();
    let js_duration_ms = result.to_number(scope).unwrap().value();

    let total_duration = start.elapsed();

    println!(
        "1000 path validations took: {:?} (JS time: {:.2}ms)",
        total_duration, js_duration_ms
    );

    // Should be reasonably fast
    assert!(
        total_duration.as_secs() < 5,
        "Security checks should complete in under 5 seconds"
    );
}
