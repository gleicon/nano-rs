//! SubtleCrypto V8 callback functions — extracted from apis.rs

// === SubtleCrypto V8 callbacks ===

/// crypto.subtle.generateKey()
pub(crate) fn subtle_generate_key(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Check argument count
    if args.length() < 3 {
        let msg = v8::String::new(scope, "generateKey requires 3 arguments: algorithm, extractable, keyUsages").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract algorithm object
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

    // Extract extractable flag
    let extractable = args.get(1).is_true();

    // Extract key usages array
    let usages_val = args.get(2);
    let mut usages = Vec::new();
    if let Some(usages_arr) = usages_val.to_object(scope) {
        if let Some(length_key) = v8::String::new(scope, "length") {
            if let Some(length_val) = usages_arr.get(scope, length_key.into()) {
                if let Some(length_num) = length_val.to_number(scope) {
                    let length = length_num.value() as usize;
                    for i in 0..length {
                        let idx = v8::Number::new(scope, i as f64);
                        if let Some(usage_val) = usages_arr.get(scope, idx.into()) {
                            if let Some(usage_str) = usage_val.to_string(scope) {
                                let usage = usage_str.to_rust_string_lossy(scope);
                                if let Some(key_usage) = crate::runtime::crypto::KeyUsage::from_str(&usage) {
                                    usages.push(key_usage);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Generate key based on algorithm
    let crypto_key = match algorithm_name.as_str() {
        "AES-GCM" => {
            // Extract key length (default to 256)
            let length_key = v8::String::new(scope, "length").unwrap();
            let length = algorithm_obj
                .get(scope, length_key.into())
                .and_then(|v| v.to_number(scope))
                .map(|n| n.value() as u16)
                .unwrap_or(256);

            crate::runtime::crypto::aes_gcm::generate_key(length, extractable, usages)
        }
        "HMAC" => {
            // Extract hash algorithm - can be string "SHA-256" or object {name: "SHA-256"}
            let hash_key = v8::String::new(scope, "hash").unwrap();
            let hash_val = algorithm_obj.get(scope, hash_key.into());

            let hash_name = if let Some(val) = hash_val {
                // Try as string first
                if let Some(s) = val.to_string(scope) {
                    s.to_rust_string_lossy(scope)
                } else if let Some(obj) = val.to_object(scope) {
                    // Try as object with name property
                    if let Some(name_key) = v8::String::new(scope, "name") {
                        obj.get(scope, name_key.into())
                            .and_then(|n| n.to_string(scope))
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let hash = crate::runtime::crypto::HashAlgorithm::from_name(&hash_name)
                .unwrap_or(crate::runtime::crypto::HashAlgorithm::Sha256);

            // Extract optional length (default based on hash)
            let length_key = v8::String::new(scope, "length").unwrap();
            let length_val = algorithm_obj.get(scope, length_key.into());
            let length: Option<u32> = if length_val.map(|v| v.is_undefined() || v.is_null()).unwrap_or(true) {
                None
            } else {
                length_val
                    .and_then(|v| v.to_number(scope))
                    .map(|n| n.value() as u32)
                    .filter(|&n| n > 0)
            };

            crate::runtime::crypto::hmac::generate_key(hash, length, extractable, usages)
        }
        _ => {
            let msg = v8::String::new(scope, &format!("Algorithm '{}' not supported", algorithm_name)).unwrap();
            let error = v8::Exception::error(scope, msg);
            retval.set(error);
            return;
        }
    };

    match crypto_key {
        Ok(key) => {
            // Create CryptoKey JavaScript object inline to avoid lifetime issues
            let obj = v8::Object::new(scope);
            let extractable = key.extractable;
            let algorithm = key.algorithm.clone();
            let usages: Vec<_> = key.usages.clone();
            let type_str = key.key_type();
            let key_ptr = Box::into_raw(Box::new(key));
            let external = v8::External::new(scope, key_ptr as *mut std::ffi::c_void);
            let external_key = v8::String::new(scope, "__crypto_key_ptr__").unwrap();
            obj.set(scope, external_key.into(), external.into());
            let type_key = v8::String::new(scope, "type").unwrap();
            let type_val = v8::String::new(scope, type_str).unwrap();
            obj.set(scope, type_key.into(), type_val.into());
            let extractable_key = v8::String::new(scope, "extractable").unwrap();
            let extractable_val = v8::Boolean::new(scope, extractable);
            obj.set(scope, extractable_key.into(), extractable_val.into());
            let algorithm_key = v8::String::new(scope, "algorithm").unwrap();
            let algorithm_obj = v8::Object::new(scope);
            let alg_name_key = v8::String::new(scope, "name").unwrap();
            let alg_name_val = v8::String::new(scope, algorithm.name()).unwrap();
            algorithm_obj.set(scope, alg_name_key.into(), alg_name_val.into());

            // Add algorithm-specific properties
            match &algorithm {
                crate::runtime::crypto::AlgorithmIdentifier::AesGcm { length } => {
                    let length_key = v8::String::new(scope, "length").unwrap();
                    let length_val = v8::Number::new(scope, *length as f64);
                    algorithm_obj.set(scope, length_key.into(), length_val.into());
                }
                crate::runtime::crypto::AlgorithmIdentifier::Hmac { hash, length } => {
                    // Add hash object with name property
                    let hash_key = v8::String::new(scope, "hash").unwrap();
                    let hash_obj = v8::Object::new(scope);
                    let hash_name_key = v8::String::new(scope, "name").unwrap();
                    let hash_name_val = v8::String::new(scope, hash.name()).unwrap();
                    hash_obj.set(scope, hash_name_key.into(), hash_name_val.into());
                    algorithm_obj.set(scope, hash_key.into(), hash_obj.into());

                    // Add length property if present
                    if let Some(len) = length {
                        let length_key = v8::String::new(scope, "length").unwrap();
                        let length_val = v8::Number::new(scope, *len as f64);
                        algorithm_obj.set(scope, length_key.into(), length_val.into());
                    }
                }
                _ => {}
            }

            obj.set(scope, algorithm_key.into(), algorithm_obj.into());
            let usages_key = v8::String::new(scope, "usages").unwrap();
            let usages_arr = v8::Array::new(scope, usages.len() as i32);
            for (i, usage) in usages.iter().enumerate() {
                let usage_str = v8::String::new(scope, usage.as_str()).unwrap();
                let idx = v8::Number::new(scope, i as f64);
                usages_arr.set(scope, idx.into(), usage_str.into());
            }
            obj.set(scope, usages_key.into(), usages_arr.into());
            retval.set(obj.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            retval.set(error);
        }
    }
}

/// crypto.subtle.importKey()
pub(crate) fn subtle_import_key(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 5 {
        let msg = v8::String::new(scope, "importKey requires 5 arguments: format, keyData, algorithm, extractable, keyUsages").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract format
    let format = args.get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Get key data (JWK object for JWK format)
    let key_data = args.get(1);

    // Extract algorithm
    let algorithm_obj = args.get(2).to_object(scope);
    if algorithm_obj.is_none() {
        let msg = v8::String::new(scope, "Third argument must be an algorithm object").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }
    let algorithm_obj = algorithm_obj.unwrap();

    // Get algorithm name
    let name_key = v8::String::new(scope, "name").unwrap();
    let algorithm_name = algorithm_obj
        .get(scope, name_key.into())
        .and_then(|v| v.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Extract extractable flag
    let extractable = args.get(3).is_true();

    // Extract key usages
    let usages_val = args.get(4);
    let mut usages = Vec::new();
    if let Some(usages_arr) = usages_val.to_object(scope) {
        if let Some(length_key) = v8::String::new(scope, "length") {
            if let Some(length_val) = usages_arr.get(scope, length_key.into()) {
                if let Some(length_num) = length_val.to_number(scope) {
                    let length = length_num.value() as usize;
                    for i in 0..length {
                        let idx = v8::Number::new(scope, i as f64);
                        if let Some(usage_val) = usages_arr.get(scope, idx.into()) {
                            if let Some(usage_str) = usage_val.to_string(scope) {
                                let usage = usage_str.to_rust_string_lossy(scope);
                                if let Some(key_usage) = crate::runtime::crypto::KeyUsage::from_str(&usage) {
                                    usages.push(key_usage);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Import based on format
    let crypto_key = match format.as_str() {
        "jwk" => {
            // Parse JWK from the key data object
            let jwk_obj = key_data.to_object(scope);
            if jwk_obj.is_none() {
                let msg = v8::String::new(scope, "JWK key data must be an object").unwrap();
                let error = v8::Exception::type_error(scope, msg);
                retval.set(error);
                return;
            }
            let jwk_obj = jwk_obj.unwrap();

            // Parse JWK
            let jwk = match crate::runtime::crypto::JwkObject::from_v8_object(scope, jwk_obj) {
                Some(jwk) => jwk,
                None => {
                    let msg = v8::String::new(scope, "Invalid JWK format").unwrap();
                    let error = v8::Exception::type_error(scope, msg);
                    retval.set(error);
                    return;
                }
            };

            // Import based on algorithm
            match algorithm_name.as_str() {
                "AES-GCM" => {
                    crate::runtime::crypto::aes_gcm::import_key_jwk(&jwk, extractable, usages)
                }
                "HMAC" => {
                    crate::runtime::crypto::hmac::import_key_jwk(&jwk, extractable, usages)
                }
                _ => {
                    Err(crate::runtime::crypto::CryptoError::InvalidAlgorithm(algorithm_name))
                }
            }
        }
        _ => {
            Err(crate::runtime::crypto::CryptoError::NotSupported)
        }
    };

    match crypto_key {
        Ok(key) => {
            // Create CryptoKey JavaScript object inline to avoid lifetime issues
            let obj = v8::Object::new(scope);
            let extractable = key.extractable;
            let algorithm = key.algorithm.clone();
            let usages: Vec<_> = key.usages.clone();
            let type_str = key.key_type();
            let key_ptr = Box::into_raw(Box::new(key));
            let external = v8::External::new(scope, key_ptr as *mut std::ffi::c_void);
            let external_key = v8::String::new(scope, "__crypto_key_ptr__").unwrap();
            obj.set(scope, external_key.into(), external.into());
            let type_key = v8::String::new(scope, "type").unwrap();
            let type_val = v8::String::new(scope, type_str).unwrap();
            obj.set(scope, type_key.into(), type_val.into());
            let extractable_key = v8::String::new(scope, "extractable").unwrap();
            let extractable_val = v8::Boolean::new(scope, extractable);
            obj.set(scope, extractable_key.into(), extractable_val.into());
            let algorithm_key = v8::String::new(scope, "algorithm").unwrap();
            let algorithm_obj = v8::Object::new(scope);
            let alg_name_key = v8::String::new(scope, "name").unwrap();
            let alg_name_val = v8::String::new(scope, algorithm.name()).unwrap();
            algorithm_obj.set(scope, alg_name_key.into(), alg_name_val.into());

            // Add algorithm-specific properties
            match &algorithm {
                crate::runtime::crypto::AlgorithmIdentifier::AesGcm { length } => {
                    let length_key = v8::String::new(scope, "length").unwrap();
                    let length_val = v8::Number::new(scope, *length as f64);
                    algorithm_obj.set(scope, length_key.into(), length_val.into());
                }
                crate::runtime::crypto::AlgorithmIdentifier::Hmac { hash, length } => {
                    // Add hash object with name property
                    let hash_key = v8::String::new(scope, "hash").unwrap();
                    let hash_obj = v8::Object::new(scope);
                    let hash_name_key = v8::String::new(scope, "name").unwrap();
                    let hash_name_val = v8::String::new(scope, hash.name()).unwrap();
                    hash_obj.set(scope, hash_name_key.into(), hash_name_val.into());
                    algorithm_obj.set(scope, hash_key.into(), hash_obj.into());

                    // Add length property if present
                    if let Some(len) = length {
                        let length_key = v8::String::new(scope, "length").unwrap();
                        let length_val = v8::Number::new(scope, *len as f64);
                        algorithm_obj.set(scope, length_key.into(), length_val.into());
                    }
                }
                _ => {}
            }

            obj.set(scope, algorithm_key.into(), algorithm_obj.into());
            let usages_key = v8::String::new(scope, "usages").unwrap();
            let usages_arr = v8::Array::new(scope, usages.len() as i32);
            for (i, usage) in usages.iter().enumerate() {
                let usage_str = v8::String::new(scope, usage.as_str()).unwrap();
                let idx = v8::Number::new(scope, i as f64);
                usages_arr.set(scope, idx.into(), usage_str.into());
            }
            obj.set(scope, usages_key.into(), usages_arr.into());
            retval.set(obj.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            retval.set(error);
        }
    }
}

/// crypto.subtle.exportKey()
pub(crate) fn subtle_export_key(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 2 {
        let msg = v8::String::new(scope, "exportKey requires 2 arguments: format, key").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract format
    let format = args.get(0)
        .to_string(scope)
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

    // Enforce non-extractable key guard (WebCrypto spec)
    if !crypto_key.extractable {
        let msg = v8::String::new(scope, "The CryptoKey is not extractable").unwrap();
        let error = v8::Exception::error(scope, msg);
        scope.throw_exception(error);
        return;
    }

    // Export based on format
    match format.as_str() {
        "jwk" => {
            // Export to JWK
            let result = match &crypto_key.algorithm {
                crate::runtime::crypto::AlgorithmIdentifier::AesGcm { .. } => {
                    crate::runtime::crypto::aes_gcm::export_key_jwk(&crypto_key)
                }
                crate::runtime::crypto::AlgorithmIdentifier::Hmac { .. } => {
                    crate::runtime::crypto::hmac::export_key_jwk(&crypto_key)
                }
                _ => {
                    Err(crate::runtime::crypto::CryptoError::InvalidKey)
                }
            };

            match result {
                Ok(jwk) => {
                    if let Some(js_jwk_global) = jwk.to_v8_object(scope) {
                        let js_jwk = v8::Local::new(scope, js_jwk_global);
                        retval.set(js_jwk.into());
                    } else {
                        let msg = v8::String::new(scope, "Failed to create JWK object").unwrap();
                        let error = v8::Exception::error(scope, msg);
                        scope.throw_exception(error);
                    }
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &e.to_string()).unwrap();
                    let error = v8::Exception::error(scope, msg);
                    scope.throw_exception(error);
                }
            }
        }
        _ => {
            let msg = v8::String::new(scope, "Only JWK format is supported for export").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
        }
    }
}

/// crypto.subtle.encrypt()
pub(crate) fn subtle_encrypt(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg = v8::String::new(scope, "encrypt requires 3 arguments: algorithm, key, data").unwrap();
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
            let tag_length: u16 = if tag_length_val.map(|v| v.is_undefined() || v.is_null()).unwrap_or(true) {
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
        _ => {
            Err(crate::runtime::crypto::CryptoError::NotSupported)
        }
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
        let msg = v8::String::new(scope, "decrypt requires 3 arguments: algorithm, key, data").unwrap();
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
            let tag_length: u16 = if tag_length_val.map(|v| v.is_undefined() || v.is_null()).unwrap_or(true) {
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
        _ => {
            Err(crate::runtime::crypto::CryptoError::NotSupported)
        }
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

/// Extract a CryptoKey from a JavaScript CryptoKey object
pub(crate) fn extract_crypto_key(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    obj: v8::Local<v8::Object>,
) -> Option<crate::runtime::crypto::CryptoKey> {
    let external_key = v8::String::new(scope, "__crypto_key_ptr__")?;
    let external_val = obj.get(scope, external_key.into())?;

    if external_val.is_external() {
        let external = external_val.cast::<v8::External>();
        let ptr = external.value() as *mut crate::runtime::crypto::CryptoKey;
        if !ptr.is_null() {
            // Clone the key so we don't accidentally drop the original when this scope ends
            return Some(unsafe { (*ptr).clone() });
        }
    }
    None
}

/// Extract bytes from an ArrayBufferView (Uint8Array, etc.)
pub(crate) fn extract_array_buffer_view(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    value: v8::Local<v8::Value>,
) -> Option<Vec<u8>> {
    if let Some(uint8array) = value
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint8Array>().ok())
    {
        let length = uint8array.byte_length();
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            if let Some(val) = uint8array.get_index(scope, i as u32) {
                if let Some(int) = val.to_integer(scope) {
                    vec.push(int.value() as u8);
                }
            }
        }
        return Some(vec);
    }

    if let Some(arraybuffer) = value
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::ArrayBuffer>().ok())
    {
        let store = arraybuffer.get_backing_store();
        let length = arraybuffer.byte_length();
        let mut vec = Vec::with_capacity(length);
        for i in 0..length {
            if let Some(cell) = store.get(i) {
                vec.push(cell.get());
            }
        }
        return Some(vec);
    }

    None
}

/// crypto.subtle.sign()
pub(crate) fn subtle_sign(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg = v8::String::new(scope, "sign requires 3 arguments: algorithm, key, data").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

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

    // Perform signing based on key algorithm
    let result = match &crypto_key.algorithm {
        crate::runtime::crypto::AlgorithmIdentifier::Hmac { .. } => {
            crate::runtime::crypto::hmac::sign(&crypto_key, &data)
        }
        _ => {
            Err(crate::runtime::crypto::CryptoError::InvalidKey)
        }
    };

    match result {
        Ok(signature) => {
            // Create ArrayBuffer and return
            let ab = v8::ArrayBuffer::new(scope, signature.len());
            let store = ab.get_backing_store();
            for (i, byte) in signature.iter().enumerate() {
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

/// crypto.subtle.verify()
pub(crate) fn subtle_verify(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 4 {
        let msg = v8::String::new(scope, "verify requires 4 arguments: algorithm, key, signature, data").unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

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

    // Get signature as bytes
    let signature = match extract_array_buffer_view(scope, args.get(2)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Third argument (signature) must be an ArrayBufferView").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(3)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Fourth argument (data) must be an ArrayBufferView").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Perform verification based on key algorithm
    tracing::debug!("subtle_verify: key algorithm={:?}, usages={:?}", crypto_key.algorithm, crypto_key.usages);
    let result = match &crypto_key.algorithm {
        crate::runtime::crypto::AlgorithmIdentifier::Hmac { .. } => {
            crate::runtime::crypto::hmac::verify(&crypto_key, &data, &signature)
        }
        _ => {
            Err(crate::runtime::crypto::CryptoError::InvalidKey)
        }
    };

    match result {
        Ok(valid) => {
            retval.set(v8::Boolean::new(scope, valid).into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}

/// crypto.subtle.digest() implementation
///
/// Computes a digest (hash) of the given data using the specified algorithm.
/// Arguments: algorithm (string), data (ArrayBufferView)
/// Returns: Promise<ArrayBuffer> containing the hash
pub(crate) fn subtle_digest(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get algorithm string
    let algorithm = match args.get(0).to_string(scope) {
        Some(s) => s.to_rust_string_lossy(scope),
        None => {
            let msg = v8::String::new(scope, "First argument (algorithm) must be a string").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(1)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Second argument (data) must be an ArrayBufferView").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Compute digest using the subtle crypto implementation
    match crate::runtime::crypto::SubtleCrypto::digest(&algorithm, &data) {
        Ok(hash_bytes) => {
            // Create ArrayBuffer from hash bytes
            let ab = v8::ArrayBuffer::new(scope, hash_bytes.len());
            let store = ab.get_backing_store();
            for (i, byte) in hash_bytes.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }

            // Return Promise.resolve(ArrayBuffer)
            let global = scope.get_current_context().global(scope);
            let promise_key = v8::String::new(scope, "Promise").unwrap();
            let resolve_key = v8::String::new(scope, "resolve").unwrap();

            if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
                if let Some(promise_obj) = promise_ctor.to_object(scope) {
                    if let Some(resolve_fn) = promise_obj.get(scope, resolve_key.into()) {
                        if resolve_fn.is_function() {
                            let resolve = resolve_fn.cast::<v8::Function>();
                            if let Some(resolved_promise) = resolve.call(scope, promise_ctor, &[ab.into()]) {
                                retval.set(resolved_promise);
                                return;
                            }
                        }
                    }
                }
            }

            // Fallback: return ArrayBuffer directly
            retval.set(ab.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e.to_string()).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}
