//! Sliver Format Constants and Types
//!
//! Defines the sliver archive format structure and version constants.
//! The format is designed to be simple, portable, and evolvable.

/// Current sliver format version.
/// v2.0: bytecode.v8bc replaces heap.bin; source lives in vfs/
pub const FORMAT_VERSION: &str = "2.0";

/// NANO runtime version for metadata
pub const NANO_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Filename for V8 UnboundScript bytecode (optional; V8-version-tied).
/// Present when packed with `--compile` (default). Absent for source-only slivers.
pub const BYTECODE_FILENAME: &str = "bytecode.v8bc";

/// Filename for JSON metadata
///
/// Contains structured information about the sliver including
/// hostname, creation time, format version, and description.
pub const METADATA_FILENAME: &str = "meta.json";

/// Filename for human-readable manifest
///
/// A plain text file listing the archive contents for inspection.
/// This is informational only and not used during loading.
pub const MANIFEST_FILENAME: &str = "manifest.txt";

/// Prefix for VFS entries in the archive
///
/// All VFS files are stored under this path prefix in the tar archive.
/// Example: vfs/data/config.json
pub const VFS_PREFIX: &str = "vfs/";

/// The sliver file extension
pub const SLIVER_EXTENSION: &str = ".sliver";

/// Format specification and capabilities
#[derive(Debug, Clone)]
pub struct SliverFormat;

impl SliverFormat {
    /// Get the current format version
    pub fn version() -> &'static str {
        FORMAT_VERSION
    }

    /// Get the NANO runtime version
    pub fn nano_version() -> &'static str {
        NANO_VERSION
    }

    pub fn is_supported_version(version: &str) -> bool {
        Self::is_supported_version_any(version)
    }

    /// Get the list of required files in a valid sliver archive
    pub fn required_files() -> &'static [&'static str] {
        &[METADATA_FILENAME]
    }

    /// Check if a format version is supported (accepts both v1.0 for read and v2.0)
    pub fn is_supported_version_any(version: &str) -> bool {
        matches!(version, "1.0" | "2.0")
    }

    /// Get the complete archive structure as a string
    ///
    /// This is used for documentation and manifest generation.
    pub fn structure_documentation() -> &'static str {
        r#"Sliver Archive Structure
========================

app.sliver (tar archive)
├── meta.json          # Metadata: hostname, created_at, version
├── bytecode.v8bc      # Optional: pre-compiled V8 UnboundScript bytes
└── vfs/               # App files (JS source, assets, modules, WASM)
    ├── index.js
    └── assets/
        └── logo.png

Required Files:
- meta.json: JSON metadata with hostname, timestamps, version info
- vfs/*: the app's files (the app runs from these)

Optional Files:
- bytecode.v8bc: pre-compiled bytecode generated at pack time (skips
  parse+compile at load); omitted with --source-only

Format Notes:
- Archive is a standard tar file (ustar or GNU format)
- There is no heap snapshot: the app runs from its VFS source
- VFS entries preserve directory structure under vfs/ prefix
- All paths use forward slashes (even on Windows)
- Binary files stored as-is without encoding
"#
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_version() {
        assert_eq!(SliverFormat::version(), "2.0");
        assert!(SliverFormat::is_supported_version("1.0")); // v1 readable for migration
        assert!(SliverFormat::is_supported_version("2.0")); // current
        assert!(!SliverFormat::is_supported_version("0.9"));
        assert!(!SliverFormat::is_supported_version("3.0"));
    }

    #[test]
    fn test_required_files() {
        let required = SliverFormat::required_files();
        assert!(required.contains(&"meta.json"));
    }

    #[test]
    fn test_constants() {
        assert_eq!(BYTECODE_FILENAME, "bytecode.v8bc");
        assert_eq!(METADATA_FILENAME, "meta.json");
        assert_eq!(MANIFEST_FILENAME, "manifest.txt");
        assert_eq!(VFS_PREFIX, "vfs/");
        assert_eq!(SLIVER_EXTENSION, ".sliver");
    }

    #[test]
    fn test_nano_version() {
        // Should match cargo package version
        assert!(!SliverFormat::nano_version().is_empty());
    }
}
