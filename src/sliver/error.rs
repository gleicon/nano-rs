//! Sliver Format Error Types
//!
//! Defines all error types for sliver format operations.
//! These errors wrap lower-level errors (IO, serialization) with context.

use std::io;
use thiserror::Error;

/// Errors that can occur during sliver operations
#[derive(Error, Debug)]
pub enum SliverError {
    /// IO error during read/write operations
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    /// JSON serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Invalid sliver format version
    #[error("Invalid format version: {version}")]
    InvalidFormat { version: String },

    /// Missing metadata file in archive
    #[error("Missing metadata file: {filename}")]
    MissingMetadata { filename: String },

    /// Sliver archive has no payload (no bytecode, VFS entries, or legacy heap blob)
    #[error("Missing payload in sliver: {filename}")]
    MissingPayload { filename: String },

    /// Corrupted or invalid tar archive structure
    #[error("Corrupted archive: {reason}")]
    CorruptedArchive { reason: String },

    /// VFS entry path is invalid
    #[error("Invalid VFS path: {path}")]
    InvalidVfsPath { path: String },

    /// VFS entry type is unsupported
    #[error("Unsupported VFS entry type: {entry_type}")]
    UnsupportedEntryType { entry_type: String },

    /// VFS restoration failed
    #[error("Failed to restore VFS entry at {path}: {reason}")]
    VfsRestore { path: String, reason: String },
}

impl From<serde_json::Error> for SliverError {
    fn from(err: serde_json::Error) -> Self {
        SliverError::SerializationError(err.to_string())
    }
}

// Note: tar crate uses std::io::Error for all I/O operations
// so we don't need a separate conversion - io::Error is already covered

/// Result type alias for sliver operations
pub type SliverResult<T> = Result<T, SliverError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversions() {
        // Test IO error conversion
        let io_err = io::Error::new(io::ErrorKind::NotFound, "test");
        let sliver_err: SliverError = io_err.into();
        assert!(matches!(sliver_err, SliverError::IoError(_)));

        // Test serde error conversion
        let json_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let sliver_err: SliverError = json_err.into();
        assert!(matches!(sliver_err, SliverError::SerializationError(_)));
    }

    #[test]
    fn test_error_display() {
        let err = SliverError::MissingMetadata {
            filename: "meta.json".to_string(),
        };
        assert_eq!(err.to_string(), "Missing metadata file: meta.json");

        let err = SliverError::InvalidFormat {
            version: "0.5".to_string(),
        };
        assert_eq!(err.to_string(), "Invalid format version: 0.5");
    }
}
