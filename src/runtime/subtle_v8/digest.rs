//! SubtleCrypto digest operation.

use super::extract_array_buffer_view;

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
            let msg =
                v8::String::new(scope, "First argument (algorithm) must be a string").unwrap();
            let error = v8::Exception::type_error(scope, msg);
            retval.set(error);
            return;
        }
    };

    // Get data as bytes
    let data = match extract_array_buffer_view(scope, args.get(1)) {
        Some(bytes) => bytes,
        None => {
            let msg = v8::String::new(scope, "Second argument (data) must be an ArrayBufferView")
                .unwrap();
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
                            if let Some(resolved_promise) =
                                resolve.call(scope, promise_ctor, &[ab.into()])
                            {
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

