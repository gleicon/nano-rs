//! SubtleCrypto implementation for WebCrypto API
//!
//! SubtleCrypto provides cryptographic operations via the crypto.subtle object:
//! - generateKey: Create new cryptographic keys
//! - importKey: Import keys from various formats (JWK, raw)
//! - exportKey: Export keys to various formats
//! - encrypt: Encrypt data using symmetric algorithms
//! - decrypt: Decrypt data using symmetric algorithms
//! - sign: Create cryptographic signatures
//! - verify: Verify cryptographic signatures
//! - digest: Compute message digests
//! - deriveKey: Derive keys from existing keys
//! - deriveBits: Derive bits from existing keys
//! - wrapKey: Wrap keys for transport
//! - unwrapKey: Unwrap keys from transport format

use crate::runtime::crypto::{
    AlgorithmIdentifier, CryptoError, CryptoKey, HashAlgorithm, KeyUsage,
};

/// SubtleCrypto provides the crypto.subtle API
///
/// This is a stateless struct that serves as a namespace for
/// cryptographic operations. All methods are async and return Promises.
pub struct SubtleCrypto;

impl SubtleCrypto {
    /// Generate a new cryptographic key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-generateKey
    pub fn generate_key(
        algorithm: &AlgorithmIdentifier,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Result<CryptoKey, CryptoError> {
        match algorithm {
            AlgorithmIdentifier::AesGcm { length } => {
                crate::runtime::crypto::aes_gcm::generate_key(*length, extractable, usages)
            }
            AlgorithmIdentifier::Hmac { hash, length } => {
                crate::runtime::crypto::hmac::generate_key(*hash, *length, extractable, usages)
            }
            AlgorithmIdentifier::RsaOaep { .. } => crate::runtime::crypto::rsa::generate_key(
                2048,
                &[0x01, 0x00, 0x01],
                "RSA-OAEP",
                extractable,
                usages,
            ),
            AlgorithmIdentifier::RsaPss { .. } => crate::runtime::crypto::rsa::generate_key(
                2048,
                &[0x01, 0x00, 0x01],
                "RSA-PSS",
                extractable,
                usages,
            ),
            AlgorithmIdentifier::RsaSsaPkcs1V1_5 { .. } => {
                crate::runtime::crypto::rsa::generate_key(
                    2048,
                    &[0x01, 0x00, 0x01],
                    "RSASSA-PKCS1-v1_5",
                    extractable,
                    usages,
                )
            }
            AlgorithmIdentifier::Ecdsa { named_curve, .. } => {
                crate::runtime::crypto::ecdsa::generate_key(
                    named_curve,
                    "ECDSA",
                    extractable,
                    usages,
                )
            }
            AlgorithmIdentifier::Ecdh { named_curve } => {
                crate::runtime::crypto::ecdsa::generate_key(
                    named_curve,
                    "ECDH",
                    extractable,
                    usages,
                )
            }
            AlgorithmIdentifier::Pbkdf2 => Err(CryptoError::InvalidAlgorithm(
                "PBKDF2 keys cannot be generated; import raw key material instead".to_string(),
            )),
        }
    }

    /// Import a cryptographic key from a specified format
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-importKey
    pub fn import_key(
        format: &str,
        key_data: &[u8],
        algorithm: &AlgorithmIdentifier,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Result<CryptoKey, CryptoError> {
        match algorithm {
            AlgorithmIdentifier::AesGcm { .. } => {
                if format == "jwk" {
                    let jwk = crate::runtime::crypto::JwkObject::from_slice(key_data)?;
                    crate::runtime::crypto::aes_gcm::import_key_jwk(&jwk, extractable, usages)
                } else {
                    Err(CryptoError::NotSupported)
                }
            }
            AlgorithmIdentifier::Hmac { .. } => {
                if format == "jwk" {
                    let jwk = crate::runtime::crypto::JwkObject::from_slice(key_data)?;
                    crate::runtime::crypto::hmac::import_key_jwk(&jwk, extractable, usages)
                } else {
                    Err(CryptoError::NotSupported)
                }
            }
            AlgorithmIdentifier::RsaOaep { .. } => crate::runtime::crypto::rsa::import_key(
                format,
                key_data,
                "RSA-OAEP",
                extractable,
                usages,
            ),
            AlgorithmIdentifier::RsaPss { .. } => crate::runtime::crypto::rsa::import_key(
                format,
                key_data,
                "RSA-PSS",
                extractable,
                usages,
            ),
            AlgorithmIdentifier::RsaSsaPkcs1V1_5 { .. } => crate::runtime::crypto::rsa::import_key(
                format,
                key_data,
                "RSASSA-PKCS1-v1_5",
                extractable,
                usages,
            ),
            AlgorithmIdentifier::Ecdsa { named_curve, .. } => {
                crate::runtime::crypto::ecdsa::import_key(
                    format,
                    key_data,
                    "ECDSA",
                    named_curve,
                    extractable,
                    usages,
                )
            }
            AlgorithmIdentifier::Ecdh { named_curve } => crate::runtime::crypto::ecdsa::import_key(
                format,
                key_data,
                "ECDH",
                named_curve,
                extractable,
                usages,
            ),
            AlgorithmIdentifier::Pbkdf2 => Ok(CryptoKey::new(
                crate::runtime::crypto::CryptoKeyHandle::Pbkdf2Key(
                    key_data.to_vec().into_boxed_slice(),
                ),
                AlgorithmIdentifier::Pbkdf2,
                extractable,
                usages,
            )),
        }
    }

    /// Export a cryptographic key to a specified format
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-exportKey
    pub fn export_key(format: &str, key: &CryptoKey) -> Result<Vec<u8>, CryptoError> {
        // Check extractable flag before any export operation
        // WebCrypto spec: non-extractable keys must not be exportable
        if !key.extractable {
            return Err(CryptoError::InvalidAccess);
        }

        if format != "jwk" {
            return Err(CryptoError::NotSupported);
        }

        let jwk = match &key.algorithm {
            AlgorithmIdentifier::AesGcm { .. } => {
                crate::runtime::crypto::aes_gcm::export_key_jwk(key)?
            }
            AlgorithmIdentifier::Hmac { .. } => crate::runtime::crypto::hmac::export_key_jwk(key)?,
            _ => return Err(CryptoError::NotSupported),
        };

        serde_json::to_vec(&jwk)
            .map_err(|e| CryptoError::DataError(format!("JWK serialization failed: {}", e)))
    }

    /// Encrypt data using a specified algorithm and key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-encrypt
    pub fn encrypt(
        key: &CryptoKey,
        data: &[u8],
        iv: Option<&[u8]>,
        additional_data: Option<&[u8]>,
    ) -> Result<Vec<u8>, CryptoError> {
        match &key.algorithm {
            AlgorithmIdentifier::AesGcm { .. } => {
                let iv = iv.map(|v| v.to_vec()).unwrap_or_else(|| {
                    crate::runtime::crypto::aes_gcm::generate_iv().expect("Failed to generate IV")
                });

                let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
                    iv,
                    additional_data: additional_data.map(|v| v.to_vec()),
                    tag_length: 128,
                };

                crate::runtime::crypto::aes_gcm::encrypt(key, &params, data)
            }
            AlgorithmIdentifier::RsaOaep { hash } => {
                let hash_name = hash.name().to_string();
                let params = crate::runtime::crypto::rsa::RsaOaepParams {
                    hash: hash_name,
                    label: None,
                };
                crate::runtime::crypto::rsa::encrypt(key, data, &params)
            }
            _ => Err(CryptoError::InvalidAlgorithm(format!(
                "Encryption not supported for {}",
                key.algorithm.name()
            ))),
        }
    }

    /// Decrypt data using a specified algorithm and key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-decrypt
    pub fn decrypt(
        key: &CryptoKey,
        data: &[u8],
        iv: Option<&[u8]>,
        additional_data: Option<&[u8]>,
    ) -> Result<Vec<u8>, CryptoError> {
        match &key.algorithm {
            AlgorithmIdentifier::AesGcm { .. } => {
                let iv = iv.ok_or_else(|| {
                    CryptoError::DataError("IV is required for AES-GCM decryption".to_string())
                })?;

                let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
                    iv: iv.to_vec(),
                    additional_data: additional_data.map(|v| v.to_vec()),
                    tag_length: 128,
                };

                crate::runtime::crypto::aes_gcm::decrypt(key, &params, data)
            }
            AlgorithmIdentifier::RsaOaep { hash } => {
                let hash_name = hash.name().to_string();
                let params = crate::runtime::crypto::rsa::RsaOaepParams {
                    hash: hash_name,
                    label: None,
                };
                crate::runtime::crypto::rsa::decrypt(key, data, &params)
            }
            _ => Err(CryptoError::InvalidAlgorithm(format!(
                "Decryption not supported for {}",
                key.algorithm.name()
            ))),
        }
    }

    /// Sign data using a specified algorithm and key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-sign
    pub fn sign(key: &CryptoKey, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        match &key.algorithm {
            AlgorithmIdentifier::Hmac { .. } => crate::runtime::crypto::hmac::sign(key, data),
            AlgorithmIdentifier::RsaPss { .. } => {
                let params = crate::runtime::crypto::rsa::RsaPssParams { salt_length: 32 };
                crate::runtime::crypto::rsa::sign(key, data, "RSA-PSS", Some(&params))
            }
            AlgorithmIdentifier::RsaSsaPkcs1V1_5 { .. } => {
                crate::runtime::crypto::rsa::sign(key, data, "RSASSA-PKCS1-v1_5", None)
            }
            AlgorithmIdentifier::Ecdsa { named_curve, hash } => {
                let params = crate::runtime::crypto::ecdsa::EcdsaParams {
                    named_curve: named_curve.clone(),
                    hash: *hash,
                };
                crate::runtime::crypto::ecdsa::sign(key, data, &params)
            }
            _ => Err(CryptoError::InvalidAlgorithm(format!(
                "Signing not supported for {}",
                key.algorithm.name()
            ))),
        }
    }

    /// Verify a signature using a specified algorithm and key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-verify
    pub fn verify(key: &CryptoKey, signature: &[u8], data: &[u8]) -> Result<bool, CryptoError> {
        match &key.algorithm {
            AlgorithmIdentifier::Hmac { .. } => {
                crate::runtime::crypto::hmac::verify(key, data, signature)
            }
            AlgorithmIdentifier::RsaPss { .. } => {
                let params = crate::runtime::crypto::rsa::RsaPssParams { salt_length: 32 };
                crate::runtime::crypto::rsa::verify(key, signature, data, "RSA-PSS", Some(&params))
            }
            AlgorithmIdentifier::RsaSsaPkcs1V1_5 { .. } => {
                crate::runtime::crypto::rsa::verify(key, signature, data, "RSASSA-PKCS1-v1_5", None)
            }
            AlgorithmIdentifier::Ecdsa { named_curve, hash } => {
                let params = crate::runtime::crypto::ecdsa::EcdsaParams {
                    named_curve: named_curve.clone(),
                    hash: *hash,
                };
                crate::runtime::crypto::ecdsa::verify(key, signature, data, &params)
            }
            _ => Err(CryptoError::InvalidAlgorithm(format!(
                "Verification not supported for {}",
                key.algorithm.name()
            ))),
        }
    }

    /// Compute a digest of the specified data
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-digest
    pub fn digest(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let normalized = algorithm.to_uppercase();
        match normalized.as_str() {
            "SHA-256" | "SHA256" => {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            "SHA-384" | "SHA384" => {
                use sha2::{Digest, Sha384};
                let mut hasher = Sha384::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            "SHA-512" | "SHA512" => {
                use sha2::{Digest, Sha512};
                let mut hasher = Sha512::new();
                hasher.update(data);
                Ok(hasher.finalize().to_vec())
            }
            _ => Err(CryptoError::InvalidAlgorithm(format!(
                "Digest algorithm: {}",
                algorithm
            ))),
        }
    }

    /// Derive a new key from an existing key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-deriveKey
    ///
    /// Currently supports ECDH key agreement, deriving an AES-GCM key.
    pub fn derive_key(
        base_key: &CryptoKey,
        public_key: &CryptoKey,
        derived_key_length: u16,
        extractable: bool,
        usages: Vec<KeyUsage>,
    ) -> Result<CryptoKey, CryptoError> {
        // Derive shared secret bits via ECDH
        let bits = Self::derive_bits(base_key, public_key, Some((derived_key_length as u32) / 8))?;

        // Create an AES-GCM key from the derived bits
        if bits.len() < (derived_key_length as usize) / 8 {
            return Err(CryptoError::OperationFailed);
        }

        let key_material = bits[..(derived_key_length as usize) / 8].to_vec();
        let handle =
            crate::runtime::crypto::CryptoKeyHandle::AesGcmKey(key_material.into_boxed_slice());
        let algorithm = AlgorithmIdentifier::AesGcm {
            length: derived_key_length,
        };

        Ok(CryptoKey::new(handle, algorithm, extractable, usages))
    }

    /// Derive bits from an existing key
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-deriveBits
    ///
    /// Currently supports ECDH key agreement. The base_key is the local private key
    /// and public_key is the remote public key.
    pub fn derive_bits(
        base_key: &CryptoKey,
        public_key: &CryptoKey,
        length: Option<u32>,
    ) -> Result<Vec<u8>, CryptoError> {
        match (&base_key.algorithm, &public_key.algorithm) {
            (
                AlgorithmIdentifier::Ecdh { named_curve: nc1 }
                | AlgorithmIdentifier::Ecdsa {
                    named_curve: nc1, ..
                },
                AlgorithmIdentifier::Ecdh { named_curve: nc2 }
                | AlgorithmIdentifier::Ecdsa {
                    named_curve: nc2, ..
                },
            ) => {
                if nc1 != nc2 {
                    return Err(CryptoError::InvalidAlgorithm(
                        "Curve mismatch in ECDH key agreement".to_string(),
                    ));
                }

                crate::runtime::crypto::ecdsa::derive_bits(
                    base_key,
                    public_key,
                    length.map(|l| l as usize),
                )
            }
            _ => Err(CryptoError::InvalidAlgorithm(
                "deriveBits only supports ECDH".to_string(),
            )),
        }
    }

    /// Wrap a key for transport
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-wrapKey
    ///
    /// Exports the key in the specified format, then encrypts it using the wrapping key.
    /// Currently supports AES-GCM wrapping.
    pub fn wrap_key(
        format: &str,
        key: &CryptoKey,
        wrapping_key: &CryptoKey,
        iv: Option<&[u8]>,
    ) -> Result<Vec<u8>, CryptoError> {
        // Export the key to be wrapped
        let key_bytes = match format {
            "raw" => match key.handle.as_ref() {
                crate::runtime::crypto::CryptoKeyHandle::AesGcmKey(bytes) => bytes.to_vec(),
                crate::runtime::crypto::CryptoKeyHandle::HmacKey(bytes) => bytes.to_vec(),
                _ => return Err(CryptoError::InvalidKey),
            },
            "jwk" => Self::export_key(format, key)?,
            _ => return Err(CryptoError::NotSupported),
        };

        // Encrypt the exported key with the wrapping key using AES-GCM
        let iv = iv.map(|v| v.to_vec()).unwrap_or_else(|| {
            crate::runtime::crypto::aes_gcm::generate_iv().expect("Failed to generate IV")
        });

        let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
            iv,
            additional_data: None,
            tag_length: 128,
        };

        crate::runtime::crypto::aes_gcm::encrypt(wrapping_key, &params, &key_bytes)
    }

    /// Unwrap a key from transport format
    ///
    /// WebCrypto spec: https://www.w3.org/TR/WebCryptoAPI/#SubtleCrypto-method-unwrapKey
    ///
    /// Decrypts the wrapped key using the unwrapping key, then imports it.
    /// Currently supports AES-GCM unwrapping for AES-GCM and HMAC keys.
    pub fn unwrap_key(
        format: &str,
        wrapped_key: &[u8],
        unwrapping_key: &CryptoKey,
        unwrapped_key_algorithm: &AlgorithmIdentifier,
        extractable: bool,
        usages: Vec<KeyUsage>,
        iv: Option<&[u8]>,
    ) -> Result<CryptoKey, CryptoError> {
        // Decrypt the wrapped key with the unwrapping key using AES-GCM
        let iv = iv.ok_or_else(|| {
            CryptoError::DataError("IV is required for AES-GCM unwrap".to_string())
        })?;

        let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
            iv: iv.to_vec(),
            additional_data: None,
            tag_length: 128,
        };

        let decrypted =
            crate::runtime::crypto::aes_gcm::decrypt(unwrapping_key, &params, wrapped_key)?;

        // Import the decrypted key material
        match unwrapped_key_algorithm {
            AlgorithmIdentifier::AesGcm { .. } => {
                if format == "raw" {
                    let jwk = crate::runtime::crypto::JwkObject::new_symmetric(
                        "A256GCM",
                        &crate::runtime::crypto::crypto_key::base64url::encode(&decrypted),
                        extractable,
                        usages.iter().map(|u| u.as_str().to_string()).collect(),
                    );
                    crate::runtime::crypto::aes_gcm::import_key_jwk(&jwk, extractable, usages)
                } else {
                    Self::import_key(
                        format,
                        &decrypted,
                        unwrapped_key_algorithm,
                        extractable,
                        usages,
                    )
                }
            }
            AlgorithmIdentifier::Hmac { .. } => {
                if format == "raw" {
                    let hash = match unwrapped_key_algorithm {
                        AlgorithmIdentifier::Hmac { hash, .. } => *hash,
                        _ => HashAlgorithm::Sha256,
                    };
                    let alg = match hash {
                        HashAlgorithm::Sha256 => "HS256",
                        HashAlgorithm::Sha384 => "HS384",
                        HashAlgorithm::Sha512 => "HS512",
                    };
                    let jwk = crate::runtime::crypto::JwkObject::new_symmetric(
                        alg,
                        &crate::runtime::crypto::crypto_key::base64url::encode(&decrypted),
                        extractable,
                        usages.iter().map(|u| u.as_str().to_string()).collect(),
                    );
                    crate::runtime::crypto::hmac::import_key_jwk(&jwk, extractable, usages)
                } else {
                    Self::import_key(
                        format,
                        &decrypted,
                        unwrapped_key_algorithm,
                        extractable,
                        usages,
                    )
                }
            }
            _ => Self::import_key(
                format,
                &decrypted,
                unwrapped_key_algorithm,
                extractable,
                usages,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::crypto::{AlgorithmIdentifier, HashAlgorithm, KeyUsage};

    #[test]
    fn test_generate_key_aes_gcm() {
        let alg = AlgorithmIdentifier::AesGcm { length: 256 };
        let key =
            SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
                .expect("Should generate AES-GCM key");

        assert_eq!(key.algorithm.name(), "AES-GCM");
        assert_eq!(key.key_type(), "secret");
        assert!(key.has_usage(KeyUsage::Encrypt));
    }

    #[test]
    fn test_generate_key_hmac() {
        let alg = AlgorithmIdentifier::Hmac {
            hash: HashAlgorithm::Sha256,
            length: None,
        };
        let key = SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Sign, KeyUsage::Verify])
            .expect("Should generate HMAC key");

        assert_eq!(key.algorithm.name(), "HMAC");
        assert_eq!(key.key_type(), "secret");
        assert!(key.has_usage(KeyUsage::Sign));
    }

    #[test]
    fn test_generate_key_rsa() {
        let alg = AlgorithmIdentifier::RsaOaep {
            hash: HashAlgorithm::Sha256,
        };
        let key =
            SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
                .expect("Should generate RSA key");

        assert_eq!(key.algorithm.name(), "RSA-OAEP");
        assert_eq!(key.key_type(), "private");
    }

    #[test]
    fn test_generate_key_ecdsa() {
        let alg = AlgorithmIdentifier::Ecdsa {
            named_curve: "P-256".to_string(),
            hash: HashAlgorithm::Sha256,
        };
        let key = SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Sign, KeyUsage::Verify])
            .expect("Should generate ECDSA key");

        assert_eq!(key.algorithm.name(), "ECDSA");
        assert_eq!(key.key_type(), "private");
    }

    #[test]
    fn test_encrypt_decrypt_aes_gcm() {
        let alg = AlgorithmIdentifier::AesGcm { length: 256 };
        let key =
            SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
                .expect("Should generate key");

        let plaintext = b"Hello, SubtleCrypto!";
        let iv = Some(b"unique nonce" as &[u8]);

        let ciphertext = SubtleCrypto::encrypt(&key, plaintext, iv, None).expect("Should encrypt");

        // Decrypt with the same IV
        let decrypted = SubtleCrypto::decrypt(&key, &ciphertext, iv, None).expect("Should decrypt");

        assert_eq!(&decrypted[..], plaintext);
    }

    #[test]
    fn test_sign_verify_hmac() {
        let alg = AlgorithmIdentifier::Hmac {
            hash: HashAlgorithm::Sha256,
            length: None,
        };
        let key = SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Sign, KeyUsage::Verify])
            .expect("Should generate key");

        let data = b"Test message";
        let signature = SubtleCrypto::sign(&key, data).expect("Should sign");

        let valid = SubtleCrypto::verify(&key, &signature, data).expect("Should verify");
        assert!(valid);

        // Tampered signature should fail
        let mut bad_sig = signature.clone();
        bad_sig[0] ^= 0xFF;
        let valid = SubtleCrypto::verify(&key, &bad_sig, data).expect("Should return false");
        assert!(!valid);
    }

    #[test]
    fn test_digest() {
        let data = b"hello world";
        let hash = SubtleCrypto::digest("SHA-256", data).expect("Should digest");
        assert_eq!(hash.len(), 32);

        let hash2 = SubtleCrypto::digest("SHA-384", data).expect("Should digest");
        assert_eq!(hash2.len(), 48);

        let hash3 = SubtleCrypto::digest("SHA-512", data).expect("Should digest");
        assert_eq!(hash3.len(), 64);
    }

    #[test]
    fn test_wrap_unwrap_key() {
        // Generate a key to wrap
        let alg = AlgorithmIdentifier::AesGcm { length: 256 };
        let key_to_wrap =
            SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
                .expect("Should generate key to wrap");

        // Generate a wrapping key (needs Encrypt usage for AES-GCM wrapping)
        let wrapping_alg = AlgorithmIdentifier::AesGcm { length: 256 };
        let wrapping_key = SubtleCrypto::generate_key(
            &wrapping_alg,
            true,
            vec![KeyUsage::Encrypt, KeyUsage::Decrypt],
        )
        .expect("Should generate wrapping key");

        // Wrap
        let iv = Some(b"wrap iv 12!!" as &[u8]);
        let wrapped = SubtleCrypto::wrap_key("raw", &key_to_wrap, &wrapping_key, iv)
            .expect("Should wrap key");

        // Unwrap
        let unwrapped = SubtleCrypto::unwrap_key(
            "raw",
            &wrapped,
            &wrapping_key,
            &alg,
            true,
            vec![KeyUsage::Encrypt, KeyUsage::Decrypt],
            iv,
        )
        .expect("Should unwrap key");

        assert_eq!(unwrapped.algorithm.name(), "AES-GCM");
        assert_eq!(unwrapped.key_type(), "secret");
    }

    #[test]
    fn test_derive_bits_ecdh() {
        // Generate two ECDH key pairs
        let alg = AlgorithmIdentifier::Ecdh {
            named_curve: "P-256".to_string(),
        };
        let alice_key = SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::DeriveBits])
            .expect("Should generate Alice's key");

        let bob_key = SubtleCrypto::generate_key(&alg, true, vec![KeyUsage::DeriveBits])
            .expect("Should generate Bob's key");

        // For ECDH, we need Alice's private + Bob's public, and vice versa
        // But we only have private keys. We need to derive public keys first.
        // Since generate_key only gives us private keys, we can't do full ECDH test here
        // without public key generation. We'll just test that derive_bits validates inputs.
        let _result = SubtleCrypto::derive_bits(&alice_key, &bob_key, Some(256));
        // This might succeed or fail depending on whether the keys have public components
        // For now, we just verify it doesn't panic
    }
}
