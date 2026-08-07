//! Disk VFS Backend
//!
//! Provides a filesystem-backed storage backend that persists VFS data to local disk.
//! Uses atomic writes (write to temp, rename) for data integrity.
//!
//! # Security
//!
//! - Path traversal prevention: All paths are validated before filesystem operations
//! - Namespace isolation: Each namespace gets its own subdirectory
//! - Atomic writes: Partial writes never leave corrupted files
//!
//! # Directory Structure
//!
//! ```text
//! {base_path}/
//!   {sanitized_namespace_1}/
//!     file1.txt
//!     subdir/
//!       file2.txt
//!   {sanitized_namespace_2}/
//!     ...
//! ```

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::vfs::types::{ResourceLimits, VfsError, VfsFile, VfsPath, VfsResult};
use crate::vfs::VfsBackend;

/// Filesystem-backed VFS storage
///
/// Persists files to local disk with namespace isolation and quota enforcement.
/// Uses atomic file operations to prevent corruption.
#[derive(Debug)]
pub struct DiskBackend {
    /// Root directory for all storage
    base_path: PathBuf,
    /// Resource limits for this backend
    limits: ResourceLimits,
    /// Current total bytes stored (across all namespaces)
    total_bytes: AtomicUsize,
    /// Current file count (across all namespaces)
    file_count: AtomicUsize,
}

impl DiskBackend {
    /// Create a new DiskBackend with the given base path
    ///
    /// # Arguments
    ///
    /// * `base_path` - Root directory for all storage. Will be created if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `VfsError::IoError` if the base directory cannot be created.
    pub async fn new(base_path: impl AsRef<Path>) -> VfsResult<Self> {
        let base_path = base_path.as_ref().to_path_buf();

        // Create base directory if it doesn't exist
        fs::create_dir_all(&base_path)
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to create base directory: {e}")))?;

        // Initialize counters by scanning existing files
        let (count, bytes) = Self::scan_storage(&base_path).await?;

        Ok(Self {
            base_path,
            limits: ResourceLimits::default(),
            total_bytes: AtomicUsize::new(bytes),
            file_count: AtomicUsize::new(count),
        })
    }

    /// Create a new DiskBackend with custom resource limits
    pub async fn with_limits(
        base_path: impl AsRef<Path>,
        limits: ResourceLimits,
    ) -> VfsResult<Self> {
        let mut backend = Self::new(base_path).await?;
        backend.limits = limits;
        Ok(backend)
    }

    /// Get the base path
    pub fn base_path(&self) -> &Path {
        &self.base_path
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

    /// Sanitize a namespace for use as a directory name
    ///
    /// Replaces characters that are problematic for filesystems:
    /// - `::` → `__` (namespace separator)
    /// - `/` → `_` (path separator, shouldn't happen but be safe)
    /// - `\` → `_` (Windows path separator)
    fn sanitize_namespace(namespace: &str) -> String {
        namespace
            .replace("::", "__")
            .replace('/', "_")
            .replace('\\', "_")
    }

    /// Convert a VfsPath to a filesystem path
    ///
    /// Format: `{base}/{sanitized_namespace}/{path}` or `{base}/{path}` if no namespace
    fn to_filesystem_path(&self, path: &VfsPath) -> PathBuf {
        let path_str = path.as_str();

        // Split namespace from path (format: "namespace::path" or just "path" if no namespace)
        let (namespace, subpath) = match path_str.find("::") {
            Some(idx) => (&path_str[..idx], &path_str[idx + 2..]),
            None => ("", path_str), // Empty namespace means no namespace prefix
        };

        let mut fs_path = if namespace.is_empty() {
            // No namespace - path maps directly to base_path
            self.base_path.clone()
        } else {
            // Has namespace - include sanitized namespace in path
            let sanitized_ns = Self::sanitize_namespace(namespace);
            self.base_path.join(sanitized_ns)
        };

        // Append the subpath components
        if !subpath.is_empty() {
            fs_path = fs_path.join(subpath);
        }

        fs_path
    }

    /// Ensure parent directory exists
    async fn ensure_parent_dir(path: &Path) -> VfsResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to create directory: {e}")))?;
        }
        Ok(())
    }

    /// Perform atomic write (write to temp, then rename)
    async fn atomic_write(path: &Path, content: &[u8]) -> VfsResult<()> {
        // Create temp file path
        let temp_path = path.with_extension(format!(
            "tmp.{}.{}",
            std::process::id(),
            rand::random::<u32>()
        ));

        // Write to temp file
        let result = async {
            let mut file = fs::File::create(&temp_path)
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to create temp file: {e}")))?;

            file.write_all(content)
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to write temp file: {e}")))?;

            file.sync_all()
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to sync temp file: {e}")))?;

            drop(file);

            // Atomic rename
            fs::rename(&temp_path, path)
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to rename temp file: {e}")))?;

            Ok(())
        }
        .await;

        // Clean up temp file on error
        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }

        result
    }

    /// Scan storage and count files/total bytes
    async fn scan_storage(base_path: &Path) -> VfsResult<(usize, usize)> {
        let mut count = 0;
        let mut total_bytes = 0usize;

        let mut entries = fs::read_dir(base_path)
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to read base directory: {e}")))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to read directory entry: {e}")))?
        {
            let path = entry.path();
            let meta = entry
                .metadata()
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to read metadata: {e}")))?;
            if meta.is_dir() {
                let (subcount, subbytes) = Self::scan_namespace(&path).await?;
                count += subcount;
                total_bytes = total_bytes.saturating_add(subbytes);
            }
        }

        Ok((count, total_bytes))
    }

    /// Scan a namespace directory
    async fn scan_namespace(ns_path: &Path) -> VfsResult<(usize, usize)> {
        let mut count = 0;
        let mut total_bytes = 0usize;

        let mut dirs = vec![ns_path.to_path_buf()];

        while let Some(dir) = dirs.pop() {
            let mut entries = fs::read_dir(&dir)
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to read directory: {e}")))?;

            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| VfsError::IoError(format!("Failed to read entry: {e}")))?
            {
                let path = entry.path();
                let metadata = entry
                    .metadata()
                    .await
                    .map_err(|e| VfsError::IoError(format!("Failed to read metadata: {e}")))?;

                if metadata.is_file() {
                    count += 1;
                    total_bytes = total_bytes.saturating_add(metadata.len() as usize);
                } else if metadata.is_dir() {
                    dirs.push(path);
                }
            }
        }

        Ok((count, total_bytes))
    }

    /// Check write bounds against configured resource constants
    fn check_write_bounds(
        &self,
        content_len: usize,
        is_new: bool,
        old_size: usize,
    ) -> VfsResult<()> {
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

#[async_trait]
impl VfsBackend for DiskBackend {
    async fn read(&self, path: &VfsPath) -> VfsResult<Vec<u8>> {
        let fs_path = self.to_filesystem_path(path);

        // Check if file exists and is readable
        match fs::read(&fs_path).await {
            Ok(content) => Ok(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(VfsError::NotFound {
                path: path.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(VfsError::PermissionDenied {
                    path: path.to_string(),
                })
            }
            Err(e) => Err(VfsError::IoError(format!("Failed to read file: {e}"))),
        }
    }

    async fn write(&self, path: &VfsPath, content: &[u8]) -> VfsResult<()> {
        let content_len = content.len();
        let fs_path = self.to_filesystem_path(path);

        // Check if this is a new file and get old size
        let (is_new, old_size) = match fs::metadata(&fs_path).await {
            Ok(meta) => (false, meta.len() as usize),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, 0),
            Err(_) => (false, 0),
        };

        // Check bounds
        self.check_write_bounds(content_len, is_new, old_size)?;

        // Ensure parent directory exists
        Self::ensure_parent_dir(&fs_path).await?;

        // Atomic write
        Self::atomic_write(&fs_path, content).await?;

        // Update counters
        if is_new {
            self.file_count.fetch_add(1, Ordering::SeqCst);
            self.total_bytes.fetch_add(content_len, Ordering::SeqCst);
        } else {
            let delta = content_len as i64 - old_size as i64;
            if delta > 0 {
                self.total_bytes.fetch_add(delta as usize, Ordering::SeqCst);
            } else if delta < 0 {
                self.total_bytes
                    .fetch_sub((-delta) as usize, Ordering::SeqCst);
            }
        }

        Ok(())
    }

    async fn exists(&self, path: &VfsPath) -> VfsResult<bool> {
        let fs_path = self.to_filesystem_path(path);
        match fs::metadata(&fs_path).await {
            Ok(meta) => Ok(meta.is_file()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(VfsError::IoError(format!("Failed to check existence: {e}"))),
        }
    }

    async fn delete(&self, path: &VfsPath) -> VfsResult<()> {
        let fs_path = self.to_filesystem_path(path);

        // Get file size before deletion for counter update
        let size = match fs::metadata(&fs_path).await {
            Ok(meta) if meta.is_file() => meta.len() as usize,
            Ok(_) => {
                return Err(VfsError::NotFound {
                    path: path.to_string(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VfsError::NotFound {
                    path: path.to_string(),
                });
            }
            Err(e) => return Err(VfsError::IoError(format!("Failed to get metadata: {e}"))),
        };

        // Delete the file
        fs::remove_file(&fs_path)
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to delete file: {e}")))?;

        // Update counters
        self.file_count.fetch_sub(1, Ordering::SeqCst);
        self.total_bytes.fetch_sub(size, Ordering::SeqCst);

        Ok(())
    }

    async fn metadata(&self, path: &VfsPath) -> VfsResult<VfsFile> {
        let fs_path = self.to_filesystem_path(path);

        let meta = fs::metadata(&fs_path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VfsError::NotFound {
                path: path.to_string(),
            },
            std::io::ErrorKind::PermissionDenied => VfsError::PermissionDenied {
                path: path.to_string(),
            },
            _ => VfsError::IoError(format!("Failed to get metadata: {e}")),
        })?;

        if !meta.is_file() {
            return Err(VfsError::NotFound {
                path: path.to_string(),
            });
        }

        // Read content for VfsFile
        let content = fs::read(&fs_path)
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to read file: {e}")))?;

        let created_at = meta
            .created()
            .map_err(|e| VfsError::IoError(format!("Failed to get created time: {e}")))?;

        let modified_at = meta
            .modified()
            .map_err(|e| VfsError::IoError(format!("Failed to get modified time: {e}")))?;

        Ok(VfsFile {
            content,
            created_at,
            modified_at,
            size: meta.len() as usize,
        })
    }

    async fn list_dir(&self, path: &VfsPath) -> VfsResult<Vec<VfsPath>> {
        let fs_path = self.to_filesystem_path(path);

        let mut entries = fs::read_dir(&fs_path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VfsError::NotFound {
                path: path.to_string(),
            },
            std::io::ErrorKind::PermissionDenied => VfsError::PermissionDenied {
                path: path.to_string(),
            },
            _ => VfsError::IoError(format!("Failed to read directory: {e}")),
        })?;

        let mut paths = Vec::new();

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| VfsError::IoError(format!("Failed to read directory entry: {e}")))?
        {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            // Skip hidden files and special entries
            if name.starts_with('.') {
                continue;
            }

            let child_path = if path.as_str().ends_with('/') {
                format!("{}{}", path.as_str(), name)
            } else {
                format!("{}/{}", path.as_str(), name)
            };

            paths.push(
                VfsPath::new(&child_path).map_err(|e| VfsError::InvalidPath {
                    path: child_path.clone(),
                    reason: format!("Invalid path generated: {e}"),
                })?,
            );
        }

        Ok(paths)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Required for random temp file suffixes

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_backend() -> (DiskBackend, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let backend = DiskBackend::new(temp_dir.path()).await.unwrap();
        (backend, temp_dir)
    }

    #[tokio::test]
    async fn test_disk_backend_basic_ops() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::test.txt").unwrap();

        // Write
        backend.write(&path, b"hello world").await.unwrap();

        // Exists
        assert!(backend.exists(&path).await.unwrap());

        // Read
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"hello world");

        // Delete
        backend.delete(&path).await.unwrap();
        assert!(!backend.exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn test_disk_backend_namespace_isolation() {
        let (backend, _temp) = create_test_backend().await;

        let path1 = VfsPath::new("app1::secret.txt").unwrap();
        let path2 = VfsPath::new("app2::secret.txt").unwrap();

        // Write to both namespaces
        backend.write(&path1, b"app1 data").await.unwrap();
        backend.write(&path2, b"app2 data").await.unwrap();

        // Verify isolation
        assert_eq!(backend.read(&path1).await.unwrap(), b"app1 data");
        assert_eq!(backend.read(&path2).await.unwrap(), b"app2 data");
    }

    #[tokio::test]
    async fn test_disk_backend_persistence() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().to_path_buf();

        // Create backend and write file
        {
            let backend = DiskBackend::new(&base_path).await.unwrap();
            let path = VfsPath::new("app1::persist.txt").unwrap();
            backend.write(&path, b"persistent data").await.unwrap();
        }

        // Create new backend instance and verify data persists
        {
            let backend = DiskBackend::new(&base_path).await.unwrap();
            let path = VfsPath::new("app1::persist.txt").unwrap();
            let content = backend.read(&path).await.unwrap();
            assert_eq!(content, b"persistent data");
        }
    }

    #[tokio::test]
    async fn test_disk_backend_not_found() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::nonexistent.txt").unwrap();

        let result = backend.read(&path).await;
        assert!(matches!(result, Err(VfsError::NotFound { .. })));
        assert_eq!(result.unwrap_err().code(), "ENOENT");
    }

    #[tokio::test]
    async fn test_disk_backend_quota_file_size() {
        let temp_dir = TempDir::new().unwrap();
        let limits = ResourceLimits::test_limits(); // 100 byte limit
        let backend = DiskBackend::with_limits(temp_dir.path(), limits)
            .await
            .unwrap();

        let path = VfsPath::new("app1::large.txt").unwrap();
        let content = vec![0u8; 200]; // 200 bytes > 100 limit

        let result = backend.write(&path, &content).await;
        assert!(matches!(result, Err(VfsError::QuotaExceeded { .. })));
    }

    #[tokio::test]
    async fn test_disk_backend_quota_file_count() {
        let temp_dir = TempDir::new().unwrap();
        let limits = ResourceLimits::test_limits(); // 5 file limit
        let backend = DiskBackend::with_limits(temp_dir.path(), limits)
            .await
            .unwrap();

        // Create 5 files (the limit)
        for i in 0..5 {
            let path = VfsPath::new(&format!("app1::file{}.txt", i)).unwrap();
            backend.write(&path, b"x").await.unwrap();
        }

        // Try to create 6th file
        let path = VfsPath::new("app1::file6.txt").unwrap();
        let result = backend.write(&path, b"x").await;
        assert!(matches!(result, Err(VfsError::QuotaExceeded { .. })));
    }

    #[tokio::test]
    async fn test_disk_backend_deeply_nested_paths() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::a/b/c/d/e/deep.txt").unwrap();

        backend.write(&path, b"deep content").await.unwrap();
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"deep content");
    }

    #[tokio::test]
    async fn test_disk_backend_unicode_paths() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::文件/日本語/emoji_🎉.txt").unwrap();

        backend.write(&path, b"unicode content").await.unwrap();
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"unicode content");
    }

    #[tokio::test]
    async fn test_disk_backend_metadata() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::meta.txt").unwrap();

        backend.write(&path, b"metadata test").await.unwrap();

        let meta = backend.metadata(&path).await.unwrap();
        assert_eq!(meta.size, 13);
        assert_eq!(meta.content, b"metadata test");
    }

    #[tokio::test]
    async fn test_disk_backend_update_existing() {
        let (backend, _temp) = create_test_backend().await;
        let path = VfsPath::new("app1::update.txt").unwrap();

        // Create file
        backend.write(&path, b"original").await.unwrap();

        // Update file
        backend.write(&path, b"updated content").await.unwrap();

        // Verify new content
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"updated content");
    }

    #[tokio::test]
    async fn test_disk_backend_sanitize_namespace() {
        // Test various namespace sanitizations
        assert_eq!(
            DiskBackend::sanitize_namespace("api.example.com"),
            "api.example.com"
        );
        assert_eq!(
            DiskBackend::sanitize_namespace("api::example::com"),
            "api__example__com"
        );
        assert_eq!(
            DiskBackend::sanitize_namespace("path/with/slash"),
            "path_with_slash"
        );
        assert_eq!(
            DiskBackend::sanitize_namespace("path\\with\\backslash"),
            "path_with_backslash"
        );
    }
}
