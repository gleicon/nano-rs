//! Request and Response types for WinterTC compatibility
//!
//! These types bridge Rust HTTP handling with JavaScript execution,
//! providing WinterTC-compliant Request and Response objects.
//!
//! # Decisions
//!
//! - **D-05:** Hybrid body handling — buffer small bodies (<1MB) in memory, stream large bodies
//! - **D-06:** Response objects via JSON serialization → V8 parse (not direct V8 API creation)
//!
//! # Content-Type Mapping
//!
//! Static file serving uses extension-based content-type detection.
//! See `content_type_from_ext()` for the full mapping table.

use crate::http::{NanoHeaders, NanoUrl};
use bytes::Bytes;

/// WinterTC-compatible Request object
///
/// Represents an HTTP request that will be passed to the JavaScript handler.
/// Contains all WinterTC Request properties: method, url, headers, body.
#[derive(Debug, Clone)]
pub struct NanoRequest {
    method: String,
    url: NanoUrl,
    headers: NanoHeaders,
    body: Option<Bytes>, // D-05: In-memory body
}

impl NanoRequest {
    /// Create a new Request
    ///
    /// # Arguments
    ///
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `url` - The request URL
    /// * `headers` - HTTP headers
    /// * `body` - Optional request body
    ///
    /// # Returns
    ///
    /// A new `NanoRequest`
    pub fn new(method: String, url: NanoUrl, headers: NanoHeaders, body: Option<Bytes>) -> Self {
        Self {
            method,
            url,
            headers,
            body,
        }
    }

    /// HTTP method
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Request/method
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Request URL
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Request/url
    pub fn url(&self) -> &NanoUrl {
        &self.url
    }

    /// Request headers
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Request/headers
    pub fn headers(&self) -> &NanoHeaders {
        &self.headers
    }

    /// Request body
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Request/body
    pub fn body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    /// Convert URL to full string representation
    pub fn url_string(&self) -> String {
        self.url.href()
    }

    /// Build a NanoRequest from axum HTTP parts.
    ///
    /// Constructs the full URL from `host` + `uri` (preserving any existing scheme),
    /// converts headers, and wraps the pre-read body. The caller is responsible for
    /// reading the body (async) before calling this method.
    pub fn from_axum_parts(
        method: &axum::http::Method,
        uri: &axum::http::Uri,
        host: &str,
        headers: &axum::http::HeaderMap,
        body: Option<Bytes>,
    ) -> Result<Self, String> {
        let full_url = if uri.scheme().is_some() {
            uri.to_string()
        } else {
            format!(
                "http://{}{}",
                host,
                uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/")
            )
        };
        let url = NanoUrl::parse(&full_url).map_err(|e| e.to_string())?;
        let nano_headers = NanoHeaders::from_axum_headers(headers);
        let nano_body = body.filter(|b| !b.is_empty());
        Ok(NanoRequest::new(
            method.to_string(),
            url,
            nano_headers,
            nano_body,
        ))
    }
}

/// WinterTC-compatible Response object
///
/// Represents an HTTP response returned from the JavaScript handler.
/// Contains all WinterTC Response properties: status, statusText, headers, body.
#[derive(Debug, Clone)]
pub struct NanoResponse {
    status: u16,
    status_text: String,
    headers: NanoHeaders,
    body: Option<Bytes>,
    /// Worker ID that processed this request (for logging/debugging)
    worker_id: Option<u32>,
    /// Isolate ID that processed this request (for logging/debugging)
    isolate_id: Option<String>,
}

impl NanoResponse {
    /// Create a new Response
    ///
    /// # Arguments
    ///
    /// * `status` - HTTP status code
    /// * `headers` - HTTP headers
    /// * `body` - Optional response body
    ///
    /// # Returns
    ///
    /// A new `NanoResponse` with auto-generated status text
    pub fn new(status: u16, headers: NanoHeaders, body: Option<Bytes>) -> Self {
        let status_text = Self::default_status_text(status);
        Self {
            status,
            status_text,
            headers,
            body,
            worker_id: None,
            isolate_id: None,
        }
    }

    /// Get the worker ID that processed this request
    pub fn worker_id(&self) -> Option<u32> {
        self.worker_id
    }

    /// Set the worker ID that processed this request
    pub fn set_worker_id(&mut self, worker_id: u32) {
        self.worker_id = Some(worker_id);
    }

    /// Get the isolate ID that processed this request
    pub fn isolate_id(&self) -> Option<&str> {
        self.isolate_id.as_deref()
    }

    /// Set the isolate ID that processed this request
    pub fn set_isolate_id(&mut self, isolate_id: impl Into<String>) {
        self.isolate_id = Some(isolate_id.into());
    }

    /// HTTP status code
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Response/status
    pub fn status(&self) -> u16 {
        self.status
    }

    /// HTTP status text
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Response/statusText
    pub fn status_text(&self) -> &str {
        &self.status_text
    }

    /// Response headers
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Response/headers
    pub fn headers(&self) -> &NanoHeaders {
        &self.headers
    }

    /// Mutable access to response headers
    pub fn headers_mut(&mut self) -> &mut NanoHeaders {
        &mut self.headers
    }

    /// Response body
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/Response/body
    pub fn body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    /// Set the response body
    pub fn set_body(&mut self, body: Bytes) {
        self.body = Some(body);
    }

    /// Convert to axum response (used in router handler)
    ///
    /// Converts this NanoResponse into an axum response that can be
    /// returned from the HTTP handler.
    ///
    /// # Returns
    ///
    /// An axum `Response` with all headers and body
    pub fn to_axum_response(&self) -> axum::response::Response<axum::body::Body> {
        let mut builder = axum::response::Response::builder().status(self.status);

        // Add headers
        for (name, values) in self.headers.entries() {
            for value in values {
                if let Ok(header_name) = axum::http::HeaderName::from_bytes(name.as_bytes()) {
                    if let Ok(header_value) = axum::http::HeaderValue::from_str(value) {
                        builder = builder.header(header_name, header_value);
                    }
                }
            }
        }

        // Build response with body
        let body = self
            .body
            .clone()
            .map(|b| axum::body::Body::from(b))
            .unwrap_or_else(axum::body::Body::empty);

        builder.body(body).unwrap_or_default()
    }

    fn default_status_text(status: u16) -> String {
        match status {
            200 => "OK".to_string(),
            201 => "Created".to_string(),
            204 => "No Content".to_string(),
            301 => "Moved Permanently".to_string(),
            302 => "Found".to_string(),
            304 => "Not Modified".to_string(),
            400 => "Bad Request".to_string(),
            401 => "Unauthorized".to_string(),
            403 => "Forbidden".to_string(),
            404 => "Not Found".to_string(),
            405 => "Method Not Allowed".to_string(),
            500 => "Internal Server Error".to_string(),
            502 => "Bad Gateway".to_string(),
            503 => "Service Unavailable".to_string(),
            _ => "Unknown".to_string(),
        }
    }
}

/// Builder pattern for NanoResponse
impl NanoResponse {
    /// Create a 200 OK response
    ///
    /// # Returns
    ///
    /// A new `NanoResponse` with status 200
    pub fn ok() -> Self {
        Self::new(200, NanoHeaders::default(), None)
    }

    /// Create a response with a specific status code
    ///
    /// # Arguments
    ///
    /// * `status` - HTTP status code
    ///
    /// # Returns
    ///
    /// A new `NanoResponse` with the given status
    pub fn with_status(status: u16) -> Self {
        Self::new(status, NanoHeaders::default(), None)
    }

    /// Add a body to the response
    ///
    /// # Arguments
    ///
    /// * `body` - The body content (any type that converts to String)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(Bytes::from(body.into()));
        self
    }

    /// Add a header to the response
    ///
    /// # Arguments
    ///
    /// * `name` - Header name
    /// * `value` - Header value
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Add a binary body to the response
    ///
    /// # Arguments
    ///
    /// * `body` - The body content as bytes
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn with_body_bytes(mut self, body: Vec<u8>) -> Self {
        self.body = Some(Bytes::from(body));
        self
    }

    /// Create a 404 Not Found response
    ///
    /// # Returns
    ///
    /// A new `NanoResponse` with status 404
    pub fn not_found() -> Self {
        Self::new(404, NanoHeaders::default(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoUrl};

    #[test]
    fn test_request_creation() {
        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let headers = NanoHeaders::new();
        let request = NanoRequest::new("GET".to_string(), url, headers, None);

        assert_eq!(request.method(), "GET");
        assert_eq!(request.url_string(), "https://example.com/api");
    }

    #[test]
    fn test_response_creation() {
        let response = NanoResponse::ok()
            .with_header("Content-Type", "text/plain")
            .with_body("Hello, World!");

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("Content-Type"),
            Some("text/plain".to_string())
        );
        assert_eq!(
            response
                .body()
                .map(|b| String::from_utf8_lossy(b).to_string()),
            Some("Hello, World!".to_string())
        );
    }

    #[test]
    fn test_response_status_text() {
        let response = NanoResponse::new(404, NanoHeaders::new(), None);
        assert_eq!(response.status_text(), "Not Found");

        let response200 = NanoResponse::new(200, NanoHeaders::new(), None);
        assert_eq!(response200.status_text(), "OK");

        let response500 = NanoResponse::new(500, NanoHeaders::new(), None);
        assert_eq!(response500.status_text(), "Internal Server Error");
    }

    #[test]
    fn test_response_builder_chaining() {
        let response = NanoResponse::ok()
            .with_header("Content-Type", "application/json")
            .with_header("X-Custom", "value")
            .with_body(r#"{"success":true}"#);

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("Content-Type"),
            Some("application/json".to_string())
        );
        assert_eq!(
            response.headers().get("X-Custom"),
            Some("value".to_string())
        );
    }

    #[test]
    fn test_response_with_status() {
        let response = NanoResponse::with_status(201)
            .with_header("Location", "/new-resource")
            .with_body("Created");

        assert_eq!(response.status(), 201);
        assert_eq!(response.status_text(), "Created");
    }

    #[test]
    fn test_to_axum_response() {
        let response = NanoResponse::ok()
            .with_header("Content-Type", "text/plain")
            .with_body("Test body");

        let axum_resp = response.to_axum_response();

        assert_eq!(axum_resp.status(), 200);
        assert_eq!(
            axum_resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain")
        );
    }
}

/// Get MIME content type from file extension
///
/// Maps file extensions to their corresponding MIME types for proper
/// Content-Type headers when serving static files.
///
/// # Arguments
///
/// * `ext` - File extension (without the dot, e.g., "html", "css")
///
/// # Returns
///
/// The MIME type string for the extension, or "application/octet-stream"
/// for unknown extensions.
///
/// # Examples
///
/// ```rust
/// use nano::http::types::content_type_from_ext;
///
/// assert_eq!(content_type_from_ext("html"), "text/html; charset=utf-8");
/// assert_eq!(content_type_from_ext("css"), "text/css; charset=utf-8");
/// assert_eq!(content_type_from_ext("png"), "image/png");
/// ```
pub fn content_type_from_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        // Text formats with UTF-8 charset
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "svg" => "image/svg+xml; charset=utf-8",

        // Image formats (no charset needed for binary)
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "avif" => "image/avif",

        // Font formats
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",

        // Archive and binary formats
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "bz2" => "application/x-bzip2",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/x-rar-compressed",

        // WebAssembly
        "wasm" => "application/wasm",

        // Media formats
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "oga" => "audio/ogg",
        "ogv" => "video/ogg",
        "weba" => "audio/webm",
        "opus" => "audio/opus",

        // Default for unknown extensions
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod content_type_tests {
    use super::content_type_from_ext;

    #[test]
    fn test_html_content_type() {
        assert_eq!(content_type_from_ext("html"), "text/html; charset=utf-8");
        assert_eq!(content_type_from_ext("htm"), "text/html; charset=utf-8");
        assert_eq!(content_type_from_ext("HTML"), "text/html; charset=utf-8");
    }

    #[test]
    fn test_css_content_type() {
        assert_eq!(content_type_from_ext("css"), "text/css; charset=utf-8");
        assert_eq!(content_type_from_ext("CSS"), "text/css; charset=utf-8");
    }

    #[test]
    fn test_js_content_type() {
        assert_eq!(
            content_type_from_ext("js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type_from_ext("mjs"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type_from_ext("JS"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_image_content_types() {
        assert_eq!(content_type_from_ext("png"), "image/png");
        assert_eq!(content_type_from_ext("jpg"), "image/jpeg");
        assert_eq!(content_type_from_ext("jpeg"), "image/jpeg");
        assert_eq!(content_type_from_ext("gif"), "image/gif");
        assert_eq!(content_type_from_ext("svg"), "image/svg+xml; charset=utf-8");
        assert_eq!(content_type_from_ext("ico"), "image/x-icon");
    }

    #[test]
    fn test_font_content_types() {
        assert_eq!(content_type_from_ext("woff2"), "font/woff2");
        assert_eq!(content_type_from_ext("woff"), "font/woff");
        assert_eq!(content_type_from_ext("ttf"), "font/ttf");
        assert_eq!(content_type_from_ext("otf"), "font/otf");
    }

    #[test]
    fn test_json_and_xml_content_types() {
        assert_eq!(
            content_type_from_ext("json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(
            content_type_from_ext("xml"),
            "application/xml; charset=utf-8"
        );
    }

    #[test]
    fn test_unknown_extension() {
        assert_eq!(content_type_from_ext("xyz"), "application/octet-stream");
        assert_eq!(content_type_from_ext(""), "application/octet-stream");
    }
}
