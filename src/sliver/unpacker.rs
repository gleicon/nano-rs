//! Sliver Unpacker
//!
//! Extracts metadata, pre-compiled bytecode, and VFS contents from a sliver archive.

use tar::Archive;

use crate::sliver::error::{SliverError, SliverResult};
use crate::sliver::format::{BYTECODE_FILENAME, METADATA_FILENAME, VFS_PREFIX};
use crate::sliver::metadata::SliverMetadata;
use crate::vfs::types::{VfsFile, VfsPath};

/// Unpacked sliver contents (v2.0 format)
#[derive(Debug, Clone)]
pub struct UnpackedSliver {
    pub metadata: SliverMetadata,
    /// Pre-compiled V8 bytecode for the entrypoint (None = source-only sliver).
    /// Valid only when metadata.v8_cache_version matches the running V8.
    pub bytecode: Option<Vec<u8>>,
    /// All bundled files: JS source, assets, modules, WASM binaries.
    pub vfs_entries: Vec<(VfsPath, VfsFile)>,
}

impl UnpackedSliver {
    pub fn new(
        metadata: SliverMetadata,
        bytecode: Option<Vec<u8>>,
        vfs_entries: Vec<(VfsPath, VfsFile)>,
    ) -> Self {
        Self { metadata, bytecode, vfs_entries }
    }

    pub fn total_size(&self) -> usize {
        let meta_size = serde_json::to_vec(&self.metadata).map(|v| v.len()).unwrap_or(0);
        let bc_size = self.bytecode.as_ref().map(|b| b.len()).unwrap_or(0);
        let vfs_size: usize = self.vfs_entries.iter().map(|(_, f)| f.content.len()).sum();
        meta_size + bc_size + vfs_size
    }

    pub fn summary(&self) -> String {
        format!(
            "Sliver: hostname={} format={} bytecode={}b vfs={} entries total={}b",
            self.metadata.hostname,
            self.metadata.format_version,
            self.bytecode.as_ref().map(|b| b.len()).unwrap_or(0),
            self.vfs_entries.len(),
            self.total_size()
        )
    }

    /// Populate an IsolateVfs with all bundled files.
    pub async fn restore_to_vfs(&self, vfs: &crate::vfs::IsolateVfs) -> SliverResult<()> {
        for (path, file) in &self.vfs_entries {
            vfs.write(path, &file.content).await.map_err(|e| SliverError::VfsRestore {
                path: path.to_string(),
                reason: e.to_string(),
            })?;
        }
        Ok(())
    }

    /// Entrypoint VFS path from metadata custom field, defaulting to /index.js.
    pub fn entrypoint(&self) -> String {
        self.metadata.custom
            .get("entrypoint")
            .map(|e| if e.starts_with('/') { e.clone() } else { format!("/{}", e) })
            .unwrap_or_else(|| "/index.js".to_string())
    }

    /// True when the sliver's bytecode is valid for the running V8.
    pub fn bytecode_matches_v8(&self) -> bool {
        match (self.bytecode.as_ref(), self.metadata.v8_cache_version) {
            (Some(_), Some(tag)) => tag == v8::script_compiler::cached_data_version_tag(),
            _ => false,
        }
    }
}

pub struct SliverUnpacker;

impl SliverUnpacker {
    /// Unpack a sliver archive.
    /// Unknown tar entries (including legacy heap.bin) are silently ignored.
    pub fn unpack(archive_data: &[u8]) -> SliverResult<UnpackedSliver> {
        let mut archive = Archive::new(archive_data);

        let mut metadata: Option<SliverMetadata> = None;
        let mut bytecode: Option<Vec<u8>> = None;
        let mut vfs_entries: Vec<(VfsPath, VfsFile)> = Vec::new();

        for entry_result in archive.entries()? {
            let mut entry = entry_result?;
            let path_str = entry.path()?.to_string_lossy().to_string();

            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut content)?;

            match path_str.as_str() {
                METADATA_FILENAME => {
                    metadata = Some(SliverMetadata::from_json(&content)?);
                }
                BYTECODE_FILENAME => {
                    bytecode = Some(content);
                }
                path if path.starts_with(VFS_PREFIX) => {
                    let vfs_path_str = &path[VFS_PREFIX.len()..];
                    match VfsPath::new(vfs_path_str) {
                        Ok(vfs_path) => {
                            let file = VfsFile {
                                content,
                                created_at: std::time::SystemTime::now(),
                                modified_at: std::time::SystemTime::now(),
                                size: 0,
                            };
                            vfs_entries.push((vfs_path, file));
                        }
                        Err(_) => {
                            return Err(SliverError::InvalidVfsPath {
                                path: vfs_path_str.to_string(),
                            });
                        }
                    }
                }
                _ => {} // heap.bin, manifest.txt, and any unknown entries — silently ignored
            }
        }

        let metadata = metadata.ok_or_else(|| SliverError::MissingMetadata {
            filename: METADATA_FILENAME.to_string(),
        })?;

        if !crate::sliver::format::SliverFormat::is_supported_version(&metadata.format_version) {
            return Err(SliverError::InvalidFormat {
                version: metadata.format_version.clone(),
            });
        }

        for (_, file) in &mut vfs_entries {
            file.size = file.content.len();
        }

        Ok(UnpackedSliver::new(metadata, bytecode, vfs_entries))
    }

    pub fn unpack_and_validate(archive_data: &[u8]) -> SliverResult<UnpackedSliver> {
        let unpacked = Self::unpack(archive_data)?;
        if unpacked.vfs_entries.is_empty() && unpacked.bytecode.is_none() {
            return Err(SliverError::CorruptedArchive {
                reason: "Sliver has no VFS entries and no bytecode".to_string(),
            });
        }
        Ok(unpacked)
    }
}

pub fn unpack_sliver(archive_data: &[u8]) -> SliverResult<UnpackedSliver> {
    SliverUnpacker::unpack(archive_data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sliver::packer::pack_sliver;

    #[test]
    fn test_unpack_basic() {
        let metadata = SliverMetadata::new("app.example.com", "1.1.0");
        let vfs_entries = vec![(
            VfsPath::new("index.js").unwrap(),
            VfsFile::new(b"const x = 1".to_vec()),
        )];
        let archive = pack_sliver(&metadata, None, Some(&vfs_entries)).unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();
        assert_eq!(unpacked.metadata.hostname, "app.example.com");
        assert!(unpacked.bytecode.is_none());
        assert_eq!(unpacked.vfs_entries.len(), 1);
    }

    #[test]
    fn test_unpack_with_bytecode() {
        let mut metadata = SliverMetadata::new("app.example.com", "1.1.0");
        metadata.v8_cache_version = Some(42);
        let vfs_entries = vec![(
            VfsPath::new("index.js").unwrap(),
            VfsFile::new(b"const x = 1".to_vec()),
        )];
        let fake_bytecode = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let archive = pack_sliver(&metadata, Some(&fake_bytecode), Some(&vfs_entries)).unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();
        assert_eq!(unpacked.bytecode, Some(fake_bytecode));
    }

    #[test]
    fn test_unpack_with_vfs() {
        let metadata = SliverMetadata::new("app.example.com", "1.1.0");
        let vfs_entries = vec![
            (VfsPath::new("index.js").unwrap(), VfsFile::new(b"const x=1".to_vec())),
            (VfsPath::new("data/config.json").unwrap(), VfsFile::new(b"{\"k\":\"v\"}".to_vec())),
        ];
        let archive = pack_sliver(&metadata, None, Some(&vfs_entries)).unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();
        assert_eq!(unpacked.vfs_entries.len(), 2);
    }

    #[test]
    fn test_unpack_missing_metadata() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_path("other.txt").unwrap();
        header.set_size(4);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &b"data"[..]).unwrap();
        builder.finish().unwrap();
        let archive = builder.into_inner().unwrap();
        assert!(matches!(unpack_sliver(&archive), Err(SliverError::MissingMetadata { .. })));
    }

    #[test]
    fn test_unpack_ignores_heap_bin() {
        // v1.0 slivers with heap.bin should load without error
        let mut metadata = SliverMetadata::new("app.example.com", "1.1.0");
        metadata.format_version = "1.0".to_string();
        let json = serde_json::to_vec(&metadata).unwrap();

        let mut builder = tar::Builder::new(Vec::new());
        let mut h = tar::Header::new_gnu();
        h.set_path("meta.json").unwrap();
        h.set_size(json.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        builder.append(&h, json.as_slice()).unwrap();

        let heap = b"NANO-DIR-v1\0index.js";
        let mut h2 = tar::Header::new_gnu();
        h2.set_path("heap.bin").unwrap();
        h2.set_size(heap.len() as u64);
        h2.set_mode(0o644);
        h2.set_cksum();
        builder.append(&h2, &heap[..]).unwrap();

        builder.finish().unwrap();
        let archive = builder.into_inner().unwrap();
        let unpacked = unpack_sliver(&archive).unwrap();
        assert!(unpacked.bytecode.is_none());
    }

    #[test]
    fn test_bytecode_matches_v8_wrong_version() {
        if !crate::v8::is_initialized() {
            crate::v8::initialize_platform().ok();
        }
        let mut metadata = SliverMetadata::new("test.example.com", "1.0");
        metadata.v8_cache_version = Some(0); // deliberately wrong version
        let sliver = UnpackedSliver::new(metadata, Some(vec![0xDE, 0xAD]), vec![]);
        assert!(!sliver.bytecode_matches_v8(), "wrong v8_cache_version → no match");
    }

    #[test]
    fn test_bytecode_matches_v8_correct_version() {
        if !crate::v8::is_initialized() {
            crate::v8::initialize_platform().ok();
        }
        let current_tag = v8::script_compiler::cached_data_version_tag();
        let mut metadata = SliverMetadata::new("test.example.com", "1.0");
        metadata.v8_cache_version = Some(current_tag);
        let sliver = UnpackedSliver::new(metadata, Some(vec![0xDE, 0xAD]), vec![]);
        assert!(sliver.bytecode_matches_v8(), "matching v8_cache_version → match");
    }

    #[test]
    fn test_bytecode_matches_v8_no_bytecode() {
        let metadata = SliverMetadata::new("test.example.com", "1.0");
        let sliver = UnpackedSliver::new(metadata, None, vec![]);
        assert!(!sliver.bytecode_matches_v8(), "no bytecode → no match");
    }

    #[test]
    fn test_restore_to_vfs_populates_entries() {
        use crate::vfs::{IsolateVfs, MemoryBackend, VfsBackendEnum, VfsNamespace};
        let entries = vec![(
            VfsPath::new("/index.js").unwrap(),
            VfsFile::new(b"const x = 1".to_vec()),
        )];
        let metadata = SliverMetadata::new("test.example.com", "1.0");
        let sliver = UnpackedSliver::new(metadata, None, entries);

        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            VfsBackendEnum::memory(MemoryBackend::default()),
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            sliver.restore_to_vfs(&vfs).await.expect("restore should succeed");
            let content = vfs.read("/index.js").await.expect("read should succeed");
            assert_eq!(content, b"const x = 1");
        });
    }

    #[test]
    fn test_entrypoint_from_metadata() {
        let mut metadata = SliverMetadata::new("app.example.com", "1.1.0");
        metadata.custom.insert("entrypoint".to_string(), "index.js".to_string());
        let sliver = UnpackedSliver::new(metadata, None, vec![]);
        assert_eq!(sliver.entrypoint(), "/index.js");
    }

    #[test]
    fn test_sliver_summary() {
        let metadata = SliverMetadata::new("app.example.com", "1.1.0");
        let sliver = UnpackedSliver::new(metadata, None, vec![]);
        let s = sliver.summary();
        assert!(s.contains("app.example.com"));
    }
}
