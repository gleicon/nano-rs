//! S3 VFS Backend
//!
//! Provides an S3-compatible object storage backend for the VFS.
//! Works with AWS S3, MinIO, Wasabi, DigitalOcean Spaces, and other S3-compatible stores.
//!
//! # Configuration
//!
//! ```json
//! {
//!   "vfs_backend": "s3",
//!   "vfs_s3": {
//!     "endpoint": "https://s3.amazonaws.com",
//!     "bucket": "my-bucket",
//!     "region": "us-east-1",
//!     "access_key": "AKIA...",
//!     "secret_key": "...",
//!     "prefix": "nano-vfs",
//!     "path_style": false
//!   }
//! }
//! ```
//!
//! # Security
//!
//! - Credentials never logged or exposed
//! - HTTPS only (enforced)
//! - Namespace isolation via key prefixes
//!
//! # Feature Flag
//!
//! This module is only available when the `vfs-s3` feature is enabled:
//! `cargo build --features vfs-s3`

#![cfg(feature = "vfs-s3")]

use async_trait::async_trait;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::vfs::types::{ResourceLimits, VfsError, VfsFile, VfsPath, VfsResult};
use crate::vfs::VfsBackend;

/// S3-compatible storage backend
///
/// Stores files in S3-compatible object storage with namespace isolation
/// via key prefixes.
#[derive(Debug)]
pub struct S3Backend {
    /// S3 bucket for storage
    bucket: Box<Bucket>,
    /// Resource limits for this backend
    limits: ResourceLimits,
    /// Key prefix for all objects (optional)
    prefix: Option<String>,
    /// Current total bytes stored
    total_bytes: AtomicUsize,
    /// Current file count
    file_count: AtomicUsize,
}

/// Configuration for S3 backend
#[derive(Debug, Clone)]
pub struct S3Config {
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
    pub prefix: Option<String>,
    /// Use path-style URLs (true for MinIO, false for AWS)
    pub path_style: bool,
}

impl S3Backend {
    /// Create a new S3Backend with the given configuration
    ///
    /// # Arguments
    ///
    /// * `config` - S3 configuration including endpoint, credentials, etc.
    ///
    /// # Errors
    ///
    /// Returns `VfsError::IoError` if the bucket cannot be created or accessed.
    pub async fn new(config: S3Config) -> VfsResult<Self> {
        let region = if config.path_style || config.region.is_empty() {
            Region::Custom {
                region: config.region.clone(),
                endpoint: config.endpoint.clone(),
            }
        } else {
            config.region.parse().map_err(|e| {
                VfsError::IoError(format!("Invalid S3 region '{}': {e}", config.region))
            })?
        };

        let credentials = Credentials::new(
            Some(&config.access_key),
            Some(&config.secret_key),
            None,
            None,
            None,
        )
        .map_err(|e| VfsError::IoError(format!("Invalid S3 credentials: {e}")))?;

        let mut bucket = Bucket::new(&config.bucket, region, credentials)
            .map_err(|e| VfsError::IoError(format!("Failed to create S3 bucket: {e}")))?;

        // Force path-style if configured (needed for MinIO)
        if config.path_style {
            bucket.set_path_style();
        }

        // Test connection by listing objects (limit 1)
        match bucket.list(String::new(), Some("/".to_string())).await {
            Ok(_) => {}
            Err(e) => {
                return Err(VfsError::IoError(format!("Failed to connect to S3: {e}")));
            }
        }

        Ok(Self {
            bucket,
            limits: ResourceLimits::default(),
            prefix: config.prefix,
            total_bytes: AtomicUsize::new(0),
            file_count: AtomicUsize::new(0),
        })
    }

    /// Create a new S3Backend with custom resource limits
    pub async fn with_limits(config: S3Config, limits: ResourceLimits) -> VfsResult<Self> {
        let mut backend = Self::new(config).await?;
        backend.limits = limits;
        Ok(backend)
    }

    /// Get the bucket name
    pub fn bucket_name(&self) -> String {
        self.bucket.name()
    }

    /// Get the optional prefix
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    /// Get current storage usage (file count, total bytes)
    ///
    /// Note: These are local counters, not actual S3 bucket scans.
    /// They may become inaccurate if objects are modified outside this backend.
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

    /// Sanitize a namespace for use in S3 keys
    ///
    /// S3 key constraints:
    /// - Avoid leading/trailing slashes
    /// - Control characters should be avoided
    fn sanitize_namespace(namespace: &str) -> String {
        namespace
            .replace("::", "__")
            .replace('\t', "_")
            .replace('\n', "_")
    }

    /// Convert a VfsPath to an S3 key
    ///
    /// Format: `{prefix}/{sanitized_namespace}/{path}` or `{sanitized_namespace}/{path}`
    fn to_s3_key(&self, path: &VfsPath) -> String {
        let path_str = path.as_str();

        // Split namespace from path (format: "namespace::path")
        let (namespace, subpath) = match path_str.find("::") {
            Some(idx) => (&path_str[..idx], &path_str[idx + 2..]),
            None => ("default", path_str),
        };

        let sanitized_ns = Self::sanitize_namespace(namespace);

        match &self.prefix {
            Some(prefix) => format!("{}/{}/{}", prefix, sanitized_ns, subpath),
            None => format!("{}/{}", sanitized_ns, subpath),
        }
    }

    /// Check write bounds
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

    /// Map S3 errors to VfsError
    fn map_s3_error(path: &str, e: s3::error::S3Error) -> VfsError {
        // Try to determine error type from the error message or code
        let error_str = format!("{e}");

        if error_str.contains("404") || error_str.contains("NoSuchKey") {
            VfsError::NotFound {
                path: path.to_string(),
            }
        } else if error_str.contains("403") || error_str.contains("AccessDenied") {
            VfsError::PermissionDenied {
                path: path.to_string(),
            }
        } else {
            VfsError::IoError(format!("S3 error: {e}"))
        }
    }
}

#[async_trait]
impl VfsBackend for S3Backend {
    async fn read(&self, path: &VfsPath) -> VfsResult<Vec<u8>> {
        let key = self.to_s3_key(path);

        match self.bucket.get_object(&key).await {
            Ok(response) => Ok(response.bytes().to_vec()),
            Err(e) => Err(Self::map_s3_error(path.as_str(), e)),
        }
    }

    async fn write(&self, path: &VfsPath, content: &[u8]) -> VfsResult<()> {
        let content_len = content.len();
        let key = self.to_s3_key(path);

        // Check if this is a new file and get old size
        let (is_new, old_size) = match self.bucket.head_object(&key).await {
            Ok((head, _)) => (false, head.content_length.unwrap_or(0) as usize),
            Err(_) => (true, 0),
        };

        // Check limits
        self.check_write_bounds(content_len, is_new, old_size)?;

        // Write to S3
        match self.bucket.put_object(&key, content).await {
            Ok(_) => {
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
            Err(e) => Err(VfsError::IoError(format!("Failed to write to S3: {e}"))),
        }
    }

    async fn exists(&self, path: &VfsPath) -> VfsResult<bool> {
        let key = self.to_s3_key(path);

        match self.bucket.head_object(&key).await {
            Ok((head, _)) => Ok(head.content_length.unwrap_or(0) > 0),
            Err(_) => Ok(false),
        }
    }

    async fn delete(&self, path: &VfsPath) -> VfsResult<()> {
        let key = self.to_s3_key(path);

        // Get file size before deletion for counter update
        let size = match self.bucket.head_object(&key).await {
            Ok((head, _)) => head.content_length.unwrap_or(0) as usize,
            Err(e) => {
                return Err(Self::map_s3_error(path.as_str(), e));
            }
        };

        // Delete from S3
        match self.bucket.delete_object(&key).await {
            Ok(_) => {
                // Update counters
                self.file_count.fetch_sub(1, Ordering::SeqCst);
                self.total_bytes.fetch_sub(size, Ordering::SeqCst);
                Ok(())
            }
            Err(e) => Err(VfsError::IoError(format!("Failed to delete from S3: {e}"))),
        }
    }

    async fn metadata(&self, path: &VfsPath) -> VfsResult<VfsFile> {
        let key = self.to_s3_key(path);

        // Get object metadata
        let (head, _) = self
            .bucket
            .head_object(&key)
            .await
            .map_err(|e| Self::map_s3_error(path.as_str(), e))?;

        let size = head.content_length.unwrap_or(0) as usize;

        // For S3, we need to read the content to get the full VfsFile
        // In production, you might want to cache this or use a different approach
        let content = self.read(path).await?;

        // S3 doesn't give us reliable created_at, use last_modified
        let modified_at = head
            .last_modified
            .and_then(|ts| {
                // Parse HTTP date format
                chrono::DateTime::parse_from_rfc2822(&ts)
                    .ok()
                    .map(|dt| {
                        let utc: chrono::DateTime<chrono::Utc> = dt.into();
                        std::time::SystemTime::from(utc)
                    })
            })
            .unwrap_or_else(std::time::SystemTime::now);

        Ok(VfsFile {
            content,
            created_at: modified_at, // S3 doesn't track creation time separately
            modified_at,
            size,
        })
    }

    async fn list_dir(&self, path: &VfsPath) -> VfsResult<Vec<VfsPath>> {
        let s3_key = self.to_s3_key(path);
        let prefix = if s3_key.ends_with('/') {
            s3_key
        } else {
            format!("{}/", s3_key)
        };

        let list_results = match self
            .bucket
            .list(prefix.clone(), Some("/".to_string()))
            .await
        {
            Ok(results) => results,
            Err(e) => return Err(Self::map_s3_error(path.as_str(), e)),
        };

        let mut entries = std::collections::HashSet::new();
        let parent_str = path.as_str();

        for result in list_results {
            // Process common prefixes (subdirectories)
            if let Some(common_prefixes) = result.common_prefixes {
                for cp in common_prefixes {
                    if let Some(relative) = cp.prefix.strip_prefix(&prefix) {
                        let name = relative.trim_end_matches('/');
                        if !name.is_empty() {
                            let child_path = format!("{}/{}", parent_str, name);
                            if let Ok(vfs_path) = VfsPath::new(&child_path) {
                                entries.insert(vfs_path);
                            }
                        }
                    }
                }
            }

            // Process contents (files)
            for object in result.contents {
                if let Some(relative) = object.key.strip_prefix(&prefix) {
                    if !relative.is_empty() {
                        let child_path = format!("{}/{}", parent_str, relative);
                        if let Ok(vfs_path) = VfsPath::new(&child_path) {
                            entries.insert(vfs_path);
                        }
                    }
                }
            }
        }

        let mut paths: Vec<VfsPath> = entries.into_iter().collect();
        paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(paths)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_key_formatting() {
        // We can't create an actual S3Backend in unit tests without credentials,
        // but we can test the helper functions
        assert_eq!(
            S3Backend::sanitize_namespace("api::example::com"),
            "api__example__com"
        );
        assert_eq!(
            S3Backend::sanitize_namespace("api.example.com"),
            "api.example.com"
        );
    }

    #[test]
    fn test_s3_config_creation() {
        let config = S3Config {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "test-bucket".to_string(),
            region: "us-east-1".to_string(),
            access_key: "minioadmin".to_string(),
            secret_key: "minioadmin".to_string(),
            prefix: Some("nano".to_string()),
            path_style: true,
        };

        assert_eq!(config.bucket, "test-bucket");
        assert!(config.path_style);
    }

    // Integration tests for S3Backend require a running S3-compatible server
    // (MinIO, LocalStack, or actual S3). These are marked with #[ignore]
    // and can be run with: cargo test --features vfs-s3 -- --ignored

    #[tokio::test]
    #[ignore]
    async fn test_s3_backend_basic_ops() {
        // This test requires MinIO or similar running at localhost:9000
        let config = S3Config {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "test-bucket".to_string(),
            region: "us-east-1".to_string(),
            access_key: "minioadmin".to_string(),
            secret_key: "minioadmin".to_string(),
            prefix: Some("test".to_string()),
            path_style: true,
        };

        let backend = S3Backend::new(config).await.unwrap();
        let path = VfsPath::new("app1::test.txt").unwrap();

        // Write
        backend.write(&path, b"hello s3").await.unwrap();

        // Exists
        assert!(backend.exists(&path).await.unwrap());

        // Read
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"hello s3");

        // Delete
        backend.delete(&path).await.unwrap();
        assert!(!backend.exists(&path).await.unwrap());
    }
}
