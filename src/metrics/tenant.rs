//! Per-tenant metrics collection for NANO Edge Runtime
//!
//! Provides comprehensive per-tenant (hostname) metrics including:
//! - Request counts (total, success, error, timeout)
//! - CPU time tracking
//! - Memory usage (heap and external)
//! - Latency histograms
//! - Context reset counts
//!
//! These metrics are automatically collected during request execution
//! and exposed via Prometheus and JSON endpoints.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::metrics::types::{Counter, Gauge, Histogram};
use crate::metrics::REQUEST_DURATION_BUCKETS;
use crate::worker::memory_monitor::MemoryPressureLevel;

/// CPU time histogram buckets in seconds
///
/// These buckets cover from 1ms up to 1s+ for per-request CPU time tracking
pub const CPU_TIME_BUCKETS: &[f64] = &[
    0.001,  // 1ms
    0.005,  // 5ms
    0.010,  // 10ms
    0.025,  // 25ms
    0.050,  // 50ms (Cloudflare limit)
    0.100,  // 100ms
    0.250,  // 250ms
    0.500,  // 500ms
    1.0,    // 1s
    f64::INFINITY,
];

/// Memory usage histogram buckets in bytes
///
/// Buckets for tracking per-request memory consumption
pub const MEMORY_BUCKETS: &[f64] = &[
    1024.0,       // 1 KB
    10240.0,      // 10 KB
    102400.0,     // 100 KB
    1048576.0,    // 1 MB
    10485760.0,   // 10 MB
    52428800.0,   // 50 MB
    104857600.0,  // 100 MB
    f64::INFINITY,
];

/// Request result type for metrics categorization
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RequestResult {
    /// Request completed successfully
    Success,
    /// Request resulted in an error
    Error,
    /// Request timed out
    Timeout,
}

impl RequestResult {
    /// Get string representation for metrics labels
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestResult::Success => "success",
            RequestResult::Error => "error",
            RequestResult::Timeout => "timeout",
        }
    }
}

/// Per-tenant metrics collection
///
/// Tracks all metrics for a single tenant (hostname) including:
/// - Request counters (total, success, error, timeout)
/// - CPU time tracking
/// - Memory usage
/// - Latency distributions
/// - Active request tracking
#[derive(Debug)]
pub struct TenantMetrics {
    /// Hostname this metrics collection is for
    pub hostname: String,
    
    // Counters (only increase)
    /// Total requests processed
    pub requests_total: Counter,
    /// Successful requests
    pub requests_success: Counter,
    /// Failed requests
    pub requests_error: Counter,
    /// Timed out requests
    pub requests_timeout: Counter,
    /// Total CPU time consumed (in seconds, stored as microseconds for precision)
    pub cpu_seconds_total: Counter,
    /// Total context resets
    pub context_resets_total: Counter,
    /// Memory pressure events by level
    pub pressure_events: HashMap<MemoryPressureLevel, Counter>,
    
    // Gauges (current value)
    /// Current memory usage in bytes
    pub memory_used_bytes: Gauge,
    /// Current external memory in bytes
    pub memory_external_bytes: Gauge,
    /// Currently active requests
    pub requests_active: Gauge,
    /// Currently active isolates for this tenant
    pub isolates_active: Gauge,
    /// Peak memory observed
    pub memory_peak_bytes: AtomicU64,
    
    // Histograms (distribution)
    /// Request duration distribution
    pub request_duration_seconds: Histogram,
    /// CPU time per request distribution
    pub cpu_time_per_request_seconds: Histogram,
    /// Memory per request distribution
    pub memory_per_request_bytes: Histogram,
}

impl TenantMetrics {
    /// Create new tenant metrics for the given hostname
    pub fn new(hostname: impl Into<String>) -> Self {
        let hostname = hostname.into();
        
        // Initialize pressure event counters
        let mut pressure_events = HashMap::new();
        pressure_events.insert(MemoryPressureLevel::Warning, Counter::new());
        pressure_events.insert(MemoryPressureLevel::Critical, Counter::new());
        pressure_events.insert(MemoryPressureLevel::Emergency, Counter::new());
        
        Self {
            hostname,
            requests_total: Counter::new(),
            requests_success: Counter::new(),
            requests_error: Counter::new(),
            requests_timeout: Counter::new(),
            cpu_seconds_total: Counter::new(),
            context_resets_total: Counter::new(),
            pressure_events,
            memory_used_bytes: Gauge::new(),
            memory_external_bytes: Gauge::new(),
            requests_active: Gauge::new(),
            isolates_active: Gauge::new(),
            memory_peak_bytes: AtomicU64::new(0),
            request_duration_seconds: Histogram::new(REQUEST_DURATION_BUCKETS),
            cpu_time_per_request_seconds: Histogram::new(CPU_TIME_BUCKETS),
            memory_per_request_bytes: Histogram::new(MEMORY_BUCKETS),
        }
    }
    
    /// Record a completed request
    ///
    /// # Arguments
    ///
    /// * `result` - Result type (success, error, timeout)
    /// * `cpu_time_us` - CPU time consumed in microseconds
    /// * `memory_bytes` - Memory used for this request
    /// * `duration_ms` - Wall-clock duration in milliseconds
    pub fn record_request(
        &self,
        result: RequestResult,
        cpu_time_us: u64,
        memory_bytes: usize,
        duration_ms: u64,
    ) {
        // Update counters
        self.requests_total.inc();
        match result {
            RequestResult::Success => self.requests_success.inc(),
            RequestResult::Error => self.requests_error.inc(),
            RequestResult::Timeout => self.requests_timeout.inc(),
        }
        
        // Update CPU counter (convert microseconds to seconds for Prometheus)
        let cpu_seconds = cpu_time_us as f64 / 1_000_000.0;
        self.cpu_seconds_total.inc_by(cpu_seconds as u64);
        
        // Update histograms
        self.request_duration_seconds.observe(duration_ms as f64 / 1000.0);
        self.cpu_time_per_request_seconds.observe(cpu_seconds);
        self.memory_per_request_bytes.observe(memory_bytes as f64);
        
        // Track peak memory
        let current_peak = self.memory_peak_bytes.load(Ordering::Relaxed);
        if memory_bytes as u64 > current_peak {
            let _ = self.memory_peak_bytes.compare_exchange(
                current_peak,
                memory_bytes as u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
    
    /// Update current memory usage
    pub fn update_memory(&self, heap_bytes: usize, external_bytes: usize) {
        self.memory_used_bytes.set(heap_bytes as u64);
        self.memory_external_bytes.set(external_bytes as u64);
        
        // Update peak if needed
        let total = heap_bytes + external_bytes;
        let current_peak = self.memory_peak_bytes.load(Ordering::Relaxed);
        if total as u64 > current_peak {
            let _ = self.memory_peak_bytes.compare_exchange(
                current_peak,
                total as u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
    
    /// Record a context reset
    pub fn record_context_reset(&self) {
        self.context_resets_total.inc();
    }
    
    /// Record a memory pressure event
    pub fn record_pressure_event(&self, level: MemoryPressureLevel) {
        if let Some(counter) = self.pressure_events.get(&level) {
            counter.inc();
        }
    }
    
    /// Increment active requests count
    pub fn inc_active_requests(&self) {
        self.requests_active.inc();
    }
    
    /// Decrement active requests count
    pub fn dec_active_requests(&self) {
        self.requests_active.dec();
    }
    
    /// Calculate requests per second (requires external timing)
    pub fn calculate_rps(&self, elapsed_seconds: f64) -> f64 {
        if elapsed_seconds > 0.0 {
            self.requests_total.get() as f64 / elapsed_seconds
        } else {
            0.0
        }
    }
    
    /// Get the current memory peak in bytes
    pub fn memory_peak_bytes(&self) -> u64 {
        self.memory_peak_bytes.load(Ordering::Relaxed)
    }
    
    /// Get histogram percentile (approximate)
    ///
    /// Returns the value at the given percentile (0.0-1.0) from the
    /// request duration histogram.
    pub fn request_duration_p99(&self) -> f64 {
        let (buckets, _, count) = self.request_duration_seconds.get();
        if count == 0 {
            return 0.0;
        }
        
        let target = (count as f64 * 0.99) as u64;
        let mut cumulative = 0u64;
        
        for (i, &bucket_count) in buckets.iter().enumerate() {
            cumulative += bucket_count;
            if cumulative >= target {
                // Return the bucket upper bound
                return REQUEST_DURATION_BUCKETS.get(i).copied().unwrap_or(f64::INFINITY);
            }
        }
        
        f64::INFINITY
    }
}

/// Global metrics across all tenants
///
/// Tracks aggregate metrics for the entire runtime.
#[derive(Debug)]
pub struct GlobalMetrics {
    /// Total requests across all tenants
    pub total_requests: Counter,
    /// Total CPU seconds across all tenants
    pub total_cpu_seconds: Counter,
    /// Total memory across all tenants
    pub total_memory_bytes: Gauge,
    /// Active isolates across all tenants
    pub active_isolates: Gauge,
    /// Pressure events by level
    pub pressure_events: HashMap<MemoryPressureLevel, Counter>,
}

impl GlobalMetrics {
    /// Create new global metrics
    pub fn new() -> Self {
        let mut pressure_events = HashMap::new();
        pressure_events.insert(MemoryPressureLevel::Warning, Counter::new());
        pressure_events.insert(MemoryPressureLevel::Critical, Counter::new());
        pressure_events.insert(MemoryPressureLevel::Emergency, Counter::new());
        
        Self {
            total_requests: Counter::new(),
            total_cpu_seconds: Counter::new(),
            total_memory_bytes: Gauge::new(),
            active_isolates: Gauge::new(),
            pressure_events,
        }
    }
    
    /// Record a request in global metrics
    pub fn record_request(&self, cpu_seconds: f64) {
        self.total_requests.inc();
        self.total_cpu_seconds.inc_by(cpu_seconds as u64);
    }
    
    /// Record a pressure event
    pub fn record_pressure_event(&self, level: MemoryPressureLevel) {
        if let Some(counter) = self.pressure_events.get(&level) {
            counter.inc();
        }
    }
}

impl Default for GlobalMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Tenant metrics collector
///
/// Manages metrics for all tenants using concurrent data structures.
/// Automatically creates metrics entries for new hostnames on first request.
#[derive(Debug)]
pub struct TenantMetricsCollector {
    /// Per-tenant metrics storage
    tenants: DashMap<String, Arc<RwLock<TenantMetrics>>>,
    /// Global aggregate metrics
    global: Arc<RwLock<GlobalMetrics>>,
}

impl TenantMetricsCollector {
    /// Create a new tenant metrics collector
    pub fn new() -> Self {
        Self {
            tenants: DashMap::new(),
            global: Arc::new(RwLock::new(GlobalMetrics::new())),
        }
    }
    
    /// Record a request completion
    ///
    /// Automatically creates a metrics entry for the hostname if needed.
    ///
    /// # Arguments
    ///
    /// * `hostname` - The tenant hostname
    /// * `result` - Request result type
    /// * `cpu_time_us` - CPU time in microseconds
    /// * `memory_bytes` - Memory usage in bytes
    /// * `duration_ms` - Wall-clock duration in milliseconds
    pub fn record_request(
        &self,
        hostname: &str,
        result: RequestResult,
        cpu_time_us: u64,
        memory_bytes: usize,
        duration_ms: u64,
    ) {
        let metrics = self.get_or_create(hostname);
        let m = metrics.write().unwrap();
        
        m.record_request(result, cpu_time_us, memory_bytes, duration_ms);
        
        // Also update global metrics
        if let Ok(global) = self.global.write() {
            global.record_request(cpu_time_us as f64 / 1_000_000.0);
        }
    }
    
    /// Update memory usage for a tenant
    pub fn update_memory(&self, hostname: &str, heap_bytes: usize, external_bytes: usize) {
        let metrics = self.get_or_create(hostname);
        let m = metrics.read().unwrap();
        m.update_memory(heap_bytes, external_bytes);
    }
    
    /// Record a context reset for a tenant
    pub fn record_context_reset(&self, hostname: &str) {
        let metrics = self.get_or_create(hostname);
        let m = metrics.read().unwrap();
        m.record_context_reset();
    }
    
    /// Record a memory pressure event
    pub fn record_pressure_event(&self, hostname: &str, level: MemoryPressureLevel) {
        // Record in tenant metrics
        let metrics = self.get_or_create(hostname);
        {
            let m = metrics.read().unwrap();
            m.record_pressure_event(level);
        }
        
        // Also record in global metrics
        if let Ok(global) = self.global.write() {
            global.record_pressure_event(level);
        }
    }
    
    /// Get or create metrics for a hostname
    fn get_or_create(&self, hostname: &str) -> Arc<RwLock<TenantMetrics>> {
        self.tenants
            .entry(hostname.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(TenantMetrics::new(hostname))))
            .clone()
    }
    
    /// Get metrics for a specific tenant
    pub fn get_tenant(&self, hostname: &str) -> Option<Arc<RwLock<TenantMetrics>>> {
        self.tenants.get(hostname).map(|entry| entry.clone())
    }
    
    /// Get the number of tracked tenants
    pub fn tenant_count(&self) -> usize {
        self.tenants.len()
    }
    
    /// Get all tenant hostnames
    pub fn tenant_hostnames(&self) -> Vec<String> {
        self.tenants.iter().map(|entry| entry.key().clone()).collect()
    }
    
    /// Get top N tenants by request count
    pub fn top_tenants_by_requests(&self, n: usize) -> Vec<(String, u64)> {
        let mut tenants: Vec<(String, u64)> = self
            .tenants
            .iter()
            .map(|entry| {
                let hostname = entry.key().clone();
                let count = entry.value().read().unwrap().requests_total.get();
                (hostname, count)
            })
            .collect();
        
        tenants.sort_by(|a, b| b.1.cmp(&a.1));
        tenants.truncate(n);
        tenants
    }
    
    /// Get top N tenants by CPU usage
    pub fn top_tenants_by_cpu(&self, n: usize) -> Vec<(String, u64)> {
        let mut tenants: Vec<(String, u64)> = self
            .tenants
            .iter()
            .map(|entry| {
                let hostname = entry.key().clone();
                let cpu = entry.value().read().unwrap().cpu_seconds_total.get();
                (hostname, cpu)
            })
            .collect();
        
        tenants.sort_by(|a, b| b.1.cmp(&a.1));
        tenants.truncate(n);
        tenants
    }
    
    /// Export all tenant metrics as Prometheus format
    pub fn to_prometheus(&self) -> String {
        let mut output = String::with_capacity(8192);
        
        // Write HELP and TYPE lines
        output.push_str("# HELP nano_tenant_requests_total Total requests per tenant\n");
        output.push_str("# TYPE nano_tenant_requests_total counter\n");
        
        // Write request counters
        for entry in self.tenants.iter() {
            let m = entry.value().read().unwrap();
            let hostname = entry.key();
            
            output.push_str(&format!(
                "nano_tenant_requests_total{{hostname=\"{}\"}} {}\n",
                hostname, m.requests_total.get()
            ));
            output.push_str(&format!(
                "nano_tenant_requests_success{{hostname=\"{}\"}} {}\n",
                hostname, m.requests_success.get()
            ));
            output.push_str(&format!(
                "nano_tenant_requests_error{{hostname=\"{}\"}} {}\n",
                hostname, m.requests_error.get()
            ));
            output.push_str(&format!(
                "nano_tenant_requests_timeout{{hostname=\"{}\"}} {}\n",
                hostname, m.requests_timeout.get()
            ));
        }
        output.push('\n');
        
        // CPU metrics
        output.push_str("# HELP nano_tenant_cpu_seconds_total Total CPU seconds per tenant\n");
        output.push_str("# TYPE nano_tenant_cpu_seconds_total counter\n");
        for entry in self.tenants.iter() {
            let m = entry.value().read().unwrap();
            let hostname = entry.key();
            output.push_str(&format!(
                "nano_tenant_cpu_seconds_total{{hostname=\"{}\"}} {}\n",
                hostname, m.cpu_seconds_total.get()
            ));
        }
        output.push('\n');
        
        // Memory metrics
        output.push_str("# HELP nano_tenant_memory_used_bytes Current memory usage per tenant\n");
        output.push_str("# TYPE nano_tenant_memory_used_bytes gauge\n");
        for entry in self.tenants.iter() {
            let m = entry.value().read().unwrap();
            let hostname = entry.key();
            output.push_str(&format!(
                "nano_tenant_memory_used_bytes{{hostname=\"{}\"}} {}\n",
                hostname, m.memory_used_bytes.get()
            ));
            output.push_str(&format!(
                "nano_tenant_memory_external_bytes{{hostname=\"{}\"}} {}\n",
                hostname, m.memory_external_bytes.get()
            ));
        }
        output.push('\n');
        
        // Active requests
        output.push_str("# HELP nano_tenant_requests_active Active requests per tenant\n");
        output.push_str("# TYPE nano_tenant_requests_active gauge\n");
        for entry in self.tenants.iter() {
            let m = entry.value().read().unwrap();
            let hostname = entry.key();
            output.push_str(&format!(
                "nano_tenant_requests_active{{hostname=\"{}\"}} {}\n",
                hostname, m.requests_active.get()
            ));
        }
        output.push('\n');
        
        // Context resets
        output.push_str("# HELP nano_tenant_context_resets_total Total context resets per tenant\n");
        output.push_str("# TYPE nano_tenant_context_resets_total counter\n");
        for entry in self.tenants.iter() {
            let m = entry.value().read().unwrap();
            let hostname = entry.key();
            output.push_str(&format!(
                "nano_tenant_context_resets_total{{hostname=\"{}\"}} {}\n",
                hostname, m.context_resets_total.get()
            ));
        }
        
        output
    }
    
    /// Create a snapshot of all tenant metrics
    pub fn snapshot(&self) -> MetricsSnapshot {
        let tenants: Vec<TenantMetricsSnapshot> = self
            .tenants
            .iter()
            .map(|entry| {
                let m = entry.value().read().unwrap();
                // Duration histogram data - buckets reserved for future percentile calculations
                let (_duration_buckets, _duration_sum, _duration_count) =
                    m.request_duration_seconds.get();
                let (_cpu_buckets, cpu_sum, cpu_count) =
                    m.cpu_time_per_request_seconds.get();
                // Memory histogram data - reserved for future memory analytics
                let (_memory_buckets, _memory_sum, _memory_count) =
                    m.memory_per_request_bytes.get();
                
                TenantMetricsSnapshot {
                    hostname: entry.key().clone(),
                    requests_total: m.requests_total.get(),
                    requests_success: m.requests_success.get(),
                    requests_error: m.requests_error.get(),
                    requests_timeout: m.requests_timeout.get(),
                    requests_active: m.requests_active.get(),
                    cpu_seconds_total: m.cpu_seconds_total.get(),
                    cpu_avg_ms: if cpu_count > 0 { 
                        (cpu_sum * 1000.0) / cpu_count as f64 
                    } else { 
                        0.0 
                    },
                    memory_used_bytes: m.memory_used_bytes.get(),
                    memory_external_bytes: m.memory_external_bytes.get(),
                    memory_peak_bytes: m.memory_peak_bytes(),
                    context_resets_total: m.context_resets_total.get(),
                    latency_p50_ms: 0.0, // Would need proper percentile calculation
                    latency_p95_ms: 0.0,
                    latency_p99_ms: m.request_duration_p99() * 1000.0,
                    isolates_active: m.isolates_active.get(),
                }
            })
            .collect();
        
        let total_requests = tenants.iter().map(|t| t.requests_total).sum();
        let total_cpu_seconds: u64 = tenants.iter().map(|t| t.cpu_seconds_total).sum();
        
        MetricsSnapshot {
            tenants,
            total_requests,
            total_cpu_seconds,
        }
    }
    

}

impl Default for TenantMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Serializable snapshot of a single tenant's metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantMetricsSnapshot {
    /// Hostname
    pub hostname: String,
    /// Total requests
    pub requests_total: u64,
    /// Successful requests
    pub requests_success: u64,
    /// Failed requests
    pub requests_error: u64,
    /// Timed out requests
    pub requests_timeout: u64,
    /// Currently active requests
    pub requests_active: u64,
    /// Total CPU seconds consumed
    pub cpu_seconds_total: u64,
    /// Average CPU time per request in milliseconds
    pub cpu_avg_ms: f64,
    /// Current memory usage in bytes
    pub memory_used_bytes: u64,
    /// Current external memory in bytes
    pub memory_external_bytes: u64,
    /// Peak memory in bytes
    pub memory_peak_bytes: u64,
    /// Total context resets
    pub context_resets_total: u64,
    /// P50 latency in milliseconds
    pub latency_p50_ms: f64,
    /// P95 latency in milliseconds
    pub latency_p95_ms: f64,
    /// P99 latency in milliseconds
    pub latency_p99_ms: f64,
    /// Active isolates
    pub isolates_active: u64,
}

/// Complete metrics snapshot for all tenants
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Per-tenant snapshots
    pub tenants: Vec<TenantMetricsSnapshot>,
    /// Total requests across all tenants
    pub total_requests: u64,
    /// Total CPU seconds across all tenants
    pub total_cpu_seconds: u64,
}


