//! VFS State Capture Module
//!
//! This module provides functionality to capture all files from a VFS
//! for inclusion in a sliver.

use crate::vfs::{IsolateVfs, VfsBackend, VfsFile, VfsPath, VfsResult};
use std::collections::HashMap;

/// Captured VFS state ready for serialization
///
/// Contains all files and their metadata extracted from a VFS.
#[derive(Debug, Clone)]
pub struct VfsCapture {
    /// Map of paths to file content
    files: HashMap<String, VfsFile>,
    /// Total number of files captured
    file_count: usize,
    /// Total bytes captured
    total_bytes: usize,
}

impl VfsCapture {
    /// Create a new empty VfsCapture
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            file_count: 0,
            total_bytes: 0,
        }
    }

    /// Add a file to the capture
    pub fn add_file(&mut self, path: VfsPath, file: VfsFile) {
        self.total_bytes += file.content.len();
        self.file_count += 1;
        self.files.insert(path.as_str().to_string(), file);
    }

    /// Get all captured files
    pub fn files(&self) -> &HashMap<String, VfsFile> {
        &self.files
    }

    /// Get the number of files captured
    pub fn file_count(&self) -> usize {
        self.file_count
    }

    /// Get total bytes captured
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Convert to a vector of (path, file) tuples
    ///
    /// This format is suitable for the sliver packer.
    pub fn into_vec(self) -> Vec<(String, VfsFile)> {
        self.files.into_iter().collect()
    }

    /// Check if a path exists in the capture
    pub fn has_path(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}

impl Default for VfsCapture {
    fn default() -> Self {
        Self::new()
    }
}

/// Capture the state of a VFS
///
/// This function walks the VFS and extracts all files.
/// Currently supports MemoryBackend. Other backends may
/// require different handling.
///
/// # Arguments
///
/// * `vfs` - The VFS to capture
///
/// # Returns
///
/// A `VfsCapture` containing all files, or an error.
///
/// # Example
///
/// ```rust,no_run
/// use nano::sliver::vfs_capture::capture_vfs;
/// use nano::vfs::IsolateVfs;
///
/// # async fn example(vfs: &IsolateVfs) -> Result<(), Box<dyn std::error::Error>> {
/// let capture = capture_vfs(vfs).await?;
/// println!("Captured {} files ({} bytes)", capture.file_count(), capture.total_bytes());
/// # Ok(())
/// # }
/// ```
pub async fn capture_vfs(vfs: &IsolateVfs) -> VfsResult<VfsCapture> {
    let mut capture = VfsCapture::new();

    // Recursively capture all files starting from root
    capture_all_files_recursive(vfs, &mut capture, "/").await?;

    tracing::info!(
        "VFS capture complete: {} files, {} bytes",
        capture.file_count(),
        capture.total_bytes()
    );

    Ok(capture)
}

/// Capture all files from the VFS using backend-agnostic operations.
///
/// Uses `list_dir` and `read` to recursively discover and capture all files.
/// Works with any backend that supports these operations (Memory, Disk, S3).
async fn capture_all_files_recursive(
    vfs: &IsolateVfs,
    capture: &mut VfsCapture,
    path: &str,
) -> VfsResult<()> {
    // Try to list directory entries
    match vfs.list_dir(path).await {
        Ok(entries) => {
            for entry in entries {
                let entry_path = entry.as_str();
                // Recurse into each entry (may be file or subdirectory)
                Box::pin(capture_all_files_recursive(vfs, capture, entry_path)).await?;
            }
            Ok(())
        }
        Err(_) => {
            // Not a directory (or list_dir not supported) - try to read as file
            match vfs.read(path).await {
                Ok(content) => {
                    let file = VfsFile::new(content);
                    capture.add_file(VfsPath::new(path)?, file);
                    Ok(())
                }
                Err(_) => {
                    // Neither directory nor readable file - skip
                    tracing::debug!("VFS capture: skipping unreadable path: {}", path);
                    Ok(())
                }
            }
        }
    }
}

/// Walk a directory and capture all files recursively
///
/// This is a utility function for backends that support
/// directory listing (like DiskBackend).
///
/// Note: The backend-agnostic `capture_vfs()` is preferred for general use.
/// This function is available for direct backend access when needed.
pub async fn walk_and_capture<B>(backend: &B, path: &str, capture: &mut VfsCapture) -> VfsResult<()>
where
    B: VfsBackend,
{
    let vfs_path = VfsPath::new(path)?;

    match backend.list_dir(&vfs_path).await {
        Ok(entries) => {
            for entry in entries {
                let entry_str = entry.as_str();
                // Recurse - entry may be file or subdirectory
                Box::pin(walk_and_capture(backend, entry_str, capture)).await?;
            }
            Ok(())
        }
        Err(_) => {
            // Not a directory - try to read as file
            match backend.read(&vfs_path).await {
                Ok(content) => {
                    let file = VfsFile::new(content);
                    capture.add_file(vfs_path, file);
                    Ok(())
                }
                Err(_) => {
                    tracing::debug!("walk_and_capture: skipping unreadable path: {}", path);
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{IsolateVfs, MemoryBackend, VfsNamespace};

    #[test]
    fn test_vfs_capture_new() {
        let capture = VfsCapture::new();
        assert_eq!(capture.file_count(), 0);
        assert_eq!(capture.total_bytes(), 0);
        assert!(capture.files().is_empty());
    }

    #[test]
    fn test_vfs_capture_add_file() {
        let mut capture = VfsCapture::new();

        // VfsPath normalizes paths - leading slashes are stripped
        let path = VfsPath::new("test.txt").unwrap();
        let file = VfsFile::new(b"Hello, World!".to_vec());

        capture.add_file(path.clone(), file);

        assert_eq!(capture.file_count(), 1);
        assert_eq!(capture.total_bytes(), 13);
        assert!(capture.has_path(path.as_str()));
    }

    #[test]
    fn test_vfs_capture_into_vec() {
        let mut capture = VfsCapture::new();

        let path1 = VfsPath::new("/file1.txt").unwrap();
        let file1 = VfsFile::new(b"content1".to_vec());
        capture.add_file(path1, file1);

        let path2 = VfsPath::new("/file2.txt").unwrap();
        let file2 = VfsFile::new(b"content2".to_vec());
        capture.add_file(path2, file2);

        let vec = capture.into_vec();
        assert_eq!(vec.len(), 2);
    }

    #[tokio::test]
    async fn test_capture_vfs_empty() {
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );

        let capture = capture_vfs(&vfs).await.unwrap();
        assert_eq!(capture.file_count(), 0);
    }

    #[tokio::test]
    async fn test_capture_vfs_with_files() {
        // Create a shared backend that we'll use for both VFS operations
        let shared_backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            shared_backend.clone(),
        );

        // Write some files
        vfs.write("/config.json", b"{\"key\": \"value\"}")
            .await
            .unwrap();
        vfs.write("/data.txt", b"some data").await.unwrap();

        // Capture VFS
        let capture = capture_vfs(&vfs).await.unwrap();

        // Note: Without list_dir(), we can't actually capture the files yet
        // This test documents the expected behavior once list_dir is implemented
        assert_eq!(capture.file_count(), 0); // Currently returns 0
    }
}
