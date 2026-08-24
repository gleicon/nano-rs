//! URL, URLSearchParams, and Headers API bindings for JavaScript runtime

/// Callback for headers.set() method (used in response_constructor)
pub(crate) fn headers_set_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    // Get the headers object (this)
    let this = args.this();

    // Get header name and value
    if args.length() >= 2 {
        let name = args
            .get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default();
        let value = args
            .get(1)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();

        let headers_key = v8::String::new(scope, "__headers__").unwrap();
        if let Some(headers_val) = this.get(scope, headers_key.into()) {
            if let Some(headers_obj) = headers_val.to_object(scope) {
                let key = v8::String::new(scope, &name).unwrap();
                let val = v8::String::new(scope, &value).unwrap();
                headers_obj.set(scope, key.into(), val.into());
            }
        }
    }
}

/// URL constructor implementation (simplified v1)
fn url_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let global = scope.get_current_context().global(scope);

    // Get the URL string argument
    let url_string = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Parse the URL to extract components
    let parsed = url::Url::parse(&url_string)
        .unwrap_or_else(|_| url::Url::parse("http://localhost/").unwrap());

    // Set href property (full URL)
    let href_key = v8::String::new(scope, "href").unwrap();
    let href_val = v8::String::new(scope, parsed.as_str()).unwrap();
    this.set(scope, href_key.into(), href_val.into());

    // Set protocol property
    let protocol_key = v8::String::new(scope, "protocol").unwrap();
    let protocol = format!("{}:", parsed.scheme());
    let protocol_val = v8::String::new(scope, &protocol).unwrap();
    this.set(scope, protocol_key.into(), protocol_val.into());

    // Set host property (hostname:port)
    let host_key = v8::String::new(scope, "host").unwrap();
    let host = if let Some(port) = parsed.port() {
        format!("{}:{}", parsed.host_str().unwrap_or(""), port)
    } else {
        parsed.host_str().unwrap_or("").to_string()
    };
    let host_val = v8::String::new(scope, &host).unwrap();
    this.set(scope, host_key.into(), host_val.into());

    // Set hostname property
    let hostname_key = v8::String::new(scope, "hostname").unwrap();
    let hostname = parsed.host_str().unwrap_or("");
    let hostname_val = v8::String::new(scope, hostname).unwrap();
    this.set(scope, hostname_key.into(), hostname_val.into());

    // Set port property
    let port_key = v8::String::new(scope, "port").unwrap();
    let port = parsed.port().map(|p| p.to_string()).unwrap_or_default();
    let port_val = v8::String::new(scope, &port).unwrap();
    this.set(scope, port_key.into(), port_val.into());

    // Set pathname property
    let pathname_key = v8::String::new(scope, "pathname").unwrap();
    let pathname = parsed.path();
    let pathname_val = v8::String::new(scope, pathname).unwrap();
    this.set(scope, pathname_key.into(), pathname_val.into());

    // Set search property (query string with ?)
    let search_key = v8::String::new(scope, "search").unwrap();
    let search = if parsed.query().is_some() {
        format!("?{}", parsed.query().unwrap_or(""))
    } else {
        String::new()
    };
    let search_val = v8::String::new(scope, &search).unwrap();
    this.set(scope, search_key.into(), search_val.into());

    // Set hash property (fragment with #)
    let hash_key = v8::String::new(scope, "hash").unwrap();
    let hash = if let Some(fragment) = parsed.fragment() {
        format!("#{}", fragment)
    } else {
        String::new()
    };
    let hash_val = v8::String::new(scope, &hash).unwrap();
    this.set(scope, hash_key.into(), hash_val.into());

    // Set searchParams property with URLSearchParams instance
    let search_params_key = v8::String::new(scope, "searchParams").unwrap();
    let search_params_ctor_key = v8::String::new(scope, "URLSearchParams").unwrap();
    if let Some(usp_ctor) = global.get(scope, search_params_ctor_key.into()) {
        if usp_ctor.is_function() {
            let usp_fn = usp_ctor.cast::<v8::Function>();
            // Pass the query string (without ?) to URLSearchParams constructor
            let query_str = parsed.query().unwrap_or("");
            let query_val = v8::String::new(scope, query_str).unwrap();
            if let Some(search_params) = usp_fn.new_instance(scope, &[query_val.into()]) {
                this.set(scope, search_params_key.into(), search_params.into());
            }
        }
    }

    retval.set(this.into());
}

/// URL.prototype.toString() callback - returns the href property
fn url_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = _args.this();

    // Get the href property from this URL object
    let href_key = v8::String::new(scope, "href").unwrap();
    if let Some(href_val) = this.get(scope, href_key.into()) {
        if let Some(href_str) = href_val.to_string(scope) {
            retval.set(href_str.into());
            return;
        }
    }

    // Fallback: return empty string
    retval.set(v8::String::new(scope, "").unwrap().into());
}

/// URL.prototype.href getter callback
fn url_href_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = _args.this();

    // Get the href property from this URL object
    let href_key = v8::String::new(scope, "href").unwrap();
    if let Some(href_val) = this.get(scope, href_key.into()) {
        retval.set(href_val);
        return;
    }

    // Fallback: return empty string
    retval.set(v8::String::new(scope, "").unwrap().into());
}

/// URLSearchParams constructor implementation
fn url_search_params_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Initialize internal params store as a plain Object (like Headers does)
    let params_key = v8::String::new(scope, "__params__").unwrap();
    let params_obj = v8::Object::new(scope);
    this.set(scope, params_key.into(), params_obj.into());

    // Parse init argument if provided
    if args.length() > 0 {
        let init = args.get(0);
        if let Some(init_str) = init.to_string(scope) {
            let query_string = init_str.to_rust_string_lossy(scope);
            // Parse query string like "foo=bar&baz=qux"
            for pair in query_string.split('&') {
                if let Some(eq_pos) = pair.find('=') {
                    let key = &pair[..eq_pos];
                    let value = &pair[eq_pos + 1..];
                    let key_val = v8::String::new(scope, key).unwrap();
                    let value_val = v8::String::new(scope, value).unwrap();
                    params_obj.set(scope, key_val.into(), value_val.into());
                } else if !pair.is_empty() {
                    let key_val = v8::String::new(scope, pair).unwrap();
                    let empty_val = v8::String::new(scope, "").unwrap();
                    params_obj.set(scope, key_val.into(), empty_val.into());
                }
            }
        }
    }

    retval.set(this.into());
}

/// URLSearchParams.get() callback
fn usp_get_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set_null();
        return;
    }

    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            if let Some(value) = params_obj.get(scope, name_key.into()) {
                if !value.is_null() && !value.is_undefined() {
                    retval.set(value);
                    return;
                }
            }
        }
    }

    retval.set_null();
}

/// URLSearchParams.set() callback
fn usp_set_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 2 {
        retval.set_undefined();
        return;
    }

    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let value = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Create string keys for consistent lookup
    let name_key = v8::String::new(scope, &name).unwrap();
    let value_key = v8::String::new(scope, &value).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            params_obj.set(scope, name_key.into(), value_key.into());
        }
    }

    retval.set_undefined();
}

/// URLSearchParams.has() callback
fn usp_has_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set(v8::Boolean::new(scope, false).into());
        return;
    }

    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            // Check if key exists directly in the object
            if let Some(val) = params_obj.get(scope, name_key.into()) {
                if !val.is_null() && !val.is_undefined() {
                    retval.set(v8::Boolean::new(scope, true).into());
                    return;
                }
            }
        }
    }

    retval.set(v8::Boolean::new(scope, false).into());
}

/// URLSearchParams.delete() callback
fn usp_delete_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        retval.set_undefined();
        return;
    }

    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    // Create the lookup key as a V8 string (must match how keys were stored)
    let name_key = v8::String::new(scope, &name).unwrap();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            // Delete directly from the object
            let _ = params_obj.delete(scope, name_key.into());
        }
    }

    retval.set_undefined();
}

/// URLSearchParams.forEach(callback) — calls callback(value, name, searchParams) per entry
fn usp_foreach_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();
    if args.length() < 1 {
        return;
    }
    let cb = args.get(0);
    if !cb.is_function() {
        return;
    }
    let cb_fn = cb.cast::<v8::Function>();

    let params_key = v8::String::new(scope, "__params__").unwrap();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            if let Some(names) = params_obj.get_own_property_names(scope, Default::default()) {
                for i in 0..names.length() {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(val) = params_obj.get(scope, key) {
                            // callback(value, name, searchParams) per WHATWG spec
                            let _ = cb_fn.call(scope, this.into(), &[val, key, this.into()]);
                        }
                    }
                }
            }
        }
    }
}

/// URLSearchParams.entries() — returns array of [name, value] pairs
fn usp_entries_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let params_key = v8::String::new(scope, "__params__").unwrap();
    let result = v8::Array::new(scope, 0);
    let mut idx = 0u32;
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            if let Some(names) = params_obj.get_own_property_names(scope, Default::default()) {
                for i in 0..names.length() {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(val) = params_obj.get(scope, key) {
                            let pair = v8::Array::new(scope, 2);
                            pair.set_index(scope, 0, key);
                            pair.set_index(scope, 1, val);
                            result.set_index(scope, idx, pair.into());
                            idx += 1;
                        }
                    }
                }
            }
        }
    }
    retval.set(result.into());
}

/// URLSearchParams.toString() callback — serialize `__params__` as `k=v&k=v`.
///
/// `__params__` is a plain object (see the constructor), so we iterate its own
/// property names and join them. Keys/values are form-urlencoded so the output
/// round-trips through the constructor and is a valid query string.
fn usp_tostring_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let params_key = v8::String::new(scope, "__params__").unwrap();

    let mut pairs: Vec<String> = Vec::new();
    if let Some(params_val) = this.get(scope, params_key.into()) {
        if let Some(params_obj) = params_val.to_object(scope) {
            if let Some(names) = params_obj.get_own_property_names(scope, Default::default()) {
                for i in 0..names.length() {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(val) = params_obj.get(scope, key) {
                            let k = key
                                .to_string(scope)
                                .map(|s| s.to_rust_string_lossy(scope))
                                .unwrap_or_default();
                            let v = val
                                .to_string(scope)
                                .map(|s| s.to_rust_string_lossy(scope))
                                .unwrap_or_default();
                            pairs.push(format!("{}={}", form_urlencode(&k), form_urlencode(&v)));
                        }
                    }
                }
            }
        }
    }

    let result = v8::String::new(scope, &pairs.join("&")).unwrap();
    retval.set(result.into());
}

/// Encode a URLSearchParams key or value per the WHATWG form-urlencoded rules:
/// space → `+`, and anything outside the unreserved set is percent-encoded.
fn form_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Headers constructor implementation (simplified v1)
fn headers_constructor(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Initialize internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    let headers_val = v8::Object::new(scope);
    this.set(scope, headers_key.into(), headers_val.into());

    // If an initial headers object is provided, copy its values
    if args.length() > 0 {
        let init = args.get(0);
        if let Some(init_obj) = init.to_object(scope) {
            // Try to iterate over the object
            if let Some(names) = init_obj.get_own_property_names(scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            // Normalize header name to lowercase (per Fetch spec)
                            let key_name = key_str.to_rust_string_lossy(scope).to_lowercase();
                            if let Some(value) = init_obj.get(scope, key.into()) {
                                if let Some(value_str) = value.to_string(scope) {
                                    let value_string = value_str.to_rust_string_lossy(scope);
                                    let hkey = v8::String::new(scope, &key_name).unwrap();
                                    let hval = v8::String::new(scope, &value_string).unwrap();
                                    headers_val.set(scope, hkey.into(), hval.into());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    bind_header_methods(scope, this);

    retval.set(this.into());
}

/// Bind the Fetch-spec Headers methods (get/set/has/delete/append/forEach) onto
/// a headers object. Single source of truth for both the `Headers` constructor
/// and the `Response` constructor's inline headers object.
pub(crate) fn bind_header_methods(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    obj: v8::Local<v8::Object>,
) {
    macro_rules! bind {
        ($name:expr, $cb:expr) => {
            if let (Some(f), Some(k)) =
                (v8::Function::new(scope, $cb), v8::String::new(scope, $name))
            {
                obj.set(scope, k.into(), f.into());
            }
        };
    }
    bind!("get", headers_get_callback);
    bind!("set", headers_set_callback);
    bind!("has", headers_has_callback);
    bind!("delete", headers_delete_callback);
    bind!("append", headers_append_callback);
    bind!("forEach", headers_foreach_callback);
}

/// Callback for Headers.get() method
pub(crate) fn headers_get_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();

    // Get the header name and normalize to lowercase (per Fetch spec)
    let name = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Get the internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            let name_key = v8::String::new(scope, &name).unwrap();
            if let Some(value) = headers_obj.get(scope, name_key.into()) {
                if !value.is_null() && !value.is_undefined() {
                    retval.set(value);
                    return;
                }
            }
        }
    }

    // Return null if not found
    retval.set_null();
}

/// Callback for Headers.set() method (version for Headers object)
/// Callback for Headers.forEach() method
pub(crate) fn headers_foreach_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();

    if args.length() < 1 {
        return;
    }

    let callback = args.get(0);
    if !callback.is_function() {
        return;
    }
    let callback_fn = callback.cast::<v8::Function>();

    // Get the internal headers store
    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            // Iterate over all properties
            if let Some(names) = headers_obj.get_own_property_names(scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            let key_name = key_str.to_rust_string_lossy(scope);
                            if let Some(value) = headers_obj.get(scope, key.into()) {
                                // Call the callback with (value, key, headers)
                                let key_js = v8::String::new(scope, &key_name).unwrap();
                                let _ = callback_fn.call(
                                    scope,
                                    this.into(),
                                    &[value, key_js.into(), this.into()],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Callback for Headers.append() — adds value, comma-joining if header already exists
pub(crate) fn headers_append_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();
    if args.length() < 2 {
        return;
    }
    let name = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
        .unwrap_or_default();
    let value = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();

    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            let name_key = v8::String::new(scope, &name).unwrap();
            let new_val = if let Some(existing) = headers_obj.get(scope, name_key.into()) {
                if !existing.is_null() && !existing.is_undefined() {
                    let existing_str = existing
                        .to_string(scope)
                        .map(|s| s.to_rust_string_lossy(scope))
                        .unwrap_or_default();
                    format!("{}, {}", existing_str, value)
                } else {
                    value
                }
            } else {
                value
            };
            let val_str = v8::String::new(scope, &new_val).unwrap();
            headers_obj.set(scope, name_key.into(), val_str.into());
        }
    }
}

/// Callback for Headers.has() — returns boolean
pub(crate) fn headers_has_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let name = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    let found = if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            let name_key = v8::String::new(scope, &name).unwrap();
            headers_obj
                .get(scope, name_key.into())
                .map(|v| !v.is_null() && !v.is_undefined())
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    retval.set(v8::Boolean::new(scope, found).into());
}

/// Callback for Headers.delete() — removes a header
pub(crate) fn headers_delete_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let this = args.this();
    let name = if args.length() > 0 {
        args.get(0)
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope).to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    };

    let headers_key = v8::String::new(scope, "__headers__").unwrap();
    if let Some(headers_val) = this.get(scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(scope) {
            let name_key = v8::String::new(scope, &name).unwrap();
            headers_obj.delete(scope, name_key.into());
        }
    }
}

/// Bind URL constructor for WinterTC compatibility
pub(crate) fn bind_url(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create URLSearchParams constructor first (needed by URL)
    let usp_template = v8::FunctionTemplate::new(&mut ctx_scope, url_search_params_constructor);
    let usp_ctor = usp_template.get_function(&mut ctx_scope).unwrap();

    // Add prototype methods to URLSearchParams
    if let Some(usp_obj) = usp_ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = usp_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                // Bind get method
                if let Some(get_fn) = v8::Function::new(&mut ctx_scope, usp_get_callback) {
                    let get_key = v8::String::new(&mut ctx_scope, "get").unwrap();
                    proto_obj.set(&mut ctx_scope, get_key.into(), get_fn.into());
                }
                // Bind set method
                if let Some(set_fn) = v8::Function::new(&mut ctx_scope, usp_set_callback) {
                    let set_key = v8::String::new(&mut ctx_scope, "set").unwrap();
                    proto_obj.set(&mut ctx_scope, set_key.into(), set_fn.into());
                }
                // Bind has method
                if let Some(has_fn) = v8::Function::new(&mut ctx_scope, usp_has_callback) {
                    let has_key = v8::String::new(&mut ctx_scope, "has").unwrap();
                    proto_obj.set(&mut ctx_scope, has_key.into(), has_fn.into());
                }
                // Bind delete method
                if let Some(delete_fn) = v8::Function::new(&mut ctx_scope, usp_delete_callback) {
                    let delete_key = v8::String::new(&mut ctx_scope, "delete").unwrap();
                    proto_obj.set(&mut ctx_scope, delete_key.into(), delete_fn.into());
                }
                // Bind toString method
                if let Some(tostring_fn) = v8::Function::new(&mut ctx_scope, usp_tostring_callback)
                {
                    let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                    proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                }
                // Bind forEach method
                if let Some(foreach_fn) = v8::Function::new(&mut ctx_scope, usp_foreach_callback) {
                    let foreach_key = v8::String::new(&mut ctx_scope, "forEach").unwrap();
                    proto_obj.set(&mut ctx_scope, foreach_key.into(), foreach_fn.into());
                }
                // Bind entries method
                if let Some(entries_fn) = v8::Function::new(&mut ctx_scope, usp_entries_callback) {
                    let entries_key = v8::String::new(&mut ctx_scope, "entries").unwrap();
                    proto_obj.set(&mut ctx_scope, entries_key.into(), entries_fn.into());
                }
            }
        }
    }

    // Attach URLSearchParams to global
    let usp_key = v8::String::new(&mut ctx_scope, "URLSearchParams").unwrap();
    global.set(&mut ctx_scope, usp_key.into(), usp_ctor.into());

    // Create URL constructor
    let template = v8::FunctionTemplate::new(&mut ctx_scope, url_constructor);
    let ctor = template.get_function(&mut ctx_scope).unwrap();

    // Add toString method to URL prototype
    if let Some(ctor_obj) = ctor.to_object(&mut ctx_scope) {
        let proto_key = v8::String::new(&mut ctx_scope, "prototype").unwrap();
        if let Some(proto) = ctor_obj.get(&mut ctx_scope, proto_key.into()) {
            if let Some(proto_obj) = proto.to_object(&mut ctx_scope) {
                if let Some(tostring_fn) = v8::Function::new(&mut ctx_scope, url_tostring_callback)
                {
                    let tostring_key = v8::String::new(&mut ctx_scope, "toString").unwrap();
                    proto_obj.set(&mut ctx_scope, tostring_key.into(), tostring_fn.into());
                }
                // Also add href getter property if not already set
                if let Some(href_fn) = v8::Function::new(&mut ctx_scope, url_href_callback) {
                    let href_key = v8::String::new(&mut ctx_scope, "href").unwrap();
                    proto_obj.set(&mut ctx_scope, href_key.into(), href_fn.into());
                }
            }
        }
    }

    // Attach to global
    let key = v8::String::new(&mut ctx_scope, "URL").unwrap();
    global.set(&mut ctx_scope, key.into(), ctor.into());
}

/// Bind Headers constructor for WinterTC compatibility
pub(crate) fn bind_headers(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    // Create Headers constructor
    let template = v8::FunctionTemplate::new(&mut ctx_scope, headers_constructor);
    let ctor = template.get_function(&mut ctx_scope).unwrap();

    // Attach to global
    let key = v8::String::new(&mut ctx_scope, "Headers").unwrap();
    global.set(&mut ctx_scope, key.into(), ctor.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::{initialize_platform, NanoIsolate};

    #[test]
    fn form_urlencode_encodes_reserved_chars() {
        assert_eq!(form_urlencode("a b"), "a+b");
        assert_eq!(form_urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(form_urlencode("plain-Text_1.*"), "plain-Text_1.*");
    }

    #[test]
    fn url_search_params_tostring_round_trips() {
        initialize_platform().expect("platform init");
        let mut isolate = NanoIsolate::new().expect("isolate");
        v8::scope!(handle_scope, isolate.isolate());
        let context = v8::Context::new(handle_scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
        bind_url(ctx_scope, context);

        // Regression: toString() used to always return "" regardless of params.
        let code = r#"new URLSearchParams('a=1&b=2').toString()"#;
        let code_str = v8::String::new(ctx_scope, code).unwrap();
        let script = v8::Script::compile(ctx_scope, code_str, None).expect("compile");
        let result = script.run(ctx_scope).expect("run");
        let out = result
            .to_string(ctx_scope)
            .unwrap()
            .to_rust_string_lossy(ctx_scope);

        assert_eq!(
            out, "a=1&b=2",
            "toString must serialize the params, not return empty"
        );
    }
}
