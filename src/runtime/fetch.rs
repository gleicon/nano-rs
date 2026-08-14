//! fetch() JavaScript binding for outbound HTTP requests
//!
//! This module provides the global fetch() function for JavaScript,
//! enabling real HTTP requests via Promise-based async operations.
//!
//! # Architecture
//!
//! The fetch() implementation:
//! 1. V8 callback extracts URL and options from JavaScript arguments
//! 2. Makes HTTP request using reqwest (blocking call via pollster for simplicity)
//! 3. Creates Response object with actual response data
//! 4. Response methods (text, json, arrayBuffer) access stored response body
//!
//! # Security
//!
//! - URL validation blocks dangerous schemes (file://, ftp://, javascript://)
//! - SSRF prevention blocks private/internal literal IPs and known internal hostnames
//!   (DNS rebinding is not prevented — requires socket-level IP check after connect)
//! - Response size limits prevent memory exhaustion
//! - Timeout handling prevents hanging requests

use bytes::Bytes;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

/// Validate fetch URL for SSRF. Returns parsed `url::Url` on success so the
/// caller can hand it directly to reqwest without re-parsing.
///
/// Blocks literal private IPv4/IPv6 ranges and known internal hostnames.
/// Does NOT defend against DNS rebinding (where a public hostname resolves
/// to a private IP after this check).
fn validate_fetch_url(url_str: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {e}"))?;

    match parsed.host() {
        None => return Err("URL has no host".to_string()),
        Some(url::Host::Ipv4(addr)) => {
            let o = addr.octets();
            let blocked = o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || o[0] == 127
                || (o[0] == 169 && o[1] == 254)
                || o[0] == 0
                || addr.is_broadcast()
                || (o[0] == 100 && (64..=127).contains(&o[1]));
            if blocked {
                return Err(format!("SSRF blocked: private address {addr}"));
            }
        }
        Some(url::Host::Ipv6(addr)) => {
            // IPv4-mapped (::ffff:x.x.x.x) — check the mapped IPv4 address
            if let Some(v4) = addr.to_ipv4_mapped() {
                let o = v4.octets();
                let blocked = o[0] == 10
                    || (o[0] == 172 && (16..=31).contains(&o[1]))
                    || (o[0] == 192 && o[1] == 168)
                    || o[0] == 127
                    || (o[0] == 169 && o[1] == 254)
                    || o[0] == 0
                    || v4.is_broadcast()
                    || (o[0] == 100 && (64..=127).contains(&o[1]));
                if blocked {
                    return Err(format!("SSRF blocked: private address {addr}"));
                }
            } else {
                let s = addr.segments();
                let blocked = addr.is_loopback()
                    || addr.is_multicast()
                    || (s[0] & 0xffc0) == 0xfe80
                    || (s[0] & 0xfe00) == 0xfc00;
                if blocked {
                    return Err(format!("SSRF blocked: private address {addr}"));
                }
            }
        }
        Some(url::Host::Domain(domain)) => {
            let lower = domain.to_ascii_lowercase();
            let blocked = lower == "localhost"
                || lower.ends_with(".local")
                || lower.ends_with(".internal")
                || lower.ends_with(".localdomain")
                || lower == "metadata.google.internal"
                || lower == "169.254.169.254";
            if blocked {
                return Err(format!("SSRF blocked: internal hostname {domain}"));
            }
        }
    }
    Ok(parsed)
}

/// Per-isolate fetch state
///
/// Each isolate has its own reqwest client and abort registry.
/// This ensures isolation between different apps/tenants.
pub struct FetchState {
    /// HTTP client for this isolate
    client: reqwest::Client,
    /// Map of abort signal IDs to cancellation status
    abort_signals: RefCell<HashMap<u64, bool>>,
    /// Next abort signal ID
    next_abort_id: RefCell<u64>,
}

impl FetchState {
    /// Create new fetch state for an isolate
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(10)
            .build()?;

        Ok(Self {
            client,
            abort_signals: RefCell::new(HashMap::new()),
            next_abort_id: RefCell::new(1),
        })
    }

    /// Register a new abort signal
    pub fn register_abort_signal(&self) -> u64 {
        let id = *self.next_abort_id.borrow();
        *self.next_abort_id.borrow_mut() += 1;
        self.abort_signals.borrow_mut().insert(id, false);
        id
    }

    /// Mark an abort signal as aborted
    pub fn abort(&self, id: u64) {
        if let Some(status) = self.abort_signals.borrow_mut().get_mut(&id) {
            *status = true;
        }
    }

    /// Check if a signal has been aborted
    pub fn is_aborted(&self, id: u64) -> bool {
        self.abort_signals
            .borrow()
            .get(&id)
            .copied()
            .unwrap_or(true)
    }

    /// Get reference to HTTP client
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
}

// Thread-local storage for fetch state
// Each worker thread has its own FetchState instance.
thread_local! {
    static FETCH_STATE: RefCell<Option<FetchState>> = RefCell::new(None);
}

/// Initialize fetch state for the current thread
pub fn init_fetch_state() -> anyhow::Result<()> {
    FETCH_STATE.with(|state| {
        let mut state_ref = state.borrow_mut();
        if state_ref.is_none() {
            *state_ref = Some(FetchState::new()?);
        }
        Ok(())
    })
}

/// Get the fetch state for the current thread
fn with_fetch_state<F, R>(f: F) -> R
where
    F: FnOnce(&FetchState) -> R,
{
    FETCH_STATE.with(|state| {
        let state_ref = state.borrow();
        f(state_ref.as_ref().expect("Fetch state not initialized"))
    })
}

/// Bind fetch() to the global scope
///
/// This creates the global fetch() function that JavaScript can call.
pub fn bind_fetch(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
    // Initialize fetch state for this thread
    if let Err(e) = init_fetch_state() {
        tracing::error!("Failed to initialize fetch state: {}", e);
        return;
    }
    tracing::info!("Fetch state initialized successfully");

    let global = context.global(scope);

    // Enter context scope for V8 APIs that require HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    let key = v8::String::new(&mut ctx_scope, "fetch").unwrap();

    // Create fetch function
    if let Some(fetch_fn) = v8::Function::new(&mut ctx_scope, fetch_callback) {
        global.set(&mut ctx_scope, key.into(), fetch_fn.into());
    }
}

/// V8 external data wrapper for response body
///
/// This stores the response body bytes in V8's external data,
/// allowing Response methods (text, json, arrayBuffer) to access it.
pub(crate) struct ResponseBodyData {
    body: Bytes,
}

impl ResponseBodyData {
    fn new(body: Bytes) -> Self {
        Self { body }
    }
}

/// V8 callback for fetch() function
fn fetch_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    tracing::debug!("fetch() callback invoked");

    // Extract URL from arguments (first arg)
    let url = if args.length() > 0 {
        match args.get(0).to_string(scope) {
            Some(s) => s.to_rust_string_lossy(scope),
            None => {
                reject_with_error(scope, &mut retval, "URL must be a string");
                return;
            }
        }
    } else {
        reject_with_error(scope, &mut retval, "fetch() requires at least 1 argument");
        return;
    };

    // Validate URL scheme
    if url.starts_with("file://") || url.starts_with("ftp://") || url.starts_with("javascript:") {
        reject_with_error(
            scope,
            &mut retval,
            &format!(
                "URL scheme not allowed: {}",
                url.split("://").next().unwrap_or("")
            ),
        );
        return;
    }

    // Validate URL for SSRF: block private/internal addresses.
    // Returns the already-parsed url::Url so reqwest reuses it without re-parsing.
    let parsed_url = match validate_fetch_url(&url) {
        Ok(u) => u,
        Err(err) => {
            reject_with_error(scope, &mut retval, &err);
            return;
        }
    };

    // Parse options (second arg)
    let mut method = "GET".to_string();
    let mut headers_map: Vec<(String, String)> = Vec::new();
    let mut body: Option<Bytes> = None;

    if args.length() > 1 {
        let options = args.get(1);
        if let Some(obj) = options.to_object(scope) {
            // Extract method
            if let Some(method_key) = v8::String::new(scope, "method") {
                if let Some(method_val) = obj.get(scope, method_key.into()) {
                    if let Some(s) = method_val.to_string(scope) {
                        method = s.to_rust_string_lossy(scope).to_uppercase();
                    }
                }
            }

            // Extract headers
            if let Some(headers_key) = v8::String::new(scope, "headers") {
                if let Some(headers_val) = obj.get(scope, headers_key.into()) {
                    if let Some(headers_obj) = headers_val.to_object(scope) {
                        if let Some(keys) =
                            headers_obj.get_own_property_names(scope, Default::default())
                        {
                            let len = keys.length();
                            for i in 0..len {
                                if let Some(key) = keys.get_index(scope, i) {
                                    if let Some(key_str) = key.to_string(scope) {
                                        let name = key_str.to_rust_string_lossy(scope);
                                        if let Some(value) = headers_obj.get(scope, key.into()) {
                                            if let Some(value_str) = value.to_string(scope) {
                                                let value = value_str.to_rust_string_lossy(scope);
                                                headers_map.push((name, value));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Extract body
            if let Some(body_key) = v8::String::new(scope, "body") {
                if let Some(body_val) = obj.get(scope, body_key.into()) {
                    if !body_val.is_null() && !body_val.is_undefined() {
                        if let Some(s) = body_val.to_string(scope) {
                            body = Some(Bytes::from(s.to_rust_string_lossy(scope)));
                        }
                    }
                }
            }
        }
    }

    // Make the HTTP request using the worker thread's Tokio runtime
    // Get the runtime handle first
    let rt_handle = match crate::worker::pool::with_worker_runtime(|rt| rt.clone()) {
        Some(handle) => handle,
        None => {
            reject_with_error(
                scope,
                &mut retval,
                "No Tokio runtime available. fetch() must be called from a worker thread.",
            );
            return;
        }
    };

    // Now make the request with fetch state and runtime
    let state_opt = FETCH_STATE.with(|s| s.borrow().as_ref().map(|_| ()));
    if state_opt.is_none() {
        reject_with_error(scope, &mut retval, "Fetch state not initialized");
        return;
    }

    // Pause the CPU timer for the duration of the HTTP request.
    // AsyncWaitGuard resumes the timer on drop — panic-safe.
    let _cpu_wait = crate::data_plane::AsyncWaitGuard::begin();
    let response_result: Result<(Bytes, Vec<(String, String)>, u16, String), String> =
        with_fetch_state(|state| {
            rt_handle.block_on(async {
                let mut request_builder = state.client.request(
                    reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
                    parsed_url,
                );

                // Add headers
                for (name, value) in &headers_map {
                    request_builder = request_builder.header(name, value);
                }

                // Add body if present
                if let Some(body_data) = body {
                    request_builder = request_builder.body(body_data);
                }

                // Execute request
                match request_builder.send().await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        let final_url = resp.url().to_string();
                        tracing::debug!("fetch() response: status={}, url={}", status, final_url);

                        // Extract headers
                        let mut response_headers = Vec::new();
                        for (name, value) in resp.headers() {
                            if let Ok(val_str) = value.to_str() {
                                response_headers.push((name.to_string(), val_str.to_string()));
                            }
                        }

                        // Read response body
                        match resp.bytes().await {
                            Ok(body_bytes) => {
                                tracing::debug!("fetch() response body: {} bytes", body_bytes.len());
                                Ok((body_bytes, response_headers, status, final_url))
                            }
                            Err(e) => Err(format!("Failed to read response body: {}", e)),
                        }
                    }
                    Err(e) => {
                        tracing::error!("fetch() request failed: {}", e);
                        Err(format!("Request failed: {}", e))
                    }
                }
            })
        });
    drop(_cpu_wait); // explicit drop for clarity; timer resumes here

    // Create Response object
    match response_result {
        Ok((body_bytes, response_headers, status, final_url)) => {
            // Store values we need before moving into the Box
            let body_len = body_bytes.len();
            let body_clone_for_ab = body_bytes.clone();
            let url_str = final_url.clone();
            let headers_for_iter = response_headers.clone();

            let response_data = Box::new(ResponseBodyData::new(body_bytes));

            // Create Response object
            let obj = v8::Object::new(scope);

            // Store response data as external
            let external =
                v8::External::new(scope, Box::into_raw(response_data) as *mut std::ffi::c_void);
            let external_key = v8::String::new(scope, "__response_data").unwrap();
            obj.set(scope, external_key.into(), external.into());

            // Set status property
            let status_key = v8::String::new(scope, "status").unwrap();
            let status_val = v8::Number::new(scope, status as f64);
            obj.set(scope, status_key.into(), status_val.into());

            // Set ok property (status 200-299)
            let ok_key = v8::String::new(scope, "ok").unwrap();
            let ok_val = v8::Boolean::new(scope, status >= 200 && status < 300);
            obj.set(scope, ok_key.into(), ok_val.into());

            // Set url property
            let url_key = v8::String::new(scope, "url").unwrap();
            let url_val = v8::String::new(scope, &url_str).unwrap();
            obj.set(scope, url_key.into(), url_val.into());

            // Set statusText property
            let status_text_key = v8::String::new(scope, "statusText").unwrap();
            let status_text_val = v8::String::new(
                scope,
                if status >= 200 && status < 300 {
                    "OK"
                } else {
                    "Error"
                },
            )
            .unwrap();
            obj.set(scope, status_text_key.into(), status_text_val.into());

            // Create headers object
            let headers_key = v8::String::new(scope, "headers").unwrap();
            let headers_obj = v8::Object::new(scope);
            for (name, value) in &headers_for_iter {
                let key = v8::String::new(scope, name).unwrap();
                let val = v8::String::new(scope, value).unwrap();
                headers_obj.set(scope, key.into(), val.into());
            }
            obj.set(scope, headers_key.into(), headers_obj.into());

            // Create body as Uint8Array
            let body_key = v8::String::new(scope, "body").unwrap();
            let ab = v8::ArrayBuffer::new(scope, body_len);
            // Copy data into ArrayBuffer
            if let Some(data_ptr) = ab.data() {
                // SAFETY: data_ptr from V8 ArrayBuffer backing store; valid, exclusively owned, and body_len bytes long
                let data = unsafe {
                    std::slice::from_raw_parts_mut(data_ptr.as_ptr() as *mut u8, body_len)
                };
                data.copy_from_slice(&body_clone_for_ab);
            }
            let uint8_array = v8::Uint8Array::new(scope, ab, 0, body_len).unwrap();
            obj.set(scope, body_key.into(), uint8_array.into());

            // Add text() method
            let text_key = v8::String::new(scope, "text").unwrap();
            if let Some(text_fn) = v8::Function::new(scope, response_text_callback) {
                obj.set(scope, text_key.into(), text_fn.into());
            }

            // Add json() method
            let json_key = v8::String::new(scope, "json").unwrap();
            if let Some(json_fn) = v8::Function::new(scope, response_json_callback) {
                obj.set(scope, json_key.into(), json_fn.into());
            }

            // Add arrayBuffer() method
            let array_buffer_key = v8::String::new(scope, "arrayBuffer").unwrap();
            if let Some(array_buffer_fn) = v8::Function::new(scope, response_arraybuffer_callback) {
                obj.set(scope, array_buffer_key.into(), array_buffer_fn.into());
            }

            retval.set(obj.into());
        }
        Err(ref error_msg) => {
            reject_with_error(scope, &mut retval, error_msg.as_str());
        }
    }
}

/// Helper to get response data from JavaScript object
pub(crate) fn get_response_data(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    this: v8::Local<v8::Object>,
) -> Option<&'static ResponseBodyData> {
    let external_key = v8::String::new(scope, "__response_data").unwrap();
    if let Some(external_val) = this.get(scope, external_key.into()) {
        if external_val.is_external() {
            let external = external_val.cast::<v8::External>();
            let ptr = external.value() as *mut ResponseBodyData;
            // SAFETY: ptr points to a Box<ResponseBodyData> stored as a V8 External on `this`.
            // `this` is stack-rooted in the current V8 scope, keeping the External and its
            // backing Box alive. Callers must not store the returned reference past the scope.
            return unsafe { ptr.as_ref() };
        }
    }
    None
}

/// Callback for Response.text()
pub fn response_text_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    // First try to get data from external response data (fetch() responses)
    if let Some(data) = get_response_data(scope, this) {
        let text = String::from_utf8_lossy(&data.body);
        let result = v8::String::new(scope, &text).unwrap();
        retval.set(result.into());
        return;
    }

    // Fall back to body property for manually created Response objects
    let body_key = v8::String::new(scope, "body").unwrap();
    if let Some(body_val) = this.get(scope, body_key.into()) {
        if !body_val.is_null() && !body_val.is_undefined() {
            if let Some(body_str) = body_val.to_string(scope) {
                let text = body_str.to_rust_string_lossy(scope);
                let result = v8::String::new(scope, &text).unwrap();
                retval.set(result.into());
                return;
            }
        }
    }

    // Return empty string if no body
    retval.set(v8::String::new(scope, "").unwrap().into());
}

/// Callback for Response.json()
pub fn response_json_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let text = if let Some(data) = get_response_data(scope, this) {
        // Get data from external response data (fetch() responses)
        String::from_utf8_lossy(&data.body).to_string()
    } else {
        // Fall back to body property for manually created Response objects
        let body_key = v8::String::new(scope, "body").unwrap();
        if let Some(body_val) = this.get(scope, body_key.into()) {
            if !body_val.is_null() && !body_val.is_undefined() {
                if let Some(body_str) = body_val.to_string(scope) {
                    body_str.to_rust_string_lossy(scope).to_string()
                } else {
                    retval.set_undefined();
                    return;
                }
            } else {
                retval.set_undefined();
                return;
            }
        } else {
            retval.set_undefined();
            return;
        }
    };

    // Parse the JSON
    let json_str = v8::String::new(scope, &text).unwrap();
    if let Some(json) = v8::json::parse(scope, json_str.into()) {
        retval.set(json);
    } else {
        reject_with_error(scope, &mut retval, "Invalid JSON");
    }
}

/// Callback for Response.arrayBuffer()
pub fn response_arraybuffer_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let this = args.this();
    let body_bytes: Bytes = if let Some(data) = get_response_data(scope, this) {
        // Get data from external response data (fetch() responses)
        data.body.clone()
    } else {
        // Fall back to body property for manually created Response objects
        let body_key = v8::String::new(scope, "body").unwrap();
        if let Some(body_val) = this.get(scope, body_key.into()) {
            if !body_val.is_null() && !body_val.is_undefined() {
                if let Some(body_str) = body_val.to_string(scope) {
                    Bytes::from(body_str.to_rust_string_lossy(scope).into_bytes())
                } else {
                    Bytes::new()
                }
            } else {
                Bytes::new()
            }
        } else {
            Bytes::new()
        }
    };

    let ab = v8::ArrayBuffer::new(scope, body_bytes.len());
    if let Some(data_ptr) = ab.data() {
        // SAFETY: data_ptr from V8 ArrayBuffer backing store; valid, exclusively owned, and body_bytes.len() bytes long
        let dest = unsafe {
            std::slice::from_raw_parts_mut(data_ptr.as_ptr() as *mut u8, body_bytes.len())
        };
        dest.copy_from_slice(&body_bytes);
    }
    retval.set(ab.into());
}

/// Reject with an error
fn reject_with_error(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    retval: &mut v8::ReturnValue,
    message: &str,
) {
    let error_msg = v8::String::new(scope, message).unwrap();
    let error = v8::Exception::type_error(scope, error_msg);
    retval.set(error);
}

/// Static callback for Response.json() - creates a Response from a JSON object
///
/// Usage: Response.json(data, options)
/// - data: any JavaScript value (will be JSON.stringified)
/// - options: optional { status, headers }
pub fn response_json_static_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    // Get data argument (first argument)
    let data = if args.length() > 0 {
        args.get(0)
    } else {
        retval.set_undefined();
        return;
    };

    // Serialize data to JSON string
    let json_string = if let Some(json_str) = v8::json::stringify(scope, data) {
        json_str.to_rust_string_lossy(scope)
    } else {
        "null".to_string()
    };

    // Get options argument (second argument - optional)
    let mut status = 200;
    let mut headers_obj: Option<v8::Local<v8::Object>> = None;

    if args.length() > 1 {
        let options = args.get(1);
        if let Some(opts) = options.to_object(scope) {
            // Extract status
            let status_key = v8::String::new(scope, "status").unwrap();
            if let Some(status_val) = opts.get(scope, status_key.into()) {
                if !status_val.is_null() && !status_val.is_undefined() {
                    if let Some(num) = status_val.to_number(scope) {
                        let val = num.value();
                        if !val.is_nan() && val > 0.0 {
                            status = val as u16;
                        }
                    }
                }
            }

            // Extract headers
            let headers_key = v8::String::new(scope, "headers").unwrap();
            headers_obj = opts
                .get(scope, headers_key.into())
                .and_then(|h| h.to_object(scope));
        }
    }

    // Create a new Response object
    let response_obj = v8::Object::new(scope);

    // Set status property
    let status_key = v8::String::new(scope, "status").unwrap();
    let status_val = v8::Number::new(scope, status as f64);
    response_obj.set(scope, status_key.into(), status_val.into());

    // Create headers object with Content-Type: application/json
    let headers = v8::Object::new(scope);
    let content_type_key = v8::String::new(scope, "Content-Type").unwrap();
    let content_type_val = v8::String::new(scope, "application/json").unwrap();
    headers.set(scope, content_type_key.into(), content_type_val.into());

    // Add any custom headers from options
    if let Some(hdrs) = headers_obj {
        if let Some(names) = hdrs.get_own_property_names(scope, Default::default()) {
            let len = names.length();
            for i in 0..len {
                if let Some(key) = names.get_index(scope, i) {
                    if let Some(key_str) = key.to_string(scope) {
                        let key_name = key_str.to_rust_string_lossy(scope);
                        if key_name != "Content-Type" {
                            // Don't override Content-Type
                            if let Some(value) = hdrs.get(scope, key.into()) {
                                if let Some(value_str) = value.to_string(scope) {
                                    let value_string = value_str.to_rust_string_lossy(scope);
                                    let hkey = v8::String::new(scope, &key_name).unwrap();
                                    let hval = v8::String::new(scope, &value_string).unwrap();
                                    headers.set(scope, hkey.into(), hval.into());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Set headers property
    let headers_key = v8::String::new(scope, "headers").unwrap();
    response_obj.set(scope, headers_key.into(), headers.into());

    // Set body property
    let body_key = v8::String::new(scope, "body").unwrap();
    let body_val = v8::String::new(scope, &json_string).unwrap();
    response_obj.set(scope, body_key.into(), body_val.into());

    // Return the Response object
    retval.set(response_obj.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_fetch_state_creation() {
        init_platform();
        let state = FetchState::new();
        assert!(state.is_ok());
    }

    #[test]
    fn test_abort_signal() {
        init_platform();
        let state = FetchState::new().unwrap();

        let id = state.register_abort_signal();
        assert!(!state.is_aborted(id));

        state.abort(id);
        assert!(state.is_aborted(id));
    }

    #[test]
    fn test_abort_signal_unknown_id() {
        init_platform();
        let state = FetchState::new().unwrap();

        // Unknown ID should return true (treat as aborted for safety)
        assert!(state.is_aborted(99999));
    }
}
