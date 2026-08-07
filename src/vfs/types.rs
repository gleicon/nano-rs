//! VFS Core Types
//!
//! Defines the fundamental types for the Virtual File System:
//! - VfsPath: Normalized path wrapper
//! - VfsFile: File metadata and content
//! - VfsError: Error types matching Node.js semantics

use std::fmt;
use std::time::SystemTime;

/// Maximum allowed path length (4096 bytes)
pub const MAX_PATH_LENGTH: usize = 4096;

/// A normalized path for the VFS
///
/// Guarantees:
/// - No leading/trailing slashes
/// - No consecutive slashes
/// - No ".." segments (path traversal prevented)
/// - No null bytes
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsPath(String);

impl VfsPath {
    /// Create a new VfsPath from a string, normalizing and validating it
    pub fn new(path: impl AsRef<str>) -> Result<Self, VfsError> {
        let path = path.as_ref();

        // Check length limit
        if path.len() > MAX_PATH_LENGTH {
            return Err(VfsError::InvalidPath {
                path: path.to_string(),
                reason: format!("Path exceeds maximum length of {} bytes", MAX_PATH_LENGTH),
            });
        }

        // Check for null bytes
        if path.contains('\0') {
            return Err(VfsError::InvalidPath {
                path: path.to_string(),
                reason: "Path contains null bytes".to_string(),
            });
        }

        // Normalize the path
        let normalized = Self::normalize(path);

        // Check for traversal attempts after normalization
        // Only reject ".." as a full path segment, not as part of filenames
        if normalized.split('/').any(|segment| segment == "..") {
            return Err(VfsError::InvalidPath {
                path: path.to_string(),
                reason: "Path contains '..' segment which is not allowed".to_string(),
            });
        }

        // Empty path check
        if normalized.is_empty() {
            return Err(VfsError::InvalidPath {
                path: path.to_string(),
                reason: "Path is empty after normalization".to_string(),
            });
        }

        Ok(Self(normalized))
    }

    /// Normalize a path string:
    /// - Strip leading/trailing slashes
    /// - Collapse multiple slashes to single
    fn normalize(path: &str) -> String {
        path.trim_start_matches('/')
            .trim_end_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Get the path as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the parent directory path, if any
    pub fn parent(&self) -> Option<Self> {
        self.0.rfind('/').map(|idx| {
            let parent = &self.0[..idx];
            Self(parent.to_string())
        })
    }

    /// Get the file name component, if any
    pub fn file_name(&self) -> Option<&str> {
        self.0
            .rfind('/')
            .map(|idx| &self.0[idx + 1..])
            .or(Some(&self.0))
    }

    /// Join with another path component
    pub fn join(&self, other: impl AsRef<str>) -> Result<Self, VfsError> {
        let other = other.as_ref();
        let combined = if self.0.is_empty() {
            other.to_string()
        } else {
            format!("{}/{}", self.0, other)
        };
        Self::new(combined)
    }
}

impl fmt::Display for VfsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for VfsPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<VfsPath> for String {
    fn from(path: VfsPath) -> Self {
        path.0
    }
}

/// File metadata and content
#[derive(Debug, Clone)]
pub struct VfsFile {
    /// File content as raw bytes
    pub content: Vec<u8>,
    /// File creation time
    pub created_at: SystemTime,
    /// Last modification time
    pub modified_at: SystemTime,
    /// File size in bytes (redundant with content.len() but convenient)
    pub size: usize,
}

impl VfsFile {
    /// Create a new VfsFile with current timestamp
    pub fn new(content: Vec<u8>) -> Self {
        let now = SystemTime::now();
        let size = content.len();
        Self {
            content,
            created_at: now,
            modified_at: now,
            size,
        }
    }

    /// Update content and modified timestamp
    pub fn update_content(&mut self, content: Vec<u8>) {
        self.size = content.len();
        self.content = content;
        self.modified_at = SystemTime::now();
    }
}

/// Error types for VFS operations
///
/// Error codes match Node.js fs error conventions:
/// - ENOENT: File not found
/// - EACCES: Permission denied
/// - EEXIST: File already exists
/// - EINVAL: Invalid argument/path
/// - EQUOTA: Quota exceeded (NANO-specific)
/// - EIO: Generic I/O error
#[derive(Debug, Clone, thiserror::Error)]
pub enum VfsError {
    /// File or directory not found (ENOENT)
    #[error("ENOENT: no such file or directory: {path}")]
    NotFound { path: String },

    /// Permission denied (EACCES)
    #[error("EACCES: permission denied: {path}")]
    PermissionDenied { path: String },

    /// File already exists (EEXIST)
    #[error("EEXIST: file already exists: {path}")]
    AlreadyExists { path: String },

    /// Invalid path or argument (EINVAL)
    #[error("EINVAL: {reason}: {path}")]
    InvalidPath { path: String, reason: String },

    /// Resource quota exceeded (EQUOTA)
    #[error("EQUOTA: quota exceeded for {resource}: limit={limit}, current={current}")]
    QuotaExceeded {
        resource: String,
        limit: u32,
        current: u32,
    },

    /// Generic I/O error (EIO)
    #[error("EIO: I/O error: {0}")]
    IoError(String),

    /// Operation not supported (ENOTSUP)
    #[error("ENOTSUP: operation not supported: {feature}")]
    NotSupported { feature: String },
}

impl VfsError {
    /// Get the Node.js-style error code
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "ENOENT",
            Self::PermissionDenied { .. } => "EACCES",
            Self::AlreadyExists { .. } => "EEXIST",
            Self::InvalidPath { .. } => "EINVAL",
            Self::QuotaExceeded { .. } => "EQUOTA",
            Self::IoError(..) => "EIO",
            Self::NotSupported { .. } => "ENOTSUP",
        }
    }

    /// Get the path associated with this error, if any
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::NotFound { path } => Some(path),
            Self::PermissionDenied { path } => Some(path),
            Self::AlreadyExists { path } => Some(path),
            Self::InvalidPath { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// Result type alias for VFS operations
pub type VfsResult<T> = Result<T, VfsError>;

/// Resource limits for VFS operations
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    /// Maximum size of a single file in bytes (default: 10MB)
    pub file_size_bytes_max: u32,
    /// Maximum total storage per namespace in bytes (default: 100MB)
    pub total_storage_bytes_max: u32,
    /// Maximum number of files per namespace (default: 1000)
    pub files_count_max: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            file_size_bytes_max: 10 * 1024 * 1024,      // 10MB
            total_storage_bytes_max: 100 * 1024 * 1024, // 100MB
            files_count_max: 1000,
        }
    }
}

impl ResourceLimits {
    /// Create limits with custom values
    pub fn new(
        file_size_bytes_max: u32,
        total_storage_bytes_max: u32,
        files_count_max: u32,
    ) -> Self {
        Self {
            file_size_bytes_max,
            total_storage_bytes_max,
            files_count_max,
        }
    }

    /// Create limits for testing (very small limits)
    pub fn test_limits() -> Self {
        Self {
            file_size_bytes_max: 100,     // 100 bytes
            total_storage_bytes_max: 500, // 500 bytes
            files_count_max: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vfs_path_normalization() {
        assert_eq!(VfsPath::new("/foo/bar").unwrap().as_str(), "foo/bar");
        assert_eq!(VfsPath::new("foo/bar/").unwrap().as_str(), "foo/bar");
        assert_eq!(VfsPath::new("//foo//bar//").unwrap().as_str(), "foo/bar");
        assert_eq!(VfsPath::new("foo//bar").unwrap().as_str(), "foo/bar");
    }

    #[test]
    fn test_vfs_path_traversal_rejected() {
        assert!(VfsPath::new("../etc/passwd").is_err());
        assert!(VfsPath::new("foo/../../etc/passwd").is_err());
        assert!(VfsPath::new("..").is_err());
        assert!(VfsPath::new("foo/..").is_err());
    }

    #[test]
    fn test_vfs_path_special_characters_allowed() {
        // Framework route patterns with [...] should be allowed
        assert!(VfsPath::new("blog/[...slug].astro").is_ok());
        assert!(VfsPath::new("routes/[[...optional]].ts").is_ok());
        assert!(VfsPath::new("[...path].js").is_ok());

        // Files with .. in name (not as segment) should be allowed
        assert!(VfsPath::new("file..name.txt").is_ok());
        assert!(VfsPath::new("foo..bar.js").is_ok());
        assert!(VfsPath::new("test...file.md").is_ok());

        // Single . in filenames should be fine
        assert!(VfsPath::new("file.name.txt").is_ok());
    }

    #[test]
    fn test_vfs_path_null_bytes_rejected() {
        assert!(VfsPath::new("file\0.txt").is_err());
    }

    #[test]
    fn test_vfs_path_empty_rejected() {
        assert!(VfsPath::new("").is_err());
        assert!(VfsPath::new("/").is_err());
    }

    #[test]
    fn test_vfs_path_join() {
        let base = VfsPath::new("foo").unwrap();
        assert_eq!(base.join("bar").unwrap().as_str(), "foo/bar");
        assert_eq!(base.join("bar/baz").unwrap().as_str(), "foo/bar/baz");
    }

    #[test]
    fn test_vfs_path_parent() {
        let path = VfsPath::new("foo/bar/baz").unwrap();
        assert_eq!(path.parent().unwrap().as_str(), "foo/bar");

        let path = VfsPath::new("foo").unwrap();
        assert!(path.parent().is_none());
    }

    #[test]
    fn test_vfs_path_file_name() {
        let path = VfsPath::new("foo/bar.txt").unwrap();
        assert_eq!(path.file_name(), Some("bar.txt"));

        let path = VfsPath::new("foo").unwrap();
        assert_eq!(path.file_name(), Some("foo"));
    }

    #[test]
    fn test_vfs_error_codes() {
        assert_eq!(
            VfsError::NotFound {
                path: "/x".to_string()
            }
            .code(),
            "ENOENT"
        );
        assert_eq!(
            VfsError::PermissionDenied {
                path: "/x".to_string()
            }
            .code(),
            "EACCES"
        );
        assert_eq!(
            VfsError::AlreadyExists {
                path: "/x".to_string()
            }
            .code(),
            "EEXIST"
        );
        assert_eq!(
            VfsError::InvalidPath {
                path: "/x".to_string(),
                reason: "x".to_string()
            }
            .code(),
            "EINVAL"
        );
        assert_eq!(
            VfsError::QuotaExceeded {
                resource: "x".to_string(),
                limit: 1,
                current: 2
            }
            .code(),
            "EQUOTA"
        );
        assert_eq!(VfsError::IoError("x".to_string()).code(), "EIO");
    }

    #[test]
    fn test_resource_limits_default() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.file_size_bytes_max, 10 * 1024 * 1024);
        assert_eq!(limits.total_storage_bytes_max, 100 * 1024 * 1024);
        assert_eq!(limits.files_count_max, 1000);
    }
}
