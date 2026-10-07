//! SubtleCrypto cipher operations: encrypt, decrypt.

use super::{extract_array_buffer_view, extract_crypto_key};

/// crypto.subtle.encrypt()
pub(crate) fn subtle_encrypt(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg =
            v8::String::new(scope, "encrypt requires 3 arguments: algorithm, key, data").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract algorithm parameters
    let algorithm_obj = args.get(0).to_object(scope);
    if algorithm_obj.is_none() {
        let msg = v8::String::new(scope, "First argument must be an algorithm object").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }
    let algorithm_obj = algorithm_obj.unwrap();

    // Get algorithm name
    let name_key = v8::String::new(scope, "name").unwrap();
    let name_val = algorithm_obj.get(scope, name_key.into());
    let algorithm_name = name_val
        .and_then(|v| v.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Get key object
    let key_obj = args.get(1).to_object(scope);
    if key_obj.is_none() {
        let msg = v8::String::new(scope, "Second argument must be a CryptoKey").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }
    let key_obj = key_obj.unwrap();

    // Extract CryptoKey from the JS object
    let crypto_key = match extract_crypto_key(scope, key_obj) {
        Some(key) => key,
        None => {
            let msg = v8::String::new(scope, "Invalid CryptoKey").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(2)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Third argument must be an ArrayBufferView").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Perform encryption based on algorithm
    let result = match algorithm_name.as_str() {
        "AES-GCM" => {
            // Extract IV
            let iv_key = v8::String::new(scope, "iv").unwrap();
            let iv = algorithm_obj
                .get(scope, iv_key.into())
                .and_then(|v| extract_array_buffer_view(scope, v))
                .unwrap_or_default();

            // Extract optional additionalData
            let aad_key = v8::String::new(scope, "additionalData").unwrap();
            let aad = algorithm_obj
                .get(scope, aad_key.into())
                .and_then(|v| extract_array_buffer_view(scope, v));

            // Extract tag length (default 128)
            let tag_length_key = v8::String::new(scope, "tagLength").unwrap();
            let tag_length_val = algorithm_obj.get(scope, tag_length_key.into());
            let tag_length: u16 = if tag_length_val
                .map(|v| v.is_undefined() || v.is_null())
                .unwrap_or(true)
            {
                128
            } else {
                tag_length_val
                    .and_then(|v| v.to_number(scope))
                    .map(|n| n.value() as u16)
                    .filter(|&n| n > 0)
                    .unwrap_or(128)
            };

            let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
                iv,
                additional_data: aad,
                tag_length,
            };

            let enc_result = crate::runtime::crypto::aes_gcm::encrypt(&crypto_key, &params, &data);
            tracing::debug!("Encrypt result: {:?}", enc_result.is_ok());
            enc_result
        }
        _ => Err(crate::runtime::crypto::CryptoError::NotSupported),
    };

    match result {
        Ok(ciphertext) => {
            // Create ArrayBuffer and return
            let ab = v8::ArrayBuffer::new(scope, ciphertext.len());
            let store = ab.get_backing_store();
            for (i, byte) in ciphertext.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }
            retval.set(ab.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}

/// crypto.subtle.decrypt()
pub(crate) fn subtle_decrypt(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg =
            v8::String::new(scope, "decrypt requires 3 arguments: algorithm, key, data").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract algorithm parameters
    let algorithm_obj = args.get(0).to_object(scope);
    if algorithm_obj.is_none() {
        let msg = v8::String::new(scope, "First argument must be an algorithm object").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }
    let algorithm_obj = algorithm_obj.unwrap();

    // Get algorithm name
    let name_key = v8::String::new(scope, "name").unwrap();
    let name_val = algorithm_obj.get(scope, name_key.into());
    let algorithm_name = name_val
        .and_then(|v| v.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Get key object
    let key_obj = args.get(1).to_object(scope);
    if key_obj.is_none() {
        let msg = v8::String::new(scope, "Second argument must be a CryptoKey").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }
    let key_obj = key_obj.unwrap();

    // Extract CryptoKey from the JS object
    let crypto_key = match extract_crypto_key(scope, key_obj) {
        Some(key) => key,
        None => {
            let msg = v8::String::new(scope, "Invalid CryptoKey").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(2)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Third argument must be an ArrayBufferView").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Perform decryption based on algorithm
    let result = match algorithm_name.as_str() {
        "AES-GCM" => {
            // Extract IV
            let iv_key = v8::String::new(scope, "iv").unwrap();
            let iv = algorithm_obj
                .get(scope, iv_key.into())
                .and_then(|v| extract_array_buffer_view(scope, v))
                .unwrap_or_default();

            // Extract optional additionalData
            let aad_key = v8::String::new(scope, "additionalData").unwrap();
            let aad = algorithm_obj
                .get(scope, aad_key.into())
                .and_then(|v| extract_array_buffer_view(scope, v));

            // Extract tag length (default 128)
            let tag_length_key = v8::String::new(scope, "tagLength").unwrap();
            let tag_length_val = algorithm_obj.get(scope, tag_length_key.into());
            let tag_length: u16 = if tag_length_val
                .map(|v| v.is_undefined() || v.is_null())
                .unwrap_or(true)
            {
                128
            } else {
                tag_length_val
                    .and_then(|v| v.to_number(scope))
                    .map(|n| n.value() as u16)
                    .filter(|&n| n > 0)
                    .unwrap_or(128)
            };

            let params = crate::runtime::crypto::aes_gcm::AesGcmParams {
                iv,
                additional_data: aad,
                tag_length,
            };

            crate::runtime::crypto::aes_gcm::decrypt(&crypto_key, &params, &data)
        }
        _ => Err(crate::runtime::crypto::CryptoError::NotSupported),
    };

    match result {
        Ok(plaintext) => {
            // Create ArrayBuffer and return
            let ab = v8::ArrayBuffer::new(scope, plaintext.len());
            let store = ab.get_backing_store();
            for (i, byte) in plaintext.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }
            retval.set(ab.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}

