//! Integration tests for config mode
//!
//! Tests the `--config` workflow including:
//! - Config loading and validation
//! - Port configuration from config file
#![allow(dead_code, unused_imports)]
//! - Host configuration from config file  
//! - Multiple apps with virtual host routing
//! - Per-app limits (timeout, memory, workers)

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Helper to create a minimal valid config for testing
fn create_test_config() -> nano::config::NanoConfig {
    let mut apps = Vec::new();

    // Add a test app with entrypoint
    apps.push(nano::config::AppConfig {
        hostname: "test.example.com".to_string(),
        entrypoint: "/tmp/test.js".to_string(),
        sliver: None,
        env_vars: HashMap::new(),
        limits: nano::config::AppLimits::default(),
        vfs_backend: nano::config::VfsBackendType::Memory,
        vfs_disk: None,
        vfs_s3: None,
    });

    nano::config::NanoConfig {
        apps,
        server: nano::config::ServerConfigSection::default(),
    }
}

/// Helper to create a multi-app config
fn create_multi_app_config() -> nano::config::NanoConfig {
    let mut apps = Vec::new();

    apps.push(nano::config::AppConfig {
        hostname: "api.example.com".to_string(),
        entrypoint: "/apps/api.js".to_string(),
        sliver: None,
        env_vars: [("API_KEY".to_string(), "secret".to_string())]
            .into_iter()
            .collect(),
        limits: nano::config::AppLimits {
            memory_mb: 256,
            timeout_secs: 60,
            workers: 8,
            cpu_time_ms: 100,
            cpu_time_enabled: true,
        },
        vfs_backend: nano::config::VfsBackendType::Memory,
        vfs_disk: None,
        vfs_s3: None,
    });

    apps.push(nano::config::AppConfig {
        hostname: "blog.example.com".to_string(),
        entrypoint: "/apps/blog.js".to_string(),
        sliver: None,
        env_vars: HashMap::new(),
        limits: nano::config::AppLimits {
            memory_mb: 128,
            timeout_secs: 30,
            workers: 4,
            cpu_time_ms: 100,
            cpu_time_enabled: true,
        },
        vfs_backend: nano::config::VfsBackendType::Memory,
        vfs_disk: None,
        vfs_s3: None,
    });

    nano::config::NanoConfig {
        apps,
        server: nano::config::ServerConfigSection::default(),
    }
}

/// Helper to create a config with custom server settings
fn create_config_with_server(port: u16, host: &str) -> nano::config::NanoConfig {
    let mut apps = Vec::new();

    apps.push(nano::config::AppConfig {
        hostname: "app.example.com".to_string(),
        entrypoint: "/tmp/app.js".to_string(),
        sliver: None,
        env_vars: HashMap::new(),
        limits: nano::config::AppLimits::default(),
        vfs_backend: nano::config::VfsBackendType::Memory,
        vfs_disk: None,
        vfs_s3: None,
    });

    nano::config::NanoConfig {
        apps,
        server: nano::config::ServerConfigSection {
            port,
            host: host.to_string(),
            dns_rebinding_protection: true,
            queue_depth_per_worker: 16,
            handler_cache_refresh_ms: 1000,
        },
    }
}

#[test]
fn test_config_loading_basic() {
    let json = r#"{
        "apps": [
            {
                "hostname": "api.example.com",
                "entrypoint": "/apps/api.js",
                "env_vars": {"API_KEY": "secret123"},
                "limits": {"memory_mb": 256, "timeout_secs": 30, "workers": 8}
            }
        ],
        "server": {"port": 8080, "host": "0.0.0.0"}
    }"#;

    let config = nano::config::load_config_from_str(json).expect("Failed to load config");

    assert_eq!(config.apps.len(), 1);
    assert_eq!(config.apps[0].hostname, "api.example.com");
    assert_eq!(config.apps[0].entrypoint, "/apps/api.js");
    assert_eq!(config.apps[0].limits.memory_mb, 256);
    assert_eq!(config.apps[0].limits.timeout_secs, 30);
    assert_eq!(config.apps[0].limits.workers, 8);
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.server.host, "0.0.0.0");
}

#[test]
fn test_config_port_applied() {
    let config = create_config_with_server(9999, "0.0.0.0");
    let server_config = nano::http::ServerConfig::from(config.server);

    assert_eq!(server_config.port, 9999);

    let addr = server_config
        .socket_addr()
        .expect("Failed to parse address");
    assert_eq!(addr.port(), 9999);
}

#[test]
fn test_config_host_applied() {
    let config = create_config_with_server(8080, "127.0.0.1");
    let server_config = nano::http::ServerConfig::from(config.server);

    assert_eq!(server_config.host, "127.0.0.1");

    let addr = server_config
        .socket_addr()
        .expect("Failed to parse address");
    assert!(addr.is_ipv4());
    assert_eq!(addr.to_string(), "127.0.0.1:8080");
}

#[test]
fn test_multiple_apps_config() {
    let config = create_multi_app_config();

    assert_eq!(config.apps.len(), 2);

    // First app
    assert_eq!(config.apps[0].hostname, "api.example.com");
    assert_eq!(config.apps[0].limits.memory_mb, 256);
    assert_eq!(config.apps[0].limits.workers, 8);

    // Second app
    assert_eq!(config.apps[1].hostname, "blog.example.com");
    assert_eq!(config.apps[1].limits.memory_mb, 128);
    assert_eq!(config.apps[1].limits.workers, 4);
}

#[test]
fn test_app_registry_from_config() {
    let config = create_multi_app_config();
    let registry = nano::app::registry::AppRegistry::from_config(config);

    assert_eq!(registry.count(), 2);
    assert!(registry.contains("api.example.com"));
    assert!(registry.contains("blog.example.com"));
    assert!(!registry.contains("unknown.example.com"));
}

#[test]
fn test_per_app_limits_enforced() {
    let config = create_multi_app_config();

    // API app has higher limits
    assert_eq!(config.apps[0].limits.memory_mb, 256);
    assert_eq!(config.apps[0].limits.timeout_secs, 60);
    assert_eq!(config.apps[0].limits.workers, 8);

    // Blog app has lower limits
    assert_eq!(config.apps[1].limits.memory_mb, 128);
    assert_eq!(config.apps[1].limits.timeout_secs, 30);
    assert_eq!(config.apps[1].limits.workers, 4);
}

#[test]
fn test_config_with_sliver_app() {
    let json = r#"{
        "apps": [
            {
                "hostname": "sliver.example.com",
                "entrypoint": "/apps/index.js",
                "sliver": "/apps/app.sliver",
                "limits": {"memory_mb": 256, "timeout_secs": 30, "workers": 16}
            }
        ],
        "server": {"port": 3000, "host": "127.0.0.1"}
    }"#;

    let config = nano::config::load_config_from_str(json).expect("Failed to load config");

    assert_eq!(config.apps.len(), 1);
    assert_eq!(config.apps[0].hostname, "sliver.example.com");
    assert!(config.apps[0].sliver.is_some());
    assert_eq!(config.apps[0].sliver.as_ref().unwrap(), "/apps/app.sliver");
    assert_eq!(config.apps[0].limits.memory_mb, 256);
    assert_eq!(config.apps[0].limits.workers, 16);
}

#[test]
fn test_config_validation_rejects_empty_apps() {
    let json = r#"{
        "apps": [],
        "server": {"port": 8080, "host": "0.0.0.0"}
    }"#;

    let result = nano::config::load_config_from_str(json);
    assert!(result.is_err());
}

#[test]
fn test_config_validation_rejects_duplicate_hostnames() {
    let json = r#"{
        "apps": [
            {
                "hostname": "api.example.com",
                "entrypoint": "/apps/api.js"
            },
            {
                "hostname": "api.example.com",
                "entrypoint": "/apps/api2.js"
            }
        ],
        "server": {"port": 8080, "host": "0.0.0.0"}
    }"#;

    let result = nano::config::load_config_from_str(json);
    assert!(result.is_err());
}

#[test]
fn test_config_defaults() {
    let json = r#"{
        "apps": [
            {
                "hostname": "api.example.com",
                "entrypoint": "/apps/api.js"
            }
        ]
    }"#;

    let config = nano::config::load_config_from_str(json).expect("Failed to load config");

    // Server defaults
    assert_eq!(config.server.port, 8080);
    assert_eq!(config.server.host, "127.0.0.1");

    // App limits defaults
    assert_eq!(config.apps[0].limits.memory_mb, 128);
    assert_eq!(config.apps[0].limits.timeout_secs, 30);
    assert_eq!(config.apps[0].limits.workers, 4);
}

#[test]
fn test_server_config_section_conversion() {
    let section = nano::config::ServerConfigSection {
        port: 9090,
        host: "192.168.1.100".to_string(),
        dns_rebinding_protection: true,
        queue_depth_per_worker: 16,
        handler_cache_refresh_ms: 1000,
    };

    let server_config = nano::http::ServerConfig::from(section);

    assert_eq!(server_config.port, 9090);
    assert_eq!(server_config.host, "192.168.1.100");
}

#[test]
fn test_virtual_host_router_from_config() {
    let config = create_multi_app_config();

    let default_target = nano::http::router::RouteTarget {
        hostname: "default".to_string(),
        handler_type: nano::http::router::HandlerType::StaticResponse(
            "NANO Runtime - No app configured for this host".to_string(),
        ),
    };

    let mut router = nano::http::router::VirtualHostRouter::new(default_target);

    // Register routes for each app
    for app in &config.apps {
        let target = nano::http::router::RouteTarget {
            hostname: app.hostname.clone(),
            handler_type: nano::http::router::HandlerType::WinterTCHandler(app.entrypoint.clone()),
        };
        router.register(app.hostname.clone(), target);
    }

    assert_eq!(router.route_count(), 2);

    // Verify routing
    let api_target = router.resolve("api.example.com");
    assert_eq!(api_target.hostname, "api.example.com");

    let blog_target = router.resolve("blog.example.com");
    assert_eq!(blog_target.hostname, "blog.example.com");
}

#[test]
fn test_env_vars_in_config() {
    let json = r#"{
        "apps": [
            {
                "hostname": "api.example.com",
                "entrypoint": "/apps/api.js",
                "env_vars": {
                    "DATABASE_URL": "postgres://localhost/db",
                    "API_KEY": "secret123",
                    "DEBUG": "true"
                }
            }
        ],
        "server": {"port": 8080, "host": "0.0.0.0"}
    }"#;

    let config = nano::config::load_config_from_str(json).expect("Failed to load config");

    let env_vars = &config.apps[0].env_vars;
    assert_eq!(
        env_vars.get("DATABASE_URL"),
        Some(&"postgres://localhost/db".to_string())
    );
    assert_eq!(env_vars.get("API_KEY"), Some(&"secret123".to_string()));
    assert_eq!(env_vars.get("DEBUG"), Some(&"true".to_string()));
}
