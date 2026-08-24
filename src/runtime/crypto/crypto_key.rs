//! CryptoKey implementation for WebCrypto API
//!
//! CryptoKey represents a cryptographic key in the WebCrypto API.
//! It provides:
//! - Opaque key material storage (CryptoKeyHandle)
//! - Algorithm identifier
//! - Extractable flag for key export control
//! - Key usages (encrypt, decrypt, sign, verify, etc.)
//! - V8 JavaScript bindings

use std::sync::Arc;
use zeroize::Zeroize;

/// Algorithm identifiers supported by the WebCrypto implementation
#[derive(Debug, Clone, PartialEq)]
pub enum AlgorithmIdentifier {
    /// AES-GCM symmetric encryption
    AesGcm { length: u16 },
    /// HMAC message authentication
    Hmac {
        hash: HashAlgorithm,
        length: Option<u32>,
    },
    /// RSA-OAEP encryption
    RsaOaep { hash: HashAlgorithm },
    /// RSA-PSS signing
    RsaPss {
        hash: HashAlgorithm,
        salt_length: Option<u32>,
    },
    /// RSASSA-PKCS1-v1_5 signing
    RsaSsaPkcs1V1_5 { hash: HashAlgorithm },
    /// ECDSA signing
    Ecdsa {
        named_curve: String,
        hash: HashAlgorithm,
    },
    /// ECDH key agreement
    Ecdh { named_curve: String },
    /// PBKDF2 key derivation
    Pbkdf2,
}

impl AlgorithmIdentifier {
    /// Get the algorithm name as a string (for JS exposure)
    pub fn name(&self) -> &'static str {
        match self {
            AlgorithmIdentifier::AesGcm { .. } => "AES-GCM",
            AlgorithmIdentifier::Hmac { .. } => "HMAC",
            AlgorithmIdentifier::RsaOaep { .. } => "RSA-OAEP",
            AlgorithmIdentifier::RsaPss { .. } => "RSA-PSS",
            AlgorithmIdentifier::RsaSsaPkcs1V1_5 { .. } => "RSASSA-PKCS1-v1_5",
            AlgorithmIdentifier::Ecdsa { .. } => "ECDSA",
            AlgorithmIdentifier::Ecdh { .. } => "ECDH",
            AlgorithmIdentifier::Pbkdf2 => "PBKDF2",
        }
    }

    /// Get the key length in bits, if applicable
    pub fn key_length(&self) -> Option<u16> {
        match self {
            AlgorithmIdentifier::AesGcm { length } => Some(*length),
            AlgorithmIdentifier::Hmac { length, .. } => length.map(|l| l as u16),
            _ => None,
        }
    }
}

/// Hash algorithms for HMAC
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HashAlgorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlgorithm {
    /// Get the hash algorithm name as a string (for JS exposure)
    pub fn name(&self) -> &'static str {
        match self {
            HashAlgorithm::Sha256 => "SHA-256",
            HashAlgorithm::Sha384 => "SHA-384",
            HashAlgorithm::Sha512 => "SHA-512",
        }
    }

    /// Get the hash output size in bytes
    pub fn output_size(&self) -> usize {
        match self {
            HashAlgorithm::Sha256 => 32,
            HashAlgorithm::Sha384 => 48,
            HashAlgorithm::Sha512 => 64,
        }
    }

    /// Parse a hash algorithm name from a string
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SHA-256" => Some(HashAlgorithm::Sha256),
            "SHA-384" => Some(HashAlgorithm::Sha384),
            "SHA-512" => Some(HashAlgorithm::Sha512),
            _ => None,
        }
    }
}

/// Key usage flags per WebCrypto spec
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyUsage {
    Encrypt,
    Decrypt,
    Sign,
    Verify,
    DeriveKey,
    DeriveBits,
    WrapKey,
    UnwrapKey,
}

impl KeyUsage {
    /// Get the key usage as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyUsage::Encrypt => "encrypt",
            KeyUsage::Decrypt => "decrypt",
            KeyUsage::Sign => "sign",
            KeyUsage::Verify => "verify",
            KeyUsage::DeriveKey => "deriveKey",
            KeyUsage::DeriveBits => "deriveBits",
            KeyUsage::WrapKey => "wrapKey",
            KeyUsage::UnwrapKey => "unwrapKey",
        }
    }

    /// Parse a key usage from a string
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "encrypt" => Some(KeyUsage::Encrypt),
            "decrypt" => Some(KeyUsage::Decrypt),
            "sign" => Some(KeyUsage::Sign),
            "verify" => Some(KeyUsage::Verify),
            "deriveKey" => Some(KeyUsage::DeriveKey),
            "deriveBits" => Some(KeyUsage::DeriveBits),
            "wrapKey" => Some(KeyUsage::WrapKey),
            "unwrapKey" => Some(KeyUsage::UnwrapKey),
            _ => None,
        }
    }
}

/// Opaque handle to key material
///
/// This enum wraps the actual key bytes. The key material is stored as raw bytes
/// because ring's key types are not Send/Sync, but we need to move keys between threads.
/// The bytes are zeroed when the key is dropped (security consideration T-09-01).
#[derive(Debug, Clone)]
pub enum CryptoKeyHandle {
    /// AES-GCM key material (16, 24, or 32 bytes for 128/192/256 bits)
    AesGcmKey(Box<[u8]>),
    /// HMAC key material (variable length, at least hash output size per RFC 2104)
    HmacKey(Box<[u8]>),
    /// RSA private key in PKCS#8 DER format
    RsaPrivateKey(Vec<u8>),
    /// RSA public key in SPKI DER format
    RsaPublicKey(Vec<u8>),
    /// ECDSA private key in PKCS#8 DER format
    EcdsaPrivateKey(Vec<u8>),
    /// ECDSA public key in SPKI DER format
    EcdsaPublicKey(Vec<u8>),
    /// PBKDF2 password/key material (raw bytes)
    Pbkdf2Key(Box<[u8]>),
}

impl CryptoKeyHandle {
    /// Get the key material as bytes
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            CryptoKeyHandle::AesGcmKey(bytes) => bytes,
            CryptoKeyHandle::HmacKey(bytes) => bytes,
            CryptoKeyHandle::RsaPrivateKey(bytes) => bytes,
            CryptoKeyHandle::RsaPublicKey(bytes) => bytes,
            CryptoKeyHandle::EcdsaPrivateKey(bytes) => bytes,
            CryptoKeyHandle::EcdsaPublicKey(bytes) => bytes,
            CryptoKeyHandle::Pbkdf2Key(bytes) => bytes,
        }
    }

    /// Get the algorithm type for this key handle
    pub fn algorithm_type(&self) -> &'static str {
        match self {
            CryptoKeyHandle::AesGcmKey(_) => "AES-GCM",
            CryptoKeyHandle::HmacKey(_) => "HMAC",
            CryptoKeyHandle::RsaPrivateKey(_) => "RSA",
            CryptoKeyHandle::RsaPublicKey(_) => "RSA",
            CryptoKeyHandle::EcdsaPrivateKey(_) => "ECDSA",
            CryptoKeyHandle::EcdsaPublicKey(_) => "ECDSA",
            CryptoKeyHandle::Pbkdf2Key(_) => "PBKDF2",
        }
    }
}

impl Drop for CryptoKeyHandle {
    fn drop(&mut self) {
        // Zeroize key material when dropped (mitigation for T-09-01)
        match self {
            CryptoKeyHandle::AesGcmKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::HmacKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::RsaPrivateKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::RsaPublicKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::EcdsaPrivateKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::EcdsaPublicKey(bytes) => bytes.zeroize(),
            CryptoKeyHandle::Pbkdf2Key(bytes) => bytes.zeroize(),
        }
    }
}

/// WebCrypto CryptoKey object
///
/// Represents a cryptographic key that can be used with the SubtleCrypto API.
/// Keys have:
/// - Opaque key material (CryptoKeyHandle)
/// - Algorithm identifier
/// - Extractable flag (controls whether key can be exported)
/// - Key usages (what operations the key can perform)
#[derive(Debug, Clone)]
pub struct CryptoKey {
    /// Opaque handle to the key material (internal, not exposed to JS)
    pub handle: Arc<CryptoKeyHandle>,
    /// Algorithm identifier (exposed as readonly property)
    pub algorithm: AlgorithmIdentifier,
    /// Whether the key can be exported (exposed as readonly property)
    pub extractable: bool,
    /// Key usages (exposed as readonly property)
    pub usages: Vec<KeyUsage>,
}

impl CryptoKey {
    /// Create a new CryptoKey
    pub fn new(
        handle: CryptoKeyHandle,
        algorithm: AlgorithmIdentifier,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Self {
        Self {
            handle: Arc::new(handle),
            algorithm,
            extractable,
            usages,
        }
    }

    /// Get the key type ("secret" for symmetric, "public" or "private" for asymmetric)
    pub fn key_type(&self) -> &'static str {
        match self.handle.as_ref() {
            CryptoKeyHandle::AesGcmKey(_) => "secret",
            CryptoKeyHandle::HmacKey(_) => "secret",
            CryptoKeyHandle::RsaPrivateKey(_) => "private",
            CryptoKeyHandle::RsaPublicKey(_) => "public",
            CryptoKeyHandle::EcdsaPrivateKey(_) => "private",
            CryptoKeyHandle::EcdsaPublicKey(_) => "public",
            CryptoKeyHandle::Pbkdf2Key(_) => "secret",
        }
    }

    /// Check if the key has a specific usage
    pub fn has_usage(&self, usage: KeyUsage) -> bool {
        self.usages.contains(&usage)
    }

    /// Check if the key can be used for a specific operation
    pub fn can(&self, usage: KeyUsage) -> bool {
        self.has_usage(usage)
    }

    /// Create a new RSA CryptoKey from a private key in PKCS#8 format
    pub fn new_rsa_private(
        algorithm: AlgorithmIdentifier,
        private_key_pkcs8: Vec<u8>,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Self {
        Self {
            handle: Arc::new(CryptoKeyHandle::RsaPrivateKey(private_key_pkcs8)),
            algorithm,
            extractable,
            usages,
        }
    }

    /// Create a new RSA CryptoKey from a public key in SPKI format
    pub fn new_rsa_public(
        algorithm: AlgorithmIdentifier,
        public_key_spki: Vec<u8>,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Self {
        Self {
            handle: Arc::new(CryptoKeyHandle::RsaPublicKey(public_key_spki)),
            algorithm,
            extractable,
            usages,
        }
    }

    /// Create a new ECDSA CryptoKey from a private key in PKCS#8 format
    pub fn new_ecdsa_private(
        algorithm: AlgorithmIdentifier,
        private_key_pkcs8: Vec<u8>,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Self {
        Self {
            handle: Arc::new(CryptoKeyHandle::EcdsaPrivateKey(private_key_pkcs8)),
            algorithm,
            extractable,
            usages,
        }
    }

    /// Create a new ECDSA CryptoKey from a public key in SPKI format
    pub fn new_ecdsa_public(
        algorithm: AlgorithmIdentifier,
        public_key_spki: Vec<u8>,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Self {
        Self {
            handle: Arc::new(CryptoKeyHandle::EcdsaPublicKey(public_key_spki)),
            algorithm,
            extractable,
            usages,
        }
    }
}

/// JWK (JSON Web Key) structure for key import/export
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JwkObject {
    /// Key type ("oct" for symmetric keys)
    pub kty: String,
    /// Algorithm identifier
    pub alg: Option<String>,
    /// Key material (base64url encoded for symmetric keys)
    pub k: Option<String>,
    /// Extractable flag
    pub ext: Option<bool>,
    /// Key operations
    pub key_ops: Option<Vec<String>>,
}

impl JwkObject {
    /// Create a new JWK for a symmetric key
    pub fn new_symmetric(alg: &str, k: &str, extractable: bool, key_ops: Vec<String>) -> Self {
        Self {
            kty: "oct".to_string(),
            alg: Some(alg.to_string()),
            k: Some(k.to_string()),
            ext: Some(extractable),
            key_ops: Some(key_ops),
        }
    }

    /// Parse a JWK from JSON bytes
    pub fn from_slice(data: &[u8]) -> Result<Self, crate::runtime::crypto::CryptoError> {
        serde_json::from_slice(data).map_err(|e| {
            crate::runtime::crypto::CryptoError::DataError(format!("Invalid JWK: {}", e))
        })
    }

    /// Parse a JWK from a JavaScript object
    pub fn from_v8_object(
        scope: &mut v8::PinnedRef<v8::HandleScope>,
        obj: v8::Local<v8::Object>,
    ) -> Option<Self> {
        // Extract kty (required)
        let kty = Self::get_string_property(scope, obj, "kty")?;

        // Extract optional fields
        let alg = Self::get_string_property(scope, obj, "alg");
        let k = Self::get_string_property(scope, obj, "k");
        let ext = Self::get_bool_property(scope, obj, "ext");
        let key_ops = Self::get_string_array_property(scope, obj, "key_ops");

        Some(Self {
            kty,
            alg,
            k,
            ext,
            key_ops,
        })
    }

    /// Convert this JWK to a V8 JavaScript object
    pub fn to_v8_object(
        &self,
        scope: &mut v8::PinnedRef<v8::HandleScope>,
    ) -> Option<v8::Global<v8::Object>> {
        let obj = v8::Object::new(scope);

        // Set kty
        if let Some(key) = v8::String::new(scope, "kty") {
            if let Some(val) = v8::String::new(scope, &self.kty) {
                obj.set(scope, key.into(), val.into());
            }
        }

        // Set alg
        if let Some(ref alg) = self.alg {
            if let Some(key) = v8::String::new(scope, "alg") {
                if let Some(val) = v8::String::new(scope, alg) {
                    obj.set(scope, key.into(), val.into());
                }
            }
        }

        // Set k
        if let Some(ref k) = self.k {
            if let Some(key) = v8::String::new(scope, "k") {
                if let Some(val) = v8::String::new(scope, k) {
                    obj.set(scope, key.into(), val.into());
                }
            }
        }

        // Set ext
        if let Some(ext) = self.ext {
            if let Some(key) = v8::String::new(scope, "ext") {
                let val = v8::Boolean::new(scope, ext);
                obj.set(scope, key.into(), val.into());
            }
        }

        // Set key_ops
        if let Some(ref key_ops) = self.key_ops {
            if let Some(key) = v8::String::new(scope, "key_ops") {
                let arr = v8::Array::new(scope, key_ops.len() as i32);
                for (i, op) in key_ops.iter().enumerate() {
                    if let Some(op_str) = v8::String::new(scope, op) {
                        let idx = v8::Number::new(scope, i as f64);
                        arr.set(scope, idx.into(), op_str.into());
                    }
                }
                obj.set(scope, key.into(), arr.into());
            }
        }

        Some(v8::Global::new(scope, obj))
    }

    // Helper methods for property extraction
    fn get_string_property(
        scope: &mut v8::PinnedRef<v8::HandleScope>,
        obj: v8::Local<v8::Object>,
        name: &str,
    ) -> Option<String> {
        let key = v8::String::new(scope, name)?;
        let val = obj.get(scope, key.into())?;
        if val.is_undefined() || val.is_null() {
            return None;
        }
        let str_val = val.to_string(scope)?;
        Some(str_val.to_rust_string_lossy(scope))
    }

    fn get_bool_property(
        scope: &mut v8::PinnedRef<v8::HandleScope>,
        obj: v8::Local<v8::Object>,
        name: &str,
    ) -> Option<bool> {
        let key = v8::String::new(scope, name)?;
        let val = obj.get(scope, key.into())?;
        if val.is_undefined() || val.is_null() {
            return None;
        }
        Some(val.is_true())
    }

    fn get_string_array_property(
        scope: &mut v8::PinnedRef<v8::HandleScope>,
        obj: v8::Local<v8::Object>,
        name: &str,
    ) -> Option<Vec<String>> {
        let key = v8::String::new(scope, name)?;
        let val = obj.get(scope, key.into())?;
        if val.is_undefined() || val.is_null() {
            return None;
        }

        let arr = val.to_object(scope)?;
        let length_key = v8::String::new(scope, "length")?;
        let length_val = arr.get(scope, length_key.into())?;
        let length = length_val.to_number(scope)?.value() as usize;

        let mut result = Vec::with_capacity(length);
        for i in 0..length {
            let idx = v8::Number::new(scope, i as f64);
            let item = arr.get(scope, idx.into())?;
            let item_str = item.to_string(scope)?;
            result.push(item_str.to_rust_string_lossy(scope));
        }

        Some(result)
    }
}

/// Utility functions for base64url encoding/decoding
pub mod base64url {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    /// Encode bytes to base64url (no padding)
    pub fn encode(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Decode base64url to bytes
    pub fn decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
        URL_SAFE_NO_PAD.decode(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_identifier_name() {
        assert_eq!(
            AlgorithmIdentifier::AesGcm { length: 256 }.name(),
            "AES-GCM"
        );
        assert_eq!(
            AlgorithmIdentifier::Hmac {
                hash: HashAlgorithm::Sha256,
                length: None
            }
            .name(),
            "HMAC"
        );
    }

    #[test]
    fn test_hash_algorithm_from_name() {
        assert_eq!(
            HashAlgorithm::from_name("SHA-256"),
            Some(HashAlgorithm::Sha256)
        );
        assert_eq!(
            HashAlgorithm::from_name("SHA-384"),
            Some(HashAlgorithm::Sha384)
        );
        assert_eq!(
            HashAlgorithm::from_name("SHA-512"),
            Some(HashAlgorithm::Sha512)
        );
        assert_eq!(HashAlgorithm::from_name("invalid"), None);
    }

    #[test]
    fn test_key_usage_parsing() {
        assert_eq!(KeyUsage::from_str("encrypt"), Some(KeyUsage::Encrypt));
        assert_eq!(KeyUsage::from_str("decrypt"), Some(KeyUsage::Decrypt));
        assert_eq!(KeyUsage::from_str("sign"), Some(KeyUsage::Sign));
        assert_eq!(KeyUsage::from_str("verify"), Some(KeyUsage::Verify));
        assert_eq!(KeyUsage::from_str("invalid"), None);
    }

    #[test]
    fn test_crypto_key_creation() {
        let key_material = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let key = CryptoKey::new(
            CryptoKeyHandle::AesGcmKey(key_material.into_boxed_slice()),
            AlgorithmIdentifier::AesGcm { length: 128 },
            true,
            vec![KeyUsage::Encrypt, KeyUsage::Decrypt],
        );

        assert_eq!(key.key_type(), "secret");
        assert!(key.extractable);
        assert!(key.has_usage(KeyUsage::Encrypt));
        assert!(key.has_usage(KeyUsage::Decrypt));
        assert!(!key.has_usage(KeyUsage::Sign));
    }

    #[test]
    fn test_base64url_encoding() {
        let data = vec![0u8, 1, 2, 255, 254, 253];
        let encoded = base64url::encode(&data);
        let decoded = base64url::decode(&encoded).unwrap();
        assert_eq!(data, decoded);
    }
}
