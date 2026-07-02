//! Application configuration types
//!
//! Defines the core data structures for application configuration including
//! per-app limits, environment variables, and the main AppConfig struct.
//!
//! # Security Considerations
//!
//! - Environment variables are explicitly configured per-app (not entire host env)
//! - Entrypoint paths are validated to prevent directory traversal
//! - Memory limits are bounded between 16-2048 MB to prevent resource exhaustion
//! - Timeouts are bounded between 1-300 seconds
//!
//! # Threat Model Coverage
//!
//! - T-05-02: Only inject explicitly configured env vars, not entire host environment
//! - T-05-04: Entrypoint path validation prevents path traversal attacks

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::worker::timeout::TimeoutConfig;

/// Environment variables for an application
///
/// Type alias for per-app environment variables. Only variables explicitly
/// configured in the config file are injected into the JS global scope,
/// not the entire host environment (per T-05-02).
pub type AppEnv = HashMap<String, String>;

/// Resource limits for an application
///
/// Defines resource constraints for each hosted application to prevent
/// one app from consuming excessive resources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppLimits {
    /// Maximum memory in MB (16-2048, default: 128)
    #[serde(default = "default_memory_mb")]
    pub memory_mb: u32,

    /// Request timeout in seconds (1-300, default: 30)
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u32,

    /// Number of worker threads (1-64, default: 4)
    #[serde(default = "default_workers")]
    pub workers: u32,

    /// CPU time limit in milliseconds (1-1000, default: 50 like Cloudflare Workers)
    #[serde(default = "default_cpu_time_ms")]
    pub cpu_time_ms: u32,

    /// Whether CPU time tracking is enabled (default: true)
    #[serde(default = "default_cpu_time_enabled")]
    pub cpu_time_enabled: bool,

    /// Maximum concurrent WebSocket connections per tenant.
    /// Default at runtime: floor(workers / 2). None means use default.
    #[serde(default)]
    pub max_ws_connections: Option<u32>,

    /// Idle timeout for WS worker threads in ms before shrink-to-zero.
    /// Default: 30000.
    #[serde(default)]
    pub ws_idle_timeout_ms: Option<u64>,
}

impl Default for AppLimits {
    fn default() -> Self {
        Self {
            memory_mb: default_memory_mb(),
            timeout_secs: default_timeout_secs(),
            workers: default_workers(),
            cpu_time_ms: default_cpu_time_ms(),
            cpu_time_enabled: default_cpu_time_enabled(),
            max_ws_connections: None,
            ws_idle_timeout_ms: None,
        }
    }
}

fn default_memory_mb() -> u32 {
    128
}

fn default_timeout_secs() -> u32 {
    30
}

fn default_workers() -> u32 {
    4
}

fn default_cpu_time_ms() -> u32 {
    50 // 50ms default like Cloudflare Workers
}

fn default_cpu_time_enabled() -> bool {
    true
}

impl AppLimits {
    /// Convert to TimeoutConfig for use with ExecutionTimer
    ///
    /// Creates a TimeoutConfig from the AppLimits settings.
    /// Uses cpu_time_ms for CPU limit and timeout_secs for wall clock limit.
    pub fn to_timeout_config(&self) -> TimeoutConfig {
        TimeoutConfig {
            cpu_time_limit_ms: if self.cpu_time_enabled { self.cpu_time_ms } else { 1000 }, // Use 1s if disabled
            wall_clock_limit_ms: self.timeout_secs * 1000,
            termination_grace_us: 100,
        }
    }

    /// Effective maximum concurrent WebSocket connections for this tenant.
    ///
    /// Returns the configured value, or floor(workers / 2) as the default per D-01b.
    /// This ensures at least half of worker threads remain available for normal HTTP.
    pub fn effective_max_ws_connections(&self) -> u32 {
        self.max_ws_connections.unwrap_or(self.workers / 2)
    }

    /// Effective idle timeout for WS worker threads in milliseconds.
    ///
    /// Returns the configured value, or 30,000 ms (30 seconds) per D-03b.
    pub fn effective_ws_idle_timeout_ms(&self) -> u64 {
        self.ws_idle_timeout_ms.unwrap_or(30_000)
    }
}

/// VFS backend type selection
///
/// Determines which storage backend is used for this application's
/// virtual file system.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VfsBackendType {
    /// In-memory storage (default, ephemeral)
    #[default]
    Memory,
    /// Local filesystem persistence
    Disk,
    /// S3-compatible object storage (requires vfs-s3 feature)
    S3,
}

/// Configuration for disk VFS backend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VfsDiskConfig {
    /// Base directory for file storage
    pub base_path: String,
}

/// Configuration for S3 VFS backend
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VfsS3Config {
    /// S3 endpoint URL (e.g., "https://s3.amazonaws.com" or "http://localhost:9000")
    pub endpoint: String,
    /// S3 bucket name
    pub bucket: String,
    /// AWS region (e.g., "us-east-1")
    pub region: String,
    /// Access key ID
    pub access_key: String,
    /// Secret access key
    pub secret_key: String,
    /// Optional key prefix for all objects
    #[serde(default)]
    pub prefix: Option<String>,
    /// Use path-style URLs (true for MinIO, false for AWS)
    #[serde(default)]
    pub path_style: bool,
}

/// Application configuration for a single hosted app
///
/// Defines all configuration for one application including its hostname,
/// entry point script, environment variables, and resource limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Hostname this app responds to (e.g., "api.example.com")
    pub hostname: String,

    /// Path to the entry point JavaScript file (required unless sliver is set)
    #[serde(default)]
    pub entrypoint: String,

    /// Path to sliver file for snapshot-based loading (alternative to entrypoint)
    #[serde(default)]
    pub sliver: Option<String>,

    /// Environment variables to inject into JS global scope (per T-05-02)
    #[serde(default)]
    pub env_vars: AppEnv,

    /// Resource limits for this app
    #[serde(default)]
    pub limits: AppLimits,

    /// VFS backend type (default: memory)
    #[serde(default)]
    pub vfs_backend: VfsBackendType,

    /// Disk backend configuration (required when vfs_backend = disk)
    #[serde(default)]
    pub vfs_disk: Option<VfsDiskConfig>,

    /// S3 backend configuration (required when vfs_backend = s3)
    #[serde(default)]
    pub vfs_s3: Option<VfsS3Config>,
}

/// Server configuration section
///
/// Global server settings for the NANO runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServerConfigSection {
    /// Port to listen on (default: 8080)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Host address to bind to (default: "0.0.0.0")
    #[serde(default = "default_host")]
    pub host: String,
}

impl Default for ServerConfigSection {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
        }
    }
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

/// Root configuration structure
///
/// The top-level configuration that defines all applications and server settings.
/// This is loaded from the JSON configuration file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NanoConfig {
    /// List of applications to host
    pub apps: Vec<AppConfig>,

    /// Server configuration
    #[serde(default)]
    pub server: ServerConfigSection,
}

/// Validation errors
///
/// Structured error information for configuration validation failures.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationErrors {
    /// List of individual error messages
    pub errors: Vec<String>,
}

impl ValidationErrors {
    /// Create a new validation errors container
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Add an error message
    pub fn add(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }

    /// Returns true if there are no errors
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Default for ValidationErrors {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, error) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "- {}", error)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}
