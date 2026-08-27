//! Per-Isolate VFS Integration
//!
//! Provides the IsolateVfs wrapper that attaches a VFS namespace to each isolate.
//! This module implements the per-isolate filesystem isolation required for
//! multi-tenant security.

use crate::vfs::types::{VfsPath, VfsResult};

/// A namespace for VFS isolation
///
/// Derived from the application hostname, this ensures each app
/// has an isolated filesystem that cannot access other apps' files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsNamespace(String);

impl VfsNamespace {
    /// Create a namespace from an application hostname.
    ///
    /// The mapping is **injective**: distinct hostnames always yield distinct
    /// namespaces. This is a tenant boundary — the namespace becomes a directory
    /// name on the disk backend and a key prefix on the memory backend, so a
    /// collision would let one tenant read/write another's files.
    ///
    /// The old scheme replaced both `.` and `-` with `_`, so `a.com` and `a-com`
    /// both mapped to `a_com` — a cross-tenant merge. Here, the DNS charset
    /// `[a-z0-9.-]` is preserved verbatim (keeping `.` and `-` distinct) and any
    /// other byte is escaped as `_XX_` (lowercase hex), which is reversible and
    /// collision-free. `_` itself is escaped so an attacker can't forge an
    /// escape sequence. The traversal-sensitive results (`.`, `..`, empty) are
    /// fully hex-escaped so the namespace is always a single safe path segment.
    pub fn from_hostname(hostname: &str) -> Self {
        let lower = hostname.to_lowercase();
        let mut out = String::with_capacity(lower.len());
        for &b in lower.as_bytes() {
            match b {
                b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' => out.push(b as char),
                _ => out.push_str(&format!("_{b:02x}_")),
            }
        }
        // Guard the path-traversal / empty cases: a namespace of "", "." or ".."
        // would resolve to the base dir or its parent. Escape the whole thing.
        if out.is_empty() || out == "." || out == ".." {
            out = lower
                .as_bytes()
                .iter()
                .map(|b| format!("_{b:02x}_"))
                .collect();
            if out.is_empty() {
                out.push_str("_empty_");
            }
        }
        Self(out)
    }

    /// Get the namespace as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VfsNamespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

use std::fmt;

/// Per-isolate VFS wrapper
///
/// Combines a namespace with a backend to provide isolated filesystem
/// access for a single isolate. This is owned by NanoIsolate.
#[derive(Clone, Debug)]
pub struct IsolateVfs {
    namespace: VfsNamespace,
    backend: crate::vfs::VfsBackendEnum,
}

impl IsolateVfs {
    /// Create a new IsolateVfs with the given namespace and backend
    pub fn new(namespace: VfsNamespace, backend: crate::vfs::VfsBackendEnum) -> Self {
        Self { namespace, backend }
    }

    /// Get the namespace
    pub fn namespace(&self) -> &VfsNamespace {
        &self.namespace
    }

    /// Get the backend reference
    pub fn backend(&self) -> &crate::vfs::VfsBackendEnum {
        &self.backend
    }

    /// Read a file from the isolate's namespace
    pub async fn read(&self, path: impl AsRef<str>) -> VfsResult<Vec<u8>> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.read(&storage_path).await
    }

    /// Write a file to the isolate's namespace
    pub async fn write(&self, path: impl AsRef<str>, content: &[u8]) -> VfsResult<()> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.write(&storage_path, content).await
    }

    /// Check if a file exists in the isolate's namespace
    pub async fn exists(&self, path: impl AsRef<str>) -> VfsResult<bool> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.exists(&storage_path).await
    }

    /// Delete a file from the isolate's namespace
    pub async fn delete(&self, path: impl AsRef<str>) -> VfsResult<()> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.delete(&storage_path).await
    }

    /// Get file metadata from the isolate's namespace
    pub async fn metadata(&self, path: impl AsRef<str>) -> VfsResult<crate::vfs::types::VfsFile> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.metadata(&storage_path).await
    }

    /// List directory entries from the isolate's namespace
    pub async fn list_dir(&self, path: impl AsRef<str>) -> VfsResult<Vec<VfsPath>> {
        let storage_path = self.prefix_namespace(path.as_ref())?;
        self.backend.list_dir(&storage_path).await
    }

    /// Prefix a path with the isolate's namespace
    fn prefix_namespace(&self, path: &str) -> VfsResult<VfsPath> {
        let path = VfsPath::new(path)?;
        let ns = self.namespace.as_str();
        // DiskBackend has per-app base_path configured by the factory; namespace
        // isolation is already implicit in the directory hierarchy. Adding a
        // hostname-derived subdirectory would break callers that place files
        // directly under base_path (the common case for disk-backed apps).
        // MemoryBackend is shared in-process so the prefix IS needed for tenant isolation.
        if ns.is_empty() || matches!(self.backend, crate::vfs::VfsBackendEnum::Disk(_)) {
            return Ok(path);
        }
        VfsPath::new(format!("{}::{}", ns, path.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::MemoryBackend;

    #[test]
    fn namespace_from_hostname_is_injective_for_dot_dash() {
        // Regression: `.` and `-` both used to collapse to `_`, merging tenants.
        assert_ne!(
            VfsNamespace::from_hostname("a.com"),
            VfsNamespace::from_hostname("a-com"),
        );
        assert_ne!(
            VfsNamespace::from_hostname("tenant-a.example"),
            VfsNamespace::from_hostname("tenant.a.example"),
        );
        // DNS charset is preserved verbatim (case-folded).
        assert_eq!(VfsNamespace::from_hostname("A.com").as_str(), "a.com");
    }

    #[test]
    fn namespace_from_hostname_escapes_traversal_and_separators() {
        // A namespace must always be a single, non-traversing path segment.
        for h in ["..", ".", "", "/", "../..", "a/b", "a::b", "a\\b"] {
            let ns = VfsNamespace::from_hostname(h).0;
            assert!(ns != "." && ns != ".." && !ns.is_empty(), "unsafe ns for {h:?}: {ns:?}");
            assert!(!ns.contains('/'), "path separator leaked for {h:?}: {ns:?}");
            assert!(!ns.contains('\\'), "backslash leaked for {h:?}: {ns:?}");
            assert!(!ns.contains("::"), "ns separator leaked for {h:?}: {ns:?}");
        }
        // Distinct escaped inputs stay distinct.
        assert_ne!(
            VfsNamespace::from_hostname("a/b"),
            VfsNamespace::from_hostname("a\\b"),
        );
        // `_` is escaped so a literal underscore can't forge an escape token.
        assert_ne!(
            VfsNamespace::from_hostname("a_2e_b"),
            VfsNamespace::from_hostname("a.b"),
        );
    }

    #[tokio::test]
    async fn test_isolate_vfs_basic() {
        let backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

        // Write
        vfs.write("/config.json", b"{\"key\": \"value\"}")
            .await
            .unwrap();

        // Read
        let content = vfs.read("/config.json").await.unwrap();
        assert_eq!(content, b"{\"key\": \"value\"}");

        // Exists
        assert!(vfs.exists("/config.json").await.unwrap());
        assert!(!vfs.exists("/missing.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_isolate_vfs_namespace_isolation() {
        let shared_backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());

        // Two isolates with different namespaces sharing the same backend
        let vfs_a = IsolateVfs::new(
            VfsNamespace::from_hostname("app-a.example.com"),
            shared_backend.clone(),
        );

        let vfs_b = IsolateVfs::new(
            VfsNamespace::from_hostname("app-b.example.com"),
            shared_backend.clone(),
        );

        // Write in app A
        vfs_a.write("/secret.txt", b"app-a-secret").await.unwrap();

        // App B cannot read
        let result = vfs_b.read("/secret.txt").await;
        assert!(matches!(
            result,
            Err(crate::vfs::types::VfsError::NotFound { .. })
        ));

        // App A can read
        let content = vfs_a.read("/secret.txt").await.unwrap();
        assert_eq!(content, b"app-a-secret");
    }

    #[tokio::test]
    async fn test_isolate_vfs_path_traversal_blocked() {
        let backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

        // Create a file
        vfs.write("/data/file.txt", b"content").await.unwrap();

        // Try traversal - should be blocked
        let result = vfs.read("../data/file.txt").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "EINVAL");

        let result = vfs.read("data/../../etc/passwd").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "EINVAL");
    }

    #[tokio::test]
    async fn test_isolate_vfs_unicode_paths() {
        let backend = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());
        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("test.example.com"), backend);

        // Unicode paths should work
        vfs.write("/文件.txt", b"content").await.unwrap();
        let content = vfs.read("/文件.txt").await.unwrap();
        assert_eq!(content, b"content");
    }

    /// DiskBackend has per-app base_path; namespace subdirs must NOT be created.
    /// Regression test for: readFile('test.txt') producing ENOENT localhost::test.txt
    #[tokio::test]
    async fn test_disk_backend_no_namespace_subdir() {
        use crate::vfs::DiskBackend;
        let temp_dir = tempfile::TempDir::new().unwrap();
        let disk = DiskBackend::new(temp_dir.path()).await.unwrap();
        let backend = crate::vfs::VfsBackendEnum::disk(disk);

        let vfs = IsolateVfs::new(VfsNamespace::from_hostname("localhost"), backend);

        // Write via IsolateVfs — should land at base_path/test.txt, not base_path/localhost/test.txt
        vfs.write("/test.txt", b"hello vfs").await.unwrap();

        // File must exist directly under base_path (no namespace subdir)
        let direct_path = temp_dir.path().join("test.txt");
        assert!(
            direct_path.exists(),
            "file should be at base_path/test.txt, not in a namespace subdir"
        );

        // Read back via API
        let content = vfs.read("/test.txt").await.unwrap();
        assert_eq!(content, b"hello vfs");
    }

    /// Standing guard for the disk-fallback isolation model. The disk backend
    /// does NOT apply the namespace prefix — isolation depends on each tenant
    /// being rooted at a distinct `base_path`. `WorkQueue::create_pool_with_fallback`
    /// derives that root as `base/{from_hostname(host)}`. This proves the collision
    /// pair `a.com` / `a-com` lands in separate subdirs and cannot read each other.
    #[tokio::test]
    async fn disk_fallback_per_tenant_subdir_isolates_collision_pair() {
        use crate::vfs::DiskBackend;
        let root = tempfile::TempDir::new().unwrap();

        let mk = |host: &str| {
            let sub = root.path().join(VfsNamespace::from_hostname(host).as_str());
            sub
        };
        let path_a = mk("a.com");
        let path_b = mk("a-com");
        assert_ne!(path_a, path_b, "collision pair must get distinct roots");

        let vfs_a = IsolateVfs::new(
            VfsNamespace::from_hostname("a.com"),
            crate::vfs::VfsBackendEnum::disk(DiskBackend::new(&path_a).await.unwrap()),
        );
        let vfs_b = IsolateVfs::new(
            VfsNamespace::from_hostname("a-com"),
            crate::vfs::VfsBackendEnum::disk(DiskBackend::new(&path_b).await.unwrap()),
        );

        vfs_a.write("/secret.txt", b"a-only").await.unwrap();
        assert!(
            vfs_b.read("/secret.txt").await.is_err(),
            "a-com must not read a.com's file"
        );
        assert_eq!(vfs_a.read("/secret.txt").await.unwrap(), b"a-only");
    }

    /// Memory backend still uses namespace prefix for multi-tenant isolation.
    #[tokio::test]
    async fn test_memory_backend_retains_namespace_isolation() {
        let shared = crate::vfs::VfsBackendEnum::memory(MemoryBackend::default());

        let vfs_a = IsolateVfs::new(VfsNamespace::from_hostname("app-a.com"), shared.clone());
        let vfs_b = IsolateVfs::new(VfsNamespace::from_hostname("app-b.com"), shared.clone());

        vfs_a.write("/data.txt", b"a-data").await.unwrap();

        // app-b must not see app-a's file
        assert!(vfs_b.read("/data.txt").await.is_err());
        assert_eq!(vfs_a.read("/data.txt").await.unwrap(), b"a-data");
    }
}
