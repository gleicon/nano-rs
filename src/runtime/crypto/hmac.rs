//! HMAC algorithm implementation
//!
//! Provides HMAC signing/verification per WebCrypto spec:
//! - Key generation for HMAC with configurable hash algorithms
//! - JWK import/export for symmetric keys
//! - HMAC-SHA256, HMAC-SHA384, HMAC-SHA512 signing
//! - Signature verification with constant-time comparison

use crate::runtime::crypto::crypto_key::JwkObject;
use crate::runtime::crypto::{
    AlgorithmIdentifier, CryptoError, CryptoKey, CryptoKeyHandle, HashAlgorithm, KeyUsage,
};

/// HMAC algorithm parameters
#[derive(Debug, Clone)]
pub struct HmacParams {
    /// Hash algorithm to use
    pub hash: HashAlgorithm,
    /// Optional key length in bits (defaults to hash output size)
    pub length: Option<u32>,
}

impl HmacParams {
    /// Get the default key length for the hash algorithm
    pub fn default_key_length(&self) -> u32 {
        match self.length {
            Some(len) => len,
            None => match self.hash {
                HashAlgorithm::Sha256 => 256,
                HashAlgorithm::Sha384 => 384,
                HashAlgorithm::Sha512 => 512,
            },
        }
    }

    /// Validate the parameters
    pub fn validate(&self) -> Result<(), CryptoError> {
        let key_len = self.default_key_length();
        let min_len = self.hash.output_size() as u32 * 8;

        // Per RFC 2104: key should be at least hash output length
        if key_len < min_len {
            return Err(CryptoError::DataError(format!(
                "HMAC key length {} is less than minimum {} bits",
                key_len, min_len
            )));
        }

        Ok(())
    }
}

/// Generate a new HMAC key
pub fn generate_key(
    hash: HashAlgorithm,
    length: Option<u32>,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    let params = HmacParams { hash, length };
    params.validate()?;

    // Determine key length in bytes
    let key_bits = params.default_key_length();
    let key_bytes = (key_bits / 8) as usize;

    // Generate random key material using ring
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    let mut key_material = vec![0u8; key_bytes];
    rng.fill(&mut key_material)
        .map_err(|_| CryptoError::OperationFailed)?;

    let handle = CryptoKeyHandle::HmacKey(key_material.into_boxed_slice());
    // Store the actual key length, not the optional parameter
    let actual_length = Some(key_bits);
    let algorithm = AlgorithmIdentifier::Hmac {
        hash,
        length: actual_length,
    };

    Ok(CryptoKey::new(handle, algorithm, extractable, usages))
}

/// Import an HMAC key from JWK format
pub fn import_key_jwk(
    jwk: &JwkObject,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    use crate::runtime::crypto::crypto_key::base64url;

    // Validate JWK type
    if jwk.kty != "oct" {
        return Err(CryptoError::DataError(format!(
            "Invalid JWK kty: expected 'oct', got '{}'",
            jwk.kty
        )));
    }

    // Determine hash algorithm from JWK alg field
    let hash = jwk
        .alg
        .as_ref()
        .and_then(|alg| match alg.as_str() {
            "HS256" => Some(HashAlgorithm::Sha256),
            "HS384" => Some(HashAlgorithm::Sha384),
            "HS512" => Some(HashAlgorithm::Sha512),
            _ => None,
        })
        .ok_or_else(|| {
            CryptoError::DataError("Missing or invalid 'alg' field in JWK for HMAC".to_string())
        })?;

    // Extract key material
    let k = jwk
        .k
        .as_ref()
        .ok_or_else(|| CryptoError::DataError("Missing 'k' field in JWK".to_string()))?;
    let key_material = base64url::decode(k).map_err(|_| {
        CryptoError::DataError("Invalid base64url encoding in JWK 'k' field".to_string())
    })?;

    // Validate key length >= hash output length per RFC 2104
    let min_len = hash.output_size();
    if key_material.len() < min_len {
        return Err(CryptoError::DataError(format!(
            "HMAC key must be at least {} bytes for {}",
            min_len,
            hash.name()
        )));
    }

    // Determine length parameter from actual key size
    let length = Some((key_material.len() * 8) as u32);

    let handle = CryptoKeyHandle::HmacKey(key_material.into_boxed_slice());
    let algorithm = AlgorithmIdentifier::Hmac { hash, length };

    Ok(CryptoKey::new(handle, algorithm, extractable, usages))
}

/// Export an HMAC key to JWK format
pub fn export_key_jwk(key: &CryptoKey) -> Result<JwkObject, CryptoError> {
    use crate::runtime::crypto::crypto_key::base64url;

    // Check extractable flag
    if !key.extractable {
        return Err(CryptoError::InvalidAccess);
    }

    // Extract key material
    let key_bytes = match key.handle.as_ref() {
        CryptoKeyHandle::HmacKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };

    // Determine algorithm name
    let alg = match key.algorithm {
        AlgorithmIdentifier::Hmac {
            hash: HashAlgorithm::Sha256,
            ..
        } => "HS256",
        AlgorithmIdentifier::Hmac {
            hash: HashAlgorithm::Sha384,
            ..
        } => "HS384",
        AlgorithmIdentifier::Hmac {
            hash: HashAlgorithm::Sha512,
            ..
        } => "HS512",
        _ => return Err(CryptoError::InvalidKey),
    };

    // Encode key material
    let k = base64url::encode(key_bytes);

    // Build key_ops from usages
    let key_ops: Vec<String> = key.usages.iter().map(|u| u.as_str().to_string()).collect();

    Ok(JwkObject::new_symmetric(alg, &k, key.extractable, key_ops))
}

/// Get the ring HMAC algorithm for a hash algorithm
fn get_hmac_algorithm(hash: &HashAlgorithm) -> &'static ring::hmac::Algorithm {
    match hash {
        HashAlgorithm::Sha256 => &ring::hmac::HMAC_SHA256,
        HashAlgorithm::Sha384 => &ring::hmac::HMAC_SHA384,
        HashAlgorithm::Sha512 => &ring::hmac::HMAC_SHA512,
    }
}

/// Sign data using HMAC
pub fn sign(key: &CryptoKey, data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    // Validate key type
    let (hash, key_bytes) = match (&key.algorithm, key.handle.as_ref()) {
        (AlgorithmIdentifier::Hmac { hash, .. }, CryptoKeyHandle::HmacKey(bytes)) => (hash, bytes),
        _ => return Err(CryptoError::InvalidKey),
    };

    // Validate key has sign usage
    if !key.has_usage(KeyUsage::Sign) {
        return Err(CryptoError::InvalidAccess);
    }

    // Create HMAC key
    let algorithm = get_hmac_algorithm(hash);
    let hmac_key = ring::hmac::Key::new(*algorithm, key_bytes);

    // Sign the data
    let signature = ring::hmac::sign(&hmac_key, data);

    // Return signature bytes
    Ok(signature.as_ref().to_vec())
}

/// Verify an HMAC signature
///
/// Returns true if signature is valid, false otherwise.
/// Uses constant-time comparison to prevent timing attacks.
pub fn verify(key: &CryptoKey, data: &[u8], signature: &[u8]) -> Result<bool, CryptoError> {
    // Validate key type
    let (hash, key_bytes) = match (&key.algorithm, key.handle.as_ref()) {
        (AlgorithmIdentifier::Hmac { hash, .. }, CryptoKeyHandle::HmacKey(bytes)) => (hash, bytes),
        _ => return Err(CryptoError::InvalidKey),
    };

    // Validate key has verify usage
    tracing::debug!(
        "hmac::verify: key usages={:?}, has Verify={}",
        key.usages,
        key.has_usage(KeyUsage::Verify)
    );
    if !key.has_usage(KeyUsage::Verify) {
        tracing::debug!("hmac::verify: rejecting - key lacks Verify usage");
        return Err(CryptoError::InvalidAccess);
    }

    // Create HMAC key
    let algorithm = get_hmac_algorithm(hash);
    let hmac_key = ring::hmac::Key::new(*algorithm, key_bytes);

    // Verify the signature
    // ring::hmac::verify uses constant-time comparison internally
    match ring::hmac::verify(&hmac_key, data, signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key_sha256() {
        let key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate HMAC-SHA256 key");

        assert_eq!(key.algorithm.name(), "HMAC");
        match &key.algorithm {
            AlgorithmIdentifier::Hmac { hash, length } => {
                assert_eq!(*hash, HashAlgorithm::Sha256);
                assert_eq!(*length, Some(256)); // Default is 256 bits
            }
            _ => panic!("Expected Hmac algorithm"),
        }

        // Key material should be 32 bytes (256 bits)
        match key.handle.as_ref() {
            CryptoKeyHandle::HmacKey(bytes) => assert_eq!(bytes.len(), 32),
            _ => panic!("Expected HmacKey handle"),
        }
    }

    #[test]
    fn test_generate_key_sha512() {
        let key = generate_key(
            HashAlgorithm::Sha512,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate HMAC-SHA512 key");

        match &key.algorithm {
            AlgorithmIdentifier::Hmac { hash, length } => {
                assert_eq!(*hash, HashAlgorithm::Sha512);
                assert_eq!(*length, Some(512)); // Default is 512 bits
            }
            _ => panic!("Expected Hmac algorithm"),
        }

        match key.handle.as_ref() {
            CryptoKeyHandle::HmacKey(bytes) => assert_eq!(bytes.len(), 64),
            _ => panic!("Expected HmacKey handle"),
        }
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");

        let message = b"Hello, HMAC World!";

        // Sign
        let signature = sign(&key, message).expect("Should sign");

        // Signature should be 32 bytes for SHA-256
        assert_eq!(signature.len(), 32);

        // Verify
        let valid = verify(&key, message, &signature).expect("Should verify");
        assert!(valid);
    }

    #[test]
    fn test_verify_tampered_signature() {
        let key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");

        let message = b"Hello, HMAC World!";
        let mut signature = sign(&key, message).expect("Should sign");

        // Tamper with signature
        signature[0] ^= 0xFF;

        // Verify should return false, not error
        let valid = verify(&key, message, &signature).expect("Should not error");
        assert!(!valid);
    }

    #[test]
    fn test_verify_wrong_message() {
        let key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");

        let message = b"Hello, HMAC World!";
        let signature = sign(&key, message).expect("Should sign");

        // Verify with different message
        let wrong_message = b"Different message";
        let valid = verify(&key, wrong_message, &signature).expect("Should not error");
        assert!(!valid);
    }

    #[test]
    fn test_sign_without_sign_usage_fails() {
        let key = generate_key(HashAlgorithm::Sha256, None, true, vec![KeyUsage::Verify]) // No Sign usage
            .expect("Should generate key");

        let message = b"Test";
        let result = sign(&key, message);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_without_verify_usage_fails() {
        let key = generate_key(HashAlgorithm::Sha256, None, true, vec![KeyUsage::Sign]) // No Verify usage
            .expect("Should generate key");

        // Need a valid signature first
        let signing_key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");
        let message = b"Test";
        let signature = sign(&signing_key, message).expect("Should sign");

        let result = verify(&key, message, &signature);
        assert!(result.is_err());
    }

    #[test]
    fn test_jwk_import_export() {
        let key = generate_key(
            HashAlgorithm::Sha256,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");

        // Export to JWK
        let jwk = export_key_jwk(&key).expect("Should export");

        assert_eq!(jwk.kty, "oct");
        assert_eq!(jwk.alg.as_deref(), Some("HS256"));
        assert!(jwk.ext.unwrap_or(false));
        assert!(jwk.k.is_some());

        // Import back
        let imported = import_key_jwk(&jwk, true, vec![KeyUsage::Sign, KeyUsage::Verify])
            .expect("Should import");

        assert_eq!(imported.algorithm.name(), "HMAC");
        assert_eq!(imported.extractable, true);

        // Verify key material matches
        match (key.handle.as_ref(), imported.handle.as_ref()) {
            (CryptoKeyHandle::HmacKey(orig), CryptoKeyHandle::HmacKey(imp)) => {
                assert_eq!(orig.as_ref(), imp.as_ref());
            }
            _ => panic!("Key handles don't match"),
        }
    }

    #[test]
    fn test_jwk_import_export_sha384() {
        let key = generate_key(
            HashAlgorithm::Sha384,
            None,
            true,
            vec![KeyUsage::Sign, KeyUsage::Verify],
        )
        .expect("Should generate key");

        let jwk = export_key_jwk(&key).expect("Should export");
        assert_eq!(jwk.alg.as_deref(), Some("HS384"));

        let imported = import_key_jwk(&jwk, true, vec![KeyUsage::Sign, KeyUsage::Verify])
            .expect("Should import");

        match &imported.algorithm {
            AlgorithmIdentifier::Hmac { hash, .. } => {
                assert_eq!(*hash, HashAlgorithm::Sha384);
            }
            _ => panic!("Expected Hmac algorithm"),
        }
    }

    #[test]
    fn test_export_non_extractable_key_fails() {
        let key = generate_key(HashAlgorithm::Sha256, None, false, vec![KeyUsage::Sign])
            .expect("Should generate non-extractable key");

        let result = export_key_jwk(&key);
        assert!(result.is_err());
    }

    #[test]
    fn test_import_short_key_fails() {
        use crate::runtime::crypto::crypto_key::base64url;

        // Try to import a key that's too short (less than hash output length)
        let short_key = vec![1u8, 2, 3, 4]; // Only 4 bytes, but SHA-256 requires at least 32
        let jwk = JwkObject::new_symmetric(
            "HS256",
            &base64url::encode(&short_key),
            true,
            vec!["sign".to_string(), "verify".to_string()],
        );

        let result = import_key_jwk(&jwk, true, vec![KeyUsage::Sign, KeyUsage::Verify]);
        assert!(result.is_err());
    }
}
