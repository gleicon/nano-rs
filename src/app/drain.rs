//! Graceful drain for in-flight requests
//!
//! Provides functionality to wait for in-flight requests to complete
//! before performing config swaps or shutdown.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{sleep, Duration};

/// Tracks in-flight requests and provides graceful drain capability
#[derive(Debug, Clone)]
pub struct RequestDrain {
    /// Counter for active requests
    active_requests: Arc<AtomicUsize>,
    /// Semaphore for waiting for drain completion
    drain_semaphore: Arc<Semaphore>,
}

impl RequestDrain {
    /// Create a new request drain tracker
    pub fn new() -> Self {
        Self {
            active_requests: Arc::new(AtomicUsize::new(0)),
            // Start with 0 permits - only add when last request completes
            drain_semaphore: Arc::new(Semaphore::new(0)),
        }
    }

    /// Increment active request count
    pub fn request_started(&self) {
        self.active_requests.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement active request count
    pub fn request_completed(&self) {
        let count = self.active_requests.fetch_sub(1, Ordering::SeqCst);
        if count == 1 {
            // Last request completed, signal drain completion
            let _ = self.drain_semaphore.add_permits(1);
        }
    }

    /// Get current active request count
    pub fn active_count(&self) -> usize {
        self.active_requests.load(Ordering::SeqCst)
    }

    /// Wait for all in-flight requests to complete
    pub async fn await_complete(&self, timeout: Duration) -> bool {
        // Quick check if already empty
        if self.active_count() == 0 {
            return true;
        }

        // Acquire permit to wait for drain
        let permit_future = self.drain_semaphore.acquire();

        match tokio::time::timeout(timeout, permit_future).await {
            Ok(Ok(_permit)) => {
                // Successfully drained
                true
            }
            _ => {
                // Timeout or error
                false
            }
        }
    }

    /// Wait for drain with polling
    pub async fn await_complete_polling(&self, timeout: Duration, poll_interval: Duration) -> bool {
        let start = tokio::time::Instant::now();

        while start.elapsed() < timeout {
            if self.active_count() == 0 {
                return true;
            }
            sleep(poll_interval).await;
        }

        // Final check
        self.active_count() == 0
    }
}

impl Default for RequestDrain {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle for tracking a single request's lifecycle
pub struct DrainHandle {
    drain: RequestDrain,
}

impl DrainHandle {
    /// Create a new drain handle
    pub fn new(drain: RequestDrain) -> Self {
        drain.request_started();
        Self { drain }
    }
}

impl Drop for DrainHandle {
    fn drop(&mut self) {
        self.drain.request_completed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_request_drain_basic() {
        let drain = RequestDrain::new();

        // Simulate requests
        drain.request_started();
        drain.request_started();
        assert_eq!(drain.active_count(), 2);

        drain.request_completed();
        assert_eq!(drain.active_count(), 1);

        drain.request_completed();
        assert_eq!(drain.active_count(), 0);
    }

    #[tokio::test]
    async fn test_drain_handle() {
        let drain = RequestDrain::new();

        {
            let _handle = DrainHandle::new(drain.clone());
            assert_eq!(drain.active_count(), 1);
        }

        assert_eq!(drain.active_count(), 0);
    }

    #[tokio::test]
    async fn test_await_complete_empty() {
        let drain = RequestDrain::new();

        // Should return immediately when empty
        let result = drain.await_complete(Duration::from_millis(100)).await;
        assert!(result);
    }

    #[tokio::test]
    async fn test_await_complete_timeout() {
        let drain = RequestDrain::new();

        // Start a request but never complete it
        drain.request_started();

        // Should timeout
        let result = drain.await_complete(Duration::from_millis(50)).await;
        assert!(!result);
    }
}
