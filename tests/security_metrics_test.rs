//! Integration test for CPU timeout and heap limit metrics
//!
//! This test verifies that the metrics counters are properly incremented
//! and exported in Prometheus format.

use nano::metrics::{MetricsRegistry, PrometheusExporter};

/// Test that heap limit hit counter increments correctly
#[test]
fn test_heap_limit_counter_increments() {
    let registry = MetricsRegistry::new();

    // Initially should be 0
    assert_eq!(registry.heap_limit_hits_total.get(), 0);

    // Record some heap limit hits
    registry.record_heap_limit_hit();
    registry.record_heap_limit_hit();
    registry.record_heap_limit_hit();

    // Counter should now be 3
    assert_eq!(registry.heap_limit_hits_total.get(), 3);
}

/// Test that CPU timeout counter increments correctly
#[test]
fn test_cpu_timeout_counter_increments() {
    let registry = MetricsRegistry::new();

    // Initially should be 0
    assert_eq!(registry.cpu_timeout_total.get(), 0);

    // Record some CPU timeouts
    registry.record_cpu_timeout();
    registry.record_cpu_timeout();

    // Counter should now be 2
    assert_eq!(registry.cpu_timeout_total.get(), 2);
}

/// Test that metrics are exported in Prometheus format
#[test]
fn test_metrics_exported_in_prometheus_format() {
    let registry = MetricsRegistry::new();
    let exporter = PrometheusExporter::new();

    // Record some security events
    registry.record_heap_limit_hit();
    registry.record_heap_limit_hit();
    registry.record_cpu_timeout();

    // Export to Prometheus format
    let output = exporter.export(&registry);

    // Check that both counters are present in output
    assert!(
        output.contains("# HELP nano_heap_limit_hits_total"),
        "Output should contain HELP for heap_limit_hits_total"
    );
    assert!(
        output.contains("# TYPE nano_heap_limit_hits_total counter"),
        "Output should contain TYPE for heap_limit_hits_total"
    );
    assert!(
        output.contains("nano_heap_limit_hits_total 2"),
        "Output should contain heap_limit_hits_total with value 2, got:\n{}",
        output
    );

    assert!(
        output.contains("# HELP nano_cpu_timeout_total"),
        "Output should contain HELP for cpu_timeout_total"
    );
    assert!(
        output.contains("# TYPE nano_cpu_timeout_total counter"),
        "Output should contain TYPE for cpu_timeout_total"
    );
    assert!(
        output.contains("nano_cpu_timeout_total 1"),
        "Output should contain cpu_timeout_total with value 1, got:\n{}",
        output
    );
}

/// Test that metric descriptions include security metrics
#[test]
fn test_metric_descriptions_include_security_counters() {
    let registry = MetricsRegistry::new();
    let descriptions = registry.metric_descriptions();

    let names: Vec<_> = descriptions.iter().map(|(n, _, _)| *n).collect();

    assert!(
        names.contains(&"nano_heap_limit_hits_total"),
        "Descriptions should include nano_heap_limit_hits_total"
    );
    assert!(
        names.contains(&"nano_cpu_timeout_total"),
        "Descriptions should include nano_cpu_timeout_total"
    );
}

/// Test counters reset for testing
#[test]
fn test_counters_can_be_reset() {
    let registry = MetricsRegistry::new();

    // Increment counters
    registry.record_heap_limit_hit();
    registry.record_cpu_timeout();
    registry.record_cpu_timeout();

    assert_eq!(registry.heap_limit_hits_total.get(), 1);
    assert_eq!(registry.cpu_timeout_total.get(), 2);

    // Reset counters (used internally for testing)
    registry.heap_limit_hits_total.reset();
    registry.cpu_timeout_total.reset();

    // Should be 0 after reset
    assert_eq!(registry.heap_limit_hits_total.get(), 0);
    assert_eq!(registry.cpu_timeout_total.get(), 0);
}
