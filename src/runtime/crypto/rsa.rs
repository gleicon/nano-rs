//! RSA-OAEP encryption/decryption and RSASSA-PKCS1-v1_5/RSASSA-PSS signing
//!
//! WebCrypto RSA implementations using the `rsa` crate.

use crate::runtime::crypto::{CryptoError, CryptoKey, HashAlgorithm, KeyUsage};
use rand::rngs::OsRng;
use rsa::{
    oaep::Oaep,
    pkcs1v15::{SigningKey as Pkcs1v15SigningKey, VerifyingKey as Pkcs1v15VerifyingKey},
    pss::{SigningKey as PssSigningKey, VerifyingKey as PssVerifyingKey},
    sha2::{Sha256, Sha384, Sha512},
    BigUint, RsaPrivateKey, RsaPublicKey,
};
use signature::{RandomizedSigner, Signer, Verifier};

/// RSA-OAEP encryption parameters
#[derive(Debug, Clone)]
pub struct RsaOaepParams {
    pub hash: String, // "SHA-256", "SHA-384", "SHA-512"
    pub label: Option<Vec<u8>>,
}

/// RSA-PSS signing parameters
#[derive(Debug, Clone)]
pub struct RsaPssParams {
    pub salt_length: usize,
}

/// Generate a new RSA key pair
///
/// WebCrypto: generateKey with algorithm "RSA-OAEP" or "RSA-PSS" or "RSASSA-PKCS1-v1_5"
pub fn generate_key(
    modulus_length: u32,
    public_exponent: &[u8],
    algorithm: &str,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    // Validate modulus length (WebCrypto requires >= 256 bits for RSA-OAEP)
    if modulus_length < 256 {
        return Err(CryptoError::DataError(
            "RSA modulus length must be at least 256".to_string(),
        ));
    }

    // Parse public exponent (typically [1, 0, 1] = 65537)
    let exp = BigUint::from_bytes_be(public_exponent);

    // Generate RSA key pair
    let private_key = RsaPrivateKey::new_with_exp(&mut OsRng, modulus_length as usize, &exp)
        .map_err(|_| CryptoError::OperationFailed)?;

    // Serialize to PKCS#8 format for storage
    use rsa::pkcs8::EncodePrivateKey;
    let private_key_bytes = private_key
        .to_pkcs8_der()
        .map_err(|_| CryptoError::OperationFailed)?
        .as_bytes()
        .to_vec();

    // Create algorithm identifier
    let alg_id = match algorithm {
        "RSA-OAEP" => crate::runtime::crypto::AlgorithmIdentifier::RsaOaep {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
        },
        "RSA-PSS" => crate::runtime::crypto::AlgorithmIdentifier::RsaPss {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
            salt_length: None,
        },
        "RSASSA-PKCS1-v1_5" => crate::runtime::crypto::AlgorithmIdentifier::RsaSsaPkcs1V1_5 {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
        },
        _ => return Err(CryptoError::InvalidAlgorithm(algorithm.to_string())),
    };

    // Create CryptoKey with the RSA key pair
    Ok(CryptoKey::new_rsa_private(
        alg_id,
        private_key_bytes.to_vec(),
        extractable,
        usages,
    ))
}

/// Import an RSA key from JWK or PKCS#8/PKCS#1 format
///
/// WebCrypto: importKey with format "jwk", "pkcs8", "spki"
pub fn import_key(
    format: &str,
    key_data: &[u8],
    algorithm: &str,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    // Create algorithm identifier
    let alg_id = match algorithm {
        "RSA-OAEP" => crate::runtime::crypto::AlgorithmIdentifier::RsaOaep {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
        },
        "RSA-PSS" => crate::runtime::crypto::AlgorithmIdentifier::RsaPss {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
            salt_length: None,
        },
        "RSASSA-PKCS1-v1_5" => crate::runtime::crypto::AlgorithmIdentifier::RsaSsaPkcs1V1_5 {
            hash: crate::runtime::crypto::HashAlgorithm::Sha256,
        },
        _ => return Err(CryptoError::InvalidAlgorithm(algorithm.to_string())),
    };

    match format {
        "pkcs8" => {
            // Import private key from PKCS#8 DER format - just store the bytes
            Ok(CryptoKey::new_rsa_private(
                alg_id,
                key_data.to_vec(),
                extractable,
                usages,
            ))
        }
        "spki" => {
            // Import public key from SPKI DER format
            use rsa::pkcs8::DecodePublicKey;
            let public_key = RsaPublicKey::from_public_key_der(key_data)
                .map_err(|e| CryptoError::DataError(format!("Failed to parse SPKI: {}", e)))?;

            // Serialize to SPKI format for storage
            use rsa::pkcs8::EncodePublicKey;
            let public_key_bytes = public_key
                .to_public_key_der()
                .map_err(|_| CryptoError::OperationFailed)?
                .as_bytes()
                .to_vec();

            Ok(CryptoKey::new_rsa_public(
                alg_id,
                public_key_bytes,
                extractable,
                usages,
            ))
        }
        "jwk" => {
            // Parse JWK JSON
            let jwk: serde_json::Value = serde_json::from_slice(key_data)
                .map_err(|e| CryptoError::DataError(format!("Invalid JWK: {}", e)))?;

            // Extract JWK components
            let n = jwk
                .get("n")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CryptoError::DataError("Missing 'n' in JWK".to_string()))?;
            let e = jwk
                .get("e")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CryptoError::DataError("Missing 'e' in JWK".to_string()))?;

            // Decode base64url components
            let n_bytes = base64_decode_url_safe(n)?;
            let e_bytes = base64_decode_url_safe(e)?;

            let modulus = BigUint::from_bytes_be(&n_bytes);
            let exponent = BigUint::from_bytes_be(&e_bytes);

            // Check if it's a private key (has 'd')
            if let Some(d) = jwk.get("d").and_then(|v| v.as_str()) {
                let d_bytes = base64_decode_url_safe(d)?;
                let private_exp = BigUint::from_bytes_be(&d_bytes);

                // For complete private key, we need p, q, dp, dq, qi
                // Simplified: reconstruct from d, n, e
                let private_key = RsaPrivateKey::from_components(
                    modulus,
                    exponent.clone(),
                    private_exp,
                    vec![], // prime_factors - empty for simplified import
                )
                .map_err(|_| CryptoError::OperationFailed)?;

                // Serialize to PKCS#8 for storage
                use rsa::pkcs8::EncodePrivateKey;
                let private_key_bytes = private_key
                    .to_pkcs8_der()
                    .map_err(|_| CryptoError::OperationFailed)?
                    .as_bytes()
                    .to_vec();

                Ok(CryptoKey::new_rsa_private(
                    alg_id,
                    private_key_bytes,
                    extractable,
                    usages,
                ))
            } else {
                // Public key only - reconstruct and store as SPKI
                let public_key = RsaPublicKey::new(modulus, exponent)
                    .map_err(|_| CryptoError::OperationFailed)?;

                // Serialize to SPKI format for storage
                use rsa::pkcs8::EncodePublicKey;
                let public_key_bytes = public_key
                    .to_public_key_der()
                    .map_err(|_| CryptoError::OperationFailed)?
                    .as_bytes()
                    .to_vec();

                Ok(CryptoKey::new_rsa_public(
                    alg_id,
                    public_key_bytes,
                    extractable,
                    usages,
                ))
            }
        }
        _ => Err(CryptoError::NotSupported),
    }
}

/// Export an RSA key to JWK or PKCS#8/PKCS#1 format
///
/// WebCrypto: exportKey with format "jwk", "pkcs8", "spki"
pub fn export_key(_format: &str, key: &CryptoKey) -> Result<Vec<u8>, CryptoError> {
    if !key.extractable {
        return Err(CryptoError::InvalidAccess);
    }

    // Get the RSA private key from the CryptoKey
    // In a real implementation, we'd need to store the key properly
    Err(CryptoError::NotSupported)
}

/// Encrypt data using RSA-OAEP
///
/// WebCrypto: encrypt with algorithm "RSA-OAEP"
pub fn encrypt(
    key: &CryptoKey,
    data: &[u8],
    params: &RsaOaepParams,
) -> Result<Vec<u8>, CryptoError> {
    // Get the RSA public key from the key pair
    let public_key = get_public_key(key)?;

    // Create OAEP padding with the specified hash
    let oaep = match params.hash.as_str() {
        "SHA-256" => Oaep::new::<Sha256>(),
        "SHA-384" => Oaep::new::<Sha384>(),
        "SHA-512" => Oaep::new::<Sha512>(),
        _ => return Err(CryptoError::InvalidAlgorithm(params.hash.clone())),
    };

    // Encrypt the data
    let mut rng = OsRng;
    let encrypted = public_key
        .encrypt(&mut rng, oaep, data)
        .map_err(|_e| CryptoError::OperationFailed)?;

    Ok(encrypted)
}

/// Decrypt data using RSA-OAEP
///
/// WebCrypto: decrypt with algorithm "RSA-OAEP"
pub fn decrypt(
    key: &CryptoKey,
    data: &[u8],
    params: &RsaOaepParams,
) -> Result<Vec<u8>, CryptoError> {
    // Get the RSA private key
    let private_key = get_private_key(key)?;

    // Create OAEP padding with the specified hash
    let oaep = match params.hash.as_str() {
        "SHA-256" => Oaep::new::<Sha256>(),
        "SHA-384" => Oaep::new::<Sha384>(),
        "SHA-512" => Oaep::new::<Sha512>(),
        _ => return Err(CryptoError::InvalidAlgorithm(params.hash.clone())),
    };

    // Decrypt the data
    let decrypted = private_key
        .decrypt(oaep, data)
        .map_err(|_e| CryptoError::OperationFailed)?;

    Ok(decrypted)
}

/// Sign data using RSA-PSS or RSASSA-PKCS1-v1_5
///
/// WebCrypto: sign with algorithm "RSA-PSS" or "RSASSA-PKCS1-v1_5"
pub fn sign(
    key: &CryptoKey,
    data: &[u8],
    algorithm: &str,
    params: Option<&RsaPssParams>,
) -> Result<Vec<u8>, CryptoError> {
    let private_key = get_private_key(key)?;

    match algorithm {
        "RSA-PSS" => {
            let _salt_len = params.map(|p| p.salt_length).unwrap_or(32);

            // Create PSS signing key
            let signing_key = PssSigningKey::<Sha256>::new(private_key);

            // Sign the data
            use rsa::signature::SignatureEncoding;
            let signature = signing_key.sign_with_rng(&mut OsRng, data);

            // Convert signature to bytes using SignatureEncoding trait
            Ok(signature.to_bytes().to_vec())
        }
        "RSASSA-PKCS1-v1_5" => {
            // Get the hash algorithm from the key's algorithm identifier
            let hash = match &key.algorithm {
                crate::runtime::crypto::AlgorithmIdentifier::RsaSsaPkcs1V1_5 { hash } => *hash,
                _ => HashAlgorithm::Sha256,
            };

            // Create PKCS#1 v1.5 signing key with the appropriate hash
            let signature = match hash {
                HashAlgorithm::Sha256 => {
                    let signing_key = Pkcs1v15SigningKey::<Sha256>::new(private_key);
                    signing_key.sign(data)
                }
                HashAlgorithm::Sha384 => {
                    let signing_key = Pkcs1v15SigningKey::<Sha384>::new(private_key);
                    signing_key.sign(data)
                }
                HashAlgorithm::Sha512 => {
                    let signing_key = Pkcs1v15SigningKey::<Sha512>::new(private_key);
                    signing_key.sign(data)
                }
            };

            use rsa::signature::SignatureEncoding;
            Ok(signature.to_bytes().to_vec())
        }
        _ => Err(CryptoError::InvalidAlgorithm(algorithm.to_string())),
    }
}

/// Verify a signature using RSA-PSS or RSASSA-PKCS1-v1_5
///
/// WebCrypto: verify with algorithm "RSA-PSS" or "RSASSA-PKCS1-v1_5"
pub fn verify(
    key: &CryptoKey,
    signature: &[u8],
    data: &[u8],
    algorithm: &str,
    _params: Option<&RsaPssParams>,
) -> Result<bool, CryptoError> {
    let public_key = get_public_key(key)?;

    match algorithm {
        "RSA-PSS" => {
            // Create PSS verifying key
            let verifying_key = PssVerifyingKey::<Sha256>::new(public_key);

            // Parse the signature
            use rsa::pss::Signature as PssSignature;
            let sig = PssSignature::try_from(signature)
                .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;

            // Verify
            match verifying_key.verify(data, &sig) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        "RSASSA-PKCS1-v1_5" => {
            // Get the hash algorithm from the key's algorithm identifier
            let hash = match &key.algorithm {
                crate::runtime::crypto::AlgorithmIdentifier::RsaSsaPkcs1V1_5 { hash } => *hash,
                _ => HashAlgorithm::Sha256,
            };

            // Verify using PKCS#1 v1.5 with the appropriate hash
            let result = match hash {
                HashAlgorithm::Sha256 => {
                    let verifying_key = Pkcs1v15VerifyingKey::<Sha256>::new(public_key);
                    use rsa::pkcs1v15::Signature as Pkcs1v15Signature;
                    let sig = Pkcs1v15Signature::try_from(signature)
                        .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;
                    verifying_key.verify(data, &sig)
                }
                HashAlgorithm::Sha384 => {
                    let verifying_key = Pkcs1v15VerifyingKey::<Sha384>::new(public_key);
                    use rsa::pkcs1v15::Signature as Pkcs1v15Signature;
                    let sig = Pkcs1v15Signature::try_from(signature)
                        .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;
                    verifying_key.verify(data, &sig)
                }
                HashAlgorithm::Sha512 => {
                    let verifying_key = Pkcs1v15VerifyingKey::<Sha512>::new(public_key);
                    use rsa::pkcs1v15::Signature as Pkcs1v15Signature;
                    let sig = Pkcs1v15Signature::try_from(signature)
                        .map_err(|_| CryptoError::DataError("Invalid signature".to_string()))?;
                    verifying_key.verify(data, &sig)
                }
            };

            match result {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        _ => Err(CryptoError::InvalidAlgorithm(algorithm.to_string())),
    }
}

/// Get the private key from a CryptoKey
fn get_private_key(key: &CryptoKey) -> Result<RsaPrivateKey, CryptoError> {
    use rsa::pkcs8::DecodePrivateKey;

    match key.handle.as_ref() {
        crate::runtime::crypto::crypto_key::CryptoKeyHandle::RsaPrivateKey(bytes) => {
            RsaPrivateKey::from_pkcs8_der(bytes).map_err(|e| {
                CryptoError::DataError(format!("Failed to parse RSA private key: {}", e))
            })
        }
        _ => Err(CryptoError::InvalidKey),
    }
}

/// Get the public key from a CryptoKey
fn get_public_key(key: &CryptoKey) -> Result<RsaPublicKey, CryptoError> {
    use rsa::pkcs8::DecodePublicKey;

    // First try to get from public key handle
    match key.handle.as_ref() {
        crate::runtime::crypto::crypto_key::CryptoKeyHandle::RsaPublicKey(bytes) => {
            return RsaPublicKey::from_public_key_der(bytes).map_err(|e| {
                CryptoError::DataError(format!("Failed to parse RSA public key: {}", e))
            });
        }
        crate::runtime::crypto::crypto_key::CryptoKeyHandle::RsaPrivateKey(_) => {
            // Derive public key from private key
            let private_key = get_private_key(key)?;
            Ok(RsaPublicKey::from(&private_key))
        }
        _ => Err(CryptoError::InvalidKey),
    }
}

/// Base64url decode without padding
fn base64_decode_url_safe(input: &str) -> Result<Vec<u8>, CryptoError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|e| CryptoError::DataError(format!("Base64 decode error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rsa_generate_key() {
        let key = generate_key(
            2048,
            &[0x01, 0x00, 0x01], // 65537
            "RSA-OAEP",
            true,
            vec![KeyUsage::Encrypt, KeyUsage::Decrypt],
        );
        assert!(key.is_ok());
    }

    #[test]
    fn test_rsa_oaep_encrypt_decrypt() {
        // This test requires full implementation
        // For now, just test that the functions exist
    }
}
