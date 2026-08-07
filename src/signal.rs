//! Signal handling and graceful shutdown coordination
//!
//! Provides SIGTERM/SIGINT signal handling for graceful shutdown
//! with request draining and configurable timeout. Integrates with
//! the existing RequestDrain to track in-flight requests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::Duration;

use crate::app::drain::RequestDrain;

/// Configuration for graceful shutdown behavior
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Timeout for draining in-flight requests (seconds)
    /// Default: 30, Range: 5-300
    pub drain_timeout_secs: u64,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout_secs: 30,
        }
    }
}

impl ShutdownConfig {
    /// Create a new shutdown config with validation
    pub fn new(drain_timeout_secs: u64) -> Self {
        // Clamp to valid range: 5-300 seconds
        let timeout = drain_timeout_secs.clamp(5, 300);
        Self {
            drain_timeout_secs: timeout,
        }
    }

    /// Get the drain timeout as a Duration
    pub fn drain_timeout(&self) -> Duration {
        Duration::from_secs(self.drain_timeout_secs)
    }
}

/// Global shutdown state shared across the application
#[derive(Debug, Clone)]
pub struct ShutdownState {
    /// Flag indicating if shutdown has been initiated
    shutting_down: Arc<AtomicBool>,
    /// Request drain tracker for in-flight requests
    drain: RequestDrain,
}

impl ShutdownState {
    /// Create a new shutdown state
    pub fn new(drain: RequestDrain) -> Self {
        Self {
            shutting_down: Arc::new(AtomicBool::new(false)),
            drain,
        }
    }

    /// Mark the system as shutting down
    pub fn mark_shutting_down(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        tracing::info!("Shutdown initiated, marking as not ready");
    }

    /// Check if shutdown has been initiated
    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::SeqCst)
    }

    /// Get the request drain for tracking in-flight requests
    pub fn drain(&self) -> &RequestDrain {
        &self.drain
    }

    /// Get the current active request count
    pub fn active_requests(&self) -> usize {
        self.drain.active_count()
    }
}

impl Default for ShutdownState {
    fn default() -> Self {
        Self::new(RequestDrain::new())
    }
}

/// Graceful shutdown coordinator
///
/// Manages the shutdown lifecycle:
/// 1. Wait for shutdown signal
/// 2. Mark as shutting down (triggers 503 on readiness)
/// 3. Drain in-flight requests
/// 4. Force shutdown after timeout
#[derive(Debug, Clone)]
pub struct GracefulShutdown {
    /// Configuration for shutdown behavior
    config: ShutdownConfig,
    /// Shared shutdown state
    state: ShutdownState,
    /// Broadcast sender for shutdown notification
    shutdown_tx: broadcast::Sender<()>,
}

impl GracefulShutdown {
    /// Create a new graceful shutdown coordinator
    pub fn new(config: ShutdownConfig, drain: RequestDrain) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        let state = ShutdownState::new(drain);

        Self {
            config,
            state,
            shutdown_tx,
        }
    }

    /// Get the shutdown state for checking readiness
    pub fn state(&self) -> &ShutdownState {
        &self.state
    }

    /// Get the broadcast sender for shutdown notification
    pub fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Subscribe to shutdown notifications
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Initiate graceful shutdown
    ///
    /// This method:
    /// 1. Marks the system as shutting down
    /// 2. Notifies all subscribers
    /// 3. Waits for in-flight requests to drain
    /// 4. Returns when complete or timeout expires
    pub async fn shutdown(&self) {
        tracing::info!(
            drain_timeout = self.config.drain_timeout_secs,
            active_requests = self.state.active_requests(),
            "Starting graceful shutdown"
        );

        // Mark as shutting down - this triggers 503 on readiness probe
        self.state.mark_shutting_down();

        // Notify all subscribers that shutdown has started
        let _ = self.shutdown_tx.send(());

        // Wait for in-flight requests to complete
        let timeout = self.config.drain_timeout();
        let drained = self.state.drain().await_complete(timeout).await;

        if drained {
            tracing::info!("Graceful shutdown completed successfully, all requests drained");
        } else {
            let remaining = self.state.active_requests();
            tracing::warn!(
                remaining_requests = remaining,
                "Drain timeout exceeded, forcing shutdown"
            );
        }
    }

    /// Create a shutdown signal future for use with axum::serve
    ///
    /// Returns a future that resolves when shutdown is triggered
    pub async fn shutdown_signal(&self) {
        let mut rx = self.subscribe();
        // Wait for shutdown notification
        let _ = rx.recv().await;
        tracing::info!("Shutdown signal received by server");

        // Perform the graceful shutdown sequence
        self.shutdown().await;
    }
}

impl Default for GracefulShutdown {
    fn default() -> Self {
        Self::new(ShutdownConfig::default(), RequestDrain::new())
    }
}

/// Create a shutdown channel that listens for SIGTERM (Unix) and SIGINT (Ctrl+C)
///
/// Returns a broadcast sender that will receive a notification when either
/// SIGTERM (on Unix) or SIGINT (Ctrl+C on all platforms) is received.
///
/// # Example
///
/// ```rust,no_run
/// use nano::signal::shutdown_channel;
///
/// # async fn example() {
/// let shutdown_tx = shutdown_channel();
/// let mut shutdown_rx = shutdown_tx.subscribe();
///
/// // Wait for shutdown signal
/// let _ = shutdown_rx.recv().await;
/// println!("Shutdown signal received!");
/// # }
/// ```
pub fn shutdown_channel() -> broadcast::Sender<()> {
    let (tx, _) = broadcast::channel(1);
    let tx_clone = tx.clone();

    tokio::spawn(async move {
        // Set up Ctrl+C handler (SIGINT on Unix, Ctrl+C on Windows)
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Ctrl+C handler failed to initialize");
            tracing::info!("Received SIGINT (Ctrl+C)");
        };

        // Set up SIGTERM handler (Unix only)
        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm =
                signal(SignalKind::terminate()).expect("SIGTERM handler failed to initialize");
            sigterm.recv().await;
            tracing::info!("Received SIGTERM");
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        // Wait for either signal
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        // Notify all subscribers
        let _ = tx_clone.send(());
    });

    tx
}

/// Create a complete graceful shutdown setup
///
/// Combines signal handling with the GracefulShutdown coordinator.
/// Returns the GracefulShutdown instance and a signal receiver.
///
/// # Arguments
///
/// * `config` - Shutdown configuration (timeout, etc.)
/// * `drain` - RequestDrain for tracking in-flight requests
///
/// # Returns
///
/// Tuple of (GracefulShutdown, broadcast::Receiver<()>) for coordinating shutdown
///
/// # Example
///
/// ```rust,no_run
/// use nano::signal::{setup_shutdown, ShutdownConfig};
/// use nano::app::drain::RequestDrain;
///
/// # async fn example() {
/// let drain = RequestDrain::new();
/// let (shutdown, mut rx) = setup_shutdown(ShutdownConfig::default(), drain);
///
/// // Wait for shutdown signal
/// let _ = rx.recv().await;
/// shutdown.shutdown().await;
/// # }
/// ```
pub fn setup_shutdown(
    config: ShutdownConfig,
    drain: RequestDrain,
) -> (GracefulShutdown, broadcast::Receiver<()>) {
    let shutdown = GracefulShutdown::new(config, drain);
    let rx = shutdown.subscribe();

    // Spawn signal handler that will trigger the shutdown
    let shutdown_tx = shutdown.shutdown_sender();

    tokio::spawn(async move {
        // Set up Ctrl+C handler
        let ctrl_c = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Ctrl+C handler failed");
            tracing::info!("Received SIGINT (Ctrl+C)");
        };

        // Set up SIGTERM handler (Unix only)
        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler failed");
            sigterm.recv().await;
            tracing::info!("Received SIGTERM");
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        // Wait for either signal
        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }

        // Notify shutdown coordinator
        let _ = shutdown_tx.send(());
    });

    (shutdown, rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_shutdown_config_default() {
        let config = ShutdownConfig::default();
        assert_eq!(config.drain_timeout_secs, 30);
    }

    #[tokio::test]
    async fn test_shutdown_config_validation() {
        // Should clamp to valid range
        let config = ShutdownConfig::new(3);
        assert_eq!(config.drain_timeout_secs, 5); // Clamped to min

        let config = ShutdownConfig::new(500);
        assert_eq!(config.drain_timeout_secs, 300); // Clamped to max

        let config = ShutdownConfig::new(60);
        assert_eq!(config.drain_timeout_secs, 60); // Unchanged
    }

    #[tokio::test]
    async fn test_shutdown_state() {
        let drain = RequestDrain::new();
        let state = ShutdownState::new(drain.clone());

        // Initially not shutting down
        assert!(!state.is_shutting_down());
        assert_eq!(state.active_requests(), 0);

        // Simulate a request starting
        drain.request_started();
        assert_eq!(state.active_requests(), 1);

        // Mark as shutting down
        state.mark_shutting_down();
        assert!(state.is_shutting_down());

        // Complete the request
        drain.request_completed();
        assert_eq!(state.active_requests(), 0);
    }

    #[tokio::test]
    async fn test_graceful_shutdown_broadcast() {
        let drain = RequestDrain::new();
        let shutdown = GracefulShutdown::new(ShutdownConfig::default(), drain);

        let mut rx1 = shutdown.subscribe();
        let mut rx2 = shutdown.subscribe();

        // Shutdown should broadcast to all subscribers
        shutdown.state().mark_shutting_down();
        let _ = shutdown.shutdown_sender().send(());

        // Both receivers should get the message
        let result1 = timeout(Duration::from_millis(100), rx1.recv()).await;
        let result2 = timeout(Duration::from_millis(100), rx2.recv()).await;

        assert!(result1.is_ok());
        assert!(result2.is_ok());
    }

    #[tokio::test]
    async fn test_shutdown_channel() {
        let tx = shutdown_channel();
        let mut rx = tx.subscribe();

        // Simulate signal by sending directly
        let _ = tx.send(());

        // Should receive the signal
        let result = timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(result.is_ok());
    }
}
