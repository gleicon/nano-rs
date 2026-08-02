//! Metrics collection registry
//!
//! Provides a centralized registry for all application metrics.
//! The [`MetricsRegistry`] holds all metric instances and provides
//! methods to record and read metric values.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::metrics::types::{Counter, CounterVec, GaugeVec, HistogramVec};
use crate::metrics::REQUEST_DURATION_BUCKETS;

/// Global metrics registry for NANO runtime
///
/// This registry holds all the metrics exposed to Prometheus.
/// It provides a thread-safe interface for recording metrics during
/// request processing and system operation.
///
/// # Metrics
///
/// | Metric | Type | Description |
/// |--------|------|-------------|
/// | `nano_requests_total` | Counter | Total HTTP requests by hostname and status |
/// | `nano_request_duration_ms` | Histogram | Request latency in milliseconds |
/// | `nano_errors_total` | Counter | Total errors by hostname and error code |
/// | `nano_isolates_active` | Gauge | Number of active isolates per hostname/worker |
/// | `nano_memory_bytes` | Gauge | Memory usage in bytes per hostname/isolate |
/// | `nano_worker_utilization` | Gauge | Worker utilization percentage |
/// | `nano_heap_limit_hits_total` | Counter | Total heap limit enforcement events |
/// | `nano_cpu_timeout_total` | Counter | Total CPU timeout enforcement events |
#[derive(Debug)]
pub struct MetricsRegistry {
    /// Total HTTP requests by hostname and status code
    pub requests_total: CounterVec,
    /// Request latency histogram by hostname
    pub request_duration: HistogramVec,
    /// Total errors by hostname and error code
    pub errors_total: CounterVec,
    /// Active isolates by hostname and worker_id
    pub isolates_active: GaugeVec,
    /// Memory usage in bytes by hostname and isolate_id
    pub memory_bytes: GaugeVec,
    /// Worker utilization percentage by hostname and worker_id
    pub worker_utilization: GaugeVec,
    /// Total heap limit enforcement events
    pub heap_limit_hits_total: Counter,
    /// Total CPU timeout enforcement events
    pub cpu_timeout_total: Counter,
    /// WASM modules rejected for exceeding MODULE_SIZE_BYTES_MAX
    pub wasm_size_rejected_total: Counter,
    /// WASM modules rejected for exceeding IMPORT_COUNT_MAX
    pub wasm_import_rejected_total: Counter,
    /// WASM modules rejected for exceeding EXPORT_COUNT_MAX
    pub wasm_export_rejected_total: Counter,
    /// Isolate OOMs attributed to WASM linear memory growth (heap limit fired during WASM execution)
    pub wasm_memory_oom_total: Counter,
    /// Total runtime uptime in seconds (monotonic)
    uptime_seconds: AtomicU64,
    /// Timestamp when registry was created (for uptime calculation)
    start_time: std::time::Instant,
}

impl MetricsRegistry {
    /// Create a new metrics registry with all predefined metrics
    pub fn new() -> Self {
        Self {
            requests_total: CounterVec::new(vec!["hostname", "status"]),
            request_duration: HistogramVec::new(
                vec!["hostname"],
                REQUEST_DURATION_BUCKETS.to_vec(),
            ),
            errors_total: CounterVec::new(vec!["hostname", "code"]),
            isolates_active: GaugeVec::new(vec!["hostname", "worker_id"]),
            memory_bytes: GaugeVec::new(vec!["hostname", "isolate_id"]),
            worker_utilization: GaugeVec::new(vec!["hostname", "worker_id"]),
            heap_limit_hits_total: Counter::new(),
            cpu_timeout_total: Counter::new(),
            wasm_size_rejected_total: Counter::new(),
            wasm_import_rejected_total: Counter::new(),
            wasm_export_rejected_total: Counter::new(),
            wasm_memory_oom_total: Counter::new(),
            uptime_seconds: AtomicU64::new(0),
            start_time: std::time::Instant::now(),
        }
    }

    /// Record a request completion
    ///
    /// Increments the request counter and records latency.
    /// This is the main method to call from request handlers.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The virtual hostname
    /// * `status` - HTTP status code as string (e.g., "200", "500")
    /// * `duration_ms` - Request processing time in milliseconds
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::metrics::MetricsRegistry;
    ///
    /// let metrics = MetricsRegistry::new();
    /// metrics.record_request("api.example.com", "200", 42.5);
    /// ```
    pub fn record_request(&self, hostname: &str, status: &str, duration_ms: f64) {
        // Increment request counter
        self.requests_total.inc(vec![hostname, status]);

        // Record latency histogram
        self.request_duration.observe(vec![hostname], duration_ms);

        // If it's an error (5xx), also increment error counter
        if status.starts_with('5') || status.starts_with('4') {
            self.errors_total.inc(vec![hostname, status]);
        }
    }

    /// Record an error
    ///
    /// Increments the error counter for the given hostname and error code.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The virtual hostname
    /// * `code` - Error code or HTTP status
    pub fn record_error(&self, hostname: &str, code: &str) {
        self.errors_total.inc(vec![hostname, code]);
    }

    /// Set active isolate count for a hostname/worker
    ///
    /// # Arguments
    ///
    /// * `hostname` - The virtual hostname
    /// * `worker_id` - Worker identifier
    /// * `count` - Number of active isolates
    pub fn set_isolates_active(&self, hostname: &str, worker_id: &str, count: u64) {
        self.isolates_active.set(vec![hostname, worker_id], count);
    }

    /// Set memory usage for a hostname/isolate
    ///
    /// # Arguments
    ///
    /// * `hostname` - The virtual hostname
    /// * `isolate_id` - Isolate identifier
    /// * `bytes` - Memory usage in bytes
    pub fn set_memory_bytes(&self, hostname: &str, isolate_id: &str, bytes: u64) {
        self.memory_bytes.set(vec![hostname, isolate_id], bytes);
    }

    /// Set worker utilization percentage
    ///
    /// # Arguments
    ///
    /// * `hostname` - The virtual hostname
    /// * `worker_id` - Worker identifier
    /// * `percent` - Utilization percentage (0-100)
    pub fn set_worker_utilization(&self, hostname: &str, worker_id: &str, percent: u64) {
        // Clamp to 0-100 range
        let clamped = percent.min(100);
        self.worker_utilization
            .set(vec![hostname, worker_id], clamped);
    }

    /// Record a heap limit enforcement event
    ///
    /// Increments the counter when an isolate is terminated due to
    /// exceeding its heap memory limit.
    pub fn record_heap_limit_hit(&self) {
        self.heap_limit_hits_total.inc();
    }

    /// Record a CPU timeout enforcement event
    ///
    /// Increments the counter when a JavaScript execution is terminated
    /// due to exceeding its CPU time limit.
    pub fn record_cpu_timeout(&self) {
        self.cpu_timeout_total.inc();
    }

    /// Update uptime counter
    ///
    /// Should be called periodically (e.g., every second) to update
    /// the monotonic uptime counter.
    pub fn update_uptime(&self) {
        let elapsed = self.start_time.elapsed().as_secs();
        self.uptime_seconds.store(elapsed, Ordering::Relaxed);
    }

    /// Get current uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.uptime_seconds.load(Ordering::Relaxed)
    }

    /// Get all metric names and their types
    ///
    /// Returns a list of (name, type, description) tuples for documentation.
    pub fn metric_descriptions(&self) -> Vec<(&'static str, &'static str, &'static str)> {
        vec![
            ("nano_requests_total", "counter", "Total HTTP requests"),
            (
                "nano_request_duration_ms",
                "histogram",
                "Request latency in milliseconds",
            ),
            (
                "nano_errors_total",
                "counter",
                "Total errors by status code",
            ),
            ("nano_isolates_active", "gauge", "Number of active isolates"),
            ("nano_memory_bytes", "gauge", "Memory usage in bytes"),
            (
                "nano_worker_utilization",
                "gauge",
                "Worker utilization percentage",
            ),
            ("nano_uptime_seconds", "gauge", "Runtime uptime in seconds"),
            (
                "nano_heap_limit_hits_total",
                "counter",
                "Total heap limit enforcement events",
            ),
            (
                "nano_cpu_timeout_total",
                "counter",
                "Total CPU timeout enforcement events",
            ),
        ]
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = MetricsRegistry::new();

        // Verify all metric collections exist
        assert_eq!(registry.requests_total.label_names().len(), 2);
        assert_eq!(registry.request_duration.label_names().len(), 1);
        assert_eq!(registry.errors_total.label_names().len(), 2);
        assert_eq!(registry.isolates_active.label_names().len(), 2);
        assert_eq!(registry.memory_bytes.label_names().len(), 2);
        assert_eq!(registry.worker_utilization.label_names().len(), 2);
    }

    #[test]
    fn test_record_request() {
        let registry = MetricsRegistry::new();

        registry.record_request("api.example.com", "200", 42.5);
        registry.record_request("api.example.com", "200", 55.0);
        registry.record_request("api.example.com", "500", 100.0);

        // Check counters
        let requests = registry.requests_total.get_all();
        assert_eq!(requests.len(), 2);

        // Check histograms
        let durations = registry.request_duration.get_all();
        assert_eq!(durations.len(), 1); // Only one hostname

        // Check errors
        let errors = registry.errors_total.get_all();
        assert_eq!(errors.len(), 1); // Only the 500 error
    }

    #[test]
    fn test_record_error() {
        let registry = MetricsRegistry::new();

        registry.record_error("api.example.com", "timeout");
        registry.record_error("api.example.com", "timeout");
        registry.record_error("api.example.com", "oom");

        let errors = registry.errors_total.get_all();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_gauge_operations() {
        let registry = MetricsRegistry::new();

        registry.set_isolates_active("api.example.com", "worker-1", 5);
        registry.set_memory_bytes("api.example.com", "iso-123", 16777216);
        registry.set_worker_utilization("api.example.com", "worker-1", 75);

        let isolates = registry.isolates_active.get_all();
        assert_eq!(isolates.len(), 1);

        let memory = registry.memory_bytes.get_all();
        assert_eq!(memory.len(), 1);

        let utilization = registry.worker_utilization.get_all();
        assert_eq!(utilization.len(), 1);

        // Check clamping
        registry.set_worker_utilization("api.example.com", "worker-1", 150);
        let utilization = registry.worker_utilization.get_all();
        assert_eq!(utilization[0].1, 100); // Should be clamped to 100
    }

    #[test]
    fn test_uptime() {
        let registry = MetricsRegistry::new();

        // Initially should be 0 or very small
        let initial = registry.uptime_seconds();
        assert!(initial < 2);

        // Update uptime
        registry.update_uptime();
        let updated = registry.uptime_seconds();

        // Should be >= initial
        assert!(updated >= initial);
    }

    #[test]
    fn test_metric_descriptions() {
        let registry = MetricsRegistry::new();
        let descriptions = registry.metric_descriptions();

        // Verify we have the expected metrics
        let names: Vec<_> = descriptions.iter().map(|(n, _, _)| *n).collect();
        assert!(names.contains(&"nano_requests_total"));
        assert!(names.contains(&"nano_request_duration_ms"));
        assert!(names.contains(&"nano_errors_total"));
        assert!(names.contains(&"nano_isolates_active"));
        assert!(names.contains(&"nano_memory_bytes"));
        assert!(names.contains(&"nano_worker_utilization"));
        assert!(names.contains(&"nano_uptime_seconds"));
    }

    #[test]
    fn test_default_registry() {
        let registry: MetricsRegistry = Default::default();

        // Should be functional
        registry.record_request("test.com", "200", 10.0);

        let requests = registry.requests_total.get_all();
        assert_eq!(requests.len(), 1);
    }
}
