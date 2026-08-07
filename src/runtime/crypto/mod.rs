//! WebCrypto API implementation for crypto.subtle
//!
//! This module provides the WebCrypto SubtleCrypto API implementation:
//! - CryptoKey object for managing cryptographic keys
//! - SubtleCrypto with generateKey, importKey, exportKey, encrypt, decrypt, sign, verify
//! - AES-GCM encryption/decryption
//! - HMAC signing/verification
//! - JWK key format import/export
//!
//! All cryptographic operations use the ring crate for safety and performance.

use thiserror::Error;

pub mod aes_gcm;
pub mod crypto_key;
pub mod ecdsa;
pub mod hmac;
pub mod rsa;
pub mod subtle;

pub use crypto_key::{
    AlgorithmIdentifier, CryptoKey, CryptoKeyHandle, HashAlgorithm, JwkObject, KeyUsage,
};
pub use subtle::SubtleCrypto;

/// Errors that can occur during cryptographic operations
#[derive(Error, Debug, Clone)]
pub enum CryptoError {
    #[error("Invalid algorithm: {0}")]
    InvalidAlgorithm(String),

    #[error("Invalid key")]
    InvalidKey,

    #[error("Operation failed")]
    OperationFailed,

    #[error("Not supported")]
    NotSupported,

    #[error("Invalid access")]
    InvalidAccess,

    #[error("Data error: {0}")]
    DataError(String),

    #[error("Syntax error: {0}")]
    SyntaxError(String),
}

/// Convert a CryptoError to a WebCrypto-compatible error type string
impl CryptoError {
    pub fn error_name(&self) -> &'static str {
        match self {
            CryptoError::InvalidAlgorithm(_) => "NotSupportedError",
            CryptoError::InvalidKey => "InvalidAccessError",
            CryptoError::OperationFailed => "OperationError",
            CryptoError::NotSupported => "NotSupportedError",
            CryptoError::InvalidAccess => "InvalidAccessError",
            CryptoError::DataError(_) => "DataError",
            CryptoError::SyntaxError(_) => "SyntaxError",
        }
    }
}
