//! In-Memory VFS Backend
//!
//! Provides a fast, in-memory storage backend using DashMap for concurrent access.
//! This is the default backend for NANO's VFS and supports resource limiting.

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::vfs::types::{ResourceLimits, VfsError, VfsFile, VfsPath, VfsResult};
use crate::vfs::VfsBackend;

/// In-memory storage backend
///
/// Uses DashMap for lock-free concurrent access and maintains
/// atomic counters for resource limit tracking.
#[derive(Debug)]
pub struct MemoryBackend {
    /// Storage map: path -> file metadata and content
    storage: DashMap<String, VfsFile>,
    /// Resource limits for this backend
    limits: ResourceLimits,
    /// Current total bytes stored
    total_bytes: AtomicUsize,
    /// Current file count
    file_count: AtomicUsize,
}

impl MemoryBackend {
    /// Create a new MemoryBackend with default limits
    pub fn new() -> Self {
        Self::with_limits(ResourceLimits::default())
    }

    /// Create a new MemoryBackend with custom limits
    pub fn with_limits(limits: ResourceLimits) -> Self {
        Self {
            storage: DashMap::new(),
            limits,
            total_bytes: AtomicUsize::new(0),
            file_count: AtomicUsize::new(0),
        }
    }

    /// Clear all stored files
    pub fn clear(&self) {
        self.storage.clear();
        self.total_bytes.store(0, Ordering::SeqCst);
        self.file_count.store(0, Ordering::SeqCst);
    }

    /// Get the number of files stored
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Check if no files are stored
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Get current storage usage (file count, total bytes)
    pub fn current_usage(&self) -> (usize, usize) {
        (
            self.file_count.load(Ordering::SeqCst),
            self.total_bytes.load(Ordering::SeqCst),
        )
    }

    /// Get the resource limits
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// Check if we can write a file of the given size
    fn check_write_bounds(&self, _path: &VfsPath, content_len: usize, is_new: bool, old_size: usize) -> VfsResult<()> {
        let file_size_max = self.limits.file_size_bytes_max;
        let file_count_max = self.limits.files_count_max;
        let total_storage_max = self.limits.total_storage_bytes_max;
        let max_file_size = file_size_max as usize;
        let max_file_count = file_count_max as usize;
        let max_total_storage = total_storage_max as usize;

        if content_len > max_file_size {
            return Err(VfsError::QuotaExceeded {
                resource: "file_size".to_string(),
                limit: self.limits.file_size_bytes_max,
                current: content_len as u32,
            });
        }

        if is_new {
            let current_count = self.file_count.load(Ordering::SeqCst);
            if current_count >= max_file_count {
                return Err(VfsError::QuotaExceeded {
                    resource: "file_count".to_string(),
                    limit: self.limits.files_count_max,
                    current: current_count as u32,
                });
            }
        }

        let size_delta = content_len as i64 - old_size as i64;
        let current_total = self.total_bytes.load(Ordering::SeqCst) as i64;
        let new_total = (current_total + size_delta) as usize;

        if new_total > max_total_storage {
            return Err(VfsError::QuotaExceeded {
                resource: "total_storage".to_string(),
                limit: self.limits.total_storage_bytes_max,
                current: current_total as u32,
            });
        }

        Ok(())
    }

}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBackend {
    /// Get all stored files as (path, file) pairs for snapshot serialization
    ///
    /// This method is used by the sliver packer to capture the complete
    /// VFS state for snapshot creation.
    pub fn snapshot_entries(&self) -> Vec<(VfsPath, VfsFile)> {
        self.storage
            .iter()
            .filter_map(|entry| {
                let path_str = entry.key();
                match VfsPath::new(path_str) {
                    Ok(path) => {
                        let file = entry.value().clone();
                        Some((path, file))
                    }
                    Err(_) => None, // Skip invalid paths
                }
            })
            .collect()
    }

    /// Restore entries from a snapshot
    ///
    /// Clears existing data and populates from the given entries.
    /// Used by the sliver unpacker to restore VFS state.
    pub fn restore_from_snapshot(&self, entries: &[(VfsPath, VfsFile)]) {
        self.clear();
        
        let mut total_bytes: usize = 0;
        for (path, file) in entries {
            total_bytes += file.content.len();
            self.storage.insert(path.as_str().to_string(), file.clone());
        }
        
        self.file_count.store(entries.len(), Ordering::SeqCst);
        self.total_bytes.store(total_bytes, Ordering::SeqCst);
    }
}

#[async_trait]
impl VfsBackend for MemoryBackend {
    async fn read(&self, path: &VfsPath) -> VfsResult<Vec<u8>> {
        match self.storage.get(path.as_str()) {
            Some(entry) => Ok(entry.content.clone()),
            None => Err(VfsError::NotFound {
                path: path.to_string(),
            }),
        }
    }

    async fn write(&self, path: &VfsPath, content: &[u8]) -> VfsResult<()> {
        let content_len = content.len();

        // Check if this is a new file and get old size BEFORE checking limits
        let is_new = !self.storage.contains_key(path.as_str());
        let old_size = if is_new {
            0
        } else {
            self.storage
                .get(path.as_str())
                .map(|entry| entry.content.len())
                .unwrap_or(0)
        };

        // Check limits
        self.check_write_bounds(path, content_len, is_new, old_size)?;

        let now = std::time::SystemTime::now();

        let file = if is_new {
            VfsFile {
                content: content.to_vec(),
                created_at: now,
                modified_at: now,
                size: content_len,
            }
        } else {
            // Preserve creation time for existing files
            let existing = self.storage.get(path.as_str()).unwrap();
            VfsFile {
                content: content.to_vec(),
                created_at: existing.created_at,
                modified_at: now,
                size: content_len,
            }
        };

        // Store the file
        self.storage.insert(path.as_str().to_string(), file);

        // Update counters
        if is_new {
            self.file_count.fetch_add(1, Ordering::SeqCst);
        }
        let size_delta = content_len as i64 - old_size as i64;
        if size_delta > 0 {
            self.total_bytes.fetch_add(size_delta as usize, Ordering::SeqCst);
        } else if size_delta < 0 {
            self.total_bytes.fetch_sub((-size_delta) as usize, Ordering::SeqCst);
        }

        Ok(())
    }

    async fn exists(&self, path: &VfsPath) -> VfsResult<bool> {
        Ok(self.storage.contains_key(path.as_str()))
    }

    async fn delete(&self, path: &VfsPath) -> VfsResult<()> {
        match self.storage.remove(path.as_str()) {
            Some((_, file)) => {
                self.file_count.fetch_sub(1, Ordering::SeqCst);
                self.total_bytes.fetch_sub(file.size, Ordering::SeqCst);
                Ok(())
            }
            None => Err(VfsError::NotFound {
                path: path.to_string(),
            }),
        }
    }

    async fn metadata(&self, path: &VfsPath) -> VfsResult<VfsFile> {
        match self.storage.get(path.as_str()) {
            Some(entry) => Ok(entry.clone()),
            None => Err(VfsError::NotFound {
                path: path.to_string(),
            }),
        }
    }

    async fn list_dir(&self, path: &VfsPath) -> VfsResult<Vec<VfsPath>> {
        let prefix = path.as_str();
        let prefix_with_slash = if prefix.ends_with('/') {
            prefix.to_string()
        } else {
            format!("{}/", prefix)
        };

        let mut entries = std::collections::HashSet::new();

        for key in self.storage.iter() {
            let key_str = key.key();
            if key_str.starts_with(&prefix_with_slash) {
                // Get the remaining path after prefix
                let remaining = &key_str[prefix_with_slash.len()..];
                // Get first segment (immediate child)
                if let Some(slash_pos) = remaining.find('/') {
                    let child = &remaining[..slash_pos];
                    entries.insert(child.to_string());
                } else if !remaining.is_empty() {
                    // Direct file in this directory
                    entries.insert(remaining.to_string());
                }
            }
        }

        let paths: Vec<VfsPath> = entries
            .into_iter()
            .map(|name| {
                let full_path = format!("{}{}", prefix_with_slash, name);
                VfsPath::new(full_path).unwrap()
            })
            .collect();

        Ok(paths)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
