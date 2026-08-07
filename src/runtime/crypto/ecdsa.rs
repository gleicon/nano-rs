//! ECDSA signing/verification implementation
//!
//! WebCrypto ECDSA using the `p256` and `p384` crates.

use crate::runtime::crypto::crypto_key::CryptoKeyHandle;
use crate::runtime::crypto::{CryptoError, CryptoKey, HashAlgorithm, KeyUsage};
use p256::{
    ecdsa::{
        Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey,
    },
    pkcs8::{DecodePublicKey, EncodePublicKey},
    PublicKey as P256PublicKey, SecretKey as P256SecretKey,
};
use p384::{
    ecdsa::{
        Signature as P384Signature, SigningKey as P384SigningKey, VerifyingKey as P384VerifyingKey,
    },
    PublicKey as P384PublicKey, SecretKey as P384SecretKey,
};
use signature::{Signer, Verifier};

/// Base64url decode without padding
fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, CryptoError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|e| CryptoError::DataError(format!("Base64 decode error: {}", e)))
}

/// ECDSA algorithm parameters
#[derive(Debug, Clone)]
pub struct EcdsaParams {
    pub named_curve: String, // "P-256" or "P-384"
    pub hash: HashAlgorithm,
}

/// Generate a new ECDSA key pair
///
/// WebCrypto: generateKey with algorithm "ECDSA" or "ECDH"
pub fn generate_key(
    named_curve: &str,
    algorithm: &str,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    match named_curve {
        "P-256" => {
            // Generate P-256 key pair
            let secret_key = P256SecretKey::random(&mut rand::rngs::OsRng);
            let _signing_key = P256SigningKey::from(&secret_key);

            // Serialize private key to PKCS#8
            let private_key_bytes = secret_key
                .to_sec1_der()
                .map_err(|_| CryptoError::OperationFailed)?;

            // Create CryptoKey
            let alg = if algorithm == "ECDH" {
                crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                    named_curve: named_curve.to_string(),
                }
            } else {
                crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                    named_curve: named_curve.to_string(),
                    hash: HashAlgorithm::Sha256, // Default hash
                }
            };

            Ok(CryptoKey::new_ecdsa_private(
                alg,
                private_key_bytes.to_vec(),
                extractable,
                usages,
            ))
        }
        "P-384" => {
            // Generate P-384 key pair
            let secret_key = P384SecretKey::random(&mut rand::rngs::OsRng);
            let _signing_key = P384SigningKey::from(&secret_key);

            // Serialize private key to PKCS#8
            let private_key_bytes = secret_key
                .to_sec1_der()
                .map_err(|_| CryptoError::OperationFailed)?;

            let alg = if algorithm == "ECDH" {
                crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                    named_curve: named_curve.to_string(),
                }
            } else {
                crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                    named_curve: named_curve.to_string(),
                    hash: HashAlgorithm::Sha384, // Default hash for P-384
                }
            };

            Ok(CryptoKey::new_ecdsa_private(
                alg,
                private_key_bytes.to_vec(),
                extractable,
                usages,
            ))
        }
        _ => Err(CryptoError::InvalidAlgorithm(format!(
            "Unsupported named curve: {}",
            named_curve
        ))),
    }
}

/// Import an ECDSA key from JWK or PKCS#8/SPKI format
///
/// WebCrypto: importKey with format "jwk", "pkcs8", "spki"
pub fn import_key(
    format: &str,
    key_data: &[u8],
    algorithm: &str,
    named_curve: &str,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    match format {
        "pkcs8" => {
            // Import private key - try SEC1 format first (what we store internally)
            // Note: SEC1 is the standard format for EC private keys
            match named_curve {
                "P-256" => {
                    // Validate by trying to parse as SEC1
                    let _secret_key = P256SecretKey::from_sec1_der(key_data).map_err(|e| {
                        CryptoError::DataError(format!("Failed to parse SEC1 key: {}", e))
                    })?;

                    let alg = if algorithm == "ECDH" {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                            named_curve: named_curve.to_string(),
                        }
                    } else {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                            named_curve: named_curve.to_string(),
                            hash: HashAlgorithm::Sha256,
                        }
                    };

                    Ok(CryptoKey::new_ecdsa_private(
                        alg,
                        key_data.to_vec(),
                        extractable,
                        usages,
                    ))
                }
                "P-384" => {
                    // Validate by trying to parse as SEC1
                    let _secret_key = P384SecretKey::from_sec1_der(key_data).map_err(|e| {
                        CryptoError::DataError(format!("Failed to parse SEC1 key: {}", e))
                    })?;

                    let alg = if algorithm == "ECDH" {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                            named_curve: named_curve.to_string(),
                        }
                    } else {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                            named_curve: named_curve.to_string(),
                            hash: HashAlgorithm::Sha384,
                        }
                    };

                    Ok(CryptoKey::new_ecdsa_private(
                        alg,
                        key_data.to_vec(),
                        extractable,
                        usages,
                    ))
                }
                _ => Err(CryptoError::InvalidAlgorithm(format!(
                    "Unsupported named curve: {}",
                    named_curve
                ))),
            }
        }
        "spki" => {
            // Import public key from SPKI DER format
            match named_curve {
                "P-256" => {
                    let public_key = P256PublicKey::from_public_key_der(key_data).map_err(|e| {
                        CryptoError::DataError(format!("Failed to parse SPKI key: {}", e))
                    })?;

                    // Re-serialize to SPKI for storage
                    let spki_bytes = public_key
                        .to_public_key_der()
                        .map_err(|_| CryptoError::OperationFailed)?
                        .as_bytes()
                        .to_vec();

                    let alg = if algorithm == "ECDH" {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                            named_curve: named_curve.to_string(),
                        }
                    } else {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                            named_curve: named_curve.to_string(),
                            hash: HashAlgorithm::Sha256,
                        }
                    };

                    Ok(CryptoKey::new_ecdsa_public(
                        alg,
                        spki_bytes,
                        extractable,
                        usages,
                    ))
                }
                "P-384" => {
                    let public_key = P384PublicKey::from_public_key_der(key_data).map_err(|e| {
                        CryptoError::DataError(format!("Failed to parse SPKI key: {}", e))
                    })?;

                    // Re-serialize to SPKI for storage
                    let spki_bytes = public_key
                        .to_public_key_der()
                        .map_err(|_| CryptoError::OperationFailed)?
                        .as_bytes()
                        .to_vec();

                    let alg = if algorithm == "ECDH" {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                            named_curve: named_curve.to_string(),
                        }
                    } else {
                        crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                            named_curve: named_curve.to_string(),
                            hash: HashAlgorithm::Sha384,
                        }
                    };

                    Ok(CryptoKey::new_ecdsa_public(
                        alg,
                        spki_bytes,
                        extractable,
                        usages,
                    ))
                }
                _ => Err(CryptoError::InvalidAlgorithm(format!(
                    "Unsupported named curve: {}",
                    named_curve
                ))),
            }
        }
        "jwk" => {
            // Parse JWK JSON
            let jwk: serde_json::Value = serde_json::from_slice(key_data)
                .map_err(|e| CryptoError::DataError(format!("Invalid JWK: {}", e)))?;

            let crv = jwk
                .get("crv")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CryptoError::DataError("Missing 'crv' in JWK".to_string()))?;

            if crv != named_curve {
                return Err(CryptoError::DataError(format!(
                    "JWK curve '{}' does not match expected '{}'",
                    crv, named_curve
                )));
            }

            let x = jwk
                .get("x")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CryptoError::DataError("Missing 'x' in JWK".to_string()))?;
            let y = jwk
                .get("y")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CryptoError::DataError("Missing 'y' in JWK".to_string()))?;

            let x_bytes = base64_decode_url_safe(x)?;
            let y_bytes = base64_decode_url_safe(y)?;

            match named_curve {
                "P-256" => {
                    if x_bytes.len() != 32 || y_bytes.len() != 32 {
                        return Err(CryptoError::DataError(
                            "Invalid P-256 JWK coordinate length".to_string(),
                        ));
                    }

                    // Construct uncompressed SEC1 point: 0x04 || x || y
                    let mut sec1_point = vec![0x04u8];
                    sec1_point.extend_from_slice(&x_bytes);
                    sec1_point.extend_from_slice(&y_bytes);

                    let public_key = P256PublicKey::from_sec1_bytes(&sec1_point).map_err(|e| {
                        CryptoError::DataError(format!("Invalid JWK public key: {}", e))
                    })?;

                    let spki_bytes = public_key
                        .to_public_key_der()
                        .map_err(|_| CryptoError::OperationFailed)?
                        .as_bytes()
                        .to_vec();

                    // Check if private key component 'd' is present
                    if let Some(d) = jwk.get("d").and_then(|v| v.as_str()) {
                        let d_bytes = base64_decode_url_safe(d)?;
                        if d_bytes.len() != 32 {
                            return Err(CryptoError::DataError(
                                "Invalid P-256 JWK private key length".to_string(),
                            ));
                        }
                        let secret_key = P256SecretKey::from_slice(&d_bytes).map_err(|e| {
                            CryptoError::DataError(format!("Invalid JWK private key: {}", e))
                        })?;
                        let private_key_bytes = secret_key
                            .to_sec1_der()
                            .map_err(|_| CryptoError::OperationFailed)?
                            .to_vec();

                        let alg = if algorithm == "ECDH" {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                                named_curve: named_curve.to_string(),
                            }
                        } else {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                                named_curve: named_curve.to_string(),
                                hash: HashAlgorithm::Sha256,
                            }
                        };

                        Ok(CryptoKey::new_ecdsa_private(
                            alg,
                            private_key_bytes,
                            extractable,
                            usages,
                        ))
                    } else {
                        let alg = if algorithm == "ECDH" {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                                named_curve: named_curve.to_string(),
                            }
                        } else {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                                named_curve: named_curve.to_string(),
                                hash: HashAlgorithm::Sha256,
                            }
                        };

                        Ok(CryptoKey::new_ecdsa_public(
                            alg,
                            spki_bytes,
                            extractable,
                            usages,
                        ))
                    }
                }
                "P-384" => {
                    if x_bytes.len() != 48 || y_bytes.len() != 48 {
                        return Err(CryptoError::DataError(
                            "Invalid P-384 JWK coordinate length".to_string(),
                        ));
                    }

                    // Construct uncompressed SEC1 point: 0x04 || x || y
                    let mut sec1_point = vec![0x04u8];
                    sec1_point.extend_from_slice(&x_bytes);
                    sec1_point.extend_from_slice(&y_bytes);

                    let public_key = P384PublicKey::from_sec1_bytes(&sec1_point).map_err(|e| {
                        CryptoError::DataError(format!("Invalid JWK public key: {}", e))
                    })?;

                    let spki_bytes = public_key
                        .to_public_key_der()
                        .map_err(|_| CryptoError::OperationFailed)?
                        .as_bytes()
                        .to_vec();

                    // Check if private key component 'd' is present
                    if let Some(d) = jwk.get("d").and_then(|v| v.as_str()) {
                        let d_bytes = base64_decode_url_safe(d)?;
                        if d_bytes.len() != 48 {
                            return Err(CryptoError::DataError(
                                "Invalid P-384 JWK private key length".to_string(),
                            ));
                        }
                        let secret_key = P384SecretKey::from_slice(&d_bytes).map_err(|e| {
                            CryptoError::DataError(format!("Invalid JWK private key: {}", e))
                        })?;
                        let private_key_bytes = secret_key
                            .to_sec1_der()
                            .map_err(|_| CryptoError::OperationFailed)?
                            .to_vec();

                        let alg = if algorithm == "ECDH" {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                                named_curve: named_curve.to_string(),
                            }
                        } else {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                                named_curve: named_curve.to_string(),
                                hash: HashAlgorithm::Sha384,
                            }
                        };

                        Ok(CryptoKey::new_ecdsa_private(
                            alg,
                            private_key_bytes,
                            extractable,
                            usages,
                        ))
                    } else {
                        let alg = if algorithm == "ECDH" {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdh {
                                named_curve: named_curve.to_string(),
                            }
                        } else {
                            crate::runtime::crypto::AlgorithmIdentifier::Ecdsa {
                                named_curve: named_curve.to_string(),
                                hash: HashAlgorithm::Sha384,
                            }
                        };

                        Ok(CryptoKey::new_ecdsa_public(
                            alg,
                            spki_bytes,
                            extractable,
                            usages,
                        ))
                    }
                }
                _ => Err(CryptoError::InvalidAlgorithm(format!(
                    "Unsupported named curve: {}",
                    named_curve
                ))),
            }
        }
        _ => Err(CryptoError::NotSupported),
    }
}

/// Sign data using ECDSA
///
/// WebCrypto: sign with algorithm "ECDSA"
pub fn sign(key: &CryptoKey, data: &[u8], params: &EcdsaParams) -> Result<Vec<u8>, CryptoError> {
    let private_key_bytes = match key.handle.as_ref() {
        crate::runtime::crypto::crypto_key::CryptoKeyHandle::EcdsaPrivateKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };

    match params.named_curve.as_str() {
        "P-256" => {
            // Parse private key
            let secret_key = P256SecretKey::from_sec1_der(private_key_bytes)
                .map_err(|_| CryptoError::InvalidKey)?;
            let signing_key = P256SigningKey::from(&secret_key);

            // Sign the data (pre-hashed)
            let signature: P256Signature = signing_key.sign(data);
            Ok(signature.to_bytes().to_vec())
        }
        "P-384" => {
            // Parse private key
            let secret_key = P384SecretKey::from_sec1_der(private_key_bytes)
                .map_err(|_| CryptoError::InvalidKey)?;
            let signing_key = P384SigningKey::from(&secret_key);

            // Sign the data (pre-hashed)
            let signature: P384Signature = signing_key.sign(data);
            Ok(signature.to_bytes().to_vec())
        }
        _ => Err(CryptoError::InvalidAlgorithm(params.named_curve.clone())),
    }
}

/// Verify an ECDSA signature
///
/// WebCrypto: verify with algorithm "ECDSA"
pub fn verify(
    key: &CryptoKey,
    signature: &[u8],
    data: &[u8],
    params: &EcdsaParams,
) -> Result<bool, CryptoError> {
    // For verification, we need the public key
    // Currently only supporting private key import which has the public component
    let private_key_bytes = match key.handle.as_ref() {
        crate::runtime::crypto::crypto_key::CryptoKeyHandle::EcdsaPrivateKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };

    match params.named_curve.as_str() {
        "P-256" => {
            // Parse private key and derive public key
            let secret_key = P256SecretKey::from_sec1_der(private_key_bytes)
                .map_err(|_| CryptoError::InvalidKey)?;
            let signing_key = P256SigningKey::from(&secret_key);
            let verifying_key = P256VerifyingKey::from(&signing_key);

            // Parse the signature
            let sig = P256Signature::try_from(signature)
                .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;

            // Verify
            match verifying_key.verify(data, &sig) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        "P-384" => {
            // Parse private key and derive public key
            let secret_key = P384SecretKey::from_sec1_der(private_key_bytes)
                .map_err(|_| CryptoError::InvalidKey)?;
            let signing_key = P384SigningKey::from(&secret_key);
            let verifying_key = P384VerifyingKey::from(&signing_key);

            // Parse the signature
            let sig = P384Signature::from_slice(signature)
                .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;

            // Verify
            match verifying_key.verify(data, &sig) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        _ => Err(CryptoError::InvalidAlgorithm(params.named_curve.clone())),
    }
}

/// Perform ECDH key agreement
///
/// WebCrypto: deriveKey with algorithm "ECDH"
///
/// Uses the elliptic curve Diffie-Hellman primitive to derive shared secret
/// bytes from an ECDH/ECDSA private key and the other party's public key.
/// Supports P-256 and P-384 curves.
pub fn derive_bits(
    private_key: &CryptoKey,
    public_key: &CryptoKey,
    length: Option<usize>,
) -> Result<Vec<u8>, CryptoError> {
    // Verify key types
    let priv_bytes = match private_key.handle.as_ref() {
        CryptoKeyHandle::EcdsaPrivateKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };
    let pub_bytes = match public_key.handle.as_ref() {
        CryptoKeyHandle::EcdsaPublicKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };

    // Extract named curve from algorithm
    let named_curve = match &private_key.algorithm {
        crate::runtime::crypto::AlgorithmIdentifier::Ecdh { named_curve } => named_curve.clone(),
        crate::runtime::crypto::AlgorithmIdentifier::Ecdsa { named_curve, .. } => {
            named_curve.clone()
        }
        _ => {
            return Err(CryptoError::InvalidAlgorithm(
                "ECDH key required".to_string(),
            ))
        }
    };

    // Perform ECDH based on curve
    let shared_secret = match named_curve.as_str() {
        "P-256" => {
            let secret_key = P256SecretKey::from_sec1_der(priv_bytes)
                .map_err(|_| CryptoError::DataError("Invalid P-256 private key".to_string()))?;
            let public_key = P256PublicKey::from_public_key_der(pub_bytes)
                .map_err(|_| CryptoError::DataError("Invalid P-256 public key".to_string()))?;
            let shared =
                p256::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine());
            (&*shared.raw_secret_bytes()).to_vec()
        }
        "P-384" => {
            let secret_key = P384SecretKey::from_sec1_der(priv_bytes)
                .map_err(|_| CryptoError::DataError("Invalid P-384 private key".to_string()))?;
            let public_key = P384PublicKey::from_public_key_der(pub_bytes)
                .map_err(|_| CryptoError::DataError("Invalid P-384 public key".to_string()))?;
            let shared =
                p384::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine());
            (&*shared.raw_secret_bytes()).to_vec()
        }
        _ => {
            return Err(CryptoError::InvalidAlgorithm(format!(
                "Unsupported curve for ECDH: {}",
                named_curve
            )))
        }
    };

    // Truncate to requested length if specified
    match length {
        Some(len) if len > 0 && len < shared_secret.len() * 8 => {
            let byte_len = (len + 7) / 8;
            Ok(shared_secret[..byte_len].to_vec())
        }
        _ => Ok(shared_secret),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ecdsa_generate_key_p256() {
        let key = generate_key(
            "P-256",
            "ECDSA",
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        );
        assert!(key.is_ok());
        assert_eq!(key.unwrap().key_type(), "private");
    }

    #[test]
    fn test_ecdsa_generate_key_p384() {
        let key = generate_key(
            "P-384",
            "ECDSA",
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        );
        assert!(key.is_ok());
        assert_eq!(key.unwrap().key_type(), "private");
    }
}
