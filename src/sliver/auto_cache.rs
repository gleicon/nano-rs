//! Auto-sliver cache for transparent optimization
//!
//! Provides automatic generation and caching of slivers
//! to optimize cold start times. This is a transparent optimization -
//! users don't need to know about slivers, they just get faster starts.
//!
//! # Workflow
//!
//! 1. First request: Compile from source (~50-100ms), generate sliver
//! 2. Store sliver in cache directory
//! 3. Subsequent starts: Restore from sliver (~5-10ms)
//! 4. Cache invalidation: V8 version change, source file modification
//!
//! # Cache Location
//!
//! Default: `~/.nano/sliver-cache/<hostname>/<hash>.sliver`
//!
//! Where `<hash>` is a content hash of the source entrypoint.
//!
//! # Hot-Loading Architecture
//!
//! Running nano-rs instances can detect newly generated slivers and hot-load them
//! for subsequent requests. This enables:
//! - Zero-downtime optimization after first request
//! - Multi-process coordination (avoids duplicate sliver generation)
//! - Rolling restart support
//!
//! ## Multi-Process Coordination
//!
//! When running multiple nano-rs processes (e.g., 4 workers behind a load balancer):
//!
//! 1. **First process** receives request, compiles from source, generates sliver
//! 2. **File locking** prevents duplicate generation across processes
//! 3. **Hot-loading** allows other processes to detect and use the sliver
//! 4. **Consistent hashing** routes requests to processes with warm slivers
//!
//! ## Sliver Lifecycle
//!
//! ```text
//! Request 1: Source compilation (50-100ms) → Generate sliver (background)
//! Request 2+: Restore from sliver (5-10ms) ← Hot-loaded by all processes
//! Source modified: Invalidate cache → Back to Request 1 pattern
//! V8 update: Invalidate all caches → Back to Request 1 pattern
//! ```

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::sliver::{pack_sliver, unpack_sliver, SliverMetadata, UnpackedSliver};
use crate::sliver::vfs_capture::VfsCapture;
use crate::vfs::{VfsFile, VfsPath};

/// Source of JavaScript handler execution
///
/// Indicates whether to execute from source files or from a sliver.
#[derive(Debug, Clone)]
pub enum JsHandlerSource {
    /// Execute from JavaScript source files
    Source {
        /// Path to the JavaScript entrypoint
        entrypoint: String,
    },
    /// Execute from sliver (VFS-first, no heap snapshot)
    Sliver {
        /// Path to the JavaScript entrypoint
        entrypoint: String,
        /// Hostname for the app
        hostname: String,
    },
}

impl JsHandlerSource {
    /// Check if this source is a sliver
    pub fn is_sliver(&self) -> bool {
        matches!(self, Self::Sliver { .. })
    }
    
    /// Get the entrypoint path
    pub fn entrypoint(&self) -> &str {
        match self {
            Self::Source { entrypoint } => entrypoint.as_str(),
            Self::Sliver { entrypoint, .. } => entrypoint.as_str(),
        }
    }
}

/// Auto-sliver cache manager
#[derive(Debug, Clone)]
pub struct SliverCache {
    /// Root directory for sliver cache
    cache_dir: PathBuf,
}

impl SliverCache {
    /// Create a new sliver cache manager
    pub fn new() -> Result<Self> {
        let cache_dir = Self::default_cache_dir()?;
        
        // Ensure cache directory exists
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create sliver cache directory: {}", cache_dir.display()))?;
        
        Ok(Self {
            cache_dir,
        })
    }
    
    /// Create cache with custom directory
    pub fn with_dir(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir)
            .with_context(|| format!("Failed to create sliver cache directory: {}", cache_dir.display()))?;
        
        Ok(Self {
            cache_dir,
        })
    }
    
    /// Get the current V8 version (lazy - only called when needed)
    fn v8_version(&self) -> String {
        crate::sliver::validation::get_runtime_v8_version()
    }
    
    /// Get the default cache directory (~/.nano/sliver-cache)
    fn default_cache_dir() -> Result<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .context("Could not determine home directory")?;
        
        Ok(PathBuf::from(home).join(".nano").join("sliver-cache"))
    }
    
    /// Generate cache key from hostname and entrypoint
    ///
    /// The key includes:
    /// - Hostname (for isolation between apps)
    /// - Source file content hash (for invalidation on source change)
    fn cache_key(&self, hostname: &str, entrypoint: &str) -> String {
        let mut hasher = DefaultHasher::new();
        hostname.hash(&mut hasher);
        entrypoint.hash(&mut hasher);
        
        // Include modification time in hash for auto-invalidation
        if let Ok(metadata) = std::fs::metadata(entrypoint) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                    duration.as_secs().hash(&mut hasher);
                }
            }
        }
        
        format!("{:x}", hasher.finish())
    }
    
    /// Get the cache path for a sliver
    pub fn cache_path(&self, hostname: &str, entrypoint: &str) -> PathBuf {
        let key = self.cache_key(hostname, entrypoint);
        self.cache_dir.join(hostname).join(format!("{}.sliver", key))
    }

    /// Get the generation lock file path for a sliver
    ///
    /// Lock files indicate that another process is currently generating
    /// a sliver for this entrypoint.
    pub fn generation_lock_path(&self, hostname: &str, entrypoint: &str) -> PathBuf {
        let key = self.cache_key(hostname, entrypoint);
        self.cache_dir.join(hostname).join(format!("{}.sliver.lock", key))
    }

    /// Try to load a cached sliver
    ///
    /// Returns None if:
    /// - No cached sliver exists
    /// - Sliver is invalid/corrupted
    /// - V8 version mismatch
    /// - Source file has been modified
    pub fn try_load(&self, hostname: &str, entrypoint: &str) -> Option<Vec<u8>> {
        let cache_path = self.cache_path(hostname, entrypoint);
        
        if !cache_path.exists() {
            debug!("No cached sliver found for {}:{}", hostname, entrypoint);
            return None;
        }
        
        // Read sliver data
        let data = match std::fs::read(&cache_path) {
            Ok(data) => data,
            Err(e) => {
                warn!("Failed to read cached sliver {}: {}", cache_path.display(), e);
                return None;
            }
        };
        
        // Validate sliver by attempting to unpack and check V8 version
        match unpack_sliver(&data) {
            Ok(unpacked) => {
                // Compare V8 version from metadata with current runtime
                let sliver_v8 = &unpacked.metadata.nano_version;
                let current_v8 = self.v8_version();
                
                // Check major version compatibility
                let sliver_major = sliver_v8.split('.').next();
                let current_major = current_v8.split('.').next();
                
                if sliver_major != current_major {
                    warn!(
                        "V8 version mismatch: sliver compiled with {}, runtime is {}. Recompiling...",
                        sliver_v8, current_v8
                    );
                    return None;
                }
                
                debug!(
                    "Loaded valid cached sliver for {}:{} (V8: {})",
                    hostname, entrypoint, sliver_v8
                );
                
                Some(data)
            }
            Err(e) => {
                warn!("Failed to unpack cached sliver: {}", e);
                None
            }
        }
    }
    
    /// Store a sliver in the cache
    pub fn store(&self, hostname: &str, entrypoint: &str, data: &[u8]) -> Result<()> {
        let cache_path = self.cache_path(hostname, entrypoint);
        
        // Ensure parent directory exists
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create cache directory: {}", parent.display()))?;
        }
        
        // Write atomically (write to temp, then rename)
        let temp_path = cache_path.with_extension("tmp");
        std::fs::write(&temp_path, data)
            .with_context(|| format!("Failed to write sliver cache: {}", temp_path.display()))?;
        
        std::fs::rename(&temp_path, &cache_path)
            .with_context(|| format!("Failed to finalize sliver cache: {}", cache_path.display()))?;
        
        info!(
            "Stored sliver cache for {}:{} ({} bytes)",
            hostname, entrypoint, data.len()
        );
        
        Ok(())
    }
    
    /// Try to acquire a generation lock for sliver creation
    ///
    /// Returns true if lock was acquired, false if another process already holds it.
    pub fn try_acquire_generation_lock(&self, hostname: &str, entrypoint: &str) -> bool {
        let lock_path = self.generation_lock_path(hostname, entrypoint);
        
        // Ensure parent directory exists
        if let Some(parent) = lock_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        
        // Try to create lock file exclusively
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path) {
            Ok(file) => {
                // Write PID for debugging
                use std::io::Write;
                let _ = writeln!(&file, "{}", std::process::id());
                true
            }
            Err(_) => false,
        }
    }
    
    /// Release a generation lock
    ///
    /// Returns true if lock was released, false if it didn't exist.
    pub fn release_generation_lock(&self, hostname: &str, entrypoint: &str) -> bool {
        let lock_path = self.generation_lock_path(hostname, entrypoint);
        
        match std::fs::remove_file(&lock_path) {
            Ok(_) => {
                debug!("Released generation lock for {}:{}", hostname, entrypoint);
                true
            }
            Err(e) => {
                debug!("Failed to release generation lock for {}:{}: {}", hostname, entrypoint, e);
                false
            }
        }
    }
    
    /// Check if a sliver generation is in progress
    pub fn is_generation_in_progress(&self, hostname: &str, entrypoint: &str) -> bool {
        let lock_path = self.generation_lock_path(hostname, entrypoint);
        lock_path.exists()
    }
    
    /// Create a sliver from optional bytecode and VFS.
    pub fn create_sliver(
        &self,
        hostname: &str,
        entrypoint: &str,
        bytecode: Option<&[u8]>,
        vfs_capture: Option<&VfsCapture>,
    ) -> Result<Vec<u8>> {
        // Create metadata
        let mut metadata = SliverMetadata::new(hostname, &self.v8_version());
        metadata.name = Some(format!("auto-cache-{}", hostname));
        metadata.description = Some(format!(
            "Auto-generated sliver cache for {}\nEntrypoint: {}\nGenerated: {:?}",
            hostname,
            entrypoint,
            SystemTime::now()
        ));
        
        // Convert VFS capture to packable format
        let vfs_entries: Option<Vec<(VfsPath, VfsFile)>> = vfs_capture.map(|capture| {
            capture.files()
                .iter()
                .map(|(path, file)| {
                    (VfsPath::new(path).unwrap_or_else(|_| VfsPath::new("/unknown").unwrap()), file.clone())
                })
                .collect()
        });
        
        // Pack sliver
        let data = pack_sliver(&metadata, bytecode, vfs_entries.as_deref())
            .context("Failed to pack sliver")?;
        
        Ok(data)
    }
    
    /// Get cache statistics
    pub fn stats(&self) -> Result<SliverCacheStats> {
        let mut total_size = 0u64;
        let mut count = 0usize;
        
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() && entry.file_name().to_string_lossy().ends_with(".sliver") {
                        total_size += metadata.len();
                        count += 1;
                    }
                }
            }
        }
        
        Ok(SliverCacheStats {
            total_size_bytes: total_size,
            sliver_count: count,
            cache_dir: self.cache_dir.clone(),
        })
    }
    
    /// Clean up old cache entries (older than max_age)
    pub fn cleanup(&self, max_age: Duration) -> Result<usize> {
        let mut cleaned = 0usize;
        let now = SystemTime::now();

        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "sliver").unwrap_or(false) {
                    if let Ok(metadata) = entry.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(age) = now.duration_since(modified) {
                                if age > max_age {
                                    if let Err(e) = std::fs::remove_file(&path) {
                                        warn!("Failed to remove old cache file {}: {}", path.display(), e);
                                    } else {
                                        cleaned += 1;
                                        debug!("Cleaned old sliver cache: {}", path.display());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if cleaned > 0 {
            info!("Cleaned {} old sliver cache entries", cleaned);
        }

        Ok(cleaned)
    }

    /// Clear entire cache
    pub fn clear(&self) -> Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)
                .context("Failed to clear sliver cache")?;
            std::fs::create_dir_all(&self.cache_dir)?;
        }
        info!("Cleared sliver cache at {}", self.cache_dir.display());
        Ok(())
    }

    /// Get the modification time of a cached sliver
    ///
    /// Used for hot-loading detection - compare with app's source modification time.
    pub fn sliver_modified_time(&self, hostname: &str, entrypoint: &str) -> Option<SystemTime> {
        let cache_path = self.cache_path(hostname, entrypoint);
        std::fs::metadata(&cache_path).ok()?.modified().ok()
    }

    /// Load and unpack a sliver if it exists and is valid
    ///
    /// This is the hot-loading entry point - call this to check if a sliver
    /// has been generated by another process or earlier in this process's lifecycle.
    pub fn load_and_unpack(&self, hostname: &str, entrypoint: &str) -> Option<UnpackedSliver> {
        let data = self.try_load(hostname, entrypoint)?;
        match unpack_sliver(&data) {
            Ok(unpacked) => {
                info!(
                    "Hot-loaded sliver for {}:{} (bytecode: {}, {} vfs entries)",
                    hostname, entrypoint,
                    unpacked.bytecode.is_some(),
                    unpacked.vfs_entries.len()
                );
                Some(unpacked)
            }
            Err(e) => {
                warn!("Failed to unpack sliver for hot-loading: {}", e);
                None
            }
        }
    }
}

// Note: Default implementation removed to prevent static initialization issues
// Use SliverCache::new() or SliverCache::with_dir() explicitly instead

/// Cache statistics
#[derive(Debug, Clone)]
pub struct SliverCacheStats {
    /// Total cache size in bytes
    pub total_size_bytes: u64,
    /// Number of cached slivers
    pub sliver_count: usize,
    /// Cache directory path
    pub cache_dir: PathBuf,
}

/// Get or create JavaScript handler source with auto-sliver optimization
///
/// This is the main entry point for transparent sliver optimization.
/// It will:
/// 1. Check if a cached sliver exists for the entrypoint
/// 2. If yes, return Sliver source for fast restoration (hot-loading)
/// 3. If no, return Source and generate sliver in background
///
/// # Multi-Process Coordination
///
/// When multiple processes are running, they coordinate via file locks:
/// - First process acquires lock, generates sliver
/// - Other processes detect lock, use source temporarily, then hot-load
///
/// # Arguments
/// * `hostname` - The virtual hostname for the app
/// * `entrypoint` - Path to the JavaScript entrypoint file
/// * `prefer_sliver` - If true, wait briefly for generation if in progress
///
/// # Returns
/// JsHandlerSource::Sliver if cached sliver available, otherwise JsHandlerSource::Source
pub fn get_optimized_handler_source(
    hostname: &str,
    entrypoint: &str,
    prefer_sliver: bool,
) -> JsHandlerSource {
    let cache = match SliverCache::new() {
        Ok(cache) => cache,
        Err(e) => {
            warn!("Failed to initialize sliver cache: {}. Using source compilation.", e);
            return JsHandlerSource::Source {
                entrypoint: entrypoint.to_string(),
            };
        }
    };

    // Try to load cached sliver (hot-loading path)
    if let Some(data) = cache.try_load(hostname, entrypoint) {
        // Found valid cached sliver
        match unpack_sliver(&data) {
            Ok(unpacked) => {
                debug!(
                    "Using cached sliver for {}:{} ({} bytes)",
                    hostname, entrypoint, data.len()
                );

                return JsHandlerSource::Sliver {
                    entrypoint: entrypoint.to_string(),
                    hostname: hostname.to_string(),
                };
            }
            Err(e) => {
                warn!("Failed to unpack cached sliver: {}. Using source.", e);
            }
        }
    }

    // Check if another process is generating the sliver
    if cache.is_generation_in_progress(hostname, entrypoint) {
        if prefer_sliver {
            // Wait briefly for generation to complete (up to 100ms)
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(10));
                if let Some(data) = cache.try_load(hostname, entrypoint) {
                    if let Ok(unpacked) = unpack_sliver(&data) {
                        info!(
                            "Hot-loaded fresh sliver for {}:{} after waiting for generation",
                            hostname, entrypoint
                        );
                        return JsHandlerSource::Sliver {
                            entrypoint: entrypoint.to_string(),
                            hostname: hostname.to_string(),
                        };
                    }
                }
            }
            debug!("Timed out waiting for sliver generation, using source");
        } else {
            debug!("Sliver generation in progress for {}:{}, using source temporarily", hostname, entrypoint);
        }
    }

    // No valid cached sliver - use source compilation
    // The caller should generate and store the sliver after compilation
    debug!("No cached sliver for {}:{}. Using source compilation.", hostname, entrypoint);

    JsHandlerSource::Source {
        entrypoint: entrypoint.to_string(),
    }
}

/// Check if a sliver is available for hot-loading
///
/// Call this periodically to detect slivers generated by other processes
/// or earlier in this process's lifecycle.
pub fn is_sliver_available(hostname: &str, entrypoint: &str) -> bool {
    match SliverCache::new() {
        Ok(cache) => cache.cache_path(hostname, entrypoint).exists(),
        Err(_) => false,
    }
}

/// Store a generated sliver in the cache (source-only; no bytecode compilation here)
pub fn store_generated_sliver(
    hostname: &str,
    entrypoint: &str,
    vfs_capture: Option<&VfsCapture>,
) -> Result<()> {
    let cache = SliverCache::new()?;
    let sliver_data = cache.create_sliver(hostname, entrypoint, None, vfs_capture)?;
    cache.store(hostname, entrypoint, &sliver_data)?;
    Ok(())
}
