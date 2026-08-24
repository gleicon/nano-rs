//! VFS Directory Loader
//!
//! Provides functionality to load entire directory trees into the VFS at once.
//! This is essential for serving static assets from multi-file applications.
//!
//! # Features
//!
//! - Recursive directory traversal
//! - Preserves directory structure in VFS paths
//! - Handles binary files (images, fonts, etc.)
//! - Graceful handling of empty directories
//! - Symlink detection and warning
//! - Performance optimized for bulk loading

use crate::vfs::{IsolateVfs, VfsError};
use std::path::Path;
use std::pin::Pin;
use tokio::fs;

/// Load all files from a directory recursively into VFS
///
/// This function traverses the source directory and loads all files into the VFS,
/// preserving the directory structure under the specified mount point.
///
/// # Arguments
///
/// * `vfs` - The IsolateVfs to load files into
/// * `source_dir` - Path to the source directory on disk
/// * `mount_point` - VFS path where files will be mounted (e.g., "/dist")
///
/// # Returns
///
/// Returns the number of files loaded on success, or a VfsError on failure.
///
/// # Examples
///
/// ```rust,no_run
/// use nano::vfs::{IsolateVfs, VfsNamespace, MemoryBackend, VfsBackendEnum};
/// use nano::vfs::loader::load_directory_to_vfs;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let vfs = IsolateVfs::new(
///     VfsNamespace::from_hostname("example.com"),
///     VfsBackendEnum::memory(MemoryBackend::default())
/// );
///
/// let count = load_directory_to_vfs(&vfs, "./dist", "/dist").await?;
/// println!("Loaded {} files into VFS", count);
/// # Ok(())
/// # }
/// ```
pub async fn load_directory_to_vfs(
    vfs: &IsolateVfs,
    source_dir: &str,
    mount_point: &str,
) -> Result<usize, VfsError> {
    // Use Box::pin to avoid infinite-sized recursive async fn
    load_directory_to_vfs_inner(
        Box::pin(vfs.clone()),
        source_dir.to_string(),
        mount_point.to_string(),
    )
    .await
}

/// Internal implementation that uses boxed future to handle recursion
async fn load_directory_to_vfs_inner(
    vfs: Pin<Box<IsolateVfs>>,
    source_dir: String,
    mount_point: String,
) -> Result<usize, VfsError> {
    let mut count = 0;
    let source_path = Path::new(&source_dir);

    // Verify source directory exists
    if !source_path.exists() {
        return Err(VfsError::InvalidPath {
            path: source_dir.to_string(),
            reason: "Source directory does not exist".to_string(),
        });
    }

    if !source_path.is_dir() {
        return Err(VfsError::InvalidPath {
            path: source_dir.to_string(),
            reason: "Source path is not a directory".to_string(),
        });
    }

    // Read directory entries
    let mut entries = fs::read_dir(source_path)
        .await
        .map_err(|e| VfsError::IoError(format!("Failed to read directory: {}", e)))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| VfsError::IoError(format!("Failed to read directory entry: {}", e)))?
    {
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Build VFS path preserving directory structure
        let vfs_path = if mount_point.ends_with('/') {
            format!("{}{}", mount_point, file_name_str)
        } else if mount_point == "/" {
            format!("/{}", file_name_str)
        } else {
            format!("{}/{}", mount_point, file_name_str)
        };

        // Handle symlinks - skip with warning
        if path.is_symlink() {
            tracing::warn!("Skipping symlink: {}", path.display());
            continue;
        }

        if path.is_dir() {
            // Recursively load subdirectory using boxed future
            let sub_path = path.to_str().ok_or_else(|| VfsError::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "Invalid UTF-8 in path".to_string(),
            })?;

            // Box the recursive call to avoid infinite type size
            let sub_count = Box::pin(load_directory_to_vfs_inner(
                vfs.clone(),
                sub_path.to_string(),
                vfs_path,
            ))
            .await?;
            count += sub_count;
        } else if path.is_file() {
            // Load file as binary (supports both text and binary files)
            let content = fs::read(&path).await.map_err(|e| {
                VfsError::IoError(format!("Failed to read file '{}': {}", path.display(), e))
            })?;

            vfs.write(&vfs_path, &content).await?;
            count += 1;

            tracing::debug!(
                "Loaded file into VFS: {} -> {} ({} bytes)",
                path.display(),
                vfs_path,
                content.len()
            );
        } else {
            // Skip other file types (sockets, pipes, etc.)
            tracing::warn!("Skipping non-regular file: {}", path.display());
        }
    }

    Ok(count)
}

/// Load a single file into VFS
///
/// A convenience wrapper for loading individual files.
///
/// # Arguments
///
/// * `vfs` - The IsolateVfs to load file into
/// * `source_path` - Path to the source file on disk
/// * `vfs_path` - VFS path where file will be stored
///
/// # Returns
///
/// Returns `Ok(())` on success, or a VfsError on failure.
pub async fn load_file_to_vfs(
    vfs: &IsolateVfs,
    source_path: &str,
    vfs_path: &str,
) -> Result<(), VfsError> {
    let path = Path::new(source_path);

    if !path.exists() {
        return Err(VfsError::NotFound {
            path: source_path.to_string(),
        });
    }

    if !path.is_file() {
        return Err(VfsError::InvalidPath {
            path: source_path.to_string(),
            reason: "Path is not a regular file".to_string(),
        });
    }

    let content = fs::read(path).await.map_err(|e| {
        VfsError::IoError(format!("Failed to read file '{}': {}", path.display(), e))
    })?;

    vfs.write(vfs_path, &content).await?;

    tracing::debug!(
        "Loaded file into VFS: {} -> {} ({} bytes)",
        source_path,
        vfs_path,
        content.len()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};

    /// Create a temporary directory with test files
    async fn create_test_dir(base_path: &str) -> std::io::Result<()> {
        // Create directory structure
        fs::create_dir_all(format!("{}/css", base_path)).await?;
        fs::create_dir_all(format!("{}/js", base_path)).await?;
        fs::create_dir_all(format!("{}/images", base_path)).await?;

        // Create files
        fs::write(format!("{}/index.html", base_path), b"<html></html>").await?;
        fs::write(
            format!("{}/css/main.css", base_path),
            b"body { color: red; }",
        )
        .await?;
        fs::write(format!("{}/js/app.js", base_path), b"console.log('hello');").await?;
        fs::write(
            format!("{}/images/logo.png", base_path),
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR",
        )
        .await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_load_directory_to_vfs() {
        let temp_dir = std::env::temp_dir().join("nano_vfs_test_load_dir");
        let temp_path = temp_dir.to_str().unwrap();

        // Create test directory
        create_test_dir(temp_path).await.unwrap();

        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Load directory
        let count = load_directory_to_vfs(&vfs, temp_path, "/dist")
            .await
            .unwrap();

        // Verify files were loaded
        assert!(count >= 4, "Expected at least 4 files, got {}", count);

        // Check specific files
        let index_html = vfs.read("/dist/index.html").await.unwrap();
        assert_eq!(index_html, b"<html></html>");

        let css = vfs.read("/dist/css/main.css").await.unwrap();
        assert_eq!(css, b"body { color: red; }");

        let js = vfs.read("/dist/js/app.js").await.unwrap();
        assert_eq!(js, b"console.log('hello');");

        // Check binary file (PNG signature)
        let png = vfs.read("/dist/images/logo.png").await.unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        // Cleanup
        let _ = fs::remove_dir_all(temp_path).await;
    }

    #[tokio::test]
    async fn test_load_directory_preserve_structure() {
        let temp_dir = std::env::temp_dir().join("nano_vfs_test_structure");
        let temp_path = temp_dir.to_str().unwrap();

        // Create nested structure
        fs::create_dir_all(format!("{}/a/b/c", temp_path))
            .await
            .unwrap();
        fs::write(format!("{}/a/file1.txt", temp_path), b"file1")
            .await
            .unwrap();
        fs::write(format!("{}/a/b/file2.txt", temp_path), b"file2")
            .await
            .unwrap();
        fs::write(format!("{}/a/b/c/file3.txt", temp_path), b"file3")
            .await
            .unwrap();

        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Load directory
        let count = load_directory_to_vfs(&vfs, temp_path, "/assets")
            .await
            .unwrap();
        assert_eq!(count, 3);

        // Verify structure preserved
        let file1 = vfs.read("/assets/a/file1.txt").await.unwrap();
        assert_eq!(file1, b"file1");

        let file2 = vfs.read("/assets/a/b/file2.txt").await.unwrap();
        assert_eq!(file2, b"file2");

        let file3 = vfs.read("/assets/a/b/c/file3.txt").await.unwrap();
        assert_eq!(file3, b"file3");

        // Cleanup
        let _ = fs::remove_dir_all(temp_path).await;
    }

    #[tokio::test]
    async fn test_load_directory_empty() {
        let temp_dir = std::env::temp_dir().join("nano_vfs_test_empty");
        let temp_path = temp_dir.to_str().unwrap();

        // Create empty directory
        fs::create_dir(temp_path).await.unwrap();

        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Load empty directory
        let count = load_directory_to_vfs(&vfs, temp_path, "/empty")
            .await
            .unwrap();
        assert_eq!(count, 0);

        // Cleanup
        let _ = fs::remove_dir(temp_path).await;
    }

    #[tokio::test]
    async fn test_load_directory_nonexistent() {
        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Try to load non-existent directory
        let result = load_directory_to_vfs(&vfs, "/nonexistent/path", "/dist").await;

        assert!(result.is_err());
        match result {
            Err(VfsError::InvalidPath { path, reason }) => {
                assert_eq!(path, "/nonexistent/path");
                assert!(reason.contains("does not exist"));
            }
            _ => panic!("Expected InvalidPath error for non-existent directory"),
        }
    }

    #[tokio::test]
    async fn test_load_file_to_vfs() {
        let temp_dir = std::env::temp_dir().join("nano_vfs_test_single");
        let temp_path = temp_dir.to_str().unwrap();

        // Create directory with single file
        fs::create_dir(temp_path).await.unwrap();
        fs::write(format!("{}/test.txt", temp_path), b"single file content")
            .await
            .unwrap();

        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Load single file
        let result = load_file_to_vfs(
            &vfs,
            &format!("{}/test.txt", temp_path),
            "/uploads/test.txt",
        )
        .await;
        assert!(result.is_ok());

        // Verify file loaded
        let content = vfs.read("/uploads/test.txt").await.unwrap();
        assert_eq!(content, b"single file content");

        // Cleanup
        let _ = fs::remove_dir_all(temp_path).await;
    }

    #[tokio::test]
    async fn test_load_file_nonexistent() {
        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Try to load non-existent file
        let result = load_file_to_vfs(&vfs, "/nonexistent/file.txt", "/file.txt").await;

        assert!(result.is_err());
        match result {
            Err(VfsError::NotFound { path }) => {
                assert_eq!(path, "/nonexistent/file.txt");
            }
            _ => panic!("Expected NotFound error for non-existent file"),
        }
    }

    #[tokio::test]
    async fn test_load_directory_performance() {
        let temp_dir = std::env::temp_dir().join("nano_vfs_test_perf");
        let temp_path = temp_dir.to_str().unwrap();

        // Clean up any leftover directory from previous test runs
        let _ = fs::remove_dir_all(temp_path).await;

        // Create directory with many files
        fs::create_dir(temp_path).await.unwrap();
        for i in 0..100 {
            let subdir = format!("{}/dir{}", temp_path, i % 10);
            fs::create_dir_all(&subdir).await.unwrap();
            fs::write(
                format!("{}/file{}.txt", subdir, i),
                format!("content {}", i),
            )
            .await
            .unwrap();
        }

        // Create VFS
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        // Measure loading time
        let start = std::time::Instant::now();
        let count = load_directory_to_vfs(&vfs, temp_path, "/dist")
            .await
            .unwrap();
        let elapsed = start.elapsed();

        // Verify all files loaded
        assert_eq!(count, 100);

        // Performance assertion: 100 files should load in less than 5 seconds
        // (very generous limit for CI environments)
        assert!(
            elapsed.as_secs() < 5,
            "Loading 100 files took too long: {:?}",
            elapsed
        );

        // Verify a sample file
        let content = vfs.read("/dist/dir5/file5.txt").await.unwrap();
        assert_eq!(content, b"content 5");

        // Cleanup
        let _ = fs::remove_dir_all(temp_path).await;
    }
}
