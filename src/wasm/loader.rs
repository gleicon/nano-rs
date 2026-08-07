//! WASM module loading utilities

use crate::wasm::error::WasmError;
use std::path::Path;

/// Loader for WASM modules from various sources
pub struct WasmLoader;

impl WasmLoader {
    /// Load WASM module from filesystem path
    pub fn from_path(path: &str) -> Result<Vec<u8>, WasmError> {
        std::fs::read(path)
            .map_err(|e| WasmError::CompileError(format!("Failed to read {}: {}", path, e)))
    }

    /// Load WASM module from bytes (for inline/embedded)
    pub fn from_bytes(bytes: &[u8]) -> Vec<u8> {
        bytes.to_vec()
    }

    /// Validate WASM module without compiling
    pub fn validate(bytes: &[u8]) -> Result<(), WasmError> {
        // Basic magic number and version check
        if bytes.len() < 8 {
            return Err(WasmError::ValidationError(
                "File too small - minimum 8 bytes required".into(),
            ));
        }
        if &bytes[0..4] != b"\0asm" {
            return Err(WasmError::ValidationError(
                "Invalid WASM magic number - expected \\0asm".into(),
            ));
        }
        // Version check (support 1.0 and 2.0)
        let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if version != 1 && version != 2 {
            return Err(WasmError::ValidationError(format!(
                "Unsupported WASM version: {}",
                version
            )));
        }
        Ok(())
    }

    /// Check if a file path is a WASM module (has .wasm extension)
    pub fn is_wasm_file(path: &str) -> bool {
        Path::new(path)
            .extension()
            .map(|ext| ext.eq_ignore_ascii_case("wasm"))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_wasm() {
        // Minimal valid WASM module (magic + version)
        let wasm_bytes = b"\0asm\x01\x00\x00\x00";
        assert!(WasmLoader::validate(wasm_bytes).is_ok());

        // WASM v2.0
        let wasm_v2 = b"\0asm\x02\x00\x00\x00";
        assert!(WasmLoader::validate(wasm_v2).is_ok());
    }

    #[test]
    fn test_validate_invalid_wasm() {
        // Too small
        let too_small = b"\0asm";
        assert!(WasmLoader::validate(too_small).is_err());

        // Wrong magic
        let wrong_magic = b"wasm\x01\x00\x00\x00";
        assert!(WasmLoader::validate(wrong_magic).is_err());

        // Wrong version
        let wrong_version = b"\0asm\xFF\x00\x00\x00";
        assert!(WasmLoader::validate(wrong_version).is_err());
    }

    #[test]
    fn test_is_wasm_file() {
        assert!(WasmLoader::is_wasm_file("module.wasm"));
        assert!(WasmLoader::is_wasm_file("module.WASM"));
        assert!(WasmLoader::is_wasm_file("/path/to/module.wasm"));
        assert!(!WasmLoader::is_wasm_file("module.js"));
        assert!(!WasmLoader::is_wasm_file("module"));
    }
}
