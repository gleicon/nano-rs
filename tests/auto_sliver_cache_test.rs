//! Auto-Sliver Cache Integration Tests
//!
//! Tests multi-process coordination, hot-loading, and cache invalidation.

use nano::sliver::auto_cache::{get_optimized_handler_source, is_sliver_available, SliverCache};
use std::time::Duration;
use tempfile::TempDir;

/// Test cache key generation is deterministic
#[test]
fn test_cache_key_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    let key1 = cache.cache_path("example.com", "/app/index.js");
    let key2 = cache.cache_path("example.com", "/app/index.js");

    assert_eq!(key1, key2, "Cache key should be deterministic");
}

/// Test different hostnames get different cache paths
#[test]
fn test_cache_key_different_hostnames() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    let key1 = cache.cache_path("example.com", "/app/index.js");
    let key2 = cache.cache_path("other.com", "/app/index.js");

    assert_ne!(
        key1, key2,
        "Different hostnames should have different cache paths"
    );
}

/// Test empty cache statistics
#[test]
fn test_cache_stats_empty() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    let stats = cache.stats().unwrap();
    assert_eq!(stats.sliver_count, 0, "New cache should be empty");
    assert_eq!(stats.total_size_bytes, 0, "New cache should have 0 bytes");
}

/// Test file locking for multi-process coordination
#[test]
fn test_generation_lock() {
    // Create a truly unique temp directory for this test
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_path = std::env::temp_dir().join(format!("nano-lock-test-{}", unique_id));
    std::fs::create_dir_all(&temp_path).unwrap();

    let cache = SliverCache::with_dir(temp_path.clone()).unwrap();

    // Use unique names
    let hostname = "lock-test.example.com";
    let entrypoint = "/app/lock-test.js";

    // Initially no lock
    assert!(
        !cache.is_generation_in_progress(hostname, entrypoint),
        "Should have no lock initially"
    );

    // Acquire lock
    let acquired = cache.try_acquire_generation_lock(hostname, entrypoint);
    assert!(acquired, "Should acquire lock successfully");
    assert!(
        cache.is_generation_in_progress(hostname, entrypoint),
        "Lock should be in progress"
    );

    // Second acquire should fail
    let second_acquire = cache.try_acquire_generation_lock(hostname, entrypoint);
    assert!(!second_acquire, "Second acquire should fail");

    // Release lock
    cache.release_generation_lock(hostname, entrypoint);
    assert!(
        !cache.is_generation_in_progress(hostname, entrypoint),
        "Lock should be released"
    );

    // Can acquire again after release
    let reacquired = cache.try_acquire_generation_lock(hostname, entrypoint);
    assert!(reacquired, "Should reacquire lock after release");

    // Clean up
    cache.release_generation_lock(hostname, entrypoint);
    let _ = std::fs::remove_dir_all(&temp_path);
}

/// Test get_optimized_handler_source returns Source when no cache
#[test]
fn test_get_optimized_source_no_cache() {
    let temp_dir = TempDir::new().unwrap();
    let _cache_dir = temp_dir.path().to_path_buf();

    // Set custom cache directory via env (test only - normally use default)
    let hostname = "test.example.com";
    let entrypoint = "/nonexistent/app.js";

    let source = get_optimized_handler_source(hostname, entrypoint, false);

    // Should return Source since no sliver exists
    assert!(!source.is_sliver());
    assert_eq!(source.entrypoint(), entrypoint);
}

/// Test is_sliver_available returns false for non-existent sliver
#[test]
fn test_is_sliver_available_false() {
    let available = is_sliver_available("nonexistent.host", "/nonexistent/path.js");
    assert!(!available);
}

/// Test cache cleanup removes old entries
#[test]
fn test_cache_cleanup() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    // Create a fake sliver file directly in cache root (test simple case)
    // Note: stats() only counts files in the root, not subdirectories
    let sliver_path = temp_dir.path().join("test.sliver");
    std::fs::write(&sliver_path, b"fake sliver data").unwrap();

    // Verify it exists
    let stats_before = cache.stats().unwrap();
    assert_eq!(stats_before.sliver_count, 1);

    // Cleanup with very short max age (1 nanosecond)
    let cleaned = cache.cleanup(Duration::from_nanos(1)).unwrap();
    assert_eq!(cleaned, 1, "Should clean up 1 old entry");

    // Verify it's gone
    let stats_after = cache.stats().unwrap();
    assert_eq!(stats_after.sliver_count, 0);
}

/// Test sliver_modified_time returns None for non-existent sliver
#[test]
fn test_sliver_modified_time_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    let modified = cache.sliver_modified_time("nonexistent", "/nonexistent.js");
    assert!(modified.is_none());
}

/// Test sliver_modified_time returns Some for existing sliver
#[test]
fn test_sliver_modified_time_existing() {
    let temp_dir = TempDir::new().unwrap();
    let cache = SliverCache::with_dir(temp_dir.path().to_path_buf()).unwrap();

    let hostname = "test.example.com";
    let entrypoint = "/app/index.js";
    let cache_path = cache.cache_path(hostname, entrypoint);

    std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
    std::fs::write(&cache_path, b"fake sliver data").unwrap();

    let modified = cache.sliver_modified_time(hostname, entrypoint);
    assert!(modified.is_some());
}
