use thiserror::Error;

/// Error type for WASM operations
#[derive(Error, Debug, Clone)]
pub enum WasmError {
    #[error("Failed to compile WASM module: {0}")]
    CompileError(String),

    #[error("Failed to instantiate WASM module: {0}")]
    InstantiationError(String),

    #[error("WASM module validation failed: {0}")]
    ValidationError(String),

    #[error("Memory limit exceeded: {used} > {max}")]
    MemoryLimitExceeded { used: u32, max: u32 },

    #[error("WASI error: {0}")]
    WasiError(String),

    #[error("Import not found: {module}.{name}")]
    ImportNotFound { module: String, name: String },
}
