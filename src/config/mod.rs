//! Configuration management for NANO runtime
//!
//! Provides configuration loading, validation, and file watching for
//! multi-app hosting scenarios. Supports JSON configuration files with
//! environment variable substitution.
//!
//! # Example
//!
//! ```rust,no_run
//! use nano::config::load_config;
//! use std::path::Path;
//!
//! # fn example() -> anyhow::Result<()> {
//! let config = load_config(Path::new("nano.json"))?;
//! println!("Loaded {} applications", config.apps.len());
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

pub mod app;
pub mod loader;
pub mod watcher;

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

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    /// Hostname for this app
    pub hostname: String,
    /// Path to the JavaScript entrypoint
    pub entrypoint: String,
    /// Path to sliver file for snapshot-based loading (alternative to entrypoint)
    #[serde(default)]
    pub sliver: Option<String>,
    /// Environment variables for this app
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    /// Resource limits
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hostname: String::new(),
            entrypoint: String::new(),
            sliver: None,
            env_vars: HashMap::new(),
            limits: AppLimits::default(),
            vfs_backend: VfsBackendType::default(),
            vfs_disk: None,
            vfs_s3: None,
        }
    }
}

/// Validation errors container
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationErrors {
    /// List of validation error messages
    pub errors: Vec<String>,
}

impl ValidationErrors {
    /// Create a new empty validation errors container
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

/// Resource limits for an application
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppLimits {
    /// Memory limit in MB (16-2048, default: 128)
    #[serde(default = "default_memory_limit")]
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
}

impl Default for AppLimits {
    fn default() -> Self {
        Self {
            memory_mb: default_memory_limit(),
            timeout_secs: default_timeout_secs(),
            workers: default_workers(),
            cpu_time_ms: default_cpu_time_ms(),
            cpu_time_enabled: default_cpu_time_enabled(),
        }
    }
}

fn default_memory_limit() -> u32 {
    128 // 128MB default
}

fn default_timeout_secs() -> u32 {
    30 // 30 seconds default
}

fn default_workers() -> u32 {
    4 // 4 workers default
}

fn default_cpu_time_ms() -> u32 {
    50 // 50ms default like Cloudflare Workers
}

fn default_cpu_time_enabled() -> bool {
    true // CPU time tracking enabled by default
}

/// Server configuration section
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ServerConfigSection {
    /// Server port (default: 8080)
    #[serde(default = "default_port")]
    pub port: u16,
    /// Server bind address (default: "127.0.0.1")
    #[serde(default = "default_bind")]
    pub host: String,
    /// Block fetch() requests to domains that resolve to private/internal IPs
    /// at connect-time (DNS rebinding protection). Default: true.
    ///
    /// WARNING: setting this to false allows tenant JS to reach the host's
    /// internal network (cloud metadata services, internal APIs) by pointing
    /// a domain they control at a private IP via short-TTL DNS.
    #[serde(default = "default_dns_rebinding_protection")]
    pub dns_rebinding_protection: bool,

    /// Bounded task queue depth per worker thread (default: 16).
    ///
    /// When a worker's queue is full, the request receives HTTP 503 with
    /// Retry-After: 1. Increase under bursty but fast workloads; lower to
    /// shed load earlier and avoid unbounded memory growth.
    #[serde(default = "default_queue_depth_per_worker")]
    pub queue_depth_per_worker: usize,

    /// How long (ms) each worker caches the entrypoint file's mtime (default: 1000).
    ///
    /// Workers check file modification time to invalidate the compiled handler
    /// cache on deploy. Setting to 0 checks on every request (maximum freshness,
    /// one sync syscall per request). Higher values reduce syscall overhead at
    /// the cost of slower hot-reload detection.
    #[serde(default = "default_handler_cache_refresh_ms")]
    pub handler_cache_refresh_ms: u64,
}

impl Default for ServerConfigSection {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_bind(),
            dns_rebinding_protection: default_dns_rebinding_protection(),
            queue_depth_per_worker: default_queue_depth_per_worker(),
            handler_cache_refresh_ms: default_handler_cache_refresh_ms(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_dns_rebinding_protection() -> bool {
    true
}

fn default_queue_depth_per_worker() -> usize {
    16
}

fn default_handler_cache_refresh_ms() -> u64 {
    1000
}

/// NANO runtime configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NanoConfig {
    /// List of applications
    pub apps: Vec<AppConfig>,
    /// Server configuration
    #[serde(default)]
    pub server: ServerConfigSection,
}

/// Legacy global settings (for backward compatibility)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalSettings {
    /// Default workers per app
    #[serde(default = "default_workers")]
    pub workers_per_app: u32,
    /// Server bind address (default: "127.0.0.1")
    #[serde(default = "default_bind")]
    pub bind_address: String,
    /// Server port
    #[serde(default = "default_port")]
    pub port: u16,
}

impl NanoConfig {
    /// Create a new empty configuration
    pub fn new() -> Self {
        Self {
            apps: Vec::new(),
            server: ServerConfigSection::default(),
        }
    }

    /// Validate the configuration (legacy method)
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_nano_config(self, None).map_err(|e| anyhow::anyhow!("{}", e))
    }
}

/// Validates an individual AppConfig
///
/// Performs comprehensive validation of an application configuration:
/// - Hostname is non-empty and valid DNS name
/// - Entrypoint path exists and is readable (if base_path provided)
/// - Memory limits within bounds (16-2048 MB)
/// - Timeout within bounds (1-300 seconds)
/// - Worker count within bounds (1-32)
/// - Environment variable keys are valid
pub fn validate_config(
    config: &AppConfig,
    base_path: Option<&std::path::Path>,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();

    // Validate hostname
    if config.hostname.is_empty() {
        errors.add("hostname cannot be empty");
    } else if !is_valid_hostname(&config.hostname) {
        errors.add(format!("'{}' is not a valid hostname", config.hostname));
    }

    // Validate entrypoint
    if config.entrypoint.is_empty() {
        errors.add("entrypoint cannot be empty");
    } else if config.entrypoint.contains("..") {
        // Path traversal prevention
        errors.add(format!(
            "entrypoint '{}' contains '..' which is not allowed for security",
            config.entrypoint
        ));
    } else if let Some(base) = base_path {
        let full_path = if std::path::Path::new(&config.entrypoint).is_absolute() {
            std::path::PathBuf::from(&config.entrypoint)
        } else {
            base.join(&config.entrypoint)
        };

        if !full_path.exists() {
            errors.add(format!(
                "entrypoint '{}' not found (resolved to: {})",
                config.entrypoint,
                full_path.display()
            ));
        } else if !full_path.is_file() {
            errors.add(format!("entrypoint '{}' is not a file", config.entrypoint));
        }
    }

    // Validate limits against TigerStyle constants
    let memory_min = 16u32;
    let memory_max = crate::limits::isolate::HEAP_SIZE_BYTES_MAX / (1024 * 1024);
    if config.limits.memory_mb < memory_min || config.limits.memory_mb > memory_max {
        errors.add(format!(
            "memory_mb must be between {} and {}, got {}",
            memory_min, memory_max, config.limits.memory_mb
        ));
    }

    let timeout_min = 1u32;
    let timeout_max = crate::limits::execution::TIMEOUT_MS / 1000;
    if config.limits.timeout_secs < timeout_min || config.limits.timeout_secs > timeout_max {
        errors.add(format!(
            "timeout_secs must be between {} and {}, got {}",
            timeout_min, timeout_max, config.limits.timeout_secs
        ));
    }

    let workers_min = 1u32;
    let workers_max = crate::limits::queue::WORKERS_PER_APP_MAX;
    if config.limits.workers < workers_min || config.limits.workers > workers_max {
        errors.add(format!(
            "workers must be between {} and {}, got {}",
            workers_min, workers_max, config.limits.workers
        ));
    }

    // Validate env vars
    for (key, value) in &config.env_vars {
        if key.is_empty() {
            errors.add("environment variable key cannot be empty");
        }
        if key.contains("..") || key.contains('/') || key.contains('\\') {
            errors.add(format!("suspicious environment variable key: '{}'", key));
        }
        if value.len() > 65536 {
            errors.add(format!(
                "environment variable '{}' value exceeds 64KB limit",
                key
            ));
        }
    }

    // Validate VFS backend configuration
    match config.vfs_backend {
        VfsBackendType::Memory => {
            // Memory backend requires no additional config
        }
        VfsBackendType::Disk => {
            if config.vfs_disk.is_none() {
                errors.add("vfs_backend is 'disk' but vfs_disk configuration is missing");
            } else {
                let disk_config = config.vfs_disk.as_ref().unwrap();
                if disk_config.base_path.is_empty() {
                    errors.add("vfs_disk.base_path cannot be empty");
                } else if disk_config.base_path.contains("..") {
                    errors
                        .add("vfs_disk.base_path contains '..' which is not allowed for security");
                }
            }
        }
        VfsBackendType::S3 => {
            if config.vfs_s3.is_none() {
                errors.add("vfs_backend is 's3' but vfs_s3 configuration is missing");
            } else {
                let s3_config = config.vfs_s3.as_ref().unwrap();
                if s3_config.endpoint.is_empty() {
                    errors.add("vfs_s3.endpoint cannot be empty");
                }
                if s3_config.bucket.is_empty() {
                    errors.add("vfs_s3.bucket cannot be empty");
                }
                if s3_config.access_key.is_empty() {
                    errors.add("vfs_s3.access_key cannot be empty");
                }
                if s3_config.secret_key.is_empty() {
                    errors.add("vfs_s3.secret_key cannot be empty");
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validates a complete NanoConfig
///
/// Validates the entire configuration including:
/// - At least one app is defined
/// - Maximum 1000 apps (DoS prevention)
/// - No duplicate hostnames (case-insensitive)
/// - Each app passes individual validation
pub fn validate_nano_config(
    config: &NanoConfig,
    base_path: Option<&std::path::Path>,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();

    // Check app count bounds
    if config.apps.is_empty() {
        errors.add("configuration must define at least one application");
    } else if config.apps.len() > 1000 {
        errors.add(format!(
            "too many applications: {} (max 1000)",
            config.apps.len()
        ));
    }

    // Check for duplicate hostnames
    let mut seen_hostnames: std::collections::HashSet<String> = std::collections::HashSet::new();
    for app in &config.apps {
        let lower_hostname = app.hostname.to_lowercase();
        if seen_hostnames.contains(&lower_hostname) {
            errors.add(format!(
                "duplicate hostname: '{}' (case-insensitive)",
                app.hostname
            ));
        } else {
            seen_hostnames.insert(lower_hostname);
        }
    }

    // Validate each app
    for (i, app) in config.apps.iter().enumerate() {
        match validate_config(app, base_path) {
            Ok(()) => {}
            Err(app_errors) => {
                for error in &app_errors.errors {
                    errors.add(format!("app[{}] ({}): {}", i, app.hostname, error));
                }
            }
        }
    }

    // Validate server config
    if config.server.port == 0 {
        errors.add("server port cannot be 0");
    }

    if config.server.host.is_empty() {
        errors.add("server host cannot be empty");
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Checks if a string is a valid hostname per RFC 1123
fn is_valid_hostname(hostname: &str) -> bool {
    if hostname.is_empty() {
        return false;
    }

    if hostname.len() > 253 {
        return false;
    }

    let labels: Vec<&str> = hostname.split('.').collect();
    for label in labels {
        if label.is_empty() {
            return false;
        }

        if label.len() > 63 {
            return false;
        }

        let bytes = label.as_bytes();

        if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
            return false;
        }

        for &b in bytes {
            if !(b.is_ascii_alphanumeric() || b == b'-') {
                return false;
            }
        }
    }

    true
}

/// Load configuration from a JSON file
pub fn load_config(path: &Path) -> anyhow::Result<NanoConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file: {}", e))?;

    let config: NanoConfig = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config JSON: {}", e))?;

    config.validate()?;

    Ok(config)
}

/// Load configuration from a string (useful for testing)
pub fn load_config_from_str(content: &str) -> anyhow::Result<NanoConfig> {
    let config: NanoConfig = serde_json::from_str(content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config JSON: {}", e))?;

    config.validate()?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation_empty() {
        let config = NanoConfig::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_duplicate_hostname() {
        let config = NanoConfig {
            apps: vec![
                AppConfig {
                    hostname: "example.com".to_string(),
                    entrypoint: "/app1.js".to_string(),
                    sliver: None,
                    env_vars: HashMap::new(),
                    limits: AppLimits::default(),
                    vfs_backend: VfsBackendType::default(),
                    vfs_disk: None,
                    vfs_s3: None,
                },
                AppConfig {
                    hostname: "example.com".to_string(),
                    entrypoint: "/app2.js".to_string(),
                    sliver: None,
                    env_vars: HashMap::new(),
                    limits: AppLimits::default(),
                    vfs_backend: VfsBackendType::default(),
                    vfs_disk: None,
                    vfs_s3: None,
                },
            ],
            server: ServerConfigSection::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("duplicate hostname"));
    }

    #[test]
    fn test_config_validation_empty_entrypoint() {
        let config = NanoConfig {
            apps: vec![AppConfig {
                hostname: "example.com".to_string(),
                entrypoint: "".to_string(),
                sliver: None,
                env_vars: HashMap::new(),
                limits: AppLimits::default(),
                vfs_backend: VfsBackendType::default(),
                vfs_disk: None,
                vfs_s3: None,
            }],
            server: ServerConfigSection::default(),
        };

        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("entrypoint"));
    }

    #[test]
    fn test_load_config_from_str() {
        let json = r#"{
            "apps": [
                {
                    "hostname": "api.example.com",
                    "entrypoint": "/apps/api.js",
                    "env_vars": {"API_KEY": "secret123"},
                    "limits": {"memory_mb": 256, "timeout_secs": 30}
                }
            ],
            "server": {
                "port": 8080,
                "host": "0.0.0.0"
            }
        }"#;

        let config = load_config_from_str(json).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.apps[0].hostname, "api.example.com");
        assert_eq!(config.apps[0].limits.memory_mb, 256);
        assert_eq!(config.server.port, 8080);
    }
}
