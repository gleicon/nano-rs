//! Benchmark Utilities for Sliver Performance Testing
//!
//! Provides helper functions and types for measuring sliver
//! cold start performance and comparison metrics.

use std::time::{Duration, Instant};

/// Timer for microsecond-precision measurements
pub struct MicroTimer {
    start: Instant,
}

impl MicroTimer {
    /// Create a new timer starting now
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Start the timer
    pub fn start(&mut self) {
        self.start = Instant::now();
    }

    /// Get elapsed time as Duration
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Get elapsed time in microseconds
    pub fn elapsed_micros(&self) -> u128 {
        self.start.elapsed().as_micros()
    }

    /// Get elapsed time in milliseconds (with fractional part)
    pub fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// Reset and return elapsed time
    pub fn lap(&mut self) -> Duration {
        let elapsed = self.start.elapsed();
        self.start = Instant::now();
        elapsed
    }
}

impl Default for MicroTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Performance comparison result
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    /// Name of the baseline (faster) operation
    pub baseline_name: String,
    /// Time for baseline operation in ms
    pub baseline_ms: f64,
    /// Name of the comparison (slower) operation
    pub comparison_name: String,
    /// Time for comparison operation in ms
    pub comparison_ms: f64,
}

impl ComparisonResult {
    /// Calculate speedup ratio (comparison / baseline)
    pub fn speedup_ratio(&self) -> f64 {
        if self.baseline_ms == 0.0 {
            return 1.0;
        }
        self.comparison_ms / self.baseline_ms
    }

    /// Calculate percentage improvement
    pub fn improvement_pct(&self) -> f64 {
        (1.0 - self.baseline_ms / self.comparison_ms) * 100.0
    }

    /// Format as pretty string
    pub fn format(&self) -> String {
        format!(
            "{}: {:.2}ms vs {}: {:.2}ms ({:.1}x faster, {:.1}% improvement)",
            self.baseline_name,
            self.baseline_ms,
            self.comparison_name,
            self.comparison_ms,
            self.speedup_ratio(),
            self.improvement_pct()
        )
    }
}

/// Timing breakdown for sliver cold start
#[derive(Debug, Clone, Default)]
pub struct ColdStartBreakdown {
    /// Time to unpack tar archive
    pub unpack_ms: f64,
    /// Time to restore VFS entries
    pub vfs_restore_ms: f64,
    /// Time to restore V8 snapshot
    pub snapshot_restore_ms: f64,
    /// Time for any additional setup
    pub setup_ms: f64,
}

impl ColdStartBreakdown {
    /// Total cold start time
    pub fn total_ms(&self) -> f64 {
        self.unpack_ms + self.vfs_restore_ms + self.snapshot_restore_ms + self.setup_ms
    }

    /// Format as pretty table
    pub fn format_table(&self) -> String {
        format!(
            "Cold Start Breakdown:\n\
             ├─ Unpack:      {:>8.2} ms\n\
             ├─ VFS Restore: {:>8.2} ms\n\
             ├─ Snapshot:    {:>8.2} ms\n\
             ├─ Setup:       {:>8.2} ms\n\
             └─ Total:       {:>8.2} ms",
            self.unpack_ms,
            self.vfs_restore_ms,
            self.snapshot_restore_ms,
            self.setup_ms,
            self.total_ms()
        )
    }
}

/// Create a test sliver with specified number of files
///
/// # Arguments
/// * `size` - Number of files to include in the sliver VFS
///
/// # Returns
/// Tuple of (metadata, heap_data, vfs_entries) ready for packing
pub fn create_test_sliver_data(size: usize) -> TestSliverData {
    use crate::sliver::SliverMetadata;
    use crate::vfs::{VfsFile, VfsPath};

    let hostname = format!("bench-{}.example.com", size);
    let metadata = SliverMetadata::new(&hostname, "1.1.0");

    // Simulate 1MB heap snapshot
    let heap_data = vec![0xABu8; 1024 * 1024];

    // Create VFS entries
    let vfs_entries: Vec<(VfsPath, VfsFile)> = (0..size)
        .map(|i| {
            let path = VfsPath::new(&format!("data/file{:04}.txt", i)).unwrap();
            // Each file is ~1KB
            let content = format!("File {} content: {}", i, "x".repeat(1000)).into_bytes();
            let file = VfsFile::new(content);
            (path, file)
        })
        .collect();

    TestSliverData {
        metadata,
        heap_data,
        vfs_entries,
    }
}

/// Test sliver data ready for packing
pub struct TestSliverData {
    /// Sliver metadata
    pub metadata: crate::sliver::SliverMetadata,
    /// Simulated V8 heap snapshot
    pub heap_data: Vec<u8>,
    /// VFS entries
    pub vfs_entries: Vec<(crate::vfs::VfsPath, crate::vfs::VfsFile)>,
}

/// Measure time for an operation
pub fn measure<T, F: FnOnce() -> T>(operation: F) -> (T, Duration) {
    let timer = MicroTimer::new();
    let result = operation();
    let elapsed = timer.elapsed();
    (result, elapsed)
}

/// Format duration in human-readable format
pub fn format_duration(duration: Duration) -> String {
    let micros = duration.as_micros();
    if micros < 1000 {
        format!("{} µs", micros)
    } else if micros < 1_000_000 {
        format!("{:.2} ms", micros as f64 / 1000.0)
    } else {
        format!("{:.2} s", micros as f64 / 1_000_000.0)
    }
}

/// Compare multiple operations
pub fn compare_operations(
    baseline: (&str, f64),
    comparisons: &[(&str, f64)],
) -> Vec<ComparisonResult> {
    comparisons
        .iter()
        .map(|(name, time)| ComparisonResult {
            baseline_name: baseline.0.to_string(),
            baseline_ms: baseline.1,
            comparison_name: name.to_string(),
            comparison_ms: *time,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_micro_timer() {
        let mut timer = MicroTimer::new();

        // Small delay
        std::thread::sleep(Duration::from_millis(1));

        let elapsed = timer.elapsed_ms();
        assert!(elapsed >= 1.0, "Timer should measure at least 1ms");

        // Reset and measure again
        timer.start();
        let elapsed2 = timer.elapsed_ms();
        assert!(elapsed2 < elapsed, "After reset, elapsed should be smaller");
    }

    #[test]
    fn test_comparison_result() {
        let result = ComparisonResult {
            baseline_name: "sliver".to_string(),
            baseline_ms: 2.0,
            comparison_name: "fresh".to_string(),
            comparison_ms: 50.0,
        };

        assert_eq!(result.speedup_ratio(), 25.0);
        assert!((result.improvement_pct() - 96.0).abs() < 0.1);

        let formatted = result.format();
        assert!(formatted.contains("sliver"));
        assert!(formatted.contains("25.0x faster"));
    }

    #[test]
    fn test_cold_start_breakdown() {
        let breakdown = ColdStartBreakdown {
            unpack_ms: 1.0,
            vfs_restore_ms: 2.0,
            snapshot_restore_ms: 3.0,
            setup_ms: 4.0,
        };

        assert_eq!(breakdown.total_ms(), 10.0);

        let table = breakdown.format_table();
        assert!(table.contains("Unpack"));
        assert!(table.contains("10.00"));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_micros(500)), "500 µs");
        assert!(format_duration(Duration::from_millis(5)).contains("5.00 ms"));
        assert!(format_duration(Duration::from_secs(2)).contains("2.00 s"));
    }
}
