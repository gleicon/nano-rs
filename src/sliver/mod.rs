//! Sliver Format Module
//!
//! A sliver is a versioned deployment artifact for nano edge apps:
//! - **Metadata**: JSON with hostname, timestamps, V8 cache version
//! - **Bytecode** (optional): Pre-compiled V8 UnboundScript bytes; skips parse+compile on load
//! - **VFS**: All files — JS source, assets, modules, WASM binaries
//!
//! Format v2.0: no heap snapshot, source lives in VFS, bytecode is optional optimization.
//!
//! ```text
//! app-v2.sliver (tar)
//! ├── meta.json        # hostname, nano_version, v8_cache_version
//! ├── bytecode.v8bc    # optional — UnboundScript bytes
//! └── vfs/             # all files
//!     ├── index.js
//!     └── config.json
//! ```

pub mod auto_cache;
pub mod benchmark;
mod error;
pub mod extractor;
mod format;
mod metadata;
mod packer;
pub mod packager;
pub mod restore;
mod unpacker;
pub mod validation;
pub mod vfs_capture;

pub use error::{SliverError, SliverResult};
pub use extractor::SliverExtractor;
pub use format::{SliverFormat, FORMAT_VERSION, BYTECODE_FILENAME, MANIFEST_FILENAME, METADATA_FILENAME, VFS_PREFIX, SLIVER_EXTENSION};
pub use metadata::SliverMetadata;
pub use packer::{pack_sliver, SliverPacker};
pub use unpacker::{unpack_sliver, SliverUnpacker, UnpackedSliver};
pub use validation::{validate_sliver_integrity, find_sliver_file, check_version_compatibility, CorruptionType};

/// Walk a VFS backend and collect all entries for serialization.
pub async fn walk_vfs_for_sliver<B>(backend: &B) -> crate::vfs::VfsResult<Vec<(crate::vfs::VfsPath, crate::vfs::VfsFile)>>
where
    B: crate::vfs::VfsBackend,
{
    let mut result = Vec::new();
    let mut to_process = vec![crate::vfs::VfsPath::new("/").unwrap()];

    while let Some(path) = to_process.pop() {
        match backend.list_dir(&path).await {
            Ok(entries) => {
                for entry in entries {
                    match backend.metadata(&entry).await {
                        Ok(_) => {
                            if let Ok(content) = backend.read(&entry).await {
                                result.push((entry, crate::vfs::VfsFile::new(content)));
                            }
                        }
                        Err(_) => {
                            to_process.push(entry);
                        }
                    }
                }
            }
            Err(_) => {
                if let Ok(content) = backend.read(&path).await {
                    result.push((path, crate::vfs::VfsFile::new(content)));
                }
            }
        }
    }

    Ok(result)
}

pub fn build_manifest(entries: &[String]) -> String {
    let mut manifest = String::new();
    manifest.push_str("# Sliver Archive Manifest\n");
    manifest.push_str("# =========================\n\n");
    for entry in entries {
        manifest.push_str(entry);
        manifest.push('\n');
    }
    manifest
}

/// Quick validation: metadata present and version supported.
pub fn validate_sliver(archive_data: &[u8]) -> SliverResult<()> {
    use tar::Archive;

    let mut archive = Archive::new(archive_data);
    let mut has_metadata = false;
    let mut metadata_version: Option<String> = None;

    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let path_str = entry.path()?.to_string_lossy().to_string();
        if path_str == METADATA_FILENAME {
            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut content)?;
            let metadata: SliverMetadata = serde_json::from_slice(&content)?;
            metadata_version = Some(metadata.format_version);
            has_metadata = true;
        }
    }

    if !has_metadata {
        return Err(SliverError::MissingMetadata {
            filename: METADATA_FILENAME.to_string(),
        });
    }

    if let Some(version) = metadata_version {
        if !format::SliverFormat::is_supported_version(&version) {
            return Err(SliverError::InvalidFormat { version });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_manifest() {
        let entries = vec!["meta.json".to_string(), "bytecode.v8bc".to_string()];
        let manifest = build_manifest(&entries);
        assert!(manifest.contains("meta.json"));
    }

    #[test]
    fn test_validate_sliver_valid() {
        let metadata = SliverMetadata::new("app.example.com", "1.1.0");
        let archive = pack_sliver(&metadata, None, None).unwrap();
        assert!(validate_sliver(&archive).is_ok());
    }

    #[test]
    fn test_module_exports() {
        let _ = SliverMetadata::new("test", "1.0.0");
        let _format_version = FORMAT_VERSION;
        let _bytecode_filename = BYTECODE_FILENAME;
        let _metadata_filename = METADATA_FILENAME;
    }
}
