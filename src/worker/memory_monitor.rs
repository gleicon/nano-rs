//! Post-execution memory monitoring with pressure detection.
//!
//! STATUS: wired. The worker pool creates a `MemoryMonitor` per worker (when a
//! memory limit is set) and calls `check_after` after each request; at Critical or
//! Emergency pressure it recycles the isolate (soft eviction) and records a pressure
//! event in the tenant metrics. See `worker/pool.rs`. The separate cross-isolate LRU
//! `EvictionManager` is not yet wired — see that module and docs/ROADMAP.md.
//!
//! This module provides memory monitoring that runs after each JavaScript
//! execution to detect memory pressure and trigger soft eviction. It tracks
//! memory trends over time to detect memory leaks and implements Cloudflare-style
//! graceful degradation under memory pressure.
//!
//! ## Architecture
//!
//! - `MemoryMonitor`: Tracks heap usage and calculates pressure levels
//! - `MemorySnapshot`: Point-in-time memory state with trend analysis
//! - `MemoryPressureLevel`: Four-tier pressure classification
//! - `MemoryTrend`: Growing/Stable/Shrinking classification
//!
//! ## Pressure Levels
//!
//! - Normal: <70% of limit - normal operation
//! - Warning: 70-85% of limit - log warning, may throttle
//! - Critical: 85-95% of limit - trigger soft eviction
//! - Emergency: >95% of limit - hard eviction required
//!
//! ## Integration
//!
//! Called from WorkerPool after each handler execution. Results feed into
//! EvictionManager for isolate lifecycle decisions.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Memory pressure classification levels
///
/// Four-tier system based on percentage of memory limit:
/// - Normal: No action needed
/// - Warning: Log and monitor closely
/// - Critical: Begin soft eviction
/// - Emergency: Hard eviction required
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryPressureLevel {
    /// <70% of limit - normal operation
    Normal = 0,
    /// 70-85% of limit - elevated usage, monitor closely
    Warning = 1,
    /// 85-95% of limit - critical pressure, trigger soft eviction
    Critical = 2,
    /// >95% of limit - emergency, hard eviction required
    Emergency = 3,
}

impl MemoryPressureLevel {
    /// Convert a percentage (0.0-1.0) to a pressure level
    ///
    /// Thresholds:
    /// - <0.70: Normal
    /// - 0.70-0.85: Warning
    /// - 0.85-0.95: Critical
    /// - >0.95: Emergency
    pub fn from_percent(percent: f64) -> Self {
        match percent {
            p if p < 0.70 => MemoryPressureLevel::Normal,
            p if p < 0.85 => MemoryPressureLevel::Warning,
            p if p < 0.95 => MemoryPressureLevel::Critical,
            _ => MemoryPressureLevel::Emergency,
        }
    }

    /// Get the threshold percentage for this level
    pub fn threshold_percent(&self) -> f64 {
        match self {
            MemoryPressureLevel::Normal => 0.70,
            MemoryPressureLevel::Warning => 0.85,
            MemoryPressureLevel::Critical => 0.95,
            MemoryPressureLevel::Emergency => 1.0,
        }
    }

    /// Check if this level requires eviction action
    pub fn requires_eviction(&self) -> bool {
        matches!(
            self,
            MemoryPressureLevel::Critical | MemoryPressureLevel::Emergency
        )
    }

    /// Check if this level requires hard (immediate) eviction
    pub fn requires_hard_eviction(&self) -> bool {
        matches!(self, MemoryPressureLevel::Emergency)
    }

    /// Get a human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            MemoryPressureLevel::Normal => "Normal memory usage",
            MemoryPressureLevel::Warning => "Elevated memory usage - monitoring",
            MemoryPressureLevel::Critical => "Critical memory pressure - soft eviction",
            MemoryPressureLevel::Emergency => "Emergency memory pressure - hard eviction",
        }
    }
}

impl Default for MemoryPressureLevel {
    fn default() -> Self {
        MemoryPressureLevel::Normal
    }
}

/// Memory trend classification
///
/// Tracks whether memory usage is growing, stable, or shrinking over time.
/// Growth/shrink rates are in MB per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemoryTrend {
    /// Memory usage is growing at the given rate (MB/s)
    Growing(f64),
    /// Memory usage is stable (within threshold)
    Stable,
    /// Memory usage is shrinking at the given rate (MB/s)
    Shrinking(f64),
}

impl MemoryTrend {
    /// Check if this trend indicates a potential memory leak
    ///
    /// Returns true if growing faster than 1MB/s sustained
    pub fn is_potential_leak(&self) -> bool {
        match self {
            MemoryTrend::Growing(rate) => *rate > 1.0, // >1MB/s is suspicious
            _ => false,
        }
    }

    /// Get the trend as a string description
    pub fn description(&self) -> String {
        match self {
            MemoryTrend::Growing(rate) => format!("Growing at {:.2} MB/s", rate),
            MemoryTrend::Stable => "Stable".to_string(),
            MemoryTrend::Shrinking(rate) => format!("Shrinking at {:.2} MB/s", rate),
        }
    }
}

impl Default for MemoryTrend {
    fn default() -> Self {
        MemoryTrend::Stable
    }
}

/// Point-in-time memory snapshot
///
/// Captures heap statistics, pressure level, and trend at a specific moment.
/// Used for history tracking and trend analysis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemorySnapshot {
    /// When this snapshot was taken
    pub timestamp: Instant,
    /// Used heap size in bytes
    pub heap_used: usize,
    /// Total heap size in bytes
    pub heap_total: usize,
    /// External memory allocated (ArrayBuffers, etc.)
    pub external: usize,
    /// Current pressure level
    pub pressure_level: MemoryPressureLevel,
    /// Memory usage trend
    pub trend: MemoryTrend,
}

impl MemorySnapshot {
    /// Create a new memory snapshot
    ///
    /// # Arguments
    /// * `heap_used` - Used heap size in bytes
    /// * `heap_total` - Total heap size in bytes
    /// * `external` - External memory in bytes
    /// * `limit_bytes` - Memory limit for pressure calculation
    pub fn new(heap_used: usize, heap_total: usize, external: usize, limit_bytes: usize) -> Self {
        let total = heap_used.saturating_add(external);
        let percent = if limit_bytes > 0 {
            total as f64 / limit_bytes as f64
        } else {
            0.0
        };

        Self {
            timestamp: Instant::now(),
            heap_used,
            heap_total,
            external,
            pressure_level: MemoryPressureLevel::from_percent(percent),
            trend: MemoryTrend::Stable,
        }
    }

    /// Get total memory (heap + external) in bytes
    pub fn total_memory_bytes(&self) -> usize {
        self.heap_used.saturating_add(self.external)
    }

    /// Get total memory in MB
    pub fn total_memory_mb(&self) -> f64 {
        self.total_memory_bytes() as f64 / (1024.0 * 1024.0)
    }

    /// Get heap used in MB
    pub fn heap_used_mb(&self) -> f64 {
        self.heap_used as f64 / (1024.0 * 1024.0)
    }

    /// Get external memory in MB
    pub fn external_mb(&self) -> f64 {
        self.external as f64 / (1024.0 * 1024.0)
    }

    /// Get percentage of limit used (0.0-1.0+)
    pub fn percent_of_limit(&self, limit_bytes: usize) -> f64 {
        if limit_bytes == 0 {
            return 0.0;
        }
        self.total_memory_bytes() as f64 / limit_bytes as f64
    }

    /// Create an empty snapshot (for testing)
    pub fn empty() -> Self {
        Self {
            timestamp: Instant::now(),
            heap_used: 0,
            heap_total: 0,
            external: 0,
            pressure_level: MemoryPressureLevel::Normal,
            trend: MemoryTrend::Stable,
        }
    }
}

/// Configuration for memory monitoring behavior
#[derive(Debug, Clone, Copy)]
pub struct MemoryMonitorConfig {
    /// Soft limit percentage (0.0-1.0, default 0.80)
    pub soft_limit_percent: f64,
    /// Critical limit percentage (0.0-1.0, default 0.95)
    pub critical_limit_percent: f64,
    /// Maximum history size (default 10)
    pub max_history: usize,
    /// Trend stability threshold in MB (default 0.5)
    pub trend_stability_threshold_mb: f64,
}

impl Default for MemoryMonitorConfig {
    fn default() -> Self {
        Self {
            soft_limit_percent: 0.80,
            critical_limit_percent: 0.95,
            max_history: 10,
            trend_stability_threshold_mb: 0.5,
        }
    }
}

/// Memory monitor for post-execution checking
///
/// Tracks memory usage after each JavaScript execution and maintains
/// a history for trend analysis. Triggers soft eviction when pressure
/// exceeds configured thresholds.
#[derive(Debug)]
pub struct MemoryMonitor {
    config: MemoryMonitorConfig,
    limit_bytes: usize,
    history: VecDeque<MemorySnapshot>,
    last_check: Option<Instant>,
}

impl MemoryMonitor {
    /// Create a new memory monitor with the given limit
    ///
    /// # Arguments
    /// * `limit_mb` - Memory limit in megabytes
    pub fn new(limit_mb: u32) -> Self {
        Self::with_config(limit_mb, MemoryMonitorConfig::default())
    }

    /// Create a new memory monitor with custom configuration
    ///
    /// # Arguments
    /// * `limit_mb` - Memory limit in megabytes
    /// * `config` - Memory monitor configuration
    pub fn with_config(limit_mb: u32, config: MemoryMonitorConfig) -> Self {
        let limit_bytes = (limit_mb as usize) * 1024 * 1024;

        Self {
            config,
            limit_bytes,
            history: VecDeque::with_capacity(config.max_history),
            last_check: None,
        }
    }

    /// Check memory after execution and return a snapshot
    ///
    /// This method should be called after every JavaScript handler execution.
    /// It queries V8 heap statistics, calculates pressure level, and updates
    /// the trend based on historical data.
    ///
    /// # Arguments
    /// * `isolate` - The V8 isolate to check
    ///
    /// # Returns
    /// A MemorySnapshot containing the current state
    pub fn check_after(&mut self, isolate: &mut v8::Isolate) -> MemorySnapshot {
        let stats = isolate.get_heap_statistics();

        let mut snapshot = MemorySnapshot::new(
            stats.used_heap_size(),
            stats.total_heap_size(),
            stats.external_memory(),
            self.limit_bytes,
        );

        // Calculate trend based on history
        snapshot.trend = self.calculate_trend(&snapshot);

        // Add to history
        self.add_to_history(snapshot);

        self.last_check = Some(Instant::now());

        snapshot
    }

    /// Get the current pressure level from the most recent snapshot
    pub fn pressure_level(&self) -> MemoryPressureLevel {
        self.history
            .back()
            .map(|s| s.pressure_level)
            .unwrap_or(MemoryPressureLevel::Normal)
    }

    /// Check if soft eviction should be triggered
    ///
    /// Returns true if current pressure is at or above Critical level
    pub fn should_trigger_soft_eviction(&self) -> bool {
        self.pressure_level().requires_eviction()
    }

    /// Check if hard eviction should be triggered
    ///
    /// Returns true if current pressure is at Emergency level
    pub fn should_trigger_hard_eviction(&self) -> bool {
        self.pressure_level().requires_hard_eviction()
    }

    /// Get the most recent snapshot if available
    pub fn latest_snapshot(&self) -> Option<&MemorySnapshot> {
        self.history.back()
    }

    /// Get all snapshots in history (oldest first)
    pub fn history(&self) -> &VecDeque<MemorySnapshot> {
        &self.history
    }

    /// Get the number of snapshots in history
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Clear the history
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Check if memory is trending toward a leak
    ///
    /// Returns true if the last 3 snapshots all show Growing trend
    /// with significant growth rates
    pub fn is_trending_to_leak(&self) -> bool {
        if self.history.len() < 3 {
            return false;
        }

        // Check last 3 snapshots
        let recent: Vec<_> = self.history.iter().rev().take(3).collect();
        recent.iter().all(|s| s.trend.is_potential_leak())
    }

    /// Get time since last check
    pub fn time_since_last_check(&self) -> Option<Duration> {
        self.last_check.map(|t| t.elapsed())
    }

    /// Calculate soft limit in bytes
    pub fn soft_limit_bytes(&self) -> usize {
        (self.limit_bytes as f64 * self.config.soft_limit_percent) as usize
    }

    /// Calculate critical limit in bytes
    pub fn critical_limit_bytes(&self) -> usize {
        (self.limit_bytes as f64 * self.config.critical_limit_percent) as usize
    }

    /// Get the memory limit in bytes
    pub fn limit_bytes(&self) -> usize {
        self.limit_bytes
    }

    /// Get the memory limit in MB
    pub fn limit_mb(&self) -> usize {
        self.limit_bytes / (1024 * 1024)
    }

    // Private methods

    fn add_to_history(&mut self, snapshot: MemorySnapshot) {
        if self.history.len() >= self.config.max_history {
            self.history.pop_front();
        }
        self.history.push_back(snapshot);
    }

    fn calculate_trend(&self, current: &MemorySnapshot) -> MemoryTrend {
        if self.history.is_empty() {
            return MemoryTrend::Stable;
        }

        let prev = self.history.back().unwrap();
        let memory_delta = current.total_memory_bytes() as f64 - prev.total_memory_bytes() as f64;
        let time_delta = current
            .timestamp
            .duration_since(prev.timestamp)
            .as_secs_f64();

        if time_delta < 0.001 {
            // Too soon to calculate meaningful trend
            return MemoryTrend::Stable;
        }

        // Convert to MB/s
        let rate = (memory_delta / (1024.0 * 1024.0)) / time_delta;
        let threshold = self.config.trend_stability_threshold_mb;

        if rate > threshold {
            MemoryTrend::Growing(rate)
        } else if rate < -threshold {
            MemoryTrend::Shrinking(rate.abs())
        } else {
            MemoryTrend::Stable
        }
    }
}

impl Default for MemoryMonitor {
    fn default() -> Self {
        Self::new(128) // Default 128MB limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pressure_level_from_percent() {
        assert_eq!(
            MemoryPressureLevel::from_percent(0.5),
            MemoryPressureLevel::Normal
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.69),
            MemoryPressureLevel::Normal
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.70),
            MemoryPressureLevel::Warning
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.84),
            MemoryPressureLevel::Warning
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.85),
            MemoryPressureLevel::Critical
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.94),
            MemoryPressureLevel::Critical
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(0.95),
            MemoryPressureLevel::Emergency
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(1.0),
            MemoryPressureLevel::Emergency
        );
        assert_eq!(
            MemoryPressureLevel::from_percent(1.5),
            MemoryPressureLevel::Emergency
        );
    }

    #[test]
    fn test_pressure_level_thresholds() {
        assert!(!MemoryPressureLevel::Normal.requires_eviction());
        assert!(!MemoryPressureLevel::Warning.requires_eviction());
        assert!(MemoryPressureLevel::Critical.requires_eviction());
        assert!(MemoryPressureLevel::Emergency.requires_eviction());

        assert!(!MemoryPressureLevel::Normal.requires_hard_eviction());
        assert!(!MemoryPressureLevel::Warning.requires_hard_eviction());
        assert!(!MemoryPressureLevel::Critical.requires_hard_eviction());
        assert!(MemoryPressureLevel::Emergency.requires_hard_eviction());
    }

    #[test]
    fn test_memory_trend_leak_detection() {
        assert!(!MemoryTrend::Growing(0.5).is_potential_leak()); // Below 1MB/s
        assert!(MemoryTrend::Growing(1.5).is_potential_leak()); // Above 1MB/s
        assert!(!MemoryTrend::Stable.is_potential_leak());
        assert!(!MemoryTrend::Shrinking(1.5).is_potential_leak());
    }

    #[test]
    fn test_memory_snapshot_creation() {
        let snapshot = MemorySnapshot::new(
            100 * 1024 * 1024, // 100MB used
            150 * 1024 * 1024, // 150MB total
            50 * 1024 * 1024,  // 50MB external
            256 * 1024 * 1024, // 256MB limit
        );

        assert_eq!(snapshot.heap_used_mb(), 100.0);
        assert_eq!(snapshot.external_mb(), 50.0);
        assert_eq!(snapshot.total_memory_mb(), 150.0);
        assert_eq!(snapshot.percent_of_limit(256 * 1024 * 1024), 150.0 / 256.0);
    }

    #[test]
    fn test_memory_snapshot_pressure_calculation() {
        // 50% of limit -> Normal
        let normal = MemorySnapshot::new(64 * 1024 * 1024, 128 * 1024 * 1024, 0, 128 * 1024 * 1024);
        assert_eq!(normal.pressure_level, MemoryPressureLevel::Normal);

        // 80% of limit -> Warning
        let warning =
            MemorySnapshot::new(102 * 1024 * 1024, 128 * 1024 * 1024, 0, 128 * 1024 * 1024);
        assert_eq!(warning.pressure_level, MemoryPressureLevel::Warning);

        // 90% of limit -> Critical
        let critical =
            MemorySnapshot::new(115 * 1024 * 1024, 128 * 1024 * 1024, 0, 128 * 1024 * 1024);
        assert_eq!(critical.pressure_level, MemoryPressureLevel::Critical);

        // 98% of limit -> Emergency
        let emergency =
            MemorySnapshot::new(125 * 1024 * 1024, 128 * 1024 * 1024, 0, 128 * 1024 * 1024);
        assert_eq!(emergency.pressure_level, MemoryPressureLevel::Emergency);
    }

    /// The worker recycles the isolate exactly when
    /// `snapshot.pressure_level.requires_eviction()` is true (see worker/pool.rs):
    /// Critical or Emergency, never Normal/Warning. Pins that recycle trigger.
    #[test]
    fn eviction_trigger_matches_worker_recycle_condition() {
        let lim = 128 * 1024 * 1024;
        let mk = |used: usize| MemorySnapshot::new(used, 128 * 1024 * 1024, 0, lim);
        assert!(!mk(64 * 1024 * 1024).pressure_level.requires_eviction(), "Normal");
        assert!(!mk(102 * 1024 * 1024).pressure_level.requires_eviction(), "Warning");
        assert!(mk(115 * 1024 * 1024).pressure_level.requires_eviction(), "Critical");
        assert!(mk(125 * 1024 * 1024).pressure_level.requires_eviction(), "Emergency");
    }

    #[test]
    fn test_memory_monitor_creation() {
        let monitor = MemoryMonitor::new(128);
        assert_eq!(monitor.limit_mb(), 128);
        assert_eq!(monitor.history_len(), 0);
    }

    #[test]
    fn test_memory_monitor_with_config() {
        let config = MemoryMonitorConfig {
            soft_limit_percent: 0.75,
            critical_limit_percent: 0.90,
            max_history: 20,
            trend_stability_threshold_mb: 1.0,
        };

        let monitor = MemoryMonitor::with_config(256, config);
        assert_eq!(monitor.limit_mb(), 256);
        assert_eq!(
            monitor.soft_limit_bytes(),
            (256.0 * 0.75 * 1024.0 * 1024.0) as usize
        );
        assert_eq!(
            monitor.critical_limit_bytes(),
            (256.0 * 0.90 * 1024.0 * 1024.0) as usize
        );
    }

    #[test]
    fn test_memory_monitor_empty_history() {
        let monitor = MemoryMonitor::new(128);
        assert_eq!(monitor.pressure_level(), MemoryPressureLevel::Normal);
        assert!(!monitor.should_trigger_soft_eviction());
        assert!(!monitor.should_trigger_hard_eviction());
        assert!(monitor.latest_snapshot().is_none());
    }

    #[test]
    fn test_memory_monitor_config_default() {
        let config = MemoryMonitorConfig::default();
        assert_eq!(config.soft_limit_percent, 0.80);
        assert_eq!(config.critical_limit_percent, 0.95);
        assert_eq!(config.max_history, 10);
        assert_eq!(config.trend_stability_threshold_mb, 0.5);
    }

    #[test]
    fn test_memory_trend_descriptions() {
        assert!(MemoryTrend::Growing(2.5).description().contains("Growing"));
        assert!(MemoryTrend::Stable.description().contains("Stable"));
        assert!(MemoryTrend::Shrinking(1.0)
            .description()
            .contains("Shrinking"));
    }

    #[test]
    fn test_pressure_level_descriptions() {
        assert!(MemoryPressureLevel::Normal.description().contains("Normal"));
        assert!(MemoryPressureLevel::Warning
            .description()
            .contains("Elevated"));
        assert!(MemoryPressureLevel::Critical
            .description()
            .contains("Critical"));
        assert!(MemoryPressureLevel::Emergency
            .description()
            .contains("Emergency"));
    }

    #[test]
    fn test_memory_snapshot_empty() {
        let empty = MemorySnapshot::empty();
        assert_eq!(empty.heap_used, 0);
        assert_eq!(empty.heap_total, 0);
        assert_eq!(empty.external, 0);
        assert_eq!(empty.pressure_level, MemoryPressureLevel::Normal);
        assert_eq!(empty.trend, MemoryTrend::Stable);
    }
}
