//! SubtleCrypto key-derivation operation: deriveBits.

use super::{extract_array_buffer_view, extract_crypto_key};

/// crypto.subtle.deriveBits() — PBKDF2 support
pub(crate) fn subtle_derive_bits(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 3 {
        let msg = v8::String::new(
            scope,
            "deriveBits requires 3 arguments: algorithm, key, length",
        )
        .unwrap();
        let error = v8::Exception::type_error(scope, msg);
        retval.set(error);
        return;
    }

    let alg_arg = args.get(0);
    let alg_obj = match alg_arg.to_object(scope) {
        Some(o) => o,
        None => {
            let msg = v8::String::new(scope, "First argument must be an algorithm object").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Extract all strings from alg_obj eagerly to avoid borrow conflicts with scope later
    let alg_name = v8::String::new(scope, "name")
        .and_then(|k| alg_obj.get(scope, k.into()))
        .and_then(|v| v.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let hash_name = v8::String::new(scope, "hash")
        .and_then(|k| alg_obj.get(scope, k.into()))
        .and_then(|v| v.to_string(scope))
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let iterations = v8::String::new(scope, "iterations")
        .and_then(|k| alg_obj.get(scope, k.into()))
        .and_then(|v| v.to_integer(scope))
        .map(|n| n.value() as u32)
        .unwrap_or(1000);
    let salt = v8::String::new(scope, "salt")
        .and_then(|k| alg_obj.get(scope, k.into()))
        .and_then(|v| extract_array_buffer_view(scope, v))
        .unwrap_or_default();

    let key_obj = match args.get(1).to_object(scope) {
        Some(o) => o,
        None => {
            let msg = v8::String::new(scope, "Second argument must be a CryptoKey").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };
    let crypto_key = match extract_crypto_key(scope, key_obj) {
        Some(k) => k,
        None => {
            let msg = v8::String::new(scope, "Invalid CryptoKey").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // `length` is attacker-controlled. Validate before allocating: the derived
    // buffer is a host-side Vec that bypasses the V8 heap limit, so an unbounded
    // value would let one tenant abort the shared process (DoS). Per the WebCrypto
    // spec `length` is a positive multiple of 8; cap it at a sane maximum.
    const MAX_DERIVE_BITS: i64 = 8192;
    let bit_length_i = args
        .get(2)
        .to_integer(scope)
        .map(|n| n.value())
        .unwrap_or(256);
    if !(bit_length_i > 0 && bit_length_i <= MAX_DERIVE_BITS && bit_length_i % 8 == 0) {
        let msg = v8::String::new(
            scope,
            "deriveBits: length must be a positive multiple of 8 no greater than 8192",
        )
        .unwrap();
        let error = v8::Exception::type_error(scope, msg);
        scope.throw_exception(error);
        return;
    }
    let byte_length = (bit_length_i / 8) as usize;

    let result: Result<Vec<u8>, String> = match alg_name.to_uppercase().as_str() {
        "PBKDF2" => {
            let password = crypto_key.handle.as_bytes().to_vec();
            let algo = match hash_name.to_uppercase().as_str() {
                "SHA-384" | "SHA384" => ring::pbkdf2::PBKDF2_HMAC_SHA384,
                "SHA-512" | "SHA512" => ring::pbkdf2::PBKDF2_HMAC_SHA512,
                _ => ring::pbkdf2::PBKDF2_HMAC_SHA256,
            };
            let iters = std::num::NonZeroU32::new(iterations.max(1)).unwrap();
            let mut out = vec![0u8; byte_length];
            ring::pbkdf2::derive(algo, iters, &salt, &password, &mut out);
            Ok(out)
        }
        _ => Err(format!("deriveBits: unsupported algorithm '{}'", alg_name)),
    };

    match result {
        Ok(bytes) => {
            let ab = v8::ArrayBuffer::new(scope, bytes.len());
            let store = ab.get_backing_store();
            for (i, byte) in bytes.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }
            let global = scope.get_current_context().global(scope);
            let promise_key = v8::String::new(scope, "Promise").unwrap();
            let resolve_key = v8::String::new(scope, "resolve").unwrap();
            if let Some(promise_ctor) = global.get(scope, promise_key.into()) {
                if let Some(promise_obj) = promise_ctor.to_object(scope) {
                    if let Some(resolve_fn) = promise_obj.get(scope, resolve_key.into()) {
                        if resolve_fn.is_function() {
                            let resolve = resolve_fn.cast::<v8::Function>();
                            if let Some(p) = resolve.call(scope, promise_ctor, &[ab.into()]) {
                                retval.set(p);
                                return;
                            }
                        }
                    }
                }
            }
            retval.set(ab.into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e).unwrap();
            let error = v8::Exception::error(scope, msg);
            scope.throw_exception(error);
        }
    }
}
