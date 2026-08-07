//! AES-GCM algorithm implementation
//!
//! Provides AES-GCM encryption/decryption per WebCrypto spec:
//! - Key generation for 128, 192, and 256-bit keys
//! - JWK import/export for symmetric keys
//! - Encryption with IV and optional additionalData
//! - Decryption with authentication tag verification

use crate::runtime::crypto::crypto_key::JwkObject;
use crate::runtime::crypto::{
    AlgorithmIdentifier, CryptoError, CryptoKey, CryptoKeyHandle, KeyUsage,
};

/// AES-GCM algorithm parameters
#[derive(Debug, Clone)]
pub struct AesGcmParams {
    /// Initialization vector (nonce)
    pub iv: Vec<u8>,
    /// Additional authenticated data (optional)
    pub additional_data: Option<Vec<u8>>,
    /// Authentication tag length in bits (default 128)
    pub tag_length: u16,
}

impl AesGcmParams {
    /// Validate the parameters
    pub fn validate(&self) -> Result<(), CryptoError> {
        // Validate tag length
        const VALID_TAG_LENGTHS: [u16; 7] = [32, 64, 96, 104, 112, 120, 128];
        if !VALID_TAG_LENGTHS.contains(&self.tag_length) {
            return Err(CryptoError::InvalidAlgorithm(format!(
                "Invalid tag length: {}",
                self.tag_length
            )));
        }
        Ok(())
    }
}

/// Generate a new AES-GCM key
pub fn generate_key(
    length: u16,
    extractable: bool,
    usages: Vec<KeyUsage>,
) -> Result<CryptoKey, CryptoError> {
    // Validate key length
    let key_bytes = match length {
        128 => 16usize,
        192 => 24usize,
        256 => 32usize,
        _ => {
            return Err(CryptoError::InvalidAlgorithm(format!(
                "Invalid AES-GCM key length: {}",
                length
            )))
        }
    };

    // Generate random key material using ring
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    let mut key_material = vec![0u8; key_bytes];
    rng.fill(&mut key_material)
        .map_err(|_| CryptoError::OperationFailed)?;

    let handle = CryptoKeyHandle::AesGcmKey(key_material.into_boxed_slice());
    let algorithm = AlgorithmIdentifier::AesGcm { length };

    Ok(CryptoKey::new(handle, algorithm, extractable, usages))
}

/// Import an AES-GCM key from JWK format
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

    // Extract key material
    let k = jwk
        .k
        .as_ref()
        .ok_or_else(|| CryptoError::DataError("Missing 'k' field in JWK".to_string()))?;
    let key_material = base64url::decode(k).map_err(|_| {
        CryptoError::DataError("Invalid base64url encoding in JWK 'k' field".to_string())
    })?;

    // Determine key length from algorithm or key material
    let length = if let Some(ref alg) = jwk.alg {
        match alg.as_str() {
            "A128GCM" => 128u16,
            "A192GCM" => 192u16,
            "A256GCM" => 256u16,
            _ => {
                return Err(CryptoError::DataError(format!(
                    "Unknown JWK algorithm: {}",
                    alg
                )))
            }
        }
    } else {
        // Infer from key material length
        match key_material.len() {
            16 => 128u16,
            24 => 192u16,
            32 => 256u16,
            _ => {
                return Err(CryptoError::DataError(format!(
                    "Invalid key length: {} bytes",
                    key_material.len()
                )))
            }
        }
    };

    let handle = CryptoKeyHandle::AesGcmKey(key_material.into_boxed_slice());
    let algorithm = AlgorithmIdentifier::AesGcm { length };

    Ok(CryptoKey::new(handle, algorithm, extractable, usages))
}

/// Export an AES-GCM key to JWK format
pub fn export_key_jwk(key: &CryptoKey) -> Result<JwkObject, CryptoError> {
    use crate::runtime::crypto::crypto_key::base64url;

    // Check extractable flag
    if !key.extractable {
        return Err(CryptoError::InvalidAccess);
    }

    // Extract key material
    let key_bytes = match key.handle.as_ref() {
        CryptoKeyHandle::AesGcmKey(bytes) => bytes,
        _ => return Err(CryptoError::InvalidKey),
    };

    // Determine algorithm name
    let alg = match key.algorithm {
        AlgorithmIdentifier::AesGcm { length: 128 } => "A128GCM",
        AlgorithmIdentifier::AesGcm { length: 192 } => "A192GCM",
        AlgorithmIdentifier::AesGcm { length: 256 } => "A256GCM",
        _ => return Err(CryptoError::InvalidKey),
    };

    // Encode key material
    let k = base64url::encode(key_bytes);

    // Build key_ops from usages
    let key_ops: Vec<String> = key.usages.iter().map(|u| u.as_str().to_string()).collect();

    Ok(JwkObject::new_symmetric(alg, &k, key.extractable, key_ops))
}

/// Encrypt data using AES-GCM
pub fn encrypt(
    key: &CryptoKey,
    params: &AesGcmParams,
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    use ring::aead::{Aad, Nonce, UnboundKey, AES_128_GCM, AES_256_GCM};

    // Validate parameters
    params.validate()?;

    // Validate key type and usage
    match key.handle.as_ref() {
        CryptoKeyHandle::AesGcmKey(_) => {}
        _ => return Err(CryptoError::InvalidKey),
    }

    if !key.has_usage(KeyUsage::Encrypt) {
        return Err(CryptoError::InvalidAccess);
    }

    // Get the ring algorithm
    let key_bytes = match key.handle.as_ref() {
        CryptoKeyHandle::AesGcmKey(bytes) => bytes,
        _ => unreachable!(),
    };

    let algorithm: &ring::aead::Algorithm = match key.algorithm {
        AlgorithmIdentifier::AesGcm { length: 128 } => &AES_128_GCM,
        AlgorithmIdentifier::AesGcm { length: 256 } => &AES_256_GCM,
        AlgorithmIdentifier::AesGcm { length: 192 } => {
            return Err(CryptoError::NotSupported);
        }
        _ => {
            return Err(CryptoError::InvalidAlgorithm(
                "Invalid AES key length".to_string(),
            ))
        }
    };

    // Create unbound key and sealing key
    let unbound_key =
        UnboundKey::new(algorithm, key_bytes).map_err(|_| CryptoError::OperationFailed)?;

    // For one-shot operation, we use LessSafeKey which doesn't require NonceSequence
    let less_safe_key = ring::aead::LessSafeKey::new(unbound_key);

    // Create nonce from IV
    let nonce = Nonce::try_assume_unique_for_key(&params.iv)
        .map_err(|_| CryptoError::DataError("Invalid IV length".to_string()))?;

    // Create AAD
    let empty_aad: &[u8] = &[];
    let aad: Aad<&[u8]> = match &params.additional_data {
        Some(data) => Aad::from(data.as_slice()),
        None => Aad::from(empty_aad),
    };

    // Prepare buffer for in-place encryption
    let mut in_out = plaintext.to_vec();

    // Seal (encrypt and generate tag separately)
    let tag = less_safe_key
        .seal_in_place_separate_tag(nonce, aad, &mut in_out)
        .map_err(|_| CryptoError::OperationFailed)?;

    // Append tag to ciphertext
    in_out.extend_from_slice(tag.as_ref());

    Ok(in_out)
}

/// Decrypt data using AES-GCM
pub fn decrypt(
    key: &CryptoKey,
    params: &AesGcmParams,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    use ring::aead::{Aad, Nonce, UnboundKey, AES_128_GCM, AES_256_GCM};

    // Validate parameters
    params.validate()?;

    // Validate key type and usage
    match key.handle.as_ref() {
        CryptoKeyHandle::AesGcmKey(_) => {}
        _ => return Err(CryptoError::InvalidKey),
    }

    if !key.has_usage(KeyUsage::Decrypt) {
        return Err(CryptoError::InvalidAccess);
    }

    // Get the ring algorithm
    let key_bytes = match key.handle.as_ref() {
        CryptoKeyHandle::AesGcmKey(bytes) => bytes,
        _ => unreachable!(),
    };

    let algorithm: &ring::aead::Algorithm = match key.algorithm {
        AlgorithmIdentifier::AesGcm { length: 128 } => &AES_128_GCM,
        AlgorithmIdentifier::AesGcm { length: 256 } => &AES_256_GCM,
        AlgorithmIdentifier::AesGcm { length: 192 } => {
            return Err(CryptoError::NotSupported);
        }
        _ => {
            return Err(CryptoError::InvalidAlgorithm(
                "Invalid AES key length".to_string(),
            ))
        }
    };

    // Create unbound key and use LessSafeKey for one-shot operation
    let unbound_key =
        UnboundKey::new(algorithm, key_bytes).map_err(|_| CryptoError::OperationFailed)?;
    let less_safe_key = ring::aead::LessSafeKey::new(unbound_key);

    // Create nonce from IV
    let nonce = Nonce::try_assume_unique_for_key(&params.iv)
        .map_err(|_| CryptoError::DataError("Invalid IV length".to_string()))?;

    // Create AAD
    let empty_aad: &[u8] = &[];
    let aad: Aad<&[u8]> = match &params.additional_data {
        Some(data) => Aad::from(data.as_slice()),
        None => Aad::from(empty_aad),
    };

    // The ciphertext includes the authentication tag at the end
    let tag_len = algorithm.tag_len();
    if ciphertext.len() < tag_len {
        return Err(CryptoError::OperationFailed);
    }

    // Prepare buffer for in-place decryption
    let mut in_out = ciphertext.to_vec();

    // Open (verify tag and decrypt)
    let plaintext = less_safe_key
        .open_in_place(nonce, aad, &mut in_out)
        .map_err(|_| CryptoError::OperationFailed)?;

    Ok(plaintext.to_vec())
}

/// Generate a random IV for AES-GCM
///
/// Per NIST SP 800-38D, the recommended IV length is 96 bits (12 bytes)
pub fn generate_iv() -> Result<Vec<u8>, CryptoError> {
    use ring::rand::{SecureRandom, SystemRandom};

    let rng = SystemRandom::new();
    let mut iv = vec![0u8; 12];
    rng.fill(&mut iv)
        .map_err(|_| CryptoError::OperationFailed)?;

    Ok(iv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::crypto::KeyUsage;

    #[test]
    fn test_generate_key_256() {
        let key = generate_key(256, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should generate 256-bit key");

        assert_eq!(key.algorithm.name(), "AES-GCM");
        assert!(key.extractable);
        assert!(key.has_usage(KeyUsage::Encrypt));
        assert!(key.has_usage(KeyUsage::Decrypt));

        // Key material should be 32 bytes (256 bits)
        match key.handle.as_ref() {
            CryptoKeyHandle::AesGcmKey(bytes) => assert_eq!(bytes.len(), 32),
            _ => panic!("Expected AesGcmKey handle"),
        }
    }

    #[test]
    fn test_generate_key_128() {
        let key = generate_key(128, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should generate 128-bit key");

        match key.handle.as_ref() {
            CryptoKeyHandle::AesGcmKey(bytes) => assert_eq!(bytes.len(), 16),
            _ => panic!("Expected AesGcmKey handle"),
        }
    }

    #[test]
    fn test_generate_key_invalid_length() {
        let result = generate_key(64, true, vec![KeyUsage::Encrypt]);
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        // Generate a key
        let key = generate_key(256, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should generate key");

        // Generate IV
        let iv = generate_iv().expect("Should generate IV");

        // Create params
        let params = AesGcmParams {
            iv: iv.clone(),
            additional_data: None,
            tag_length: 128,
        };

        // Plaintext
        let plaintext = b"Hello, AES-GCM World!";

        // Encrypt
        let ciphertext = encrypt(&key, &params, plaintext).expect("Should encrypt");

        // Ciphertext should be longer than plaintext (includes 16-byte auth tag)
        assert_eq!(ciphertext.len(), plaintext.len() + 16);

        // Decrypt
        let decrypted = decrypt(&key, &params, &ciphertext).expect("Should decrypt");

        // Should match original
        assert_eq!(&decrypted[..], plaintext);
    }

    #[test]
    fn test_decrypt_tampered_ciphertext() {
        // Generate a key
        let key = generate_key(256, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should generate key");

        let iv = generate_iv().expect("Should generate IV");
        let params = AesGcmParams {
            iv,
            additional_data: None,
            tag_length: 128,
        };

        let plaintext = b"Secret message";
        let mut ciphertext = encrypt(&key, &params, plaintext).expect("Should encrypt");

        // Tamper with the ciphertext
        ciphertext[0] ^= 0xFF;

        // Decrypt should fail
        let result = decrypt(&key, &params, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_jwk_import_export() {
        // Generate a key
        let key = generate_key(256, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should generate key");

        // Export to JWK
        let jwk = export_key_jwk(&key).expect("Should export");

        assert_eq!(jwk.kty, "oct");
        assert_eq!(jwk.alg.as_deref(), Some("A256GCM"));
        assert!(jwk.ext.unwrap_or(false));
        assert!(jwk.k.is_some());

        // Import back
        let imported = import_key_jwk(&jwk, true, vec![KeyUsage::Encrypt, KeyUsage::Decrypt])
            .expect("Should import");

        assert_eq!(imported.algorithm.name(), "AES-GCM");
        assert_eq!(imported.extractable, true);

        // Verify key material matches
        match (key.handle.as_ref(), imported.handle.as_ref()) {
            (CryptoKeyHandle::AesGcmKey(orig), CryptoKeyHandle::AesGcmKey(imp)) => {
                assert_eq!(orig.as_ref(), imp.as_ref());
            }
            _ => panic!("Key handles don't match"),
        }
    }

    #[test]
    fn test_export_non_extractable_key_fails() {
        let key = generate_key(256, false, vec![KeyUsage::Encrypt])
            .expect("Should generate non-extractable key");

        let result = export_key_jwk(&key);
        assert!(result.is_err());
    }
}
