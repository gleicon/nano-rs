//! WebAssembly support for NANO runtime
//!
//! Provides WASM module loading and JavaScript API bindings.
//! Integrates with VFS for loading WASM files and supports sliver snapshots.
//!
//! # Architecture
//!
//! - **Engine** (`engine.rs`): Core WASM compilation using V8's native APIs
//! - **Loader** (`loader.rs`): WASM file loading and validation
//! - **JavaScript API** (`js_api.rs`): WebAssembly.* bindings for JS code
//! - **Sliver** (`sliver.rs`): WASM caching in sliver snapshots

pub mod engine;
pub mod error;
pub mod js_api;
pub mod loader;
pub mod sliver;

pub use engine::{
    compile_module, compute_hash, global_wasm_cache, validate_wasm_bytes, WasmCompileError,
    WasmModuleCache, WasmValidationError,
};
pub use error::WasmError;
pub use js_api::WebAssemblyAPI;
pub use loader::WasmLoader;
pub use sliver::SliverWasmCache;

/// WASM module handle - stores the raw WASM bytes
#[derive(Debug, Clone)]
pub struct WasmModule {
    bytes: Vec<u8>,
    path: Option<String>,
}

impl WasmModule {
    /// Create a new WASM module from bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, WasmError> {
        WasmLoader::validate(&bytes)?;
        Ok(Self { bytes, path: None })
    }

    /// Create a new WASM module from bytes with a path
    pub fn from_bytes_with_path(
        bytes: Vec<u8>,
        path: impl Into<String>,
    ) -> Result<Self, WasmError> {
        WasmLoader::validate(&bytes)?;
        Ok(Self {
            bytes,
            path: Some(path.into()),
        })
    }

    /// Get the WASM bytes
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Get the module path if available
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Get the module size in bytes
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}
