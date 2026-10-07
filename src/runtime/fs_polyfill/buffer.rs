//! Node `buffer` module callbacks.

use super::extract_bytes_arg;

// ============== Buffer module callbacks ==============

pub(super) fn buffer_from(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let bytes = if args.length() > 0 {
        extract_bytes_arg(scope, &args, 0).unwrap_or_default()
    } else {
        Vec::new()
    };
    let ab = v8::ArrayBuffer::new(scope, bytes.len());
    {
        let store = ab.get_backing_store();
        for (i, &byte) in bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(byte);
            }
        }
    }
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
        attach_buffer_tostring(scope, arr);
        retval.set(arr.into());
    }
}

/// Attach a `toString(encoding)` method to a Uint8Array so Buffer instances
/// support `.toString('hex' | 'base64' | 'utf8')`.
pub(super) fn attach_buffer_tostring(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    arr: v8::Local<v8::Uint8Array>,
) {
    if let Some(f) = v8::Function::new(scope, buffer_tostring_callback) {
        if let Some(k) = v8::String::new(scope, "toString") {
            arr.set(scope, k.into(), f.into());
        }
    }
}

pub(super) fn buffer_alloc(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let size = args
        .get(0)
        .to_integer(scope)
        .map(|n| n.value() as usize)
        .unwrap_or(0);
    let ab = v8::ArrayBuffer::new(scope, size);
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, size) {
        retval.set(arr.into());
    }
}

pub(super) fn buffer_is_buffer(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let is_buf = args.length() > 0 && args.get(0).is_uint8_array();
    retval.set(v8::Boolean::new(scope, is_buf).into());
}

pub(super) fn buffer_concat(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut all_bytes: Vec<u8> = Vec::new();
    if args.length() > 0 {
        let arr_val = args.get(0);
        if arr_val.is_array() {
            let arr = arr_val.cast::<v8::Array>();
            for i in 0..arr.length() {
                if let Some(item) = arr.get_index(scope, i) {
                    if let Ok(uint8arr) = item.try_cast::<v8::Uint8Array>() {
                        for j in 0..uint8arr.byte_length() {
                            if let Some(v) = uint8arr.get_index(scope, j as u32) {
                                if let Some(n) = v.to_integer(scope) {
                                    all_bytes.push(n.value() as u8);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let ab = v8::ArrayBuffer::new(scope, all_bytes.len());
    {
        let store = ab.get_backing_store();
        for (i, &byte) in all_bytes.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(byte);
            }
        }
    }
    if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, all_bytes.len()) {
        attach_buffer_tostring(scope, arr);
        retval.set(arr.into());
    }
}

/// toString(encoding) method attached directly to Uint8Array instances returned by buffer_from/buffer_concat
pub(super) fn buffer_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let encoding = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_else(|| "utf8".to_string())
    } else {
        "utf8".to_string()
    };

    let bytes: Vec<u8> =
        crate::runtime::v8_helpers::extract_bytes_from_v8_value(scope, this.into())
            .unwrap_or_default();

    let result = match encoding.as_str() {
        "hex" => bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>(),
        "base64" => {
            use base64::{engine::general_purpose, Engine as _};
            general_purpose::STANDARD.encode(&bytes)
        }
        _ => {
            // utf8 / latin1 / ascii — all default to UTF-8 lossy
            String::from_utf8_lossy(&bytes).into_owned()
        }
    };

    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

