//! Unit tests for tenant metrics — extracted from src/metrics/tenant.rs
use nano::metrics::{RequestResult, TenantMetrics, TenantMetricsCollector, GlobalMetrics};

#[test]
fn test_tenant_metrics_creation() {
    let metrics = TenantMetrics::new("api.example.com");
    assert_eq!(metrics.hostname, "api.example.com");
    assert_eq!(metrics.requests_total.get(), 0);
}

#[test]
fn test_record_request_success() {
    let metrics = TenantMetrics::new("api.example.com");

    metrics.record_request(RequestResult::Success, 5000, 1024, 10);

    assert_eq!(metrics.requests_total.get(), 1);
    assert_eq!(metrics.requests_success.get(), 1);
    assert_eq!(metrics.requests_error.get(), 0);
    assert_eq!(metrics.cpu_seconds_total.get(), 0);
}

#[test]
fn test_record_request_error() {
    let metrics = TenantMetrics::new("api.example.com");

    metrics.record_request(RequestResult::Error, 10000, 2048, 20);

    assert_eq!(metrics.requests_total.get(), 1);
    assert_eq!(metrics.requests_success.get(), 0);
    assert_eq!(metrics.requests_error.get(), 1);
}

#[test]
fn test_record_multiple_requests() {
    let metrics = TenantMetrics::new("api.example.com");

    metrics.record_request(RequestResult::Success, 5000, 1024, 10);
    metrics.record_request(RequestResult::Success, 6000, 1024, 12);
    metrics.record_request(RequestResult::Error, 8000, 2048, 15);

    assert_eq!(metrics.requests_total.get(), 3);
    assert_eq!(metrics.requests_success.get(), 2);
    assert_eq!(metrics.requests_error.get(), 1);
}

#[test]
fn test_context_reset() {
    let metrics = TenantMetrics::new("api.example.com");

    metrics.record_context_reset();
    metrics.record_context_reset();

    assert_eq!(metrics.context_resets_total.get(), 2);
}

#[test]
fn test_memory_update() {
    let metrics = TenantMetrics::new("api.example.com");

    metrics.update_memory(1048576, 2048);

    assert_eq!(metrics.memory_used_bytes.get(), 1048576);
    assert_eq!(metrics.memory_external_bytes.get(), 2048);
}

#[test]
fn test_collector_creation() {
    let collector = TenantMetricsCollector::new();
    assert_eq!(collector.tenant_count(), 0);
}

#[test]
fn test_collector_record_request() {
    let collector = TenantMetricsCollector::new();

    collector.record_request("api.example.com", RequestResult::Success, 5000, 1024, 10);
    collector.record_request("api.example.com", RequestResult::Success, 6000, 1024, 12);
    collector.record_request("blog.example.com", RequestResult::Success, 3000, 512, 8);

    assert_eq!(collector.tenant_count(), 2);

    let api_metrics = collector.get_tenant("api.example.com").unwrap();
    let api = api_metrics.read().unwrap();
    assert_eq!(api.requests_total.get(), 2);

    let blog_metrics = collector.get_tenant("blog.example.com").unwrap();
    let blog = blog_metrics.read().unwrap();
    assert_eq!(blog.requests_total.get(), 1);
}

#[test]
fn test_top_tenants_by_requests() {
    let collector = TenantMetricsCollector::new();

    collector.record_request("api.example.com", RequestResult::Success, 1000, 1024, 10);
    collector.record_request("api.example.com", RequestResult::Success, 1000, 1024, 10);
    collector.record_request("blog.example.com", RequestResult::Success, 1000, 1024, 10);
    collector.record_request("shop.example.com", RequestResult::Success, 1000, 1024, 10);
    collector.record_request("shop.example.com", RequestResult::Success, 1000, 1024, 10);
    collector.record_request("shop.example.com", RequestResult::Success, 1000, 1024, 10);

    let top = collector.top_tenants_by_requests(2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].0, "shop.example.com");
    assert_eq!(top[0].1, 3);
    assert_eq!(top[1].0, "api.example.com");
    assert_eq!(top[1].1, 2);
}

#[test]
fn test_prometheus_export() {
    let collector = TenantMetricsCollector::new();

    collector.record_request("api.example.com", RequestResult::Success, 5000, 1024, 10);
    collector.record_request("api.example.com", RequestResult::Error, 8000, 2048, 15);

    let output = collector.to_prometheus();

    assert!(output.contains("nano_tenant_requests_total"));
    assert!(output.contains("nano_tenant_requests_success"));
    assert!(output.contains("nano_tenant_requests_error"));
    assert!(output.contains("api.example.com"));
}

#[test]
fn test_metrics_snapshot() {
    let collector = TenantMetricsCollector::new();

    collector.record_request("api.example.com", RequestResult::Success, 5000, 1024, 10);
    collector.record_context_reset("api.example.com");

    let snapshot = collector.snapshot();

    assert_eq!(snapshot.tenants.len(), 1);
    assert_eq!(snapshot.total_requests, 1);
    assert_eq!(snapshot.tenants[0].hostname, "api.example.com");
    assert_eq!(snapshot.tenants[0].requests_total, 1);
    assert_eq!(snapshot.tenants[0].context_resets_total, 1);
}

#[test]
fn test_request_result_enum() {
    assert_eq!(RequestResult::Success.as_str(), "success");
    assert_eq!(RequestResult::Error.as_str(), "error");
    assert_eq!(RequestResult::Timeout.as_str(), "timeout");
}

#[test]
fn test_global_metrics() {
    let global = GlobalMetrics::new();

    global.record_request(0.5);
    global.record_request(1.0);

    assert_eq!(global.total_requests.get(), 2);
}
