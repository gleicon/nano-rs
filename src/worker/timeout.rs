//! Timer-based execution termination with CPU time limits
//!
//! This module provides timer-based termination for JavaScript execution,
//! using periodic CPU time checks. It integrates with V8's TerminateExecution
//! for safe script termination.
//!
//! ## Architecture
//!
//! - `ExecutionTimer`: Manages timer lifecycle and CPU time tracking
//! - `TimeoutConfig`: Configuration for CPU and wall-clock limits
//! - `TimeoutError`: Error types for timeout conditions
//!
//! ## Safety
//!
//! CPU time checks are performed periodically during async execution.
//! When CPU limit is exceeded, `isolate.terminate_execution()` is called
//! from the main thread (never from signal handlers).
//!
//! ## Platform Support
//!
//! - Linux: Uses `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`
//! - macOS: Uses `getrusage(RUSAGE_SELF)`
//! - Other platforms: Falls back to wall-clock time
//!
//! ## Integration
//!
//! The ExecutionTimer wraps handler execution and enforces limits.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio::time::Instant;

use crate::worker::cpu_tracker::{CpuTimeError, CpuTracker};

/// Configuration for timeout behavior
///
/// Defines both CPU time limits (primary) and wall-clock limits (backup).
/// CPU time is preferred as it accounts for actual computation, not
/// time spent waiting for I/O.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutConfig {
    /// CPU time limit in milliseconds (default: 50ms like Cloudflare Workers)
    pub cpu_time_limit_ms: u32,
    /// Wall clock limit in milliseconds (default: 30s as backup)
    pub wall_clock_limit_ms: u32,
    /// Grace period in microseconds before termination (default: 100us)
    pub termination_grace_us: u32,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            cpu_time_limit_ms: 50,       // 50ms default like Cloudflare
            wall_clock_limit_ms: 30_000, // 30 seconds
            termination_grace_us: 100,   // 100 microseconds
        }
    }
}

impl TimeoutConfig {
    /// Create a new config with specific CPU time limit
    ///
    /// # Arguments
    ///
    /// * `cpu_time_limit_ms` - CPU time limit in milliseconds
    pub fn with_cpu_limit(cpu_time_limit_ms: u32) -> Self {
        Self {
            cpu_time_limit_ms,
            ..Default::default()
        }
    }

    /// Get CPU time limit as Duration
    pub fn cpu_duration(&self) -> Duration {
        Duration::from_millis(self.cpu_time_limit_ms as u64)
    }

    /// Get wall clock limit as Duration
    pub fn wall_clock_duration(&self) -> Duration {
        Duration::from_millis(self.wall_clock_limit_ms as u64)
    }

    /// Get termination grace period as Duration
    pub fn grace_duration(&self) -> Duration {
        Duration::from_micros(self.termination_grace_us as u64)
    }

    /// Validate the configuration
    ///
    /// Returns error if limits are unreasonable (e.g., too short or too long).
    pub fn validate(&self) -> Result<(), String> {
        if self.cpu_time_limit_ms < 1 {
            return Err("CPU time limit must be at least 1ms".to_string());
        }
        if self.cpu_time_limit_ms > 1000 {
            return Err("CPU time limit must not exceed 1000ms (1s)".to_string());
        }
        if self.wall_clock_limit_ms < self.cpu_time_limit_ms {
            return Err("Wall clock limit must be >= CPU time limit".to_string());
        }
        if self.wall_clock_limit_ms > 300_000 {
            return Err("Wall clock limit must not exceed 300s (5min)".to_string());
        }
        Ok(())
    }
}

/// Error types for timeout conditions
#[derive(Error, Debug, Clone)]
pub enum TimeoutError {
    /// CPU time limit exceeded
    #[error("CPU time limit exceeded: used {}ms, limit {}ms", used_ms, limit_ms)]
    CpuTimeExceeded {
        /// CPU time used in milliseconds
        used_ms: u64,
        /// Configured CPU limit in milliseconds
        limit_ms: u64,
    },

    /// Wall clock time limit exceeded
    #[error("Wall clock time limit exceeded: limit {}ms", limit_ms)]
    WallClockExceeded {
        /// Configured wall clock limit in milliseconds
        limit_ms: u64,
    },

    /// Execution was terminated by V8
    #[error("Execution terminated by runtime")]
    ExecutionTerminated,

    /// Timeout configuration error
    #[error("Timeout configuration error: {}", message)]
    ConfigError {
        /// Error message
        message: String,
    },

    /// Internal timer error
    #[error("Timer error: {}", message)]
    TimerError {
        /// Error message
        message: String,
    },
}

impl TimeoutError {
    /// Check if this is a CPU time error
    pub fn is_cpu_timeout(&self) -> bool {
        matches!(self, TimeoutError::CpuTimeExceeded { .. })
    }

    /// Check if this is a wall clock error
    pub fn is_wall_clock_timeout(&self) -> bool {
        matches!(self, TimeoutError::WallClockExceeded { .. })
    }

    /// Get the limit that was exceeded (if applicable)
    pub fn limit_ms(&self) -> Option<u64> {
        match self {
            TimeoutError::CpuTimeExceeded { limit_ms, .. } => Some(*limit_ms),
            TimeoutError::WallClockExceeded { limit_ms, .. } => Some(*limit_ms),
            _ => None,
        }
    }
}

/// Global flag for signaling timeout
///
/// This is checked by the execution loop to determine if
/// V8 termination should be requested.
static TIMEOUT_SIGNALED: AtomicBool = AtomicBool::new(false);

/// Check if timeout has been signaled
pub fn is_timeout_signaled() -> bool {
    TIMEOUT_SIGNALED.load(Ordering::SeqCst)
}

/// Reset the timeout flag
pub fn reset_timeout_signal() {
    TIMEOUT_SIGNALED.store(false, Ordering::SeqCst);
}

/// Execution timer for enforcing CPU and wall-clock limits
///
/// This struct manages the lifecycle of timers and CPU tracking
/// to enforce execution limits safely.
pub struct ExecutionTimer {
    /// Configuration for limits
    config: TimeoutConfig,
    /// CPU tracker for monitoring thread CPU time
    cpu_tracker: CpuTracker,
    /// Whether this timer is currently active
    active: AtomicBool,
}

impl std::fmt::Debug for ExecutionTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionTimer")
            .field("config", &self.config)
            .field("cpu_tracker", &self.cpu_tracker)
            .field("active", &self.is_active())
            .finish()
    }
}

impl ExecutionTimer {
    /// Create a new execution timer with default config
    pub fn new() -> Self {
        Self::with_config(TimeoutConfig::default())
    }

    /// Create a new execution timer with specific config
    ///
    /// # Arguments
    ///
    /// * `config` - Timeout configuration
    pub fn with_config(config: TimeoutConfig) -> Self {
        // Validate config or use defaults
        let config = match config.validate() {
            Ok(_) => config,
            Err(_) => TimeoutConfig::default(),
        };

        Self {
            cpu_tracker: CpuTracker::new(config.cpu_time_limit_ms),
            config,
            active: AtomicBool::new(false),
        }
    }

    /// Create a new execution timer with specific CPU limit
    ///
    /// # Arguments
    ///
    /// * `cpu_limit_ms` - CPU time limit in milliseconds
    pub fn with_cpu_limit(cpu_limit_ms: u32) -> Self {
        Self::with_config(TimeoutConfig::with_cpu_limit(cpu_limit_ms))
    }

    /// Check if timer is currently active
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Get the current configuration
    pub fn config(&self) -> &TimeoutConfig {
        &self.config
    }

    /// Get the CPU tracker reference
    pub fn cpu_tracker(&self) -> &CpuTracker {
        &self.cpu_tracker
    }

    /// Run a future with timeout enforcement
    ///
    /// This method wraps a future with both CPU time and wall-clock
    /// timeout enforcement. If the CPU limit is exceeded, it returns
    /// a TimeoutError::CpuTimeExceeded.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate to potentially terminate
    /// * `f` - The future to execute
    ///
    /// # Returns
    ///
    /// `Ok(T)` if execution completed within limits,
    /// `Err(TimeoutError)` if any limit was exceeded
    pub async fn run_with_timeout<F, T>(
        &self,
        isolate: &mut v8::Isolate,
        f: F,
    ) -> Result<T, TimeoutError>
    where
        F: Future<Output = T>,
    {
        // Start CPU tracking
        let cpu_start = self
            .cpu_tracker
            .start()
            .map_err(|e| TimeoutError::ConfigError {
                message: format!("Failed to start CPU tracking: {}", e),
            })?;

        self.active.store(true, Ordering::SeqCst);
        reset_timeout_signal();

        // Use tokio::select! to race the future against the wall-clock timeout
        let wall_duration = self.config.wall_clock_duration();
        let wall_sleep = tokio::time::sleep(wall_duration);

        tokio::pin!(wall_sleep);
        tokio::pin!(f);

        let start_time = Instant::now();
        let check_interval = Duration::from_millis(5); // Check every 5ms

        // Main execution loop with periodic CPU checks
        loop {
            tokio::select! {
                biased;

                // Check wall-clock timeout
                _ = &mut wall_sleep => {
                    self.active.store(false, Ordering::SeqCst);
                    return Err(TimeoutError::WallClockExceeded {
                        limit_ms: self.config.wall_clock_limit_ms as u64,
                    });
                }

                // Check timeout signal flag
                _ = tokio::time::sleep(Duration::from_micros(100)) => {
                    if is_timeout_signaled() {
                        reset_timeout_signal();
                        isolate.terminate_execution();
                        self.active.store(false, Ordering::SeqCst);
                        return Err(TimeoutError::ExecutionTerminated);
                    }
                }

                // Attempt to poll the future with a short timeout
                // This allows periodic CPU checks
                _ = tokio::time::sleep(check_interval) => {
                    // Try to poll the future without awaiting
                    let mut cx = std::task::Context::from_waker(
                        std::task::Waker::noop()
                    );
                    match f.as_mut().poll(&mut cx) {
                        std::task::Poll::Ready(result) => {
                            // Future completed - final CPU check
                            match self.cpu_tracker.check_cpu(&cpu_start) {
                                Ok(_) => {
                                    self.active.store(false, Ordering::SeqCst);
                                    return Ok(result);
                                }
                                Err(CpuTimeError::LimitExceeded { used_us, limit_us }) => {
                                    isolate.terminate_execution();
                                    self.active.store(false, Ordering::SeqCst);
                                    return Err(TimeoutError::CpuTimeExceeded {
                                        used_ms: used_us / 1000,
                                        limit_ms: limit_us / 1000,
                                    });
                                }
                                Err(_) => {
                                    // Ignore tracking errors
                                    self.active.store(false, Ordering::SeqCst);
                                    return Ok(result);
                                }
                            }
                        }
                        std::task::Poll::Pending => {
                            // Future not ready, continue loop for CPU check
                        }
                    }
                }
            }

            // Periodic CPU check during execution (after grace period)
            if start_time.elapsed() > self.config.grace_duration() {
                match self.cpu_tracker.check_cpu(&cpu_start) {
                    Ok(_) => {}
                    Err(CpuTimeError::LimitExceeded { used_us, limit_us }) => {
                        isolate.terminate_execution();
                        self.active.store(false, Ordering::SeqCst);
                        return Err(TimeoutError::CpuTimeExceeded {
                            used_ms: used_us / 1000,
                            limit_ms: limit_us / 1000,
                        });
                    }
                    Err(_) => {}
                }
            }
        }
    }

    /// Execute a closure and check CPU time after completion
    ///
    /// This is a simpler synchronous version that checks CPU time
    /// after the closure completes. Good for short operations.
    ///
    /// # Arguments
    ///
    /// * `isolate` - The V8 isolate
    /// * `f` - The closure to execute
    pub fn execute_and_check<T>(
        &self,
        isolate: &mut v8::Isolate,
        f: impl FnOnce() -> T,
    ) -> Result<T, TimeoutError> {
        // Start CPU tracking
        let cpu_start = self
            .cpu_tracker
            .start()
            .map_err(|e| TimeoutError::ConfigError {
                message: format!("Failed to start CPU tracking: {}", e),
            })?;

        self.active.store(true, Ordering::SeqCst);
        reset_timeout_signal();

        // Execute the closure
        let result = f();

        // Check CPU time after execution
        let cpu_result = self.cpu_tracker.check_cpu(&cpu_start);

        self.active.store(false, Ordering::SeqCst);

        match cpu_result {
            Ok(_) => Ok(result),
            Err(CpuTimeError::LimitExceeded { used_us, limit_us }) => {
                isolate.terminate_execution();
                Err(TimeoutError::CpuTimeExceeded {
                    used_ms: used_us / 1000,
                    limit_ms: limit_us / 1000,
                })
            }
            Err(_) => Ok(result), // Ignore tracking errors
        }
    }
}

impl Default for ExecutionTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.cpu_time_limit_ms, 50);
        assert_eq!(config.wall_clock_limit_ms, 30_000);
        assert_eq!(config.termination_grace_us, 100);
    }

    #[test]
    fn test_timeout_config_with_cpu_limit() {
        let config = TimeoutConfig::with_cpu_limit(100);
        assert_eq!(config.cpu_time_limit_ms, 100);
        assert_eq!(config.wall_clock_limit_ms, 30_000); // default
    }

    #[test]
    fn test_timeout_config_durations() {
        let config = TimeoutConfig::default();
        assert_eq!(config.cpu_duration(), Duration::from_millis(50));
        assert_eq!(config.wall_clock_duration(), Duration::from_millis(30_000));
        assert_eq!(config.grace_duration(), Duration::from_micros(100));
    }

    #[test]
    fn test_timeout_config_validation_valid() {
        let config = TimeoutConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_timeout_config_validation_invalid_cpu_low() {
        let config = TimeoutConfig {
            cpu_time_limit_ms: 0,
            wall_clock_limit_ms: 1000,
            termination_grace_us: 100,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_timeout_config_validation_invalid_cpu_high() {
        let config = TimeoutConfig {
            cpu_time_limit_ms: 2000,
            wall_clock_limit_ms: 3000,
            termination_grace_us: 100,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_timeout_config_validation_invalid_wall_low() {
        let config = TimeoutConfig {
            cpu_time_limit_ms: 100,
            wall_clock_limit_ms: 50, // Less than CPU
            termination_grace_us: 100,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_timeout_config_validation_invalid_wall_high() {
        let config = TimeoutConfig {
            cpu_time_limit_ms: 50,
            wall_clock_limit_ms: 400_000, // More than 300s
            termination_grace_us: 100,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_execution_timer_creation() {
        let timer = ExecutionTimer::new();
        assert_eq!(timer.config().cpu_time_limit_ms, 50);
        assert!(!timer.is_active());
    }

    #[test]
    fn test_execution_timer_with_config() {
        let config = TimeoutConfig::with_cpu_limit(100);
        let timer = ExecutionTimer::with_config(config);
        assert_eq!(timer.config().cpu_time_limit_ms, 100);
    }

    #[test]
    fn test_execution_timer_with_cpu_limit() {
        let timer = ExecutionTimer::with_cpu_limit(75);
        assert_eq!(timer.config().cpu_time_limit_ms, 75);
    }

    #[test]
    fn test_execution_timer_default() {
        let timer: ExecutionTimer = Default::default();
        assert_eq!(timer.config().cpu_time_limit_ms, 50);
    }

    #[test]
    fn test_timeout_error_cpu_exceeded() {
        let err = TimeoutError::CpuTimeExceeded {
            used_ms: 60,
            limit_ms: 50,
        };
        assert!(err.is_cpu_timeout());
        assert!(!err.is_wall_clock_timeout());
        assert_eq!(err.limit_ms(), Some(50));
    }

    #[test]
    fn test_timeout_error_wall_clock_exceeded() {
        let err = TimeoutError::WallClockExceeded { limit_ms: 1000 };
        assert!(!err.is_cpu_timeout());
        assert!(err.is_wall_clock_timeout());
        assert_eq!(err.limit_ms(), Some(1000));
    }

    #[test]
    fn test_timeout_error_execution_terminated() {
        let err = TimeoutError::ExecutionTerminated;
        assert!(!err.is_cpu_timeout());
        assert!(!err.is_wall_clock_timeout());
        assert_eq!(err.limit_ms(), None);
    }

    #[test]
    fn test_timeout_error_display_cpu() {
        let err = TimeoutError::CpuTimeExceeded {
            used_ms: 75,
            limit_ms: 50,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("CPU time limit exceeded"));
        assert!(msg.contains("75ms"));
        assert!(msg.contains("50ms"));
    }

    #[test]
    fn test_timeout_error_display_wall_clock() {
        let err = TimeoutError::WallClockExceeded { limit_ms: 30_000 };
        let msg = format!("{}", err);
        assert!(msg.contains("Wall clock time limit exceeded"));
        assert!(msg.contains("30000ms"));
    }

    #[test]
    fn test_timeout_error_display_terminated() {
        let err = TimeoutError::ExecutionTerminated;
        let msg = format!("{}", err);
        assert!(msg.contains("Execution terminated"));
    }

    #[test]
    fn test_timeout_error_display_config() {
        let err = TimeoutError::ConfigError {
            message: "Invalid config".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Timeout configuration error"));
        assert!(msg.contains("Invalid config"));
    }

    #[test]
    fn test_timeout_error_display_timer() {
        let err = TimeoutError::TimerError {
            message: "Timer failed".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Timer error"));
        assert!(msg.contains("Timer failed"));
    }

    #[test]
    fn test_timeout_signal_flag() {
        reset_timeout_signal();
        assert!(!is_timeout_signaled());

        TIMEOUT_SIGNALED.store(true, Ordering::SeqCst);
        assert!(is_timeout_signaled());

        reset_timeout_signal();
        assert!(!is_timeout_signaled());
    }

    #[test]
    fn test_execute_and_check_success() {
        use crate::v8::platform;
        if !platform::is_initialized() {
            platform::initialize_platform().expect("Failed to initialize V8 platform");
        }

        let timer = ExecutionTimer::with_cpu_limit(5000); // 5 seconds
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let result = timer.execute_and_check(&mut isolate, || "success");

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert!(!timer.is_active());
    }

    #[test]
    fn test_execute_and_check_disabled() {
        use crate::v8::platform;
        if !platform::is_initialized() {
            platform::initialize_platform().expect("Failed to initialize V8 platform");
        }

        let timer = ExecutionTimer::with_config(TimeoutConfig {
            cpu_time_limit_ms: 0, // Will be replaced with default
            wall_clock_limit_ms: 30_000,
            termination_grace_us: 100,
        });
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        // Should use default 50ms limit since 0 is invalid
        assert_eq!(timer.config().cpu_time_limit_ms, 50);

        let result = timer.execute_and_check(&mut isolate, || 42);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
