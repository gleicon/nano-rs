//! Hot-reload orchestration for configuration changes
//!
//! Provides functionality to reload configuration without downtime,
//! including graceful drain of in-flight requests.

use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Duration;

use crate::app::drain::RequestDrain;
use crate::app::registry::AppRegistry;
use crate::config::loader::load_config;

/// Configuration diff for tracking changes
#[derive(Debug, Clone)]
pub struct ConfigDiff {
    /// Apps to add
    pub added: Vec<String>,
    /// Apps to remove
    pub removed: Vec<String>,
    /// Apps that were modified
    pub modified: Vec<String>,
}

impl ConfigDiff {
    /// Create empty diff
    pub fn new() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
            modified: Vec::new(),
        }
    }

    /// Check if any changes exist
    pub fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty() || !self.modified.is_empty()
    }
}

impl Default for ConfigDiff {
    fn default() -> Self {
        Self::new()
    }
}

/// Reload configuration with graceful drain
pub async fn reload_config(
    config_path: &Path,
    current_registry: Arc<RwLock<AppRegistry>>,
    drain: &RequestDrain,
    drain_timeout: Duration,
) -> Result<(Arc<RwLock<AppRegistry>>, ConfigDiff), ReloadError> {
    // Load new configuration
    let new_config = load_config(config_path)
        .await
        .map_err(|e| ReloadError::ConfigLoadError(e.to_string()))?;

    // Create new registry from config
    let new_registry = Arc::new(AppRegistry::from_config(new_config));

    // Calculate diff
    let diff = {
        let current = current_registry.read().await;
        calculate_diff(&current, &new_registry)
    };

    if !diff.has_changes() {
        return Ok((current_registry, diff));
    }

    // Wait for graceful drain
    let drained = drain.await_complete(drain_timeout).await;
    if !drained {
        tracing::warn!(
            "Drain timeout reached with {} requests still active",
            drain.active_count()
        );
    }

    // Perform atomic swap
    let mut current = current_registry.write().await;
    *current = (*new_registry).clone();

    tracing::info!(
        "Config reloaded: {} added, {} removed, {} modified",
        diff.added.len(),
        diff.removed.len(),
        diff.modified.len()
    );

    Ok((Arc::clone(&current_registry), diff))
}

/// Calculate diff between current and new registry
fn calculate_diff(current: &AppRegistry, new: &AppRegistry) -> ConfigDiff {
    let mut diff = ConfigDiff::new();

    let current_hostnames: std::collections::HashSet<_> = current.all_hostnames().collect();
    let new_hostnames: std::collections::HashSet<_> = new.all_hostnames().collect();

    // Find added apps
    for hostname in &new_hostnames {
        if !current_hostnames.contains(hostname) {
            diff.added.push(hostname.clone());
        }
    }

    // Find removed apps
    for hostname in &current_hostnames {
        if !new_hostnames.contains(hostname) {
            diff.removed.push(hostname.clone());
        }
    }

    // Find modified apps
    for hostname in &new_hostnames {
        if current_hostnames.contains(hostname) {
            let current_app = current.get(hostname);
            let new_app = new.get(hostname);

            if current_app != new_app {
                diff.modified.push(hostname.clone());
            }
        }
    }

    diff
}

/// Reload error types
#[derive(Debug)]
pub enum ReloadError {
    ConfigLoadError(String),
    DrainTimeout,
}

impl std::fmt::Display for ReloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReloadError::ConfigLoadError(e) => write!(f, "Config load error: {}", e),
            ReloadError::DrainTimeout => write!(f, "Drain timeout"),
        }
    }
}

impl std::error::Error for ReloadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, AppLimits};
    use std::collections::HashMap;

    fn create_test_registry(hostnames: Vec<&str>) -> AppRegistry {
        let apps: HashMap<String, AppConfig> = hostnames
            .into_iter()
            .map(|h| {
                let app = AppConfig {
                    hostname: h.to_string(),
                    entrypoint: format!("./{}.js", h),
                    sliver: None,
                    compat: Default::default(),
                    env_vars: HashMap::new(),
                    limits: AppLimits::default(),
                    vfs_backend: Default::default(),
                    vfs_disk: None,
                    vfs_s3: None,
                };
                (h.to_string(), app)
            })
            .collect();

        AppRegistry::new(apps)
    }

    #[test]
    fn test_config_diff_has_changes() {
        let empty = ConfigDiff::new();
        assert!(!empty.has_changes());

        let with_changes = ConfigDiff {
            added: vec!["app1".to_string()],
            removed: Vec::new(),
            modified: Vec::new(),
        };
        assert!(with_changes.has_changes());
    }

    #[test]
    fn test_calculate_diff_add() {
        let current = create_test_registry(vec!["app1"]);
        let new = create_test_registry(vec!["app1", "app2"]);

        let diff = calculate_diff(&current, &new);

        assert_eq!(diff.added, vec!["app2"]);
        assert!(diff.removed.is_empty());
        assert!(diff.modified.is_empty());
    }

    #[test]
    fn test_calculate_diff_remove() {
        let current = create_test_registry(vec!["app1", "app2"]);
        let new = create_test_registry(vec!["app1"]);

        let diff = calculate_diff(&current, &new);

        assert!(diff.added.is_empty());
        assert_eq!(diff.removed, vec!["app2"]);
        assert!(diff.modified.is_empty());
    }
}
