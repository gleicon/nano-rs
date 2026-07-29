//! Application registry for multi-app hosting
//!
//! Manages the mapping of hostnames to application configurations
//! and provides lookup functionality for request routing.
//!
//! # Sliver Support
//!
//! The registry can hold both entrypoint-based and sliver-based apps.
//! Sliver data is stored separately and can be retrieved by workers
//! for snapshot-based isolate restoration.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::sliver::UnpackedSliver;

/// Registry of all hosted applications
#[derive(Debug, Clone)]
pub struct AppRegistry {
    /// Map of hostnames to application configurations
    apps: Arc<HashMap<String, AppConfig>>,
    /// Map of hostnames to unpacked sliver data (for sliver-based apps)
    sliver_data: Arc<HashMap<String, UnpackedSliver>>,
}

impl AppRegistry {
    /// Create a new registry from a map of apps
    pub fn new(apps: HashMap<String, AppConfig>) -> Self {
        Self {
            apps: Arc::new(apps),
            sliver_data: Arc::new(HashMap::new()),
        }
    }

    /// Create registry from config
    pub fn from_config(config: crate::config::NanoConfig) -> Self {
        let apps: HashMap<String, AppConfig> = config
            .apps
            .into_iter()
            .map(|app| (app.hostname.clone(), app))
            .collect();
        Self::new(apps)
    }

    /// Create a new registry with sliver data
    ///
    /// This constructor is used when apps are loaded from sliver files.
    pub fn with_sliver_data(
        apps: HashMap<String, AppConfig>,
        sliver_data: HashMap<String, UnpackedSliver>,
    ) -> Self {
        Self {
            apps: Arc::new(apps),
            sliver_data: Arc::new(sliver_data),
        }
    }

    /// Register an app from a sliver file
    ///
    /// Reads the sliver file, unpacks it, and registers the app
    /// with its hostname extracted from the sliver metadata.
    ///
    /// # Arguments
    /// * `sliver_path` - Path to the sliver file
    /// * `config_base` - Optional base configuration to merge with
    ///
    /// # Returns
    /// The hostname of the registered app, or an error if registration fails
    pub fn register_from_sliver(
        &mut self,
        sliver_path: &std::path::Path,
        config_base: Option<AppConfig>,
    ) -> anyhow::Result<String> {
        use anyhow::Context;
        use crate::sliver::unpack_sliver;

        // Read sliver file
        let sliver_data = std::fs::read(sliver_path)
            .with_context(|| format!("Failed to read sliver file: {}", sliver_path.display()))?;

        // Unpack sliver
        let unpacked = unpack_sliver(&sliver_data)
            .with_context(|| format!("Failed to unpack sliver: {}", sliver_path.display()))?;

        // Extract hostname from metadata
        let hostname = unpacked.metadata.hostname.clone();

        // Create or update app config
        let mut app_config = config_base.unwrap_or_default();
        app_config.hostname = hostname.clone();
        app_config.sliver = Some(sliver_path.to_string_lossy().to_string());

        // For sliver-based apps, entrypoint can be empty (snapshot-based)
        // The actual entrypoint is encoded in the V8 heap snapshot
        if app_config.entrypoint.is_empty() {
            // This is expected for pure sliver-based apps
            tracing::debug!("Sliver-based app '{}' has no entrypoint (uses snapshot)", hostname);
        }

        // Get mutable references to update the registry
        let apps = Arc::make_mut(&mut self.apps);
        let sliver_data_map = Arc::make_mut(&mut self.sliver_data);

        // Store app config and sliver data
        apps.insert(hostname.clone(), app_config);
        sliver_data_map.insert(hostname.clone(), unpacked);

        tracing::info!("Registered sliver-based app '{}' from {}", hostname, sliver_path.display());

        Ok(hostname)
    }

    /// Get sliver data for a hostname
    ///
    /// Returns the unpacked sliver data if this is a sliver-based app.
    pub fn get_sliver_data(&self, hostname: &str) -> Option<UnpackedSliver> {
        self.sliver_data.get(hostname).cloned()
    }

    /// Check if an app is sliver-based
    pub fn is_sliver_app(&self, hostname: &str) -> bool {
        self.sliver_data.contains_key(hostname)
    }

    /// Get application configuration by hostname
    pub fn get(&self, hostname: &str) -> Option<AppConfig> {
        self.apps.get(hostname).cloned()
    }

    /// Check if hostname is registered
    pub fn contains(&self, hostname: &str) -> bool {
        self.apps.contains_key(hostname)
    }

    /// Get all registered hostnames
    pub fn all_hostnames(&self) -> impl Iterator<Item = String> + '_ {
        self.apps.keys().cloned()
    }

    /// Get count of registered apps
    pub fn count(&self) -> usize {
        self.apps.len()
    }
}

impl Default for AppRegistry {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

impl From<HashMap<String, AppConfig>> for AppRegistry {
    fn from(apps: HashMap<String, AppConfig>) -> Self {
        Self::new(apps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_config(hostname: &str) -> AppConfig {
        AppConfig {
            hostname: hostname.to_string(),
            entrypoint: format!("./{}.js", hostname),
            sliver: None,
            env_vars: HashMap::new(),
            limits: crate::config::AppLimits::default(),
            vfs_backend: Default::default(),
            vfs_disk: None,
            vfs_s3: None,
        }
    }

    #[test]
    fn test_registry_get() {
        let mut apps = HashMap::new();
        apps.insert("app1".to_string(), create_test_config("app1"));

        let registry = AppRegistry::new(apps);

        assert!(registry.get("app1").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_contains() {
        let mut apps = HashMap::new();
        apps.insert("app1".to_string(), create_test_config("app1"));

        let registry = AppRegistry::new(apps);

        assert!(registry.contains("app1"));
        assert!(!registry.contains("nonexistent"));
    }

    #[test]
    fn test_registry_all_hostnames() {
        let mut apps = HashMap::new();
        apps.insert("app1".to_string(), create_test_config("app1"));
        apps.insert("app2".to_string(), create_test_config("app2"));

        let registry = AppRegistry::new(apps);
        let hostnames: Vec<_> = registry.all_hostnames().collect();

        assert_eq!(hostnames.len(), 2);
        assert!(hostnames.contains(&"app1".to_string()));
        assert!(hostnames.contains(&"app2".to_string()));
    }

    #[test]
    fn test_registry_default() {
        let registry = AppRegistry::default();
        assert_eq!(registry.count(), 0);
        assert!(!registry.contains("any"));
    }

    #[test]
    fn test_registry_from_hashmap() {
        let mut apps = HashMap::new();
        apps.insert("app1".to_string(), create_test_config("app1"));

        let registry: AppRegistry = apps.into();
        assert_eq!(registry.count(), 1);
        assert!(registry.contains("app1"));
    }

    #[test]
    fn test_registry_count() {
        let mut apps = HashMap::new();
        apps.insert("app1".to_string(), create_test_config("app1"));
        apps.insert("app2".to_string(), create_test_config("app2"));

        let registry = AppRegistry::new(apps);
        assert_eq!(registry.count(), 2);
    }

    // Sliver-based app tests
    use crate::sliver::{pack_sliver, SliverMetadata};
    use tempfile::TempDir;

    fn create_test_sliver(hostname: &str) -> (TempDir, std::path::PathBuf, Vec<u8>) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let sliver_path = temp_dir.path().join("test.sliver");

        let metadata = SliverMetadata::new(hostname, "1.1.0");
        let archive = pack_sliver(&metadata, None, None).unwrap();

        std::fs::write(&sliver_path, &archive).expect("Failed to write sliver");

        (temp_dir, sliver_path, archive)
    }

    #[test]
    fn test_register_from_sliver_success() {
        let (_temp_dir, sliver_path, _archive) = create_test_sliver("sliver-app.example.com");

        let mut registry = AppRegistry::default();
        let hostname = registry.register_from_sliver(&sliver_path, None).unwrap();

        assert_eq!(hostname, "sliver-app.example.com");
        assert!(registry.contains(&hostname));
        assert!(registry.is_sliver_app(&hostname));

        // Verify app config
        let app_config = registry.get(&hostname).unwrap();
        assert!(app_config.sliver.is_some());
        assert!(app_config.sliver.as_ref().unwrap().contains("test.sliver"));
    }

    #[test]
    fn test_register_from_sliver_with_config() {
        let (_temp_dir, sliver_path, _archive) = create_test_sliver("configured.example.com");

        let base_config = AppConfig {
            hostname: "ignored.example.com".to_string(), // Will be overridden
            entrypoint: "".to_string(),
            sliver: None,
            env_vars: [("KEY".to_string(), "VALUE".to_string())].into_iter().collect(),
            limits: crate::config::AppLimits {
                memory_mb: 256,
                timeout_secs: 60,
                workers: 8,
                cpu_time_ms: 100,
                cpu_time_enabled: true,
            },
            vfs_backend: Default::default(),
            vfs_disk: None,
            vfs_s3: None,
        };

        let mut registry = AppRegistry::default();
        let hostname = registry
            .register_from_sliver(&sliver_path, Some(base_config))
            .unwrap();

        // Hostname should come from sliver, not base config
        assert_eq!(hostname, "configured.example.com");

        // But other settings should be preserved
        let app_config = registry.get(&hostname).unwrap();
        assert_eq!(app_config.env_vars.get("KEY"), Some(&"VALUE".to_string()));
        assert_eq!(app_config.limits.memory_mb, 256);
        assert_eq!(app_config.limits.workers, 8);
    }

    #[test]
    fn test_get_sliver_data() {
        let (_temp_dir, sliver_path, _archive) = create_test_sliver("data-test.example.com");

        let mut registry = AppRegistry::default();
        let hostname = registry.register_from_sliver(&sliver_path, None).unwrap();

        // Get sliver data
        let sliver_data = registry.get_sliver_data(&hostname);
        assert!(sliver_data.is_some());

        let data = sliver_data.unwrap();
        assert_eq!(data.metadata.hostname, "data-test.example.com");
        assert!(data.bytecode.is_none()); // source-only sliver has no bytecode
    }

    #[test]
    fn test_is_sliver_app() {
        let (_temp_dir, sliver_path, _archive) = create_test_sliver("sliver-only.example.com");

        let mut registry = AppRegistry::default();
        registry.register_from_sliver(&sliver_path, None).unwrap();

        assert!(registry.is_sliver_app("sliver-only.example.com"));
        assert!(!registry.is_sliver_app("not-a-sliver.example.com"));
    }

    #[test]
    fn test_register_from_sliver_missing_file() {
        let mut registry = AppRegistry::default();
        let result = registry.register_from_sliver(std::path::Path::new("/nonexistent.sliver"), None);

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to read sliver file"));
    }

    #[test]
    fn test_register_from_sliver_invalid_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let invalid_path = temp_dir.path().join("not-a-sliver.txt");
        std::fs::write(&invalid_path, "not valid sliver data").unwrap();

        let mut registry = AppRegistry::default();
        let result = registry.register_from_sliver(&invalid_path, None);

        assert!(result.is_err());
    }

    #[test]
    fn test_with_sliver_data() {
        let metadata = SliverMetadata::new("preloaded.example.com", "1.1.0");
        let unpacked = crate::sliver::UnpackedSliver::new(metadata, None, vec![]);

        let mut apps = HashMap::new();
        apps.insert(
            "preloaded.example.com".to_string(),
            create_test_config("preloaded.example.com"),
        );

        let mut sliver_data = HashMap::new();
        sliver_data.insert("preloaded.example.com".to_string(), unpacked);

        let registry = AppRegistry::with_sliver_data(apps, sliver_data);

        assert!(registry.is_sliver_app("preloaded.example.com"));
        assert!(registry.get_sliver_data("preloaded.example.com").is_some());
    }
}
