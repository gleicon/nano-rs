//! File watcher for configuration hot-reload
//!
//! Provides asynchronous file watching with debouncing and checksum-based
//! change detection to prevent duplicate events from rapid successive changes.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::time::sleep;

/// Events emitted by the config watcher
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigEvent {
    /// File was modified (with new checksum)
    Modified { path: PathBuf, checksum: String },
    /// File was deleted
    Deleted { path: PathBuf },
    /// File was created
    Created { path: PathBuf },
}

/// File watcher for configuration files
///
/// Watches a file or directory for changes with a configurable poll interval.
/// Uses SHA-256 checksums to detect actual content changes, not just mtime updates.
#[derive(Debug)]
pub struct ConfigWatcher {
    /// Path to watch
    watch_path: PathBuf,
    /// Poll interval (default 2 seconds)
    poll_interval: Duration,
    /// Last seen checksum
    last_checksum: Option<String>,
    /// Last seen modification time
    last_modified: Option<SystemTime>,
    /// Whether the file was previously present
    was_present: bool,
}

impl ConfigWatcher {
    /// Create a new config watcher for the given path
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the config file or directory to watch
    ///
    /// # Returns
    ///
    /// A new `ConfigWatcher` with default 2-second poll interval
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::config::watcher::ConfigWatcher;
    ///
    /// let watcher = ConfigWatcher::new("/etc/nano/apps.json");
    /// ```
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let was_present = path.exists();

        // If file already exists, compute initial checksum
        let (last_checksum, last_modified) = if was_present {
            match compute_checksum(&path) {
                Ok((checksum, modified)) => (Some(checksum), Some(modified)),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

        Self {
            watch_path: path,
            poll_interval: Duration::from_secs(2),
            last_checksum,
            last_modified,
            was_present,
        }
    }

    /// Configure a custom poll interval
    ///
    /// # Arguments
    ///
    /// * `secs` - Poll interval in seconds
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::config::watcher::ConfigWatcher;
    ///
    /// let watcher = ConfigWatcher::new("/etc/nano/apps.json")
    ///     .with_interval(5);
    /// ```
    pub fn with_interval(mut self, secs: u64) -> Self {
        self.poll_interval = Duration::from_secs(secs);
        self
    }

    /// Wait for a config change event (async)
    ///
    /// Polls the file until a change is detected, then returns the event.
    /// Uses checksum-based debouncing to avoid duplicate events.
    ///
    /// # Returns
    ///
    /// A `ConfigEvent` describing what changed
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use nano::config::watcher::{ConfigWatcher, ConfigEvent};
    ///
    /// # async fn example() {
    /// let mut watcher = ConfigWatcher::new("/etc/nano/apps.json");
    /// loop {
    ///     match watcher.watch().await {
    ///         ConfigEvent::Modified { path, .. } => println!("Config changed: {:?}", path),
    ///         ConfigEvent::Deleted { path } => println!("Config deleted: {:?}", path),
    ///         ConfigEvent::Created { path } => println!("Config created: {:?}", path),
    ///     }
    /// }
    /// # }
    /// ```
    pub async fn watch(&mut self) -> ConfigEvent {
        loop {
            if let Some(event) = self.check_now() {
                return event;
            }
            sleep(self.poll_interval).await;
        }
    }

    /// Non-blocking check for config changes
    ///
    /// Returns immediately with `Some(event)` if a change was detected,
    /// or `None` if no changes.
    ///
    /// # Returns
    ///
    /// `Some(ConfigEvent)` if changed, `None` if no change
    pub fn check_now(&mut self) -> Option<ConfigEvent> {
        let is_present = self.watch_path.exists();

        // Handle creation
        if is_present && !self.was_present {
            self.was_present = true;
            match compute_checksum(&self.watch_path) {
                Ok((checksum, modified)) => {
                    self.last_checksum = Some(checksum.clone());
                    self.last_modified = Some(modified);
                    return Some(ConfigEvent::Created {
                        path: self.watch_path.clone(),
                    });
                }
                Err(_) => {
                    // File was created but we can't read it yet
                    return Some(ConfigEvent::Created {
                        path: self.watch_path.clone(),
                    });
                }
            }
        }

        // Handle deletion
        if !is_present && self.was_present {
            self.was_present = false;
            self.last_checksum = None;
            self.last_modified = None;
            return Some(ConfigEvent::Deleted {
                path: self.watch_path.clone(),
            });
        }

        // No change in presence
        if !is_present {
            return None;
        }

        // Check for modification using checksum
        match compute_checksum(&self.watch_path) {
            Ok((checksum, modified)) => {
                // Check if checksum changed
                if self.last_checksum.as_ref() != Some(&checksum) {
                    self.last_checksum = Some(checksum.clone());
                    self.last_modified = Some(modified);
                    return Some(ConfigEvent::Modified {
                        path: self.watch_path.clone(),
                        checksum,
                    });
                }
            }
            Err(_) => {
                // Couldn't read file, might be mid-write
                // Don't emit an event, wait for next poll
            }
        }

        None
    }

    /// Get the watch path
    pub fn path(&self) -> &Path {
        &self.watch_path
    }

    /// Get the poll interval
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }
}

/// Compute SHA-256 checksum of file contents
///
/// Returns the hex-encoded checksum and modification time
fn compute_checksum(path: &Path) -> anyhow::Result<(String, SystemTime)> {
    use sha2::{Digest, Sha256};
    use std::fs;

    let content =
        fs::read(path).map_err(|e| anyhow::anyhow!("Failed to read file for checksum: {}", e))?;

    let mut hasher = Sha256::new();
    hasher.update(&content);
    let checksum = format!("{:x}", hasher.finalize());

    let metadata =
        fs::metadata(path).map_err(|e| anyhow::anyhow!("Failed to get file metadata: {}", e))?;
    let modified = metadata
        .modified()
        .map_err(|e| anyhow::anyhow!("Failed to get modification time: {}", e))?;

    Ok((checksum, modified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_watcher_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create initial file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        let watcher = ConfigWatcher::new(&config_path);
        assert_eq!(watcher.path(), config_path);
        assert_eq!(watcher.poll_interval(), Duration::from_secs(2));
        assert!(watcher.last_checksum.is_some());
    }

    #[tokio::test]
    async fn test_watcher_detects_modification() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create initial file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        let mut watcher = ConfigWatcher::new(&config_path);

        // Should not detect change immediately
        assert!(watcher.check_now().is_none());

        // Modify file
        tokio::time::sleep(Duration::from_millis(100)).await;
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{\"updated\": true}").unwrap();
        }

        // Should detect modification
        let event = watcher.check_now();
        assert!(event.is_some());
        assert!(matches!(event.unwrap(), ConfigEvent::Modified { .. }));
    }

    #[tokio::test]
    async fn test_watcher_detects_deletion() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create initial file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        let mut watcher = ConfigWatcher::new(&config_path);
        assert!(watcher.was_present);

        // Delete file
        std::fs::remove_file(&config_path).unwrap();

        // Should detect deletion
        let event = watcher.check_now();
        assert!(event.is_some());
        assert!(matches!(event.unwrap(), ConfigEvent::Deleted { .. }));
        assert!(!watcher.was_present);
    }

    #[tokio::test]
    async fn test_watcher_detects_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create watcher before file exists
        let mut watcher = ConfigWatcher::new(&config_path);
        assert!(!watcher.was_present);

        // Create file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        // Should detect creation
        let event = watcher.check_now();
        assert!(event.is_some());
        assert!(matches!(event.unwrap(), ConfigEvent::Created { .. }));
        assert!(watcher.was_present);
    }

    #[tokio::test]
    async fn test_watcher_debounces_duplicate_changes() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create initial file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        let mut watcher = ConfigWatcher::new(&config_path);

        // First check should not return anything (file exists with initial checksum)
        assert!(watcher.check_now().is_none());

        // Same content write (same checksum)
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        // Should not detect change (same checksum)
        assert!(watcher.check_now().is_none());
    }

    #[tokio::test]
    async fn test_watcher_watch_async() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create initial file
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"{}").unwrap();
        }

        let mut watcher = ConfigWatcher::new(&config_path);
        watcher.poll_interval = Duration::from_millis(100);

        // Spawn a task to modify the file after a short delay
        let path_clone = config_path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let mut file = std::fs::File::create(&path_clone).unwrap();
            file.write_all(b"{\"updated\": true}").unwrap();
        });

        // Watch should return the modification event
        let event = watcher.watch().await;
        assert!(matches!(event, ConfigEvent::Modified { .. }));
    }

    #[tokio::test]
    async fn test_watcher_custom_interval() {
        let watcher = ConfigWatcher::new("/tmp/test.json").with_interval(5);
        assert_eq!(watcher.poll_interval(), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_watcher_checksum_comparison() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        // Create file with content A
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"content A").unwrap();
        }

        let mut watcher = ConfigWatcher::new(&config_path);
        let initial_checksum = watcher.last_checksum.clone();

        // Different content
        {
            let mut file = std::fs::File::create(&config_path).unwrap();
            file.write_all(b"content B").unwrap();
        }

        let event = watcher.check_now();
        assert!(event.is_some());

        if let ConfigEvent::Modified { checksum, .. } = event.unwrap() {
            assert_ne!(Some(checksum), initial_checksum);
        } else {
            panic!("Expected Modified event");
        }
    }
}
