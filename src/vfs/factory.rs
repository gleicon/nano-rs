//! Backend Factory
//!
//! Provides a factory for creating VFS backends based on configuration.
//! This allows dynamic backend selection at runtime based on app configuration.

use crate::config::{VfsBackendType, VfsDiskConfig, VfsS3Config};
use crate::vfs::types::{ResourceLimits, VfsError, VfsResult};
use crate::vfs::{DiskBackend, MemoryBackend, VfsBackendEnum};

/// Factory for creating VFS backends
///
/// Creates appropriate backend instances based on configuration type.
#[derive(Debug, Clone)]
pub struct BackendFactory;

impl BackendFactory {
    /// Create a new backend factory
    pub fn new() -> Self {
        Self
    }

    /// Create a VFS backend based on configuration
    ///
    /// # Arguments
    ///
    /// * `backend_type` - The type of backend to create
    /// * `disk_config` - Disk configuration (required when backend_type is Disk)
    /// * `s3_config` - S3 configuration (required when backend_type is S3)
    ///
    /// # Returns
    ///
    /// `Ok(VfsBackendEnum)` on success, `Err(VfsError)` if configuration is invalid
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use nano::vfs::factory::BackendFactory;
    /// use nano::config::VfsBackendType;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let factory = BackendFactory::new();
    /// let backend = factory.create_backend(
    ///     VfsBackendType::Memory,
    ///     None,
    ///     None,
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_backend(
        &self,
        backend_type: VfsBackendType,
        disk_config: Option<&VfsDiskConfig>,
        s3_config: Option<&VfsS3Config>,
    ) -> VfsResult<VfsBackendEnum> {
        match backend_type {
            VfsBackendType::Memory => Ok(VfsBackendEnum::memory(MemoryBackend::default())),
            VfsBackendType::Disk => {
                let config = disk_config.ok_or_else(|| VfsError::InvalidPath {
                    path: "vfs_disk".to_string(),
                    reason: "Disk backend requires vfs_disk configuration".to_string(),
                })?;

                let backend = DiskBackend::new(&config.base_path).await?;
                Ok(VfsBackendEnum::disk(backend))
            }
            VfsBackendType::S3 => {
                #[cfg(feature = "vfs-s3")]
                {
                    let config = s3_config.ok_or_else(|| VfsError::InvalidPath {
                        path: "vfs_s3".to_string(),
                        reason: "S3 backend requires vfs_s3 configuration".to_string(),
                    })?;

                    let s3_config = crate::vfs::s3::S3Config {
                        endpoint: config.endpoint.clone(),
                        bucket: config.bucket.clone(),
                        region: config.region.clone(),
                        access_key: config.access_key.clone(),
                        secret_key: config.secret_key.clone(),
                        prefix: config.prefix.clone(),
                        path_style: config.path_style,
                    };

                    let backend = crate::vfs::S3Backend::new(s3_config).await?;
                    Ok(VfsBackendEnum::s3(backend))
                }

                #[cfg(not(feature = "vfs-s3"))]
                {
                    let _ = s3_config; // Suppress unused warning
                    Err(VfsError::IoError(
                        "S3 backend requires vfs-s3 feature".to_string(),
                    ))
                }
            }
        }
    }

    /// Create a backend with custom resource limits
    pub async fn create_backend_with_limits(
        &self,
        backend_type: VfsBackendType,
        disk_config: Option<&VfsDiskConfig>,
        s3_config: Option<&VfsS3Config>,
        limits: ResourceLimits,
    ) -> VfsResult<VfsBackendEnum> {
        match backend_type {
            VfsBackendType::Memory => {
                Ok(VfsBackendEnum::memory(MemoryBackend::with_limits(limits)))
            }
            VfsBackendType::Disk => {
                let config = disk_config.ok_or_else(|| VfsError::InvalidPath {
                    path: "vfs_disk".to_string(),
                    reason: "Disk backend requires vfs_disk configuration".to_string(),
                })?;

                let backend = DiskBackend::with_limits(&config.base_path, limits).await?;
                Ok(VfsBackendEnum::disk(backend))
            }
            VfsBackendType::S3 => {
                #[cfg(feature = "vfs-s3")]
                {
                    let config = s3_config.ok_or_else(|| VfsError::InvalidPath {
                        path: "vfs_s3".to_string(),
                        reason: "S3 backend requires vfs_s3 configuration".to_string(),
                    })?;

                    let s3_config = crate::vfs::s3::S3Config {
                        endpoint: config.endpoint.clone(),
                        bucket: config.bucket.clone(),
                        region: config.region.clone(),
                        access_key: config.access_key.clone(),
                        secret_key: config.secret_key.clone(),
                        prefix: config.prefix.clone(),
                        path_style: config.path_style,
                    };

                    let backend = crate::vfs::S3Backend::with_limits(s3_config, limits).await?;
                    Ok(VfsBackendEnum::s3(backend))
                }

                #[cfg(not(feature = "vfs-s3"))]
                {
                    let _ = s3_config; // Suppress unused warning
                    Err(VfsError::IoError(
                        "S3 backend requires vfs-s3 feature".to_string(),
                    ))
                }
            }
        }
    }
}

impl Default for BackendFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_factory_creates_memory_backend() {
        let factory = BackendFactory::new();
        let backend = factory
            .create_backend(VfsBackendType::Memory, None, None)
            .await
            .unwrap();

        // Verify it's a memory backend by using it
        use crate::vfs::VfsPath;
        let path = VfsPath::new("test::file.txt").unwrap();
        backend.write(&path, b"test").await.unwrap();
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"test");
    }

    #[tokio::test]
    async fn test_factory_creates_disk_backend() {
        let temp_dir = TempDir::new().unwrap();
        let disk_config = VfsDiskConfig {
            base_path: temp_dir.path().to_str().unwrap().to_string(),
        };

        let factory = BackendFactory::new();
        let backend = factory
            .create_backend(VfsBackendType::Disk, Some(&disk_config), None)
            .await
            .unwrap();

        // Verify it works
        use crate::vfs::VfsPath;
        let path = VfsPath::new("test::file.txt").unwrap();
        backend.write(&path, b"disk test").await.unwrap();
        let content = backend.read(&path).await.unwrap();
        assert_eq!(content, b"disk test");
    }

    #[tokio::test]
    async fn test_factory_requires_disk_config() {
        let factory = BackendFactory::new();
        let result = factory
            .create_backend(VfsBackendType::Disk, None, None)
            .await;

        match result {
            Err(e) => assert!(
                e.to_string().contains("vfs_disk"),
                "Error should mention vfs_disk: {}",
                e
            ),
            Ok(_) => panic!("Expected error for missing disk config"),
        }
    }

    #[tokio::test]
    async fn test_factory_memory_with_limits() {
        let limits = ResourceLimits::test_limits();
        let factory = BackendFactory::new();
        let backend = factory
            .create_backend_with_limits(VfsBackendType::Memory, None, None, limits)
            .await
            .unwrap();

        // Verify limits are enforced
        use crate::vfs::VfsPath;
        let path = VfsPath::new("test::file.txt").unwrap();
        let large_content = vec![0u8; 200]; // Exceeds 100 byte limit
        let result = backend.write(&path, &large_content).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_factory_s3_without_feature_fails() {
        // When vfs-s3 feature is not enabled, creating S3 backend should fail
        let s3_config = VfsS3Config {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "test".to_string(),
            region: "us-east-1".to_string(),
            access_key: "test".to_string(),
            secret_key: "test".to_string(),
            prefix: None,
            path_style: true,
        };

        let factory = BackendFactory::new();
        let result = factory
            .create_backend(VfsBackendType::S3, None, Some(&s3_config))
            .await;

        // Without vfs-s3 feature, this should fail
        #[cfg(not(feature = "vfs-s3"))]
        assert!(result.is_err());
    }
}
