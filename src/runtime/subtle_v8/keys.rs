//! SubtleCrypto key operations: generateKey, importKey, exportKey.

use super::{extract_array_buffer_view, extract_crypto_key};

/// crypto.subtle.generateKey()
pub(crate) fn subtle_generate_key(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Check argument count
    if args.length() < 3 {
        let msg = v8::String::new(
            scope,
            "generateKey requires 3 arguments: algorithm, extractable, keyUsages",
        )
        .unwrap();
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
                                if let Some(key_usage) =
                                    crate::runtime::crypto::KeyUsage::from_str(&usage)
                                {
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
            let length: Option<u32> = if length_val
                .map(|v| v.is_undefined() || v.is_null())
                .unwrap_or(true)
            {
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
            let msg = v8::String::new(
                scope,
                &format!("Algorithm '{}' not supported", algorithm_name),
            )
            .unwrap();
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
        let msg = v8::String::new(
            scope,
            "importKey requires 5 arguments: format, keyData, algorithm, extractable, keyUsages",
        )
        .unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    // Extract format
    let format = args
        .get(0)
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
                                if let Some(key_usage) =
                                    crate::runtime::crypto::KeyUsage::from_str(&usage)
                                {
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
                "HMAC" => crate::runtime::crypto::hmac::import_key_jwk(&jwk, extractable, usages),
                _ => Err(crate::runtime::crypto::CryptoError::InvalidAlgorithm(
                    algorithm_name,
                )),
            }
        }
        "raw" => {
            let key_bytes = match extract_array_buffer_view(scope, key_data) {
                Some(bytes) => bytes,
                None => {
                    let msg =
                        v8::String::new(scope, "Raw key data must be an ArrayBufferView").unwrap();
                    let error = v8::Exception::type_error(scope, msg);
                    retval.set(error);
                    return;
                }
            };
            let alg_name = if args.get(2).is_string() {
                args.get(2)
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default()
            } else {
                algorithm_name.clone()
            };
            match alg_name.to_uppercase().as_str() {
                "PBKDF2" => Ok(crate::runtime::crypto::CryptoKey::new(
                    crate::runtime::crypto::CryptoKeyHandle::Pbkdf2Key(
                        key_bytes.into_boxed_slice(),
                    ),
                    crate::runtime::crypto::AlgorithmIdentifier::Pbkdf2,
                    extractable,
                    usages,
                )),
                "AES-GCM" => {
                    let length = (key_bytes.len() * 8) as u16;
                    Ok(crate::runtime::crypto::CryptoKey::new(
                        crate::runtime::crypto::CryptoKeyHandle::AesGcmKey(
                            key_bytes.into_boxed_slice(),
                        ),
                        crate::runtime::crypto::AlgorithmIdentifier::AesGcm { length },
                        extractable,
                        usages,
                    ))
                }
                _ => Err(crate::runtime::crypto::CryptoError::InvalidAlgorithm(
                    alg_name,
                )),
            }
        }
        _ => Err(crate::runtime::crypto::CryptoError::NotSupported),
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
    let format = args
        .get(0)
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
                _ => Err(crate::runtime::crypto::CryptoError::InvalidKey),
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

