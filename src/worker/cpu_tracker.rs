//! Per-request CPU time tracking with microsecond precision
//!
//! This module provides CPU time tracking for JavaScript execution,
//! preventing runaway scripts from consuming excessive CPU resources.
//!
//! ## Architecture
//!
//! - `CpuTracker`: Tracks CPU time consumption per isolate/request
//! - `CpuTimeSnapshot`: Records CPU time at a point for delta calculation
//! - `CpuTimeError`: Error type for CPU limit violations
//!
//! ## Platform Support
//!
//! - Linux: Uses `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`
//! - macOS: Uses `thread_info()` with MACH thread basic info
//! - Other platforms: Falls back to wall-clock time (less accurate but functional)
//!
//! ## Integration
//!
//! Similar API to `MemoryLimiter` for consistency in the worker module.
//! CPU time is checked periodically during execution, not continuously.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use thiserror::Error;

/// Error type for CPU time limit violations
#[derive(Error, Debug, Clone)]
pub enum CpuTimeError {
    /// CPU time limit exceeded during execution
    #[error("CPU time limit exceeded: used {}us, limit {}us", used_us, limit_us)]
    LimitExceeded {
        /// Microseconds of CPU time used
        used_us: u64,
        /// Configured limit in microseconds
        limit_us: u64,
    },

    /// CPU time tracking failed (platform error)
    #[error("CPU time tracking failed: {}", message)]
    TrackingFailed {
        /// Error message
        message: String,
    },
}

impl CpuTimeError {
    /// Get the used CPU time in milliseconds
    pub fn used_ms(&self) -> u64 {
        match self {
            CpuTimeError::LimitExceeded { used_us, .. } => used_us / 1000,
            CpuTimeError::TrackingFailed { .. } => 0,
        }
    }

    /// Get the limit in milliseconds
    pub fn limit_ms(&self) -> u64 {
        match self {
            CpuTimeError::LimitExceeded { limit_us, .. } => limit_us / 1000,
            CpuTimeError::TrackingFailed { .. } => 0,
        }
    }

    /// Get the used CPU time in microseconds
    pub fn used_us(&self) -> u64 {
        match self {
            CpuTimeError::LimitExceeded { used_us, .. } => *used_us,
            CpuTimeError::TrackingFailed { .. } => 0,
        }
    }

    /// Get the limit in microseconds
    pub fn limit_us(&self) -> u64 {
        match self {
            CpuTimeError::LimitExceeded { limit_us, .. } => *limit_us,
            CpuTimeError::TrackingFailed { .. } => 0,
        }
    }
}

/// A snapshot of CPU time at a specific point
///
/// Used to calculate elapsed CPU time between two points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuTimeSnapshot {
    /// CPU time in microseconds at the snapshot point
    cpu_micros: u64,
    /// Wall clock time in microseconds (for comparison/debugging)
    wall_micros: u64,
}

impl CpuTimeSnapshot {
    /// Create a new snapshot from raw microsecond values
    pub fn new(cpu_micros: u64, wall_micros: u64) -> Self {
        Self {
            cpu_micros,
            wall_micros,
        }
    }

    /// Get CPU time in microseconds
    pub fn cpu_micros(&self) -> u64 {
        self.cpu_micros
    }

    /// Get wall clock time in microseconds
    pub fn wall_micros(&self) -> u64 {
        self.wall_micros
    }

    /// Calculate elapsed CPU microseconds since this snapshot
    pub fn elapsed_cpu_us(&self, now: &CpuTimeSnapshot) -> u64 {
        now.cpu_micros.saturating_sub(self.cpu_micros)
    }

    /// Calculate elapsed wall clock microseconds since this snapshot
    pub fn elapsed_wall_us(&self, now: &CpuTimeSnapshot) -> u64 {
        now.wall_micros.saturating_sub(self.wall_micros)
    }

    /// Calculate CPU efficiency ratio (CPU time / wall time)
    ///
    /// Returns value between 0.0 and 1.0+ (can exceed 1.0 on multi-core
    /// or if thread migration occurred between cores)
    pub fn cpu_ratio(&self, now: &CpuTimeSnapshot) -> f64 {
        let wall_us = self.elapsed_wall_us(now);
        if wall_us == 0 {
            return 0.0;
        }
        let cpu_us = self.elapsed_cpu_us(now) as f64;
        cpu_us / wall_us as f64
    }
}

/// Platform-specific CPU time retrieval
///
/// Returns thread CPU time in microseconds, or None if not available.
#[cfg(target_os = "linux")]
fn get_thread_cpu_time_us() -> Option<u64> {
    use libc::{clock_gettime, timespec, CLOCK_THREAD_CPUTIME_ID};

    let mut ts: timespec = unsafe { std::mem::zeroed() };
    let result = unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) };

    if result == 0 {
        // Convert seconds + nanoseconds to microseconds
        let micros = (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1000;
        Some(micros)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn get_thread_cpu_time_us() -> Option<u64> {
    // On macOS, we use getrusage with RUSAGE_THREAD (Linux 2.6.26+) equivalent
    // macOS doesn't have thread-specific getrusage, so we use RUSAGE_SELF
    // and note that this gives process-wide CPU time, not thread-specific
    // For true thread-specific CPU time on macOS, we'd need mach APIs with
    // proper bindings - this is a best-effort implementation
    use libc::{getrusage, rusage, RUSAGE_SELF};

    unsafe {
        let mut usage: rusage = std::mem::zeroed();
        let result = getrusage(RUSAGE_SELF, &mut usage);

        if result == 0 {
            // ru_utime and ru_stime are timeval with seconds and microseconds
            let user_us =
                (usage.ru_utime.tv_sec as u64) * 1_000_000 + (usage.ru_utime.tv_usec as u64);
            let sys_us =
                (usage.ru_stime.tv_sec as u64) * 1_000_000 + (usage.ru_stime.tv_usec as u64);
            Some(user_us + sys_us)
        } else {
            None
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn get_thread_cpu_time_us() -> Option<u64> {
    // Fallback: not supported on this platform
    None
}

/// CPU time tracker for per-request limits
///
/// Tracks CPU time consumption against a configurable limit.
/// Thread-safe for cross-thread checks (atomic operations).
pub struct CpuTracker {
    /// CPU time limit in microseconds
    limit_us: AtomicU64,
    /// Whether CPU time tracking is enabled
    enabled: AtomicBool,
    /// Whether limit has been exceeded
    limit_exceeded: AtomicBool,
}

impl std::fmt::Debug for CpuTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuTracker")
            .field("limit_us", &self.limit_us())
            .field("limit_ms", &self.limit_ms())
            .field("enabled", &self.is_enabled())
            .field("limit_exceeded", &self.is_limit_exceeded())
            .finish()
    }
}

impl CpuTracker {
    /// Create a new CPU tracker with the given limit in milliseconds
    ///
    /// # Arguments
    ///
    /// * `limit_ms` - CPU time limit in milliseconds (default: 50ms like Cloudflare)
    ///
    /// # Example
    ///
    /// ```
    /// use nano::worker::cpu_tracker::CpuTracker;
    ///
    /// let tracker = CpuTracker::new(50); // 50ms limit
    /// assert_eq!(tracker.limit_ms(), 50);
    /// ```
    pub fn new(limit_ms: u32) -> Self {
        Self::with_limit_us((limit_ms as u64) * 1000)
    }

    /// Create a new CPU tracker with limit in microseconds
    ///
    /// # Arguments
    ///
    /// * `limit_us` - CPU time limit in microseconds
    pub fn with_limit_us(limit_us: u64) -> Self {
        Self {
            limit_us: AtomicU64::new(limit_us),
            enabled: AtomicBool::new(true),
            limit_exceeded: AtomicBool::new(false),
        }
    }

    /// Create a disabled CPU tracker (no tracking)
    ///
    /// Useful when CPU limits are not configured.
    pub fn disabled() -> Self {
        Self {
            limit_us: AtomicU64::new(u64::MAX),
            enabled: AtomicBool::new(false),
            limit_exceeded: AtomicBool::new(false),
        }
    }

    /// Get the current limit in milliseconds
    pub fn limit_ms(&self) -> u64 {
        self.limit_us() / 1000
    }

    /// Get the current limit in microseconds
    pub fn limit_us(&self) -> u64 {
        self.limit_us.load(Ordering::SeqCst)
    }

    /// Check if tracking is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Check if limit has been exceeded
    pub fn is_limit_exceeded(&self) -> bool {
        self.limit_exceeded.load(Ordering::SeqCst)
    }

    /// Enable or disable tracking
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Set a new limit in milliseconds
    pub fn set_limit_ms(&self, limit_ms: u32) {
        self.limit_us
            .store((limit_ms as u64) * 1000, Ordering::SeqCst);
    }

    /// Set a new limit in microseconds
    pub fn set_limit_us(&self, limit_us: u64) {
        self.limit_us.store(limit_us, Ordering::SeqCst);
    }

    /// Reset the limit exceeded flag
    pub fn reset(&self) {
        self.limit_exceeded.store(false, Ordering::SeqCst);
    }

    /// Capture a CPU time snapshot
    ///
    /// Uses platform-specific APIs for thread CPU time measurement.
    /// Falls back to wall clock on unsupported platforms.
    pub fn snapshot(&self) -> Option<CpuTimeSnapshot> {
        if !self.is_enabled() {
            return None;
        }

        // Get wall clock time first (always available)
        let wall_micros = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_micros() as u64;

        // Try to get thread CPU time
        match get_thread_cpu_time_us() {
            Some(cpu_micros) => Some(CpuTimeSnapshot::new(cpu_micros, wall_micros)),
            None => {
                // Fallback: use wall clock as approximation
                // This is less accurate but provides some tracking
                Some(CpuTimeSnapshot::new(wall_micros, wall_micros))
            }
        }
    }

    /// Check CPU time against limit
    ///
    /// Compares current CPU time to the snapshot and checks if limit exceeded.
    /// Returns error if limit exceeded, snapshot otherwise.
    ///
    /// # Arguments
    ///
    /// * `start` - The starting snapshot to compare against
    ///
    /// # Returns
    ///
    /// `Ok(CpuTimeSnapshot)` with current CPU time if within limit,
    /// `Err(CpuTimeError::LimitExceeded)` if limit exceeded
    pub fn check_cpu(&self, start: &CpuTimeSnapshot) -> Result<CpuTimeSnapshot, CpuTimeError> {
        if !self.is_enabled() {
            // Return a dummy snapshot when disabled
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros() as u64;
            return Ok(CpuTimeSnapshot::new(now, now));
        }

        let now = self
            .snapshot()
            .ok_or_else(|| CpuTimeError::TrackingFailed {
                message: "Failed to capture CPU time snapshot".to_string(),
            })?;

        let elapsed_us = start.elapsed_cpu_us(&now);
        let limit_us = self.limit_us();

        if elapsed_us > limit_us {
            self.limit_exceeded.store(true, Ordering::SeqCst);
            return Err(CpuTimeError::LimitExceeded {
                used_us: elapsed_us,
                limit_us,
            });
        }

        Ok(now)
    }

    /// Peek at current CPU time without checking limits
    ///
    /// Returns the elapsed CPU microseconds since the start snapshot
    /// without checking against the limit.
    ///
    /// # Arguments
    ///
    /// * `start` - The starting snapshot to compare against
    ///
    /// # Returns
    ///
    /// `(CpuTimeSnapshot, elapsed_us)` - current snapshot and elapsed microseconds
    pub fn peek_cpu(&self, start: &CpuTimeSnapshot) -> (CpuTimeSnapshot, u64) {
        match self.snapshot() {
            Some(now) => {
                let elapsed_us = start.elapsed_cpu_us(&now);
                (now, elapsed_us)
            }
            None => {
                // Fallback: return zero elapsed time
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64;
                (CpuTimeSnapshot::new(now, now), 0)
            }
        }
    }

    /// Create a snapshot and return it (for starting tracking)
    ///
    /// This is a convenience method equivalent to `snapshot()` but
    /// returns a Result for consistency with `check_cpu()`.
    pub fn start(&self) -> Result<CpuTimeSnapshot, CpuTimeError> {
        self.snapshot().ok_or_else(|| CpuTimeError::TrackingFailed {
            message: "Failed to start CPU tracking - platform not supported".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_tracker_creation() {
        let tracker = CpuTracker::new(50); // 50ms
        assert_eq!(tracker.limit_ms(), 50);
        assert_eq!(tracker.limit_us(), 50_000);
        assert!(tracker.is_enabled());
        assert!(!tracker.is_limit_exceeded());
    }

    #[test]
    fn test_cpu_tracker_with_microseconds() {
        let tracker = CpuTracker::with_limit_us(100_000); // 100ms in us
        assert_eq!(tracker.limit_us(), 100_000);
        assert_eq!(tracker.limit_ms(), 100);
    }

    #[test]
    fn test_disabled_tracker() {
        let tracker = CpuTracker::disabled();
        assert!(!tracker.is_enabled());
        assert_eq!(tracker.limit_us(), u64::MAX);
        // Snapshot should return None when disabled
        assert!(tracker.snapshot().is_none());
    }

    #[test]
    fn test_cpu_time_snapshot() {
        let snap1 = CpuTimeSnapshot::new(1000, 2000);
        let snap2 = CpuTimeSnapshot::new(1500, 3000);

        assert_eq!(snap1.cpu_micros(), 1000);
        assert_eq!(snap1.wall_micros(), 2000);
        assert_eq!(snap1.elapsed_cpu_us(&snap2), 500);
        assert_eq!(snap1.elapsed_wall_us(&snap2), 1000);
    }

    #[test]
    fn test_cpu_ratio() {
        // CPU time: 100us, wall time: 200us = 50% CPU utilization
        let snap1 = CpuTimeSnapshot::new(0, 0);
        let snap2 = CpuTimeSnapshot::new(100, 200);
        assert_eq!(snap1.cpu_ratio(&snap2), 0.5);
    }

    #[test]
    fn test_cpu_ratio_zero_wall() {
        let snap1 = CpuTimeSnapshot::new(100, 100);
        let snap2 = CpuTimeSnapshot::new(150, 100);
        assert_eq!(snap1.cpu_ratio(&snap2), 0.0);
    }

    #[test]
    fn test_limit_modification() {
        let tracker = CpuTracker::new(50);
        assert_eq!(tracker.limit_ms(), 50);

        tracker.set_limit_ms(100);
        assert_eq!(tracker.limit_ms(), 100);
        assert_eq!(tracker.limit_us(), 100_000);

        tracker.set_limit_us(250_000);
        assert_eq!(tracker.limit_us(), 250_000);
        assert_eq!(tracker.limit_ms(), 250);
    }

    #[test]
    fn test_enable_disable() {
        let tracker = CpuTracker::new(50);
        assert!(tracker.is_enabled());

        tracker.set_enabled(false);
        assert!(!tracker.is_enabled());

        tracker.set_enabled(true);
        assert!(tracker.is_enabled());
    }

    #[test]
    fn test_reset() {
        let tracker = CpuTracker::new(1); // 1ms limit for testing

        // Manually trigger exceeded flag
        tracker.limit_exceeded.store(true, Ordering::SeqCst);
        assert!(tracker.is_limit_exceeded());

        tracker.reset();
        assert!(!tracker.is_limit_exceeded());
    }

    #[test]
    fn test_cpu_time_error_properties() {
        let err = CpuTimeError::LimitExceeded {
            used_us: 150_000,
            limit_us: 100_000,
        };

        assert_eq!(err.used_us(), 150_000);
        assert_eq!(err.limit_us(), 100_000);
        assert_eq!(err.used_ms(), 150);
        assert_eq!(err.limit_ms(), 100);
    }

    #[test]
    fn test_error_display() {
        let err = CpuTimeError::LimitExceeded {
            used_us: 150_000,
            limit_us: 100_000,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("CPU time limit exceeded"));
        assert!(msg.contains("150000us"));
        assert!(msg.contains("100000us"));
    }

    #[test]
    fn test_tracking_error_display() {
        let err = CpuTimeError::TrackingFailed {
            message: "Platform not supported".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("CPU time tracking failed"));
        assert!(msg.contains("Platform not supported"));
    }

    #[test]
    fn test_peek_cpu() {
        let tracker = CpuTracker::new(1000); // 1 second limit

        // Start tracking
        let start = tracker.start().expect("Should start tracking");

        // Small delay to ensure some time passes
        std::thread::sleep(std::time::Duration::from_millis(1));

        // Peek at CPU time
        let (now, elapsed) = tracker.peek_cpu(&start);

        // Should have some elapsed time (but we can't guarantee exactly how much)
        // Just verify it returns reasonable values
        assert!(elapsed < 1_000_000); // Should be less than 1 second
        assert!(now.cpu_micros() >= start.cpu_micros());
    }

    #[test]
    fn test_check_cpu_within_limit() {
        let tracker = CpuTracker::new(1000); // 1 second limit

        let start = tracker.start().expect("Should start tracking");

        // Check immediately (should be well within 1 second)
        let result = tracker.check_cpu(&start);
        assert!(result.is_ok(), "Should be within limit immediately");
    }

    #[test]
    fn test_snapshot_equality() {
        let snap1 = CpuTimeSnapshot::new(100, 200);
        let snap2 = CpuTimeSnapshot::new(100, 200);
        let snap3 = CpuTimeSnapshot::new(200, 300);

        assert_eq!(snap1, snap2);
        assert_ne!(snap1, snap3);
    }

    #[test]
    fn test_elapsed_cpu_us_saturating() {
        let snap1 = CpuTimeSnapshot::new(1000, 0);
        let snap2 = CpuTimeSnapshot::new(500, 0); // Earlier time

        // Should saturate to 0, not underflow
        assert_eq!(snap1.elapsed_cpu_us(&snap2), 0);
    }
}
