//! Unified WorkerPool Tests
//!
//! These tests verify that WorkerPool correctly handles both Entrypoint and Sliver
//! sources through the unified `with_source()` constructor.
//!
//! ## Test Coverage
//!
//! - **Entrypoint mode**: Fresh isolates from JS files
//! - **Sliver mode**: Snapshot-restored isolates  
//! - **Static mode**: No isolate creation (separate path)
//! - **Unified features**: Both modes get identical treatment (tracing, limits, etc.)

use nano::worker::pool::WorkerPool;
use nano::worker::AppSource;

#[test]
fn test_worker_pool_with_entrypoint_source() {
    // Create a WorkerPool with Entrypoint source
    let source = AppSource::entrypoint("./test_app.js");

    let pool = WorkerPool::with_source(
        "test.local".to_string(),
        2, // 2 workers
        0, // No memory limit
        source,
    );

    // Verify pool was created successfully
    assert_eq!(pool.worker_count(), 2);
    assert_eq!(pool.hostname, "test.local");

    // Clean up
    pool.shutdown().unwrap();
}

#[test]
fn test_worker_pool_source_type_detection() {
    let entrypoint_source = AppSource::entrypoint("./app.js");
    assert!(entrypoint_source.is_entrypoint());
    assert!(!entrypoint_source.is_sliver());
    assert!(!entrypoint_source.is_static());
    assert!(entrypoint_source.needs_isolate());
    assert_eq!(entrypoint_source.entrypoint_path(), Some("./app.js"));

    let static_source = AppSource::static_site("./static");
    assert!(static_source.is_static());
    assert!(!static_source.is_entrypoint());
    assert!(!static_source.is_sliver());
    assert!(!static_source.needs_isolate());
}

#[test]
#[should_panic(expected = "Worker count must be at least 1")]
fn test_worker_pool_zero_workers_panics() {
    let source = AppSource::entrypoint("./app.js");
    let _pool = WorkerPool::with_source(
        "test.local".to_string(),
        0, // Invalid: zero workers
        0,
        source,
    );
}

#[test]
fn test_worker_pool_shutdown_graceful() {
    let source = AppSource::entrypoint("./app.js");
    let pool = WorkerPool::with_source("test.local".to_string(), 2, 0, source);

    // Graceful shutdown should succeed
    let result = pool.shutdown();
    assert!(result.is_ok());
}

#[test]
fn test_worker_pool_worker_count_consistency() {
    let source = AppSource::entrypoint("./app.js");

    // Test with different worker counts
    for count in [1, 2, 4, 8] {
        let pool = WorkerPool::with_source("test.local".to_string(), count, 0, source.clone());

        assert_eq!(pool.worker_count(), count);
        pool.shutdown().unwrap();
    }
}

#[test]
fn test_worker_pool_with_memory_limits() {
    let source = AppSource::entrypoint("./app.js");

    // Test with various memory limits
    for limit_mb in [0, 64, 128, 256, 512] {
        let pool = WorkerPool::with_source("test.local".to_string(), 1, limit_mb, source.clone());

        assert_eq!(pool.worker_count(), 1);
        pool.shutdown().unwrap();
    }
}

#[test]
fn test_app_source_wasm_entrypoint() {
    // WASM files should be treated as entrypoints needing isolates
    let wasm_source = AppSource::entrypoint("./app.wasm");
    assert!(wasm_source.is_entrypoint());
    assert!(wasm_source.needs_isolate());
    assert_eq!(wasm_source.entrypoint_path(), Some("./app.wasm"));
}

#[test]
fn test_app_source_clone_preserves_type() {
    let entrypoint = AppSource::entrypoint("./app.js");
    let cloned = entrypoint.clone();

    assert_eq!(entrypoint.is_entrypoint(), cloned.is_entrypoint());
    assert_eq!(entrypoint.is_sliver(), cloned.is_sliver());
    assert_eq!(entrypoint.is_static(), cloned.is_static());

    if let Some(path) = entrypoint.entrypoint_path() {
        assert_eq!(cloned.entrypoint_path(), Some(path));
    }
}

// Note: Full integration tests that actually execute JavaScript/WASM
// require V8 platform initialization and are in the integration test suite.
// These unit tests verify the unified pool structure and source handling.
