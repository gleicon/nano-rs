//! SubtleCrypto V8 callback functions — extracted from apis.rs.
//!
//! Split by operation family; shared key/buffer extraction helpers live here.

mod cipher;
mod derive;
mod digest;
mod keys;
mod signing;

pub(crate) use cipher::{subtle_decrypt, subtle_encrypt};
pub(crate) use derive::subtle_derive_bits;
pub(crate) use digest::subtle_digest;
pub(crate) use keys::{subtle_export_key, subtle_generate_key, subtle_import_key};
pub(crate) use signing::{subtle_sign, subtle_verify};

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

