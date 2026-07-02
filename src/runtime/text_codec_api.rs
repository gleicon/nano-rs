//! TextEncoder and TextDecoder API bindings for JavaScript runtime

/// TextEncoder constructor callback
fn text_encoder_constructor(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Constructor - creates TextEncoder instance
    // No internal state needed for basic UTF-8 encoding
}

/// TextEncoder.encode() implementation
fn text_encoder_encode(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get first argument as string
    if args.length() == 0 {
        // Return empty Uint8Array
        let empty = v8::ArrayBuffer::new(scope, 0);
        if let Some(uint8array) = v8::Uint8Array::new(scope, empty, 0, 0) {
            retval.set(uint8array.into());
        }
        return;
    }

    let arg = args.get(0);
    let text = if let Some(s) = arg.to_string(scope) {
        s.to_rust_string_lossy(scope)
    } else {
        String::new()
    };

    // Encode to UTF-8 bytes
    let bytes = text.into_bytes();

    // Create ArrayBuffer and copy bytes
    let ab = v8::ArrayBuffer::new(scope, bytes.len());
    let store = ab.get_backing_store();

    // Copy bytes into ArrayBuffer
    for (i, byte) in bytes.iter().enumerate() {
        if let Some(cell) = store.get(i) {
            cell.set(*byte);
        }
    }

    // Create Uint8Array view
    if let Some(uint8array) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
        retval.set(uint8array.into());
    } else {
        retval.set(ab.into());
    }
}

/// TextDecoder constructor callback
fn text_decoder_constructor(
    _scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Constructor - TextDecoder always uses UTF-8 in WinterTC
    // No internal state needed
}

/// TextDecoder.decode() implementation
fn text_decoder_decode(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get first argument (should be ArrayBuffer or Uint8Array)
    if args.length() == 0 {
        retval.set(v8::String::new(scope, "").unwrap().into());
        return;
    }

    let arg = args.get(0);

    // Try to extract bytes from Uint8Array
    let bytes = if arg.is_uint8_array() {
        let uint8array = arg.cast::<v8::Uint8Array>();
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
    } else if arg.is_array_buffer() {
        let arraybuffer = arg.cast::<v8::ArrayBuffer>();
        // Extract bytes from ArrayBuffer
        let store = arraybuffer.get_backing_store();
        let length = arraybuffer.byte_length();
        (0..length)
            .filter_map(|i| store.get(i).map(|cell| cell.get()))
            .collect()
    } else {
        Vec::new()
    };

    // Decode UTF-8 bytes to string (with replacement for invalid sequences)
    let text = String::from_utf8_lossy(&bytes);

    // Return as JS string
    if let Some(s) = v8::String::new(scope, &text) {
        retval.set(s.into());
    }
}

/// Bind TextEncoder API to global scope
pub(crate) fn bind_text_encoder(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create TextEncoder constructor function
    let encoder_template = v8::FunctionTemplate::new(&mut ctx_scope, text_encoder_constructor);

    // Add encode method to prototype via instance template
    let instance_template = encoder_template.prototype_template(&mut &mut ctx_scope);
    let encode_fn = v8::FunctionTemplate::new(&mut ctx_scope, text_encoder_encode);
    let encode_key = v8::String::new(&mut ctx_scope, "encode").unwrap();
    instance_template.set(encode_key.into(), encode_fn.into());

    let encoder_ctor = encoder_template.get_function(&mut &mut ctx_scope).unwrap();

    // Attach TextEncoder to global
    let key = v8::String::new(&mut ctx_scope, "TextEncoder").unwrap();
    global.set(&mut ctx_scope, key.into(), encoder_ctor.into());
}

/// Bind TextDecoder API to global scope
pub(crate) fn bind_text_decoder(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create TextDecoder constructor function
    let decoder_template = v8::FunctionTemplate::new(&mut ctx_scope, text_decoder_constructor);

    // Add decode method to prototype via instance template
    let instance_template = decoder_template.prototype_template(&mut &mut ctx_scope);
    let decode_fn = v8::FunctionTemplate::new(&mut ctx_scope, text_decoder_decode);
    let decode_key = v8::String::new(&mut ctx_scope, "decode").unwrap();
    instance_template.set(decode_key.into(), decode_fn.into());

    let decoder_ctor = decoder_template.get_function(&mut &mut ctx_scope).unwrap();

    // Attach TextDecoder to global
    let key = v8::String::new(&mut ctx_scope, "TextDecoder").unwrap();
    global.set(&mut ctx_scope, key.into(), decoder_ctor.into());
}
