//! WASM module support in sliver snapshots
//!
//! Enables pre-compiled WASM modules to be stored in slivers
//! for faster cold starts (skip compilation step).

use crate::wasm::WasmError;
use dashmap::DashMap;

/// Cache of compiled WASM modules for sliver storage
pub struct SliverWasmCache {
    /// Path -> compiled module data
    modules: DashMap<String, CompiledWasmModule>,
}

/// A WASM module stored in a sliver
#[derive(Clone, Debug)]
pub struct CompiledWasmModule {
    /// Original WASM bytes hash (for verification)
    pub source_hash: [u8; 32],
    /// WASM module bytes (source or pre-compiled)
    pub module_bytes: Vec<u8>,
    /// Module size in bytes
    pub size: usize,
    /// Module path
    pub path: String,
}

impl SliverWasmCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            modules: DashMap::new(),
        }
    }

    /// Check if module is cached
    pub fn get(&self, path: &str) -> Option<CompiledWasmModule> {
        self.modules.get(path).map(|m| m.clone())
    }

    /// Store a WASM module
    pub fn insert(&self, path: String, module: CompiledWasmModule) {
        self.modules.insert(path, module);
    }

    /// Add a module from raw bytes
    pub fn add_module(&self, path: impl Into<String>, bytes: Vec<u8>) {
        use sha2::{Digest, Sha256};

        let path = path.into();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let source_hash = hasher.finalize().into();
        let size = bytes.len();

        let module = CompiledWasmModule {
            source_hash,
            module_bytes: bytes,
            size,
            path: path.clone(),
        };

        self.modules.insert(path, module);
    }

    /// Serialize all cached modules for sliver storage
    ///
    /// Format: [count: u32][entries...]
    /// Each entry: [path_len: u32][path][hash: 32 bytes][bytes_len: u32][bytes]
    pub fn serialize(&self) -> Vec<u8> {
        let mut result = Vec::new();

        // Write count
        let count = self.modules.len() as u32;
        result.extend_from_slice(&count.to_le_bytes());

        // Write each module
        for entry in self.modules.iter() {
            let path = entry.key();
            let module = entry.value();

            // Path
            let path_bytes = path.as_bytes();
            result.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
            result.extend_from_slice(path_bytes);

            // Source hash (32 bytes)
            result.extend_from_slice(&module.source_hash);

            // Module bytes
            result.extend_from_slice(&(module.module_bytes.len() as u32).to_le_bytes());
            result.extend_from_slice(&module.module_bytes);
        }

        result
    }

    /// Deserialize from sliver bytes
    pub fn deserialize(bytes: &[u8]) -> Result<Self, WasmError> {
        let cache = Self::new();

        if bytes.len() < 4 {
            return Err(WasmError::CompileError(
                "Invalid WASM cache: too small".into(),
            ));
        }

        let mut offset = 0;

        // Read count
        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        offset += 4;

        for _ in 0..count {
            // Read path length
            if bytes.len() < offset + 4 {
                return Err(WasmError::CompileError(
                    "Invalid WASM cache: truncated".into(),
                ));
            }
            let path_len = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;

            // Read path
            if bytes.len() < offset + path_len {
                return Err(WasmError::CompileError(
                    "Invalid WASM cache: truncated path".into(),
                ));
            }
            let path = String::from_utf8_lossy(&bytes[offset..offset + path_len]).to_string();
            offset += path_len;

            // Read source hash (32 bytes)
            if bytes.len() < offset + 32 {
                return Err(WasmError::CompileError(
                    "Invalid WASM cache: truncated hash".into(),
                ));
            }
            let mut source_hash = [0u8; 32];
            source_hash.copy_from_slice(&bytes[offset..offset + 32]);
            offset += 32;

            // Read module bytes length
            if bytes.len() < offset + 4 {
                return Err(WasmError::CompileError(
                    "Invalid WASM cache: truncated length".into(),
                ));
            }
            let module_len = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;

            // Read module bytes
            if bytes.len() < offset + module_len {
                return Err(WasmError::CompileError(
                    "Invalid WASM cache: truncated data".into(),
                ));
            }
            let module_bytes = bytes[offset..offset + module_len].to_vec();
            offset += module_len;

            let size = module_bytes.len();

            let module = CompiledWasmModule {
                source_hash,
                module_bytes,
                size,
                path: path.clone(),
            };

            cache.modules.insert(path, module);
        }

        Ok(cache)
    }

    /// Get all module paths
    pub fn module_paths(&self) -> Vec<String> {
        self.modules.iter().map(|e| e.key().clone()).collect()
    }

    /// Total size of all cached modules
    pub fn total_size(&self) -> usize {
        self.modules.iter().map(|e| e.value().size).sum()
    }

    /// Module count
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }
}

impl Default for SliverWasmCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_serialization() {
        let cache = SliverWasmCache::new();

        let module = CompiledWasmModule {
            source_hash: [0u8; 32],
            module_bytes: b"\0asm\x01\x00\x00\x00".to_vec(),
            size: 8,
            path: "test.wasm".to_string(),
        };

        cache.insert("test.wasm".to_string(), module);

        // Serialize
        let serialized = cache.serialize();

        // Deserialize
        let restored = SliverWasmCache::deserialize(&serialized).unwrap();

        // Verify
        assert_eq!(restored.module_count(), 1);
        assert_eq!(restored.total_size(), 8);

        let restored_module = restored.get("test.wasm").unwrap();
        assert_eq!(restored_module.module_bytes, b"\0asm\x01\x00\x00\x00");
        assert_eq!(restored_module.size, 8);
    }

    #[test]
    fn test_empty_cache() {
        let cache = SliverWasmCache::new();
        let serialized = cache.serialize();
        let restored = SliverWasmCache::deserialize(&serialized).unwrap();
        assert_eq!(restored.module_count(), 0);
    }

    #[test]
    fn test_multiple_modules() {
        let cache = SliverWasmCache::new();

        for i in 0..3 {
            let module = CompiledWasmModule {
                source_hash: [i as u8; 32],
                module_bytes: vec![i; 10],
                size: 10,
                path: format!("module{}.wasm", i),
            };
            cache.insert(format!("module{}.wasm", i), module);
        }

        let serialized = cache.serialize();
        let restored = SliverWasmCache::deserialize(&serialized).unwrap();

        assert_eq!(restored.module_count(), 3);
        assert_eq!(restored.total_size(), 30);

        let paths = restored.module_paths();
        assert!(paths.contains(&"module0.wasm".to_string()));
        assert!(paths.contains(&"module1.wasm".to_string()));
        assert!(paths.contains(&"module2.wasm".to_string()));
    }

    #[test]
    fn test_add_module() {
        let cache = SliverWasmCache::new();
        cache.add_module("test.wasm", b"\0asm\x01\x00\x00\x00".to_vec());

        assert_eq!(cache.module_count(), 1);

        let module = cache.get("test.wasm").unwrap();
        assert_eq!(module.module_bytes, b"\0asm\x01\x00\x00\x00");
        assert_eq!(module.path, "test.wasm");
    }
}
