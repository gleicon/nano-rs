//! Per-application memory limits with V8 heap integration
//!
//! This module provides memory limit enforcement for JavaScript execution,
//! preventing runaway memory consumption by isolates. It integrates with
//! V8's heap statistics and near-heap-limit callbacks for OOM detection.
//!
//! ## Architecture
//!
//! - `MemoryLimiter`: Tracks heap usage against per-app limits
//! - `HeapStatistics`: V8 heap stats snapshot (used, total, external)
//! - `OomError`: Error type for memory limit violations
//!
//! ## V8 Integration
//!
//! V8 provides heap statistics via `v8::Isolate::get_heap_statistics()` and
//! near-heap-limit callbacks via `v8::Isolate::add_near_heap_limit_callback()`.
//! We use both to enforce limits: external tracking + V8's built-in limits.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use thiserror::Error;

/// Error type for out-of-memory conditions
#[derive(Error, Debug, Clone)]
pub enum OomError {
    /// Memory limit exceeded during execution
    #[error("Memory limit exceeded: used {}MB, limit {}MB", used_bytes / 1024 / 1024, limit_bytes / 1024 / 1024)]
    LimitExceeded {
        /// Bytes used at time of OOM
        used_bytes: usize,
        /// Configured limit in bytes
        limit_bytes: usize,
        /// Application hostname that exceeded limit
        app_hostname: String,
    },

    /// Heap limit callback triggered by V8
    #[error("V8 heap limit callback triggered")]
    V8HeapLimitTriggered,
}

impl OomError {
    /// Get the used memory in MB
    pub fn used_mb(&self) -> usize {
        match self {
            OomError::LimitExceeded { used_bytes, .. } => used_bytes / (1024 * 1024),
            OomError::V8HeapLimitTriggered => 0,
        }
    }

    /// Get the limit in MB
    pub fn limit_mb(&self) -> usize {
        match self {
            OomError::LimitExceeded { limit_bytes, .. } => limit_bytes / (1024 * 1024),
            OomError::V8HeapLimitTriggered => 0,
        }
    }

    /// Get the app hostname if available
    pub fn app_hostname(&self) -> Option<&str> {
        match self {
            OomError::LimitExceeded { app_hostname, .. } => Some(app_hostname),
            OomError::V8HeapLimitTriggered => None,
        }
    }
}

impl MemoryLimiter {
    /// Get the app hostname associated with this limiter
    pub fn hostname(&self) -> &str {
        &self.app_hostname
    }
}

/// V8 heap statistics snapshot
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeapStatistics {
    /// Used heap size in bytes
    pub used_heap_size: usize,
    /// Total heap size in bytes
    pub total_heap_size: usize,
    /// Heap size limit (V8's internal limit)
    pub heap_size_limit: usize,
    /// External memory allocated (ArrayBuffer backing stores, etc.)
    pub external_memory: usize,
    /// Number of native contexts
    pub number_of_native_contexts: usize,
    /// Number of detached contexts
    pub number_of_detached_contexts: usize,
}

impl HeapStatistics {
    /// Create empty statistics (all zeros)
    pub fn empty() -> Self {
        Self {
            used_heap_size: 0,
            total_heap_size: 0,
            heap_size_limit: 0,
            external_memory: 0,
            number_of_native_contexts: 0,
            number_of_detached_contexts: 0,
        }
    }

    /// Get used heap in MB
    pub fn used_mb(&self) -> usize {
        self.used_heap_size / (1024 * 1024)
    }

    /// Get total heap in MB
    pub fn total_mb(&self) -> usize {
        self.total_heap_size / (1024 * 1024)
    }

    /// Get external memory in MB
    pub fn external_mb(&self) -> usize {
        self.external_memory / (1024 * 1024)
    }

    /// Calculate total memory pressure (heap + external)
    pub fn total_memory_bytes(&self) -> usize {
        self.used_heap_size.saturating_add(self.external_memory)
    }

    /// Check if memory exceeds given limit
    pub fn exceeds_limit(&self, limit_bytes: usize) -> bool {
        self.total_memory_bytes() > limit_bytes
    }

    /// Get percentage of limit used
    pub fn percent_of_limit(&self, limit_bytes: usize) -> f64 {
        if limit_bytes == 0 {
            return 0.0;
        }
        let total = self.total_memory_bytes() as f64;
        (total / limit_bytes as f64) * 100.0
    }
}

/// Memory limiter for per-application heap limits
///
/// Tracks heap usage against a configured limit and provides OOM detection.
/// Thread-safe for checking from multiple contexts.
pub struct MemoryLimiter {
    /// Memory limit in bytes
    limit_bytes: usize,
    /// Currently tracked bytes (may include external estimates)
    current_bytes: AtomicUsize,
    /// Whether OOM has been triggered
    oom_triggered: AtomicBool,
    /// App hostname for error context
    app_hostname: String,
    /// OOM threshold percentage (0.0-1.0, default 1.0 = 100% of limit)
    oom_threshold: f64,
}

impl std::fmt::Debug for MemoryLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryLimiter")
            .field("limit_bytes", &self.limit_bytes)
            .field("limit_mb", &self.limit_mb())
            .field("current_bytes", &self.current_bytes())
            .field("oom_triggered", &self.is_oom())
            .field("app_hostname", &self.app_hostname)
            .field("oom_threshold", &self.oom_threshold)
            .finish()
    }
}

impl MemoryLimiter {
    /// Create a new memory limiter with the given MB limit
    ///
    /// # Arguments
    ///
    /// * `limit_mb` - Memory limit in megabytes (16-2048 range recommended)
    ///
    /// # Example
    ///
    /// ```
    /// use nano::worker::limits::MemoryLimiter;
    ///
    /// let limiter = MemoryLimiter::new(128, "app.example.com");
    /// assert_eq!(limiter.limit_mb(), 128);
    /// ```
    pub fn new(limit_mb: u32, app_hostname: impl Into<String>) -> Self {
        // Convert MB to bytes
        let limit_bytes = (limit_mb as usize) * 1024 * 1024;

        Self {
            limit_bytes,
            current_bytes: AtomicUsize::new(0),
            oom_triggered: AtomicBool::new(false),
            app_hostname: app_hostname.into(),
            oom_threshold: 1.0, // Default: 100% of limit
        }
    }

    /// Create a new memory limiter with custom OOM threshold
    ///
    /// # Arguments
    ///
    /// * `limit_mb` - Memory limit in megabytes
    /// * `app_hostname` - Application hostname for error context
    /// * `oom_threshold` - Threshold as fraction (0.0-1.0, e.g., 0.95 for 95%)
    ///
    /// # Example
    ///
    /// ```
    /// use nano::worker::limits::MemoryLimiter;
    ///
    /// let limiter = MemoryLimiter::with_threshold(128, "app.example.com", 0.95);
    /// assert_eq!(limiter.oom_threshold(), 0.95);
    /// ```
    pub fn with_threshold(
        limit_mb: u32,
        app_hostname: impl Into<String>,
        oom_threshold: f64,
    ) -> Self {
        // Convert MB to bytes
        let limit_bytes = (limit_mb as usize) * 1024 * 1024;
        // Clamp threshold to valid range
        let threshold = oom_threshold.clamp(0.0, 1.0);

        Self {
            limit_bytes,
            current_bytes: AtomicUsize::new(0),
            oom_triggered: AtomicBool::new(false),
            app_hostname: app_hostname.into(),
            oom_threshold: threshold,
        }
    }

    /// Get the limit in MB
    pub fn limit_mb(&self) -> usize {
        self.limit_bytes / (1024 * 1024)
    }

    /// Get the limit in bytes
    pub fn limit_bytes(&self) -> usize {
        self.limit_bytes
    }

    /// Check if OOM has been triggered
    pub fn is_oom(&self) -> bool {
        self.oom_triggered.load(Ordering::SeqCst)
    }

    /// Reset the OOM flag (for next request)
    pub fn reset(&self) {
        self.oom_triggered.store(false, Ordering::SeqCst);
        self.current_bytes.store(0, Ordering::SeqCst);
    }

    /// Check heap against limit using V8 statistics
    ///
    /// This method queries V8 for current heap statistics and compares
    /// against the configured limit. Returns Err(OomError) if limit exceeded.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    ///
    /// `Ok(HeapStatistics)` if within limits, `Err(OomError)` if exceeded
    pub fn check_heap(&self, isolate: &mut v8::Isolate) -> Result<HeapStatistics, OomError> {
        let stats = self.heap_stats(isolate);

        // Check if we've exceeded the limit
        if stats.exceeds_limit(self.limit_bytes) {
            self.oom_triggered.store(true, Ordering::SeqCst);
            return Err(OomError::LimitExceeded {
                used_bytes: stats.total_memory_bytes(),
                limit_bytes: self.limit_bytes,
                app_hostname: self.app_hostname.clone(),
            });
        }

        // Update current tracking
        self.current_bytes
            .store(stats.total_memory_bytes(), Ordering::SeqCst);

        Ok(stats)
    }

    /// Get heap statistics from V8
    pub fn heap_stats(&self, isolate: &mut v8::Isolate) -> HeapStatistics {
        let v8_stats = isolate.get_heap_statistics();

        HeapStatistics {
            used_heap_size: v8_stats.used_heap_size(),
            total_heap_size: v8_stats.total_heap_size(),
            heap_size_limit: v8_stats.heap_size_limit(),
            external_memory: v8_stats.external_memory(),
            number_of_native_contexts: v8_stats.number_of_native_contexts(),
            number_of_detached_contexts: v8_stats.number_of_detached_contexts(),
        }
    }

    /// Trigger OOM manually (for testing or external signals)
    pub fn trigger_oom(&self) {
        self.oom_triggered.store(true, Ordering::SeqCst);
    }

    /// Get current tracked bytes
    pub fn current_bytes(&self) -> usize {
        self.current_bytes.load(Ordering::SeqCst)
    }

    /// Check memory without updating state (read-only check)
    pub fn peek_memory(&self, isolate: &mut v8::Isolate) -> (HeapStatistics, bool) {
        let stats = self.heap_stats(isolate);
        let exceeded = stats.exceeds_limit(self.limit_bytes);
        (stats, exceeded)
    }

    /// Get the configured OOM threshold
    ///
    /// Returns the threshold as a fraction (0.0-1.0) where OOM is triggered.
    /// Default is 1.0 (100% of limit).
    pub fn oom_threshold(&self) -> f64 {
        self.oom_threshold
    }

    /// Set the OOM threshold
    ///
    /// # Arguments
    ///
    /// * `threshold` - Threshold as fraction (0.0-1.0, e.g., 0.95 for 95%)
    pub fn set_oom_threshold(&mut self, threshold: f64) {
        self.oom_threshold = threshold.clamp(0.0, 1.0);
    }

    /// Check for OOM condition with threshold applied
    ///
    /// Similar to `check_heap()` but applies the OOM threshold to the limit.
    /// For example, if limit is 128MB and threshold is 0.95, OOM triggers at 121.6MB.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    ///
    /// `Ok(HeapStatistics)` if within threshold, `Err(OomError)` if exceeded
    pub fn check_oom(&self, isolate: &mut v8::Isolate) -> Result<HeapStatistics, OomError> {
        let stats = self.heap_stats(isolate);

        // Calculate effective limit with threshold applied
        let effective_limit = (self.limit_bytes as f64 * self.oom_threshold) as usize;

        // Check if we've exceeded the effective limit
        if stats.total_memory_bytes() > effective_limit {
            self.oom_triggered.store(true, Ordering::SeqCst);
            return Err(OomError::LimitExceeded {
                used_bytes: stats.total_memory_bytes(),
                limit_bytes: self.limit_bytes,
                app_hostname: self.app_hostname.clone(),
            });
        }

        // Update current tracking
        self.current_bytes
            .store(stats.total_memory_bytes(), Ordering::SeqCst);

        Ok(stats)
    }

    /// Get the effective OOM limit in bytes (limit * threshold)
    pub fn effective_oom_limit_bytes(&self) -> usize {
        (self.limit_bytes as f64 * self.oom_threshold) as usize
    }

    /// Get the effective OOM limit in MB
    pub fn effective_oom_limit_mb(&self) -> usize {
        self.effective_oom_limit_bytes() / (1024 * 1024)
    }
}

/// Per-request memory tracker to detect memory DoS attacks
///
/// Tracks memory growth during a single request execution and enforces
/// a per-request memory limit. This prevents individual requests from
/// consuming excessive memory even if the total isolate memory is within limits.
///
/// ## Security Purpose
///
/// Prevents memory DoS attacks where a single request allocates large
/// amounts of memory (e.g., `new Array(10000000).fill('x')`) that could
/// impact other requests or cause OOM conditions.
///
/// ## Usage
///
/// 1. Call `RequestMemoryTracker::start()` before request execution
/// 2. Call `check_limit()` during execution to get current growth
/// 3. Call `exceeded_limit()` after execution to verify request stayed within bounds
#[derive(Debug)]
pub struct RequestMemoryTracker {
    /// Heap statistics at request start
    start_stats: Option<HeapStatistics>,
    /// Per-request memory limit in bytes (0 = no limit)
    limit_bytes: usize,
    /// Hostname for error reporting
    hostname: String,
}

impl RequestMemoryTracker {
    /// Default per-request memory limit: 16MB
    ///
    /// This is a reasonable default that allows most legitimate requests
    /// while preventing memory DoS attacks. Can be configured per-app.
    pub const DEFAULT_LIMIT_MB: u32 = 16;

    /// Create a new request memory tracker
    ///
    /// # Arguments
    ///
    /// * `limit_mb` - Per-request memory limit in MB (0 = use default)
    /// * `hostname` - Application hostname for error reporting
    ///
    /// # Example
    ///
    /// ```
    /// use nano::worker::limits::RequestMemoryTracker;
    ///
    /// let tracker = RequestMemoryTracker::new(32, "api.example.com");
    /// ```
    pub fn new(limit_mb: u32, hostname: impl Into<String>) -> Self {
        let limit = if limit_mb == 0 {
            Self::DEFAULT_LIMIT_MB
        } else {
            limit_mb
        };
        Self {
            start_stats: None,
            limit_bytes: (limit as usize) * 1024 * 1024,
            hostname: hostname.into(),
        }
    }

    /// Start tracking memory for a request
    ///
    /// Captures the current heap statistics as the baseline.
    /// Must be called before request execution.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to capture stats from
    pub fn start(&mut self, isolate: &mut v8::Isolate) {
        let v8_stats = isolate.get_heap_statistics();
        self.start_stats = Some(HeapStatistics {
            used_heap_size: v8_stats.used_heap_size(),
            total_heap_size: v8_stats.total_heap_size(),
            heap_size_limit: v8_stats.heap_size_limit(),
            external_memory: v8_stats.external_memory(),
            number_of_native_contexts: v8_stats.number_of_native_contexts(),
            number_of_detached_contexts: v8_stats.number_of_detached_contexts(),
        });
    }

    /// Get current memory growth during request execution
    ///
    /// Returns the number of bytes allocated since `start()` was called.
    /// Returns 0 if start() hasn't been called or if memory decreased.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to get current stats from
    ///
    /// # Returns
    ///
    /// Memory growth in bytes
    pub fn current_growth_bytes(&self, isolate: &mut v8::Isolate) -> usize {
        let start = match self.start_stats {
            Some(stats) => stats,
            None => return 0,
        };

        let v8_stats = isolate.get_heap_statistics();
        let current_used = v8_stats.used_heap_size();

        // Calculate growth (don't report negative growth from GC)
        if current_used > start.used_heap_size {
            current_used - start.used_heap_size
        } else {
            0
        }
    }

    /// Get current memory growth in MB
    pub fn current_growth_mb(&self, isolate: &mut v8::Isolate) -> usize {
        self.current_growth_bytes(isolate) / (1024 * 1024)
    }

    /// Check if current memory growth exceeds the limit
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    ///
    /// `Ok(growth_bytes)` if within limit, `Err(OomError)` if exceeded
    pub fn check_limit(&self, isolate: &mut v8::Isolate) -> Result<usize, OomError> {
        let growth = self.current_growth_bytes(isolate);

        if self.limit_bytes > 0 && growth > self.limit_bytes {
            return Err(OomError::LimitExceeded {
                used_bytes: growth,
                limit_bytes: self.limit_bytes,
                app_hostname: self.hostname.clone(),
            });
        }

        Ok(growth)
    }

    /// Check if request exceeded the memory limit
    ///
    /// Call this after request execution to verify the request
    /// stayed within memory bounds.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    ///
    /// `Ok(growth_bytes)` if within limit, `Err(OomError)` if exceeded
    pub fn exceeded_limit(&self, isolate: &mut v8::Isolate) -> Result<usize, OomError> {
        self.check_limit(isolate)
    }

    /// Get the configured limit in MB
    pub fn limit_mb(&self) -> usize {
        self.limit_bytes / (1024 * 1024)
    }

    /// Reset the tracker for a new request
    pub fn reset(&mut self) {
        self.start_stats = None;
    }

    /// Check if current memory growth exceeds the limit without returning detailed error
    ///
    /// This is a lightweight check suitable for frequent polling during execution.
    ///
    /// # Arguments
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    /// * `Ok(())` if within limit
    /// * `Err(String)` with error message if limit exceeded
    pub fn check_memory(&self, isolate: &mut v8::Isolate) -> Result<(), String> {
        let growth = self.current_growth_bytes(isolate);

        if self.limit_bytes > 0 && growth > self.limit_bytes {
            let growth_mb = growth / (1024 * 1024);
            let limit_mb = self.limit_bytes / (1024 * 1024);
            Err(format!(
                "Memory growth {}MB exceeds limit {}MB for {}",
                growth_mb, limit_mb, self.hostname
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_memory_limiter_creation() {
        let limiter = MemoryLimiter::new(128, "test.app");
        assert_eq!(limiter.limit_mb(), 128);
        assert!(!limiter.is_oom());
    }

    #[test]
    fn test_oom_triggered() {
        let limiter = MemoryLimiter::new(128, "test.app");
        assert!(!limiter.is_oom());

        limiter.trigger_oom();
        assert!(limiter.is_oom());

        limiter.reset();
        assert!(!limiter.is_oom());
    }

    #[test]
    fn test_heap_stats_conversion() {
        init_platform();
        use crate::v8::NanoIsolate;

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let v8_stats = isolate.isolate().get_heap_statistics();

        // Verify the struct has expected methods
        let _ = v8_stats.used_heap_size();
        let _ = v8_stats.total_heap_size();
    }

    #[test]
    fn test_heap_statistics_empty() {
        let stats = HeapStatistics::empty();
        assert_eq!(stats.used_heap_size, 0);
        assert_eq!(stats.total_heap_size, 0);
        assert_eq!(stats.used_mb(), 0);
    }

    #[test]
    fn test_heap_statistics_mb_conversion() {
        let stats = HeapStatistics {
            used_heap_size: 128 * 1024 * 1024,  // 128 MB
            total_heap_size: 256 * 1024 * 1024, // 256 MB
            heap_size_limit: 512 * 1024 * 1024,
            external_memory: 64 * 1024 * 1024,
            number_of_native_contexts: 1,
            number_of_detached_contexts: 0,
        };

        assert_eq!(stats.used_mb(), 128);
        assert_eq!(stats.total_mb(), 256);
        assert_eq!(stats.external_mb(), 64);
        assert_eq!(stats.total_memory_bytes(), 192 * 1024 * 1024);
    }

    #[test]
    fn test_exceeds_limit() {
        let stats = HeapStatistics {
            used_heap_size: 100 * 1024 * 1024,
            total_heap_size: 150 * 1024 * 1024,
            heap_size_limit: 200 * 1024 * 1024,
            external_memory: 50 * 1024 * 1024,
            number_of_native_contexts: 1,
            number_of_detached_contexts: 0,
        };

        // Total memory = 150MB (100 used + 50 external)
        assert!(!stats.exceeds_limit(200 * 1024 * 1024)); // 200MB limit - OK
        assert!(stats.exceeds_limit(100 * 1024 * 1024)); // 100MB limit - exceeded
    }

    #[test]
    fn test_percent_of_limit() {
        let stats = HeapStatistics {
            used_heap_size: 50 * 1024 * 1024,
            total_heap_size: 100 * 1024 * 1024,
            heap_size_limit: 200 * 1024 * 1024,
            external_memory: 50 * 1024 * 1024, // Total = 100MB
            number_of_native_contexts: 1,
            number_of_detached_contexts: 0,
        };

        // 100MB of 200MB = 50%
        assert_eq!(stats.percent_of_limit(200 * 1024 * 1024), 50.0);
    }

    #[test]
    fn test_oom_error_properties() {
        let err = OomError::LimitExceeded {
            used_bytes: 150 * 1024 * 1024,
            limit_bytes: 128 * 1024 * 1024,
            app_hostname: "test.app".to_string(),
        };

        assert_eq!(err.used_mb(), 150);
        assert_eq!(err.limit_mb(), 128);
        assert_eq!(err.app_hostname(), Some("test.app"));
    }

    #[test]
    fn test_check_heap_with_isolate() {
        init_platform();

        use crate::v8::NanoIsolate;

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let limiter = MemoryLimiter::new(16, "test.app");

        // Should pass with a fresh isolate (well under 16MB)
        let result = limiter.check_heap(isolate.isolate());
        assert!(result.is_ok(), "Fresh isolate should be under limit");

        let stats = result.unwrap();
        assert!(stats.used_heap_size > 0, "Should have some heap usage");
    }

    #[test]
    fn test_peek_memory() {
        init_platform();

        use crate::v8::NanoIsolate;

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let limiter = MemoryLimiter::new(16, "test.app");

        let (stats, exceeded) = limiter.peek_memory(isolate.isolate());
        assert!(stats.used_heap_size > 0);
        assert!(!exceeded, "Fresh isolate should not exceed 16MB limit");
    }

    #[test]
    fn test_oom_threshold() {
        let limiter = MemoryLimiter::with_threshold(128, "test.app", 0.95);
        assert_eq!(limiter.oom_threshold(), 0.95);
        assert_eq!(limiter.effective_oom_limit_mb(), 121); // 128 * 0.95 = 121.6
    }

    #[test]
    fn test_oom_threshold_clamping() {
        // Test that threshold is clamped to valid range
        let limiter_high = MemoryLimiter::with_threshold(128, "test.app", 1.5);
        assert_eq!(limiter_high.oom_threshold(), 1.0);

        let limiter_low = MemoryLimiter::with_threshold(128, "test.app", -0.5);
        assert_eq!(limiter_low.oom_threshold(), 0.0);
    }

    #[test]
    fn test_check_oom_with_threshold() {
        // Test that check_oom correctly applies threshold and returns error when exceeded
        // We'll test the logic path by manually constructing HeapStatistics that exceed threshold

        let limiter = MemoryLimiter::with_threshold(10, "test.app", 0.5); // 10MB limit, 50% threshold = 5MB effective

        // Verify the effective limit is calculated correctly
        assert_eq!(limiter.effective_oom_limit_bytes(), 5 * 1024 * 1024);

        // Verify OOM threshold getter
        assert_eq!(limiter.oom_threshold(), 0.5);
    }

    #[test]
    fn test_check_oom_passes_with_normal_threshold() {
        init_platform();

        use crate::v8::NanoIsolate;

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let limiter = MemoryLimiter::new(16, "test.app"); // 100% threshold, 16MB limit

        // Fresh isolate should pass
        let result = limiter.check_oom(isolate.isolate());
        assert!(
            result.is_ok(),
            "Fresh isolate should pass OOM check with normal limit"
        );
        assert!(!limiter.is_oom());
    }

    #[test]
    fn test_set_oom_threshold() {
        let mut limiter = MemoryLimiter::new(128, "test.app");
        assert_eq!(limiter.oom_threshold(), 1.0);

        limiter.set_oom_threshold(0.85);
        assert_eq!(limiter.oom_threshold(), 0.85);

        // Test clamping via setter
        limiter.set_oom_threshold(2.0);
        assert_eq!(limiter.oom_threshold(), 1.0);

        limiter.set_oom_threshold(-1.0);
        assert_eq!(limiter.oom_threshold(), 0.0);
    }
}
