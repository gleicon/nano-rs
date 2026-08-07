//! Sliver Extractor
//!
//! Utilities for working with sliver VFS contents.
//! In v2.0, VFS entries are loaded directly into the in-memory IsolateVfs —
//! no temp directory extraction needed.

use crate::sliver::unpacker::UnpackedSliver;
use crate::vfs::types::{VfsFile, VfsPath};

/// Utilities for inspecting sliver VFS entries.
pub struct SliverExtractor;

impl SliverExtractor {
    /// Detect the JS entrypoint path from VFS entries.
    /// Returns a VFS-relative path like `/index.js`.
    pub fn detect_entrypoint(vfs_entries: &[(VfsPath, VfsFile)]) -> String {
        let candidates = [
            "/index.js",
            "/app.js",
            "/main.js",
            "/server.js",
            "index.js",
            "app.js",
            "main.js",
            "server.js",
        ];
        for candidate in &candidates {
            if vfs_entries.iter().any(|(p, _)| p.as_str() == *candidate) {
                let s = candidate.trim_start_matches('/');
                return format!("/{}", s);
            }
        }
        "/index.js".to_string()
    }

    /// Resolve the entrypoint for a sliver: metadata custom["entrypoint"] wins,
    /// then auto-detect from VFS entries.
    pub fn resolve_entrypoint(unpacked: &UnpackedSliver) -> String {
        if let Some(ep) = unpacked.metadata.custom.get("entrypoint") {
            return if ep.starts_with('/') {
                ep.clone()
            } else {
                format!("/{}", ep)
            };
        }
        Self::detect_entrypoint(&unpacked.vfs_entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_entrypoint_index() {
        let entries = vec![(
            VfsPath::new("/index.js").unwrap(),
            VfsFile::new(b"const x=1".to_vec()),
        )];
        assert_eq!(SliverExtractor::detect_entrypoint(&entries), "/index.js");
    }

    #[test]
    fn test_detect_entrypoint_fallback() {
        let entries: Vec<(VfsPath, VfsFile)> = vec![];
        assert_eq!(SliverExtractor::detect_entrypoint(&entries), "/index.js");
    }

    #[test]
    fn test_resolve_from_metadata() {
        use crate::sliver::metadata::SliverMetadata;
        let mut meta = SliverMetadata::new("test.example.com", "1.0");
        meta.custom
            .insert("entrypoint".to_string(), "app.js".to_string());
        let sliver = UnpackedSliver::new(meta, None, vec![]);
        assert_eq!(SliverExtractor::resolve_entrypoint(&sliver), "/app.js");
    }
}
