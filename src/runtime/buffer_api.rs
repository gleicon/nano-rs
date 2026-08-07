//! Node.js Buffer API bindings for JavaScript runtime

/// Buffer constructor callback
fn buffer_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get size argument
    let size = if args.length() > 0 {
        let arg = args.get(0);
        if let Some(num) = arg.to_number(scope) {
            num.value() as usize
        } else if let Some(str) = arg.to_string(scope) {
            str.to_rust_string_lossy(scope).len()
        } else {
            0
        }
    } else {
        0
    };

    // Create Uint8Array as buffer backing
    let buffer = v8::ArrayBuffer::new(scope, size);
    let uint8_array = v8::Uint8Array::new(scope, buffer, 0, size).unwrap();

    // Add toString method for Buffer compatibility
    add_buffer_tostring_to_instance(scope, uint8_array.into());

    retval.set(uint8_array.into());
}

/// Helper to add toString method to a Uint8Array instance for Buffer compatibility
fn add_buffer_tostring_to_instance(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    obj: v8::Local<v8::Object>,
) {
    // Create the toString function
    if let Some(tostring_fn) = v8::Function::new(scope, buffer_tostring_callback) {
        let tostring_key = v8::String::new(scope, "toString").unwrap();
        // Set as own property (not prototype) to override Uint8Array's toString
        let _ = obj.set(scope, tostring_key.into(), tostring_fn.into());
    }
}

/// Buffer.from() static method
fn buffer_from_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() == 0 {
        let empty = v8::ArrayBuffer::new(scope, 0);
        let arr = v8::Uint8Array::new(scope, empty, 0, 0).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    let arg = args.get(0);

    // Handle ArrayBuffer input
    if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(arg) {
        let store = ab.get_backing_store();
        let len = store.len();
        let out = v8::ArrayBuffer::new(scope, len);
        let out_store = out.get_backing_store();
        for i in 0..len {
            if let (Some(src), Some(dst)) = (store.get(i), out_store.get(i)) {
                dst.set(src.get());
            }
        }
        let arr = v8::Uint8Array::new(scope, out, 0, len).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    // Handle ArrayBufferView (Uint8Array, etc.) input
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(arg) {
        let len = view.byte_length();
        let buffer = v8::ArrayBuffer::new(scope, len);
        let store = buffer.get_backing_store();
        let mut tmp = vec![0u8; len];
        view.copy_contents(&mut tmp);
        for (i, byte) in tmp.iter().enumerate() {
            if let Some(cell) = store.get(i) {
                cell.set(*byte);
            }
        }
        let arr = v8::Uint8Array::new(scope, buffer, 0, len).unwrap();
        add_buffer_tostring_to_instance(scope, arr.into());
        retval.set(arr.into());
        return;
    }

    // Handle array-like input (check BEFORE string coercion — to_string() coerces arrays)
    if arg.is_array() {
        if let Some(obj) = arg.to_object(scope) {
            let len_key = v8::String::new(scope, "length").unwrap();
            if let Some(len_val) = obj.get(scope, len_key.into()) {
                if let Some(len_num) = len_val.to_number(scope) {
                    let len = len_num.value() as usize;
                    let buffer = v8::ArrayBuffer::new(scope, len);
                    let store = buffer.get_backing_store();
                    for i in 0..len {
                        let idx = v8::Number::new(scope, i as f64);
                        if let Some(val) = obj.get(scope, idx.into()) {
                            if let Some(num) = val.to_number(scope) {
                                if let Some(cell) = store.get(i) {
                                    cell.set(num.value() as u8);
                                }
                            }
                        }
                    }
                    let arr = v8::Uint8Array::new(scope, buffer, 0, len).unwrap();
                    add_buffer_tostring_to_instance(scope, arr.into());
                    retval.set(arr.into());
                    return;
                }
            }
        }
    }

    // Handle string input (after array check — to_string() would coerce arrays)
    if arg.is_string() {
        if let Some(str_val) = arg.to_string(scope) {
            let text = str_val.to_rust_string_lossy(scope);

            // Check encoding argument (args[1]).
            let encoding = if args.length() > 1 {
                if let Some(enc) = args.get(1).to_string(scope) {
                    enc.to_rust_string_lossy(scope).to_ascii_lowercase()
                } else {
                    "utf8".to_string()
                }
            } else {
                "utf8".to_string()
            };

            let bytes: Vec<u8> = match encoding.as_str() {
                "hex" => {
                    // Decode hex pairs: "68656c6c6f" → [0x68, 0x65, …]
                    // Odd-length or invalid chars produce truncated/best-effort output
                    // (matches Node.js behaviour).
                    (0..text.len())
                        .step_by(2)
                        .filter_map(|i| {
                            text.get(i..i + 2)
                                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                        })
                        .collect()
                }
                // "utf8" | "utf-8" | "ascii" | "latin1" | "binary" | anything else
                _ => text.as_bytes().to_vec(),
            };

            let buffer = v8::ArrayBuffer::new(scope, bytes.len());
            let store = buffer.get_backing_store();
            for (i, byte) in bytes.iter().enumerate() {
                if let Some(cell) = store.get(i) {
                    cell.set(*byte);
                }
            }
            let arr = v8::Uint8Array::new(scope, buffer, 0, bytes.len()).unwrap();
            add_buffer_tostring_to_instance(scope, arr.into());
            retval.set(arr.into());
            return;
        }
    }

    // Default: return empty buffer
    let empty = v8::ArrayBuffer::new(scope, 0);
    let arr = v8::Uint8Array::new(scope, empty, 0, 0).unwrap();
    add_buffer_tostring_to_instance(scope, arr.into());
    retval.set(arr.into());
}

/// Buffer.alloc() static method
fn buffer_alloc_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let size = if args.length() > 0 {
        let arg = args.get(0);
        if let Some(num) = arg.to_number(scope) {
            num.value() as usize
        } else {
            0
        }
    } else {
        0
    };

    let fill_value = if args.length() > 1 {
        let arg = args.get(1);
        if let Some(num) = arg.to_number(scope) {
            num.value() as u8
        } else {
            0
        }
    } else {
        0
    };

    let buffer = v8::ArrayBuffer::new(scope, size);
    if fill_value != 0 {
        let store = buffer.get_backing_store();
        for i in 0..size {
            if let Some(cell) = store.get(i) {
                cell.set(fill_value);
            }
        }
    }
    let arr = v8::Uint8Array::new(scope, buffer, 0, size).unwrap();
    add_buffer_tostring_to_instance(scope, arr.into());
    retval.set(arr.into());
}

/// Buffer.prototype.toString() callback - decodes buffer to UTF-8 string
fn buffer_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Extract bytes from the Uint8Array (which is what Buffer is)
    let bytes = if let Some(uint8array) = this
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
        vec
    } else {
        // Fallback: return empty string
        retval.set(v8::String::new(scope, "").unwrap().into());
        return;
    };

    // Decode bytes to UTF-8 string
    let text = String::from_utf8_lossy(&bytes);

    // Return the decoded string
    if let Some(s) = v8::String::new(scope, &text) {
        retval.set(s.into());
    } else {
        retval.set(v8::String::new(scope, "").unwrap().into());
    }
}

/// Bind Node.js Buffer API
pub(crate) fn bind_buffer(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create Buffer constructor function
    let buffer_template = v8::FunctionTemplate::new(&mut ctx_scope, buffer_constructor);
    let buffer_ctor = buffer_template.get_function(&mut ctx_scope).unwrap();

    // Attach static methods
    let from_key = v8::String::new(&mut ctx_scope, "from").unwrap();
    if let Some(from_fn) = v8::Function::new(&mut ctx_scope, buffer_from_callback) {
        buffer_ctor.set(&mut ctx_scope, from_key.into(), from_fn.into());
    }

    let alloc_key = v8::String::new(&mut ctx_scope, "alloc").unwrap();
    if let Some(alloc_fn) = v8::Function::new(&mut ctx_scope, buffer_alloc_callback) {
        buffer_ctor.set(&mut ctx_scope, alloc_key.into(), alloc_fn.into());
    }

    // Add toString method to Buffer prototype for Node.js compatibility
    if let Some(ctor_obj) = buffer_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(tostring_fn) =
                    v8::Function::new(&mut ctx_scope, buffer_tostring_callback)
                {
                    let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                    proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                }
            }
        }
    }

    // Attach to global
    let key = v8::String::new(&mut ctx_scope, "Buffer").unwrap();
    global.set(&mut ctx_scope, key.into(), buffer_ctor.into());
}
