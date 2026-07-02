//! Web API V8 callbacks — crypto, performance, structuredClone, DOMException, Blob, FormData, Response

use std::cell::Cell;
use std::time::Instant;

thread_local! {
    pub(crate) static PERFORMANCE_BASELINE: Cell<Option<Instant>> = Cell::new(None);
}

pub(crate) fn crypto_get_random_values(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 1 {
        retval.set_undefined();
        return;
    }

    let arg = args.get(0);

    if let Some(uint8array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint8Array>().ok())
    {
        let length = uint8array.byte_length();
        if length == 0 {
            retval.set(arg);
            return;
        }
        let mut buffer = vec![0u8; length];
        if getrandom::getrandom(&mut buffer).is_err() {
            retval.set_undefined();
            return;
        }
        for (i, byte) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *byte as f64);
            uint8array.set(scope, idx.into(), val.into());
        }
        retval.set(arg);
        return;
    }

    if let Some(uint16array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint16Array>().ok())
    {
        let length = uint16array.byte_length() / 2;
        if length == 0 {
            retval.set(arg);
            return;
        }
        let mut buffer = vec![0u16; length];
        let byte_buffer = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 2)
        };
        if getrandom::getrandom(byte_buffer).is_err() {
            retval.set_undefined();
            return;
        }
        for (i, value) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *value as f64);
            uint16array.set(scope, idx.into(), val.into());
        }
        retval.set(arg);
        return;
    }

    if let Some(uint32array) = arg
        .to_object(scope)
        .and_then(|o| o.try_cast::<v8::Uint32Array>().ok())
    {
        let length = uint32array.byte_length() / 4;
        if length == 0 {
            retval.set(arg);
            return;
        }
        let mut buffer = vec![0u32; length];
        let byte_buffer = unsafe {
            std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, buffer.len() * 4)
        };
        if getrandom::getrandom(byte_buffer).is_err() {
            retval.set_undefined();
            return;
        }
        for (i, value) in buffer.iter().enumerate() {
            let idx = v8::Number::new(scope, i as f64);
            let val = v8::Number::new(scope, *value as f64);
            uint32array.set(scope, idx.into(), val.into());
        }
        retval.set(arg);
        return;
    }

    retval.set_undefined();
}

pub(crate) fn performance_now(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let now = Instant::now();
    let elapsed_ms = PERFORMANCE_BASELINE.with(|baseline| {
        if let Some(base) = baseline.get() {
            now.duration_since(base).as_nanos() as f64 / 1_000_000.0
        } else {
            0.0
        }
    });
    retval.set(v8::Number::new(scope, elapsed_ms).into());
}

pub(crate) fn structured_clone(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    if args.length() < 1 {
        retval.set_undefined();
        return;
    }
    let value = args.get(0);
    if let Some(json_string) = v8::json::stringify(scope, value) {
        if let Some(cloned) = v8::json::parse(scope, json_string.into()) {
            retval.set(cloned);
            return;
        }
    }
    // Non-JSON-serializable value (function, Symbol, circular ref) — throw DataCloneError
    let msg = v8::String::new(scope, "The object could not be cloned.").unwrap();
    let err = v8::Exception::error(scope, msg);
    scope.throw_exception(err);
}

pub(crate) fn dom_exception_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let message = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let name = if args.length() > 1 {
        args.get(1)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "Error".to_string())
    } else {
        "Error".to_string()
    };
    let msg_key = v8::String::new(scope, "message").unwrap();
    let msg_val = v8::String::new(scope, &message).unwrap();
    this.set(scope, msg_key.into(), msg_val.into());
    let name_key = v8::String::new(scope, "name").unwrap();
    let name_val = v8::String::new(scope, &name).unwrap();
    this.set(scope, name_key.into(), name_val.into());
    let stack_key = v8::String::new(scope, "stack").unwrap();
    let stack_str = format!("DOMException: {}", message);
    let stack_val = v8::String::new(scope, &stack_str).unwrap();
    this.set(scope, stack_key.into(), stack_val.into());
    retval.set(this.into());
}

pub(crate) fn blob_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let mut total_size: usize = 0;
    let mut parts: Vec<String> = Vec::new();

    if args.length() > 0 {
        let arg = args.get(0);
        if arg.is_array() {
            if let Some(array) = arg.to_object(scope) {
                if let Some(length_key) = v8::String::new(scope, "length") {
                    if let Some(length_val) = array.get(scope, length_key.into()) {
                        if let Some(length_num) = length_val.to_number(scope) {
                            let length = length_num.value() as usize;
                            for i in 0..length {
                                let idx = v8::Number::new(scope, i as f64);
                                if let Some(item) = array.get(scope, idx.into()) {
                                    if let Some(item_str) = item.to_string(scope) {
                                        let item_rust = item_str.to_rust_string_lossy(scope);
                                        total_size += item_rust.len();
                                        parts.push(item_rust);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut blob_type = String::new();
    if args.length() > 1 {
        let options = args.get(1);
        if let Some(options_obj) = options.to_object(scope) {
            if let Some(type_key) = v8::String::new(scope, "type") {
                if let Some(type_val) = options_obj.get(scope, type_key.into()) {
                    if let Some(type_str) = type_val.to_string(scope) {
                        blob_type = type_str.to_rust_string_lossy(scope);
                    }
                }
            }
        }
    }

    let size_key = v8::String::new(scope, "size").unwrap();
    let size_val = v8::Number::new(scope, total_size as f64);
    this.set(scope, size_key.into(), size_val.into());
    let type_key = v8::String::new(scope, "type").unwrap();
    let type_val = v8::String::new(scope, &blob_type).unwrap();
    this.set(scope, type_key.into(), type_val.into());
    let parts_key = v8::String::new(scope, "__blob_parts__").unwrap();
    let parts_val = v8::String::new(scope, &parts.join("")).unwrap();
    this.set(scope, parts_key.into(), parts_val.into());
    retval.set(this.into());
}

pub(crate) fn form_data_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let data_key = v8::String::new(scope, "__form_data__").unwrap();
    let data_val = v8::String::new(scope, "{}").unwrap();
    this.set(scope, data_key.into(), data_val.into());
    retval.set(this.into());
}

pub(crate) fn response_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let mut body_string = String::new();
    if args.length() > 0 {
        let arg = args.get(0);
        if !arg.is_null() && !arg.is_undefined() {
            if let Some(s) = arg.to_string(scope) {
                body_string = s.to_rust_string_lossy(scope);
            }
        }
    }

    let mut status = 200u16;
    let mut headers_obj: Option<v8::Local<v8::Object>> = None;

    if args.length() > 1 {
        let options = args.get(1);
        if let Some(opts) = options.to_object(scope) {
            let status_key = v8::String::new(scope, "status").unwrap();
            if let Some(status_val) = opts.get(scope, status_key.into()) {
                if !status_val.is_null() && !status_val.is_undefined() {
                    if let Some(num) = status_val.to_number(scope) {
                        let val = num.value();
                        if !val.is_nan() && val > 0.0 && val <= 65535.0 && val.fract() == 0.0 {
                            status = val as u16;
                        }
                    }
                }
            }
            let headers_key = v8::String::new(scope, "headers").unwrap();
            headers_obj = opts.get(scope, headers_key.into()).and_then(|h| h.to_object(scope));
        }
    }

    let status_key = v8::String::new(scope, "status").unwrap();
    let status_val = v8::Number::new(scope, status as f64);
    this.set(scope, status_key.into(), status_val.into());

    let headers = v8::Object::new(scope);
    let internal_headers_key = v8::String::new(scope, "__headers__").unwrap();
    let internal_headers = v8::Object::new(scope);
    headers.set(scope, internal_headers_key.into(), internal_headers.into());

    if let Some(hdrs) = headers_obj {
        if let Some(names) = hdrs.get_own_property_names(scope, Default::default()) {
            let len = names.length();
            for i in 0..len {
                if let Some(key) = names.get_index(scope, i) {
                    if let Some(key_str) = key.to_string(scope) {
                        let key_name = key_str.to_rust_string_lossy(scope);
                        if let Some(value) = hdrs.get(scope, key.into()) {
                            if let Some(value_str) = value.to_string(scope) {
                                let value_string = value_str.to_rust_string_lossy(scope);
                                let hkey = v8::String::new(scope, &key_name).unwrap();
                                let hval = v8::String::new(scope, &value_string).unwrap();
                                headers.set(scope, hkey.into(), hval.into());
                                internal_headers.set(scope, hkey.into(), hval.into());
                            }
                        }
                    }
                }
            }
        }
    }

    let headers_key = v8::String::new(scope, "headers").unwrap();
    this.set(scope, headers_key.into(), headers.into());
    let body_key = v8::String::new(scope, "body").unwrap();
    let body_val = v8::String::new(scope, &body_string).unwrap();
    this.set(scope, body_key.into(), body_val.into());

    let set_key = v8::String::new(scope, "set").unwrap();
    if let Some(set_fn) = v8::Function::new(scope, crate::runtime::url_api::headers_set_callback) {
        headers.set(scope, set_key.into(), set_fn.into());
    }

    retval.set(this.into());
}
