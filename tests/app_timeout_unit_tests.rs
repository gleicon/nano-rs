//! Unit tests for app timeout — extracted from src/app/timeout.rs
use std::time::Duration;

use nano::app::timeout::{with_timeout, TimeoutConfig, TimeoutError, TimeoutWatchdog};

#[test]
fn test_timeout_config() {
    let config = TimeoutConfig::new(5);
    assert_eq!(config.timeout_secs, 5);
    assert!(config.enabled);
    assert!(config.is_valid());

    let config = TimeoutConfig::disabled();
    assert!(!config.enabled);
    assert!(!config.is_valid());
}

#[test]
fn test_watchdog_creation() {
    let watchdog = TimeoutWatchdog::new(5, "test.app");
    let remaining = watchdog.remaining_ms();
    assert!(
        remaining >= 4990 && remaining <= 5000,
        "Expected remaining_ms around 5000, got {}",
        remaining
    );
    assert!(!watchdog.check_expired());
}

#[test]
fn test_watchdog_remaining_decreases() {
    let watchdog = TimeoutWatchdog::new(1, "test.app");

    std::thread::sleep(Duration::from_millis(50));

    let remaining = watchdog.remaining_ms();
    assert!(remaining < 1000, "Remaining should decrease");
    assert!(remaining > 900, "Should still have most of the time");
}

#[test]
fn test_watchdog_expires() {
    let watchdog = TimeoutWatchdog::new(0, "test.app");
    assert!(watchdog.check_expired());
}

#[test]
fn test_timeout_error_properties() {
    let err = TimeoutError::RequestTimeout {
        timeout_secs: 5,
        elapsed_ms: 5100,
        app_hostname: "test.app".to_string(),
    };

    assert_eq!(err.timeout_secs(), Some(5));
    assert_eq!(err.elapsed_ms(), Some(5100));
    assert_eq!(err.app_hostname(), Some("test.app"));
    assert!(err.is_timeout());
}

#[test]
fn test_cancelled_error() {
    let err = TimeoutError::Cancelled;
    assert!(!err.is_timeout());
    assert_eq!(err.timeout_secs(), None);
}

#[tokio::test]
async fn test_run_future_success() {
    let watchdog = TimeoutWatchdog::new(5, "test.app");

    let result = watchdog
        .run_future(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            42
        })
        .await;

    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn test_run_future_timeout() {
    let watchdog = TimeoutWatchdog::new(0, "test.app");

    let result = watchdog
        .run_future(async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            42
        })
        .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        TimeoutError::RequestTimeout { .. }
    ));
}

#[tokio::test]
async fn test_run_blocking_success() {
    let watchdog = TimeoutWatchdog::new(5, "test.app");

    let result = watchdog
        .run_blocking(|| {
            std::thread::sleep(Duration::from_millis(10));
            42
        })
        .await;

    assert_eq!(result.unwrap(), 42);
}

#[tokio::test]
async fn test_with_timeout_success() {
    let result = with_timeout(5, "test.app", async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        "success"
    })
    .await;

    assert_eq!(result.unwrap(), "success");
}

#[tokio::test]
async fn test_with_timeout_expires() {
    let result = with_timeout(0, "test.app", async {
        tokio::time::sleep(Duration::from_secs(10)).await;
        "should not reach"
    })
    .await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        TimeoutError::RequestTimeout { .. }
    ));
}

#[test]
fn test_watchdog_cancel() {
    let watchdog = TimeoutWatchdog::new(5, "test.app");
    assert!(!watchdog.is_cancelled());

    watchdog.cancel();
    assert!(watchdog.is_cancelled());
}
