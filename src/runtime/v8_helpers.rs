use v8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("V8 platform init failed");
    }

    fn with_scope<F: FnOnce(&mut v8::ContextScope<v8::HandleScope>)>(f: F) {
        init_platform();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(handle_scope, &mut isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);
        f(scope);
    }

    #[test]
    fn test_extract_bytes_arraybuffer() {
        with_scope(|scope| {
            let ab = v8::ArrayBuffer::new(scope, 3);
            {
                let store = ab.get_backing_store();
                store.get(0).unwrap().set(10);
                store.get(1).unwrap().set(20);
                store.get(2).unwrap().set(30);
            }
            let val: v8::Local<v8::Value> = ab.into();
            let bytes = extract_bytes_from_v8_value(scope, val).unwrap();
            assert_eq!(bytes, vec![10, 20, 30]);
        });
    }

    #[test]
    fn test_extract_bytes_string_fallback() {
        with_scope(|scope| {
            let s = v8::String::new(scope, "hello").unwrap();
            let val: v8::Local<v8::Value> = s.into();
            let bytes = extract_bytes_from_v8_value(scope, val).unwrap();
            assert_eq!(bytes, b"hello");
        });
    }

    #[test]
    fn test_extract_bytes_uint8array_with_offset() {
        with_scope(|scope| {
            // Create a 5-byte buffer, slice bytes 1..3 as a Uint8Array
            let ab = v8::ArrayBuffer::new(scope, 5);
            {
                let store = ab.get_backing_store();
                for i in 0..5u8 {
                    store.get(i as usize).unwrap().set(i + 1);
                }
            }
            // offset=1, length=3 → bytes [2, 3, 4]
            let arr = v8::Uint8Array::new(scope, ab, 1, 3).unwrap();
            let val: v8::Local<v8::Value> = arr.into();
            let bytes = extract_bytes_from_v8_value(scope, val).unwrap();
            assert_eq!(bytes, vec![2, 3, 4]);
        });
    }

    #[test]
    fn test_extract_bytes_null_returns_none() {
        with_scope(|scope| {
            let val: v8::Local<v8::Value> = v8::null(scope).into();
            assert!(extract_bytes_from_v8_value(scope, val).is_none());
        });
    }
}

/// Extract bytes from a V8 value (Uint8Array, ArrayBuffer, or string).
///
/// Uses backing store access for typed arrays — avoids per-element V8 API overhead.
pub fn extract_bytes_from_v8_value(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    val: v8::Local<v8::Value>,
) -> Option<Vec<u8>> {
    if val.is_null_or_undefined() {
        return None;
    }

    if let Ok(arr) = val.try_cast::<v8::Uint8Array>() {
        let len = arr.byte_length();
        if let Some(ab) = arr.buffer(scope) {
            let store = ab.get_backing_store();
            let offset = arr.byte_offset();
            return Some(
                (offset..offset + len)
                    .filter_map(|i| store.get(i).map(|c| c.get()))
                    .collect(),
            );
        }
        // Fallback: backing store unavailable in this V8 context — use element-wise access
        let mut bytes = Vec::with_capacity(len);
        for i in 0..len as u32 {
            if let Some(v) = arr.get_index(scope, i) {
                if let Some(n) = v.to_integer(scope) {
                    bytes.push(n.value() as u8);
                }
            }
        }
        return Some(bytes);
    }

    if let Ok(ab) = val.try_cast::<v8::ArrayBuffer>() {
        let store = ab.get_backing_store();
        let len = ab.byte_length();
        return Some(
            (0..len)
                .filter_map(|i| store.get(i).map(|c| c.get()))
                .collect(),
        );
    }

    if let Some(s) = val.to_string(scope) {
        return Some(s.to_rust_string_lossy(scope).into_bytes());
    }

    None
}
