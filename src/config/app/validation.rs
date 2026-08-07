//! Configuration validation functions
use super::types::*;
use crate::limits;

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

    // Validate entrypoint or sliver (at least one must be specified)
    let has_entrypoint = !config.entrypoint.is_empty();
    let has_sliver = config.sliver.is_some();

    if !has_entrypoint && !has_sliver {
        errors.add("either entrypoint or sliver must be specified");
    }

    // Validate entrypoint if provided
    if has_entrypoint {
        if config.entrypoint.contains("..") {
            // Path traversal prevention (per T-05-04)
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
    }

    // Validate sliver path if provided
    if let Some(ref sliver) = config.sliver {
        if sliver.is_empty() {
            errors.add("sliver path cannot be empty");
        } else if sliver.contains("..") {
            // Path traversal prevention
            errors.add(format!(
                "sliver path '{}' contains '..' which is not allowed for security",
                sliver
            ));
        } else if let Some(base) = base_path {
            let full_path = if std::path::Path::new(sliver).is_absolute() {
                std::path::PathBuf::from(sliver)
            } else {
                base.join(sliver)
            };

            if !full_path.exists() {
                errors.add(format!(
                    "sliver '{}' not found (resolved to: {})",
                    sliver,
                    full_path.display()
                ));
            } else if !full_path.is_file() {
                errors.add(format!("sliver '{}' is not a file", sliver));
            }
        }
    }

    // Validate limits against TigerStyle constants
    let memory_min = 16u32;
    let memory_max = limits::isolate::HEAP_SIZE_BYTES_MAX / (1024 * 1024);
    if config.limits.memory_mb < memory_min || config.limits.memory_mb > memory_max {
        errors.add(format!(
            "memory_mb must be between {} and {}, got {}",
            memory_min, memory_max, config.limits.memory_mb
        ));
    }

    let timeout_min = 1u32;
    let timeout_max = 300u32;
    if config.limits.timeout_secs < timeout_min || config.limits.timeout_secs > timeout_max {
        errors.add(format!(
            "timeout_secs must be between {} and {}, got {}",
            timeout_min, timeout_max, config.limits.timeout_secs
        ));
    }

    let workers_min = 1u32;
    let workers_max = limits::queue::WORKERS_PER_APP_MAX;
    if config.limits.workers < workers_min || config.limits.workers > workers_max {
        errors.add(format!(
            "workers must be between {} and {}, got {}",
            workers_min, workers_max, config.limits.workers
        ));
    }

    // Validate CPU time limits (1-1000ms)
    let cpu_min = 1u32;
    let cpu_max = 1000u32;
    if config.limits.cpu_time_ms < cpu_min || config.limits.cpu_time_ms > cpu_max {
        errors.add(format!(
            "cpu_time_ms must be between {} and {}, got {}",
            cpu_min, cpu_max, config.limits.cpu_time_ms
        ));
    }

    // Validate env vars (per T-05-02: check for suspicious patterns)
    for (key, value) in &config.env_vars {
        // Check for empty keys
        if key.is_empty() {
            errors.add("environment variable key cannot be empty");
        }
        // Check for keys that look like path traversal attempts
        if key.contains("..") || key.contains('/') || key.contains('\\') {
            errors.add(format!("suspicious environment variable key: '{}'", key));
        }
        // Check for overly long values (potential DoS)
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
/// - Maximum 1000 apps (per T-05-03: DoS prevention)
/// - No duplicate hostnames (case-insensitive)
/// - Each app passes individual validation
///
/// # Arguments
///
/// * `config` - The NanoConfig to validate
/// * `base_path` - Optional base directory for entrypoint validation
///
/// # Returns
///
/// `Ok(())` if valid, `Err(ValidationErrors)` with all detected issues
pub fn validate_nano_config(
    config: &NanoConfig,
    base_path: Option<&std::path::Path>,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();

    // Check app count bounds (per T-05-03)
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

/// Checks if a string is a valid hostname
///
/// Validates hostname format per RFC 1123:
/// - Only alphanumeric characters, hyphens, and dots
/// - Each label <= 63 characters
/// - Total length <= 253 characters
/// - Cannot start or end with hyphen
/// - No consecutive dots
fn is_valid_hostname(hostname: &str) -> bool {
    if hostname.is_empty() {
        return false;
    }

    // Check total length
    if hostname.len() > 253 {
        return false;
    }

    // Check each label
    let labels: Vec<&str> = hostname.split('.').collect();
    for label in labels {
        // Empty label (consecutive dots or leading/trailing dot)
        if label.is_empty() {
            return false;
        }

        // Label too long
        if label.len() > 63 {
            return false;
        }

        // Check characters
        let bytes = label.as_bytes();

        // Cannot start or end with hyphen
        if bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
            return false;
        }

        // Only alphanumeric and hyphens allowed
        for &b in bytes {
            if !(b.is_ascii_alphanumeric() || b == b'-') {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_app_limits_defaults() {
        let limits = AppLimits::default();
        assert_eq!(limits.memory_mb, 128);
        assert_eq!(limits.timeout_secs, 30);
        assert_eq!(limits.workers, 4);
    }

    #[test]
    fn test_app_config_deserialization() {
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app/index.js",
            "env_vars": {"API_KEY": "secret123"},
            "limits": {"memory_mb": 256, "timeout_secs": 60, "workers": 8}
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hostname, "api.example.com");
        assert_eq!(config.entrypoint, "/app/index.js");
        assert_eq!(
            config.env_vars.get("API_KEY"),
            Some(&"secret123".to_string())
        );
        assert_eq!(config.limits.memory_mb, 256);
        assert_eq!(config.limits.timeout_secs, 60);
        assert_eq!(config.limits.workers, 8);
    }

    #[test]
    fn test_app_config_deserialization_defaults() {
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app/index.js"
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hostname, "api.example.com");
        assert_eq!(config.entrypoint, "/app/index.js");
        assert!(config.env_vars.is_empty());
        assert_eq!(config.limits.memory_mb, 128); // default
        assert_eq!(config.limits.timeout_secs, 30); // default
        assert_eq!(config.limits.workers, 4); // default
    }

    #[test]
    fn test_validation_rejects_empty_hostname() {
        let config = AppConfig {
            hostname: "".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("hostname")));
    }

    #[test]
    fn test_validation_rejects_empty_entrypoint() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("entrypoint")));
    }

    #[test]
    fn test_validation_rejects_invalid_hostname() {
        let config = AppConfig {
            hostname: "not a valid hostname!".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("hostname")));
    }

    #[test]
    fn test_validation_rejects_path_traversal() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "../../../etc/passwd".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("..") || e.contains("security")));
    }

    #[test]
    fn test_validation_rejects_invalid_memory() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 5, // too low
                timeout_secs: 30,
                workers: 4,
                cpu_time_ms: 50,
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("memory_mb")));
    }

    #[test]
    fn test_validation_rejects_invalid_timeout() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 128,
                timeout_secs: 0, // too low
                workers: 4,
                cpu_time_ms: 50,
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("timeout_secs")));
    }

    #[test]
    fn test_validation_rejects_invalid_workers() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 128,
                timeout_secs: 30,
                workers: 100, // too high
                cpu_time_ms: 50,
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("workers")));
    }

    #[test]
    fn test_validation_accepts_valid_config() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_hostname_variations() {
        // Valid hostnames
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("api.example.com"));
        assert!(is_valid_hostname("a.b.c.d.example.com"));
        assert!(is_valid_hostname("test-123.example-site.org"));
        assert!(is_valid_hostname("localhost"));

        // Invalid hostnames
        assert!(!is_valid_hostname("")); // empty
        assert!(!is_valid_hostname("-example.com")); // starts with hyphen
        assert!(!is_valid_hostname("example-.com")); // label ends with hyphen
        assert!(!is_valid_hostname("example..com")); // consecutive dots
        assert!(!is_valid_hostname(".example.com")); // starts with dot
        assert!(!is_valid_hostname("example.com.")); // ends with dot (for our purposes)
        assert!(!is_valid_hostname("ex ample.com")); // space
        assert!(!is_valid_hostname("ex_ample.com")); // underscore
    }

    #[test]
    fn test_nano_config_deserialization() {
        let json = r#"{
            "apps": [
                {
                    "hostname": "api.example.com",
                    "entrypoint": "/apps/api/index.js",
                    "env_vars": {"API_KEY": "secret123"},
                    "limits": {"memory_mb": 128, "timeout_secs": 30, "workers": 4}
                },
                {
                    "hostname": "blog.example.com",
                    "entrypoint": "/apps/blog/index.js",
                    "env_vars": {"DB_URL": "localhost"},
                    "limits": {"memory_mb": 64, "timeout_secs": 10, "workers": 2}
                }
            ],
            "server": {"port": 8080, "host": "0.0.0.0"}
        }"#;

        let config: NanoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.apps.len(), 2);
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
    }

    #[test]
    fn test_validate_nano_config_rejects_duplicates() {
        let config = NanoConfig {
            apps: vec![
                AppConfig {
                    hostname: "api.example.com".to_string(),
                    entrypoint: "/app1.js".to_string(),
                    env_vars: Default::default(),
                    limits: Default::default(),
                    ..Default::default()
                },
                AppConfig {
                    hostname: "API.EXAMPLE.COM".to_string(), // same as above, different case
                    entrypoint: "/app2.js".to_string(),
                    env_vars: Default::default(),
                    limits: Default::default(),
                    ..Default::default()
                },
            ],
            server: Default::default(),
        };

        let result = validate_nano_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("duplicate")));
    }

    #[test]
    fn test_validate_nano_config_rejects_empty_apps() {
        let config = NanoConfig {
            apps: vec![],
            server: Default::default(),
        };

        let result = validate_nano_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("at least one")));
    }

    #[test]
    fn test_validate_nano_config_rejects_too_many_apps() {
        let mut apps = Vec::new();
        for i in 0..1001 {
            apps.push(AppConfig {
                hostname: format!("app{}.example.com", i),
                entrypoint: "/app.js".to_string(),
                env_vars: Default::default(),
                limits: Default::default(),
                ..Default::default()
            });
        }

        let config = NanoConfig {
            apps,
            server: Default::default(),
        };

        let result = validate_nano_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("too many")));
    }

    #[test]
    fn test_validate_nano_config_accepts_valid() {
        let config = NanoConfig {
            apps: vec![AppConfig {
                hostname: "api.example.com".to_string(),
                entrypoint: "/app.js".to_string(),
                env_vars: Default::default(),
                limits: Default::default(),
                ..Default::default()
            }],
            server: Default::default(),
        };

        let result = validate_nano_config(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_deny_unknown_fields() {
        // This should fail to deserialize because of unknown field
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app/index.js",
            "unknown_field": "value"
        }"#;

        let result: Result<AppConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_env_var_validation() {
        let mut env_vars = HashMap::new();
        env_vars.insert("../etc/passwd".to_string(), "value".to_string()); // suspicious key

        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            env_vars,
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("suspicious")));
    }

    #[test]
    fn test_vfs_backend_type_default() {
        let backend_type: VfsBackendType = Default::default();
        assert_eq!(backend_type, VfsBackendType::Memory);
    }

    #[test]
    fn test_vfs_backend_type_deserialization() {
        assert_eq!(
            serde_json::from_str::<VfsBackendType>("\"memory\"").unwrap(),
            VfsBackendType::Memory
        );
        assert_eq!(
            serde_json::from_str::<VfsBackendType>("\"disk\"").unwrap(),
            VfsBackendType::Disk
        );
        assert_eq!(
            serde_json::from_str::<VfsBackendType>("\"s3\"").unwrap(),
            VfsBackendType::S3
        );
    }

    #[test]
    fn test_vfs_disk_config_deserialization() {
        let json = r#"{"base_path": "/data/nano"}"#;
        let config: VfsDiskConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.base_path, "/data/nano");
    }

    #[test]
    fn test_vfs_s3_config_deserialization() {
        let json = r#"{
            "endpoint": "http://localhost:9000",
            "bucket": "nano-vfs",
            "region": "us-east-1",
            "access_key": "minioadmin",
            "secret_key": "minioadmin",
            "prefix": "app1",
            "path_style": true
        }"#;
        let config: VfsS3Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.endpoint, "http://localhost:9000");
        assert_eq!(config.bucket, "nano-vfs");
        assert_eq!(config.prefix, Some("app1".to_string()));
        assert!(config.path_style);
    }

    #[test]
    fn test_app_config_with_vfs_disk() {
        let json = r#"{
            "hostname": "api.example.com",
            "entrypoint": "/app/index.js",
            "vfs_backend": "disk",
            "vfs_disk": {
                "base_path": "/data/api"
            }
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.vfs_backend, VfsBackendType::Disk);
        assert!(config.vfs_disk.is_some());
        assert_eq!(config.vfs_disk.unwrap().base_path, "/data/api");
    }

    #[test]
    fn test_validation_rejects_disk_without_config() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            vfs_backend: VfsBackendType::Disk,
            vfs_disk: None,
            vfs_s3: None,
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("vfs_disk") && e.contains("missing")));
    }

    #[test]
    fn test_validation_rejects_s3_without_config() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            vfs_backend: VfsBackendType::S3,
            vfs_disk: None,
            vfs_s3: None,
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("vfs_s3") && e.contains("missing")));
    }

    #[test]
    fn test_validation_rejects_disk_path_traversal() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            env_vars: Default::default(),
            limits: Default::default(),
            vfs_backend: VfsBackendType::Disk,
            vfs_disk: Some(VfsDiskConfig {
                base_path: "../../../etc/passwd".to_string(),
            }),
            vfs_s3: None,
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("base_path") && e.contains("..")));
    }

    #[test]
    fn test_validation_rejects_neither_entrypoint_nor_sliver() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "".to_string(),
            sliver: None,
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("either") && e.contains("entrypoint") && e.contains("sliver")));
    }

    #[test]
    fn test_validation_accepts_sliver_without_entrypoint() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "".to_string(),
            sliver: Some("./app.sliver".to_string()),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_accepts_both_entrypoint_and_sliver() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app.js".to_string(),
            sliver: Some("./app.sliver".to_string()),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_rejects_sliver_path_traversal() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "".to_string(),
            sliver: Some("../../../etc/passwd.sliver".to_string()),
            env_vars: Default::default(),
            limits: Default::default(),
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .errors
            .iter()
            .any(|e| e.contains("sliver") && e.contains("..")));
    }

    #[test]
    fn test_app_config_deserialization_with_sliver() {
        let json = r#"{
            "hostname": "api.example.com",
            "sliver": "./api-v1.sliver",
            "limits": {
                "workers": 4
            }
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hostname, "api.example.com");
        assert_eq!(config.sliver, Some("./api-v1.sliver".to_string()));
        assert!(config.entrypoint.is_empty());
    }

    #[test]
    fn test_app_limits_cpu_time_defaults() {
        let limits = AppLimits::default();
        assert_eq!(limits.cpu_time_ms, 50); // Cloudflare default
        assert!(limits.cpu_time_enabled); // Enabled by default
    }

    #[test]
    fn test_app_limits_deserialization_with_cpu_time() {
        let json = r#"{
            "memory_mb": 128,
            "timeout_secs": 30,
            "workers": 4,
            "cpu_time_ms": 100,
            "cpu_time_enabled": false
        }"#;

        let limits: AppLimits = serde_json::from_str(json).unwrap();
        assert_eq!(limits.cpu_time_ms, 100);
        assert!(!limits.cpu_time_enabled);
    }

    #[test]
    fn test_app_limits_deserialization_defaults() {
        let json = r#"{
            "memory_mb": 128
        }"#;

        let limits: AppLimits = serde_json::from_str(json).unwrap();
        assert_eq!(limits.cpu_time_ms, 50); // Default
        assert!(limits.cpu_time_enabled); // Default
    }

    #[test]
    fn test_validation_rejects_invalid_cpu_time_low() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 128,
                timeout_secs: 30,
                workers: 4,
                cpu_time_ms: 0, // too low
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("cpu_time_ms")));
    }

    #[test]
    fn test_validation_rejects_invalid_cpu_time_high() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 128,
                timeout_secs: 30,
                workers: 4,
                cpu_time_ms: 2000, // too high
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.errors.iter().any(|e| e.contains("cpu_time_ms")));
    }

    #[test]
    fn test_validation_accepts_valid_cpu_time() {
        let config = AppConfig {
            hostname: "api.example.com".to_string(),
            entrypoint: "/app/index.js".to_string(),
            env_vars: Default::default(),
            limits: AppLimits {
                memory_mb: 128,
                timeout_secs: 30,
                workers: 4,
                cpu_time_ms: 50,
                cpu_time_enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let result = validate_config(&config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_app_limits_to_timeout_config() {
        let limits = AppLimits {
            memory_mb: 128,
            timeout_secs: 30,
            workers: 4,
            cpu_time_ms: 100,
            cpu_time_enabled: true,
            ..Default::default()
        };

        let timeout_config = limits.to_timeout_config();
        assert_eq!(timeout_config.cpu_time_limit_ms, 100);
        assert_eq!(timeout_config.wall_clock_limit_ms, 30_000); // 30 seconds
    }

    #[test]
    fn test_app_limits_to_timeout_config_disabled() {
        let limits = AppLimits {
            memory_mb: 128,
            timeout_secs: 30,
            workers: 4,
            cpu_time_ms: 50,
            cpu_time_enabled: false,
            ..Default::default()
        };

        let timeout_config = limits.to_timeout_config();
        // When disabled, uses 1000ms (1 second) as the effective limit
        assert_eq!(timeout_config.cpu_time_limit_ms, 1000);
        assert_eq!(timeout_config.wall_clock_limit_ms, 30_000);
    }

    #[test]
    fn test_app_limits_ws_defaults() {
        // Default AppLimits has max_ws_connections = None
        let limits = AppLimits::default();
        assert_eq!(limits.max_ws_connections, None);
        assert_eq!(limits.ws_idle_timeout_ms, None);

        // effective_max_ws_connections() with workers=4 returns 2 (floor(4/2))
        assert_eq!(limits.effective_max_ws_connections(), 2);

        // effective_ws_idle_timeout_ms() returns 30000
        assert_eq!(limits.effective_ws_idle_timeout_ms(), 30_000);

        // Custom workers value: floor(workers / 2)
        let limits_8 = AppLimits {
            workers: 8,
            ..Default::default()
        };
        assert_eq!(limits_8.effective_max_ws_connections(), 4);

        // Configured values override defaults
        let limits_custom = AppLimits {
            max_ws_connections: Some(10),
            ws_idle_timeout_ms: Some(60_000),
            ..Default::default()
        };
        assert_eq!(limits_custom.effective_max_ws_connections(), 10);
        assert_eq!(limits_custom.effective_ws_idle_timeout_ms(), 60_000);

        // Backward compat: JSON without WS fields deserializes without error
        // (deny_unknown_fields applies to extra fields, not missing ones with #[serde(default)])
        let json = r#"{"memory_mb":128,"timeout_secs":30,"workers":4,"cpu_time_ms":50,"cpu_time_enabled":true}"#;
        let deserialized: AppLimits = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.max_ws_connections, None);
        assert_eq!(deserialized.ws_idle_timeout_ms, None);
        assert_eq!(deserialized.effective_max_ws_connections(), 2);
        assert_eq!(deserialized.effective_ws_idle_timeout_ms(), 30_000);
    }
}
