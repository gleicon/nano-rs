//! Prometheus text format exporter
//!
//! Renders collected metrics in the Prometheus text exposition format
//! as specified in the [Prometheus exposition formats](https://prometheus.io/docs/instrumenting/exposition_formats/).
//!
//! # Output Format
//!
//! ```text
//! # HELP metric_name description
//! # TYPE metric_name type
//! metric_name{label1="value1",label2="value2"} value
//! ```

use std::fmt::Write;

use crate::metrics::collector::MetricsRegistry;

/// Prometheus text format exporter
///
/// Renders metrics from a [`MetricsRegistry`] into the Prometheus text format.
/// This format is used by Prometheus scrapers and compatible tools.
#[derive(Debug)]
pub struct PrometheusExporter;

impl PrometheusExporter {
    /// Create a new exporter instance
    pub fn new() -> Self {
        Self
    }

    /// Export all metrics from the registry as Prometheus text
    ///
    /// # Arguments
    ///
    /// * `registry` - The metrics registry to export
    ///
    /// # Returns
    ///
    /// A string containing the Prometheus-formatted metrics
    ///
    /// # Example Output
    ///
    /// ```text
    /// # HELP nano_requests_total Total HTTP requests
    /// # TYPE nano_requests_total counter
    /// nano_requests_total{hostname="api.example.com",status="200"} 1423
    /// ```
    pub fn export(&self, registry: &MetricsRegistry) -> String {
        let mut output = String::with_capacity(4096);

        // Export requests_total counter
        self.export_counter_vec(
            &mut output,
            "nano_requests_total",
            "Total HTTP requests",
            &registry.requests_total,
        );

        // Export request_duration_ms histogram
        self.export_histogram_vec(
            &mut output,
            "nano_request_duration_ms",
            "Request latency in milliseconds",
            &registry.request_duration,
        );

        // Export errors_total counter
        self.export_counter_vec(
            &mut output,
            "nano_errors_total",
            "Total errors by status code",
            &registry.errors_total,
        );

        // Export isolates_active gauge
        self.export_gauge_vec(
            &mut output,
            "nano_isolates_active",
            "Number of active isolates",
            &registry.isolates_active,
        );

        // Export memory_bytes gauge
        self.export_gauge_vec(
            &mut output,
            "nano_memory_bytes",
            "Memory usage in bytes",
            &registry.memory_bytes,
        );

        // Export worker_utilization gauge
        self.export_gauge_vec(
            &mut output,
            "nano_worker_utilization",
            "Worker utilization percentage",
            &registry.worker_utilization,
        );

        // Export uptime_seconds gauge
        self.export_uptime(&mut output, registry);

        // Export heap_limit_hits_total counter
        self.export_counter(
            &mut output,
            "nano_heap_limit_hits_total",
            "Total heap limit enforcement events",
            registry.heap_limit_hits_total.get(),
        );

        // Export cpu_timeout_total counter
        self.export_counter(
            &mut output,
            "nano_cpu_timeout_total",
            "Total CPU timeout enforcement events",
            registry.cpu_timeout_total.get(),
        );

        // Export WASM limit counters
        self.export_counter(
            &mut output,
            "nano_wasm_size_rejected_total",
            "WASM modules rejected for exceeding MODULE_SIZE_BYTES_MAX",
            registry.wasm_size_rejected_total.get(),
        );
        self.export_counter(
            &mut output,
            "nano_wasm_import_rejected_total",
            "WASM modules rejected for exceeding IMPORT_COUNT_MAX",
            registry.wasm_import_rejected_total.get(),
        );
        self.export_counter(
            &mut output,
            "nano_wasm_export_rejected_total",
            "WASM modules rejected for exceeding EXPORT_COUNT_MAX",
            registry.wasm_export_rejected_total.get(),
        );
        self.export_counter(
            &mut output,
            "nano_wasm_memory_oom_total",
            "Isolate OOM events while WASM linear memory growth was active (see heap_limit_hits_total for total OOM)",
            registry.wasm_memory_oom_total.get(),
        );

        output
    }

    /// Export a simple Counter as Prometheus format
    fn export_counter(&self, output: &mut String, name: &str, help: &str, value: u64) {
        writeln!(output, "# HELP {} {}", name, help).unwrap();
        writeln!(output, "# TYPE {} counter", name).unwrap();
        writeln!(output, "{} {}", name, value).unwrap();
        writeln!(output).unwrap();
    }

    /// Export a CounterVec as Prometheus format
    fn export_counter_vec(
        &self,
        output: &mut String,
        name: &str,
        help: &str,
        counter_vec: &crate::metrics::types::CounterVec,
    ) {
        // Write HELP and TYPE lines
        writeln!(output, "# HELP {} {}", name, help).unwrap();
        writeln!(output, "# TYPE {} counter", name).unwrap();

        // Write metric lines
        let entries = counter_vec.get_all();
        if entries.is_empty() {
            // Write a zero value with empty labels if no data
            writeln!(output, "{} 0", name).unwrap();
        } else {
            for (labels, value) in entries {
                let label_str = self.format_labels(&counter_vec.label_names(), &labels);
                writeln!(output, "{}{} {}", name, label_str, value).unwrap();
            }
        }

        writeln!(output).unwrap(); // Empty line between metrics
    }

    /// Export a GaugeVec as Prometheus format
    fn export_gauge_vec(
        &self,
        output: &mut String,
        name: &str,
        help: &str,
        gauge_vec: &crate::metrics::types::GaugeVec,
    ) {
        // Write HELP and TYPE lines
        writeln!(output, "# HELP {} {}", name, help).unwrap();
        writeln!(output, "# TYPE {} gauge", name).unwrap();

        // Write metric lines
        let entries = gauge_vec.get_all();
        if entries.is_empty() {
            // Write a zero value with empty labels if no data
            writeln!(output, "{} 0", name).unwrap();
        } else {
            for (labels, value) in entries {
                let label_str = self.format_labels(&gauge_vec.label_names(), &labels);
                writeln!(output, "{}{} {}", name, label_str, value).unwrap();
            }
        }

        writeln!(output).unwrap(); // Empty line between metrics
    }

    /// Export a HistogramVec as Prometheus format
    fn export_histogram_vec(
        &self,
        output: &mut String,
        name: &str,
        help: &str,
        hist_vec: &crate::metrics::types::HistogramVec,
    ) {
        // Write HELP and TYPE lines
        writeln!(output, "# HELP {} {}", name, help).unwrap();
        writeln!(output, "# TYPE {} histogram", name).unwrap();

        // Write metric lines for each label combination
        let entries = hist_vec.get_all();
        if entries.is_empty() {
            // Write empty histogram buckets if no data
            for bucket in hist_vec.buckets() {
                let bucket_label = self.format_bucket_label(bucket);
                writeln!(output, "{}_bucket{{le=\"{}\"}} 0", name, bucket_label).unwrap();
            }
            writeln!(output, "{}_sum 0", name).unwrap();
            writeln!(output, "{}_count 0", name).unwrap();
        } else {
            for (labels, bucket_counts, sum, count) in entries {
                let label_names = hist_vec.label_names();
                let base_labels = self.format_labels(&label_names, &labels);

                // Get bucket boundaries
                let buckets: Vec<f64> = hist_vec.buckets().to_vec();

                // Write bucket counts with cumulative values
                let mut cumulative = 0u64;
                for (i, &bucket_count) in bucket_counts.iter().enumerate() {
                    cumulative += bucket_count;
                    let bucket_bound = buckets.get(i).copied().unwrap_or(f64::INFINITY);
                    let bucket_label = self.format_bucket_label(&bucket_bound);

                    // Format with or without base labels
                    if base_labels.is_empty() {
                        writeln!(
                            output,
                            "{}_bucket{{le=\"{}\"}} {}",
                            name, bucket_label, cumulative
                        )
                        .unwrap();
                    } else {
                        // Build the bucket label string: insert le="X" before the closing brace
                        let base_without_close = &base_labels[..base_labels.len() - 1]; // Remove trailing }
                        let comma = if base_labels.len() > 2 { "," } else { "" };
                        let bucket_labels =
                            format!("{}{}le=\"{}\"}}", base_without_close, comma, bucket_label);
                        writeln!(output, "{}_bucket{} {}", name, bucket_labels, cumulative)
                            .unwrap();
                    }
                }

                // Write sum and count
                if base_labels.is_empty() {
                    writeln!(output, "{}_sum {}", name, sum).unwrap();
                    writeln!(output, "{}_count {}", name, count).unwrap();
                } else {
                    // For histograms, sum and count use the same labels as base
                    writeln!(output, "{}_sum{} {}", name, base_labels, sum).unwrap();
                    writeln!(output, "{}_count{} {}", name, base_labels, count).unwrap();
                }
            }
        }

        writeln!(output).unwrap(); // Empty line between metrics
    }

    /// Export uptime_seconds gauge
    fn export_uptime(&self, output: &mut String, registry: &MetricsRegistry) {
        writeln!(
            output,
            "# HELP nano_uptime_seconds Runtime uptime in seconds"
        )
        .unwrap();
        writeln!(output, "# TYPE nano_uptime_seconds gauge").unwrap();
        writeln!(output, "nano_uptime_seconds {}", registry.uptime_seconds()).unwrap();
        writeln!(output).unwrap();
    }

    /// Format labels as Prometheus label string
    ///
    /// Returns formatted string like `{hostname="api.example.com",status="200"}`
    /// or empty string if there are no labels.
    fn format_labels(&self, names: &[String], values: &[String]) -> String {
        if names.is_empty() || values.is_empty() {
            return String::new();
        }

        let mut result = String::with_capacity(128);
        result.push('{');

        for (i, (name, value)) in names.iter().zip(values.iter()).enumerate() {
            if i > 0 {
                result.push(',');
            }
            // Escape special characters in value
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n");
            write!(result, "{}=\"{}\"", name, escaped).unwrap();
        }

        result.push('}');
        result
    }

    /// Format a bucket boundary for the `le` label
    fn format_bucket_label(&self, bound: &f64) -> String {
        if bound.is_infinite() {
            "+Inf".to_string()
        } else {
            format!("{:.3}", bound)
        }
    }
}

impl Default for PrometheusExporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the Prometheus content type header value
///
/// Returns the standard Prometheus text format content type.
pub fn prometheus_content_type() -> &'static str {
    "text/plain; version=0.0.4; charset=utf-8"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricsRegistry;

    #[test]
    fn test_exporter_creation() {
        let exporter = PrometheusExporter::new();
        let _ = exporter; // Just verify it creates
    }

    #[test]
    fn test_export_empty_registry() {
        let registry = MetricsRegistry::new();
        let exporter = PrometheusExporter::new();

        let output = exporter.export(&registry);

        // Should contain at least HELP/TYPE for each metric
        assert!(output.contains("# HELP nano_requests_total"));
        assert!(output.contains("# TYPE nano_requests_total counter"));
        assert!(output.contains("# HELP nano_request_duration_ms"));
        assert!(output.contains("# TYPE nano_request_duration_ms histogram"));
    }

    #[test]
    fn test_export_with_requests() {
        let registry = MetricsRegistry::new();
        let exporter = PrometheusExporter::new();

        // Record some requests
        registry.record_request("api.example.com", "200", 42.5);
        registry.record_request("api.example.com", "200", 55.0);
        registry.record_request("api.example.com", "500", 100.0);
        registry.record_request("blog.example.com", "200", 30.0);

        let output = exporter.export(&registry);

        // Check counter output
        assert!(
            output.contains("nano_requests_total{hostname=\"api.example.com\",status=\"200\"} 2")
        );
        assert!(
            output.contains("nano_requests_total{hostname=\"api.example.com\",status=\"500\"} 1")
        );
        assert!(
            output.contains("nano_requests_total{hostname=\"blog.example.com\",status=\"200\"} 1")
        );

        // Check histogram output (buckets)
        assert!(output.contains("nano_request_duration_ms_bucket"));
        assert!(output.contains("nano_request_duration_ms_sum"));
        assert!(output.contains("nano_request_duration_ms_count"));
    }

    #[test]
    fn test_format_labels() {
        let exporter = PrometheusExporter::new();

        let names = vec!["hostname".to_string(), "status".to_string()];
        let values = vec!["api.example.com".to_string(), "200".to_string()];

        let formatted = exporter.format_labels(&names, &values);
        assert!(formatted.contains("hostname=\"api.example.com\""));
        assert!(formatted.contains("status=\"200\""));
    }

    #[test]
    fn test_format_labels_escaping() {
        let exporter = PrometheusExporter::new();

        let names = vec!["hostname".to_string()];
        let values = vec!["api\"example.com".to_string()];

        let formatted = exporter.format_labels(&names, &values);
        assert!(formatted.contains("hostname=\"api\\\"example.com\""));
    }

    #[test]
    fn test_format_bucket_label() {
        let exporter = PrometheusExporter::new();

        assert_eq!(exporter.format_bucket_label(&1.0), "1.000");
        assert_eq!(exporter.format_bucket_label(&10.0), "10.000");
        assert_eq!(exporter.format_bucket_label(&f64::INFINITY), "+Inf");
    }

    #[test]
    fn test_prometheus_content_type() {
        let content_type = prometheus_content_type();
        assert!(content_type.contains("text/plain"));
        assert!(content_type.contains("version=0.0.4"));
        assert!(content_type.contains("charset=utf-8"));
    }

    #[test]
    fn test_export_gauges() {
        let registry = MetricsRegistry::new();
        let exporter = PrometheusExporter::new();

        registry.set_isolates_active("api.example.com", "worker-1", 5);
        registry.set_memory_bytes("api.example.com", "iso-123", 16777216);

        let output = exporter.export(&registry);

        assert!(output.contains("nano_isolates_active"));
        assert!(output.contains("nano_memory_bytes"));
    }

    #[test]
    fn test_export_errors() {
        let registry = MetricsRegistry::new();
        let exporter = PrometheusExporter::new();

        registry.record_error("api.example.com", "timeout");
        registry.record_error("api.example.com", "timeout");

        let output = exporter.export(&registry);

        assert!(output.contains("nano_errors_total"));
        assert!(output.contains("code=\"timeout\""));
    }

    #[test]
    fn test_export_uptime() {
        let registry = MetricsRegistry::new();
        let exporter = PrometheusExporter::new();

        // Update uptime
        registry.update_uptime();

        let output = exporter.export(&registry);

        assert!(output.contains("nano_uptime_seconds"));
        assert!(output.contains("# TYPE nano_uptime_seconds gauge"));
    }

    #[test]
    fn test_default_exporter() {
        let exporter: PrometheusExporter = Default::default();
        let registry = MetricsRegistry::new();

        let output = exporter.export(&registry);
        assert!(!output.is_empty());
    }
}
