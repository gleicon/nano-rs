//! Sliver VFS Restoration Coordinator
//!
//! Coordinates the restoration of VFS state from slivers,
//! selecting the appropriate backend strategy.

use crate::sliver::error::SliverResult;
use crate::sliver::unpacker::UnpackedSliver;
use crate::vfs::IsolateVfs;

/// Restore VFS entries to an isolate
///
/// This is the main entry point for VFS restoration. It uses
/// the IsolateVfs interface to restore all files from the sliver.
///
/// # Arguments
/// * `unpacked` - The unpacked sliver containing VFS entries
/// * `vfs` - The target VFS for restoration
///
/// # Returns
/// SliverResult indicating success
pub async fn restore_vfs(
    unpacked: &UnpackedSliver,
    vfs: &IsolateVfs,
) -> SliverResult<()> {
    tracing::info!(
        "Starting VFS restoration for {} ({} entries)",
        unpacked.metadata.hostname,
        unpacked.vfs_entries.len()
    );

    // Use the UnpackedSliver's restore method
    unpacked.restore_to_vfs(vfs).await?;

    tracing::info!("VFS restoration complete for {}", unpacked.metadata.hostname);
    Ok(())
}

/// Verify VFS restoration
///
/// Checks that all expected files were restored correctly.
///
/// # Arguments
/// * `unpacked` - The original unpacked sliver (reference)
/// * `vfs` - The VFS to verify
///
/// # Returns
/// SliverResult with verification report
pub async fn verify_vfs_restoration(
    unpacked: &UnpackedSliver,
    vfs: &IsolateVfs,
) -> SliverResult<VerificationReport> {
    let mut missing = Vec::new();
    let mut mismatched = Vec::new();
    let mut verified = 0;

    for (path, expected_file) in &unpacked.vfs_entries {
        match vfs.read(path).await {
            Ok(content) => {
                if content == expected_file.content {
                    verified += 1;
                } else {
                    mismatched.push(path.to_string());
                }
            }
            Err(_) => {
                missing.push(path.to_string());
            }
        }
    }

    Ok(VerificationReport {
        total: unpacked.vfs_entries.len(),
        verified,
        missing,
        mismatched,
    })
}

/// Verification report for VFS restoration
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub total: usize,
    pub verified: usize,
    pub missing: Vec<String>,
    pub mismatched: Vec<String>,
}

impl VerificationReport {
    /// Check if all files were restored correctly
    pub fn is_complete(&self) -> bool {
        self.verified == self.total && self.missing.is_empty() && self.mismatched.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sliver::metadata::SliverMetadata;
    use crate::sliver::packer::pack_sliver;
    use crate::sliver::unpacker::unpack_sliver;
    use crate::vfs::{IsolateVfs, MemoryBackend, VfsFile, VfsNamespace, VfsPath};
    

    #[tokio::test]
    async fn test_restore_vfs_to_isolate() {
        // Create sliver with VFS entries
        let metadata = SliverMetadata::new("test.example.com", "1.1.0");
        let heap_data = vec![0u8; 100];
        
        let vfs_entries = vec![
            (VfsPath::new("config.json").unwrap(), VfsFile::new(b"{}".to_vec())),
            (VfsPath::new("data/users.txt").unwrap(), VfsFile::new(b"user1\nuser2".to_vec())),
        ];

        let archive = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries)).unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();

        // Create target VFS
        let backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("target"), backend.clone());

        // Restore
        restore_vfs(&unpacked, &vfs).await.unwrap();

        // Verify
        assert!(vfs.exists(&VfsPath::new("config.json").unwrap()).await.unwrap());
        assert!(vfs.exists(&VfsPath::new("data/users.txt").unwrap()).await.unwrap());
        
        let config_content = vfs.read(&VfsPath::new("config.json").unwrap()).await.unwrap();
        assert_eq!(config_content, b"{}");
    }

    #[tokio::test]
    async fn test_verify_vfs_restoration() {
        let metadata = SliverMetadata::new("test.example.com", "1.1.0");
        let heap_data = vec![0u8; 100];
        
        let vfs_entries = vec![
            (VfsPath::new("file1.txt").unwrap(), VfsFile::new(b"content1".to_vec())),
        ];

        let archive = pack_sliver(&metadata, Some(&heap_data), Some(&vfs_entries)).unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();

        let backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("target"), backend);

        // Restore
        restore_vfs(&unpacked, &vfs).await.unwrap();

        // Verify
        let report = verify_vfs_restoration(&unpacked, &vfs).await.unwrap();
        assert!(report.is_complete());
        assert_eq!(report.verified, 1);
    }

    #[test]
    fn test_verification_report_is_complete() {
        let complete = VerificationReport {
            total: 3,
            verified: 3,
            missing: vec![],
            mismatched: vec![],
        };
        assert!(complete.is_complete());

        let incomplete = VerificationReport {
            total: 3,
            verified: 2,
            missing: vec!["file.txt".to_string()],
            mismatched: vec![],
        };
        assert!(!incomplete.is_complete());
    }
}
