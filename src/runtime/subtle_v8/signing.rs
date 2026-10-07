//! SubtleCrypto signing operations: sign, verify.

use super::{extract_array_buffer_view, extract_crypto_key};

/// crypto.subtle.sign()
pub(crate) fn subtle_sign(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg =
            v8::String::new(scope, "sign requires 3 arguments: algorithm, key, data").unwrap();
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
        _ => Err(crate::runtime::crypto::CryptoError::InvalidKey),
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
        let msg = v8::String::new(
            scope,
            "verify requires 4 arguments: algorithm, key, signature, data",
        )
        .unwrap();
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
            let msg = v8::String::new(
                scope,
                "Third argument (signature) must be an ArrayBufferView",
            )
            .unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(3)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Fourth argument (data) must be an ArrayBufferView")
                .unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Perform verification based on key algorithm
    tracing::debug!(
        "subtle_verify: key algorithm={:?}, usages={:?}",
        crypto_key.algorithm,
        crypto_key.usages
    );
    let result = match &crypto_key.algorithm {
        crate::runtime::crypto::AlgorithmIdentifier::Hmac { .. } => {
            crate::runtime::crypto::hmac::verify(&crypto_key, &data, &signature)
        }
        _ => Err(crate::runtime::crypto::CryptoError::InvalidKey),
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

