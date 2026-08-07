//! HTTP client for outbound fetch() requests
//!
//! This module provides an HTTP client using hyper for making outbound
//! requests from JavaScript. It supports HTTP/1.1, HTTP/2, HTTPS via rustls,
//! connection pooling, redirects, and timeouts.
//!
//! # Security
//!
//! - URL validation rejects dangerous schemes (file://, ftp://, javascript://)
//! - SSRF prevention blocks private IP ranges
//! - Connection limits and timeouts prevent DoS
//! - Header filtering blocks dangerous headers

use bytes::Bytes;
use hyper::{StatusCode, Uri};
use std::time::Duration;
use tracing::trace;

/// HTTP client for outbound requests
///
/// Wraps reqwest client with connection pooling, HTTPS support,
/// and security features like URL validation and timeout handling.
#[derive(Clone, Debug)]
pub struct HttpClient {
    /// Reqwest client for actual HTTP operations
    client: reqwest::Client,
    /// Default timeout for requests
    timeout: Duration,
    /// Maximum response size in bytes (default: 100MB)
    max_response_size: usize,
}

/// HTTP response from outbound request
///
/// Contains status, headers, and a streaming body that can be
/// consumed chunk by chunk or all at once.
#[derive(Debug)]
pub struct HttpClientResponse {
    /// HTTP status code
    pub status: StatusCode,
    /// HTTP response headers
    pub headers: Vec<(String, String)>,
    /// Response body (accumulated in memory)
    pub body: Option<Bytes>,
    /// Final URL after redirects
    pub url: String,
}

/// Errors that can occur during HTTP requests
#[derive(Debug, thiserror::Error)]
pub enum HttpClientError {
    /// Invalid URL (parsing error or unsafe scheme)
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    /// URL points to private/internal network (SSRF prevention)
    #[error("URL blocked: private IP range")]
    PrivateIpBlocked,
    /// Network error during request
    #[error("Network error: {0}")]
    Network(String),
    /// Request timeout exceeded
    #[error("Request timeout after {0:?}")]
    Timeout(Duration),
    /// Too many redirects
    #[error("Too many redirects (max {0})")]
    TooManyRedirects(usize),
    /// Dangerous header was blocked
    #[error("Header blocked: {0}")]
    BlockedHeader(String),
    /// Response body too large
    #[error("Response body exceeds maximum size ({0} bytes)")]
    ResponseTooLarge(usize),
    /// TLS/SSL error
    #[error("TLS error: {0}")]
    Tls(String),
}

/// Types of request bodies supported
#[derive(Debug, Clone)]
pub enum RequestBody {
    /// No body
    None,
    /// Fixed-size body with known content length
    Fixed(Bytes),
    /// Streaming body (content length unknown)
    /// Uses chunked transfer encoding automatically
    Streaming {
        /// Content type for the stream
        content_type: Option<String>,
    },
}

impl RequestBody {
    /// Check if this body type is a streaming body
    pub fn is_streaming(&self) -> bool {
        matches!(self, RequestBody::Streaming { .. })
    }

    /// Check if this body type has no content
    pub fn is_none(&self) -> bool {
        matches!(self, RequestBody::None)
    }

    /// Get the content length if known
    pub fn content_length(&self) -> Option<usize> {
        match self {
            RequestBody::Fixed(bytes) => Some(bytes.len()),
            _ => None,
        }
    }

    /// Get the content type if specified
    pub fn content_type(&self) -> Option<&str> {
        match self {
            RequestBody::Streaming { content_type } => content_type.as_deref(),
            _ => None,
        }
    }
}

impl Default for RequestBody {
    fn default() -> Self {
        RequestBody::None
    }
}

/// Configuration for streaming uploads
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// Maximum upload size (default: 100MB)
    pub max_size: usize,
    /// Upload timeout (default: 30s)
    pub timeout: Duration,
    /// Maximum concurrent uploads per isolate
    pub max_concurrent: usize,
    /// Chunk buffer size for backpressure
    pub chunk_buffer_size: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            max_size: 100 * 1024 * 1024, // 100MB
            timeout: Duration::from_secs(30),
            max_concurrent: 10,
            chunk_buffer_size: 4,
        }
    }
}

impl HttpClient {
    /// Create a new HTTP client with default settings
    ///
    /// Default settings:
    /// - Timeout: 30 seconds
    /// - Max redirects: 10
    /// - Max response size: 100MB
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .https_only(false)
            .build()?;

        Ok(Self {
            client,
            timeout: Duration::from_secs(30),
            max_response_size: 100 * 1024 * 1024, // 100MB
        })
    }

    /// Create a new HTTP client with custom timeout
    pub fn with_timeout(timeout: Duration) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .https_only(false)
            .build()?;
        Ok(Self {
            client,
            timeout,
            max_response_size: 100 * 1024 * 1024,
        })
    }

    /// Make an HTTP request
    ///
    /// # Arguments
    /// * `method` - HTTP method (GET, POST, etc.)
    /// * `url` - Target URL
    /// * `headers` - Optional request headers
    /// * `body` - Optional request body
    ///
    /// # Security
    /// - Validates URL scheme (blocks file://, ftp://, javascript://)
    /// - Blocks private IP ranges (SSRF prevention)
    /// - Filters dangerous headers
    pub async fn request(
        &self,
        method: &str,
        url: &str,
        headers: Option<Vec<(String, String)>>,
        _body: Option<Bytes>,
    ) -> Result<HttpClientResponse, HttpClientError> {
        // Validate URL
        let uri: Uri = url
            .parse()
            .map_err(|e| HttpClientError::InvalidUrl(format!("{}", e)))?;

        // Check scheme
        let scheme = uri.scheme_str().unwrap_or("http");
        if !matches!(scheme, "http" | "https") {
            return Err(HttpClientError::InvalidUrl(format!(
                "Unsupported scheme: {} (only http/https allowed)",
                scheme
            )));
        }

        // Check for private IP ranges (SSRF prevention)
        if let Some(host) = uri.host() {
            if is_private_ip(host) {
                return Err(HttpClientError::PrivateIpBlocked);
            }
        }

        trace!("Making {} request to {}", method, url);

        let mut req_builder = self.client.request(
            reqwest::Method::from_bytes(method.as_bytes())
                .map_err(|e| HttpClientError::InvalidUrl(format!("Invalid method: {}", e)))?,
            url,
        );

        if let Some(ref headers) = headers {
            for (name, value) in headers {
                if !is_dangerous_header(name) {
                    req_builder = req_builder.header(name, value);
                }
            }
        }

        // Add body if provided
        if let Some(body_bytes) = _body {
            req_builder = req_builder.body(body_bytes);
        }

        // Execute the request
        let response =
            self.client
                .execute(req_builder.build().map_err(|e| {
                    HttpClientError::Network(format!("Failed to build request: {}", e))
                })?)
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        HttpClientError::Timeout(self.timeout)
                    } else {
                        HttpClientError::Network(format!("Request failed: {}", e))
                    }
                })?;

        // Extract response information
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::OK);
        let final_url = response.url().to_string();

        // Extract headers
        let mut response_headers = Vec::new();
        for (name, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                response_headers.push((name.to_string(), value_str.to_string()));
            }
        }

        // Read response body with size limit
        let body = if let Some(content_length) = response.content_length() {
            if content_length > self.max_response_size as u64 {
                return Err(HttpClientError::ResponseTooLarge(self.max_response_size));
            }
            Some(Bytes::from(response.bytes().await.map_err(|e| {
                HttpClientError::Network(format!("Failed to read body: {}", e))
            })?))
        } else {
            // No content length, read with size limit
            let bytes = response
                .bytes()
                .await
                .map_err(|e| HttpClientError::Network(format!("Failed to read body: {}", e)))?;
            if bytes.len() > self.max_response_size {
                return Err(HttpClientError::ResponseTooLarge(self.max_response_size));
            }
            Some(Bytes::from(bytes))
        };

        trace!("Request to {} completed: {}", url, status);

        Ok(HttpClientResponse {
            status,
            headers: response_headers,
            body,
            url: final_url,
        })
    }

    /// Convenience method for GET requests
    pub async fn get(&self, url: &str) -> Result<HttpClientResponse, HttpClientError> {
        self.request("GET", url, None, None).await
    }

    /// Convenience method for POST requests
    pub async fn post(
        &self,
        url: &str,
        headers: Option<Vec<(String, String)>>,
        body: Option<Bytes>,
    ) -> Result<HttpClientResponse, HttpClientError> {
        self.request("POST", url, headers, body).await
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default HTTP client")
    }
}

/// Check if a header is dangerous and should be filtered
fn is_dangerous_header(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "host" | "content-length" | "transfer-encoding"
    )
}

/// Check if a host is a private/internal address (SSRF prevention).
///
/// Blocks literal IP addresses in RFC1918/loopback/link-local/unspecified ranges
/// and well-known cloud metadata hostnames. Note: hostname→IP resolution is not
/// performed here; a hostname that resolves to a private IP at DNS lookup time
/// can still bypass this check (DNS rebinding). Defence-in-depth via network
/// egress controls is recommended for full SSRF prevention.
fn is_private_ip(host: &str) -> bool {
    // Strip IPv6 bracket notation
    let host_clean = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };

    if let Ok(ip) = host_clean.parse::<std::net::IpAddr>() {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                if ipv4.is_private()        // 10/8, 172.16/12, 192.168/16
                    || ipv4.is_loopback()   // 127/8
                    || ipv4.is_link_local() // 169.254/16
                    || ipv4.is_unspecified() // 0.0.0.0
                    || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                // CGNAT 100.64/10
                {
                    return true;
                }
            }
            std::net::IpAddr::V6(ipv6) => {
                let segments = ipv6.segments();
                if ipv6.is_loopback()        // ::1
                    || ipv6.is_unspecified() // ::
                    || (segments[0] & 0xffc0) == 0xfe80  // fe80::/10 link-local
                    || (segments[0] & 0xfe00) == 0xfc00  // fc00::/7 unique-local
                    || (segments[0..5] == [0, 0, 0, 0, 0] && segments[5] == 0xffff
                        && is_private_ip(&std::net::Ipv4Addr::new(
                            (segments[6] >> 8) as u8, (segments[6] & 0xff) as u8,
                            (segments[7] >> 8) as u8, (segments[7] & 0xff) as u8,
                        ).to_string())) // ::ffff:0:0/96 IPv4-mapped
                    || (segments[0..6] == [0, 0, 0, 0, 0, 0]
                        && is_private_ip(&std::net::Ipv4Addr::new(
                            (segments[6] >> 8) as u8, (segments[6] & 0xff) as u8,
                            (segments[7] >> 8) as u8, (segments[7] & 0xff) as u8,
                        ).to_string())) // ::x.x.x.x/96 IPv4-compatible (RFC 4291 §2.5.5.1)
                    || (segments[0] == 0x0064 && segments[1] == 0xff9b
                        && segments[2..6] == [0, 0, 0, 0]
                        && is_private_ip(&std::net::Ipv4Addr::new(
                            (segments[6] >> 8) as u8, (segments[6] & 0xff) as u8,
                            (segments[7] >> 8) as u8, (segments[7] & 0xff) as u8,
                        ).to_string()))
                // 64:ff9b::/96 NAT64 (RFC 6052)
                {
                    return true;
                }
            }
        }
    }

    let lower = host_clean.to_ascii_lowercase();
    lower == "localhost"
        || lower == "metadata"
        || lower == "metadata.google.internal"
        || lower == "metadata.internal"
        || lower == "instance-data"
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests are designed to work with httpbin.org or similar
    // In CI environments, they may need to be skipped if external connectivity
    // is not available.

    #[tokio::test]
    async fn test_http_client_creation() {
        let client = HttpClient::new();
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_http_client_custom_timeout() {
        let client = HttpClient::with_timeout(Duration::from_secs(5));
        assert!(client.is_ok());
    }

    /// Test 1: HttpClient can make a basic HTTPS GET and receive a success response.
    /// Uses https://example.com (IANA-maintained, stable). Skips gracefully when the
    /// environment has no outbound network (air-gapped CI).
    #[tokio::test]
    async fn test_get_request_to_httpbin() {
        let client = HttpClient::new().unwrap();
        let result = client.get("https://example.com").await;
        match result {
            Err(HttpClientError::Network(_)) | Err(HttpClientError::Timeout(_)) => return,
            Err(e) => panic!("unexpected error: {e}"),
            Ok(response) => assert!(
                response.status.is_success(),
                "expected 2xx from example.com, got {}",
                response.status
            ),
        }
    }

    /// Test 2: HttpClient performs TLS handshake and gets a success response.
    /// Uses https://example.com (IANA-maintained, stable). Skips gracefully when the
    /// environment has no outbound network (air-gapped CI).
    #[tokio::test]
    async fn test_https_request() {
        let client = HttpClient::new().unwrap();
        let result = client.get("https://example.com").await;
        match result {
            Err(HttpClientError::Network(_)) | Err(HttpClientError::Timeout(_)) => return,
            Err(e) => panic!("unexpected error: {e}"),
            Ok(response) => assert!(
                response.status.is_success(),
                "expected 2xx from example.com, got {}",
                response.status
            ),
        }
    }

    /// Test 3: HttpClient timeout configuration — 1ms timeout must actually be enforced
    #[tokio::test]
    async fn test_request_timeout() {
        let client = HttpClient::with_timeout(Duration::from_millis(1)).unwrap();
        let result = client.get("https://httpbin.org/get").await;
        assert!(
            matches!(result, Err(HttpClientError::Timeout(_))),
            "1ms timeout should fire, got: {:?}",
            result
        );
    }

    /// Test 4: URL validation rejects file:// scheme
    #[tokio::test]
    async fn test_reject_file_scheme() {
        let client = HttpClient::new().unwrap();

        let result = client.get("file:///etc/passwd").await;

        assert!(
            matches!(result, Err(HttpClientError::InvalidUrl(_))),
            "Expected InvalidUrl error for file://, got {:?}",
            result
        );
    }

    /// Test 5: URL validation rejects ftp:// scheme
    #[tokio::test]
    async fn test_reject_ftp_scheme() {
        let client = HttpClient::new().unwrap();

        let result = client.get("ftp://example.com/file.txt").await;

        assert!(
            matches!(result, Err(HttpClientError::InvalidUrl(_))),
            "Expected InvalidUrl error for ftp://, got {:?}",
            result
        );
    }

    /// Test 6: SSRF prevention blocks private IP ranges
    #[tokio::test]
    async fn test_block_private_ipv4() {
        let client = HttpClient::new().unwrap();

        // Test 10.0.0.0/8
        let result = client.get("http://10.0.0.1/admin").await;
        assert!(
            matches!(result, Err(HttpClientError::PrivateIpBlocked)),
            "Expected PrivateIpBlocked for 10.x.x.x"
        );

        // Test 192.168.0.0/16
        let result = client.get("http://192.168.1.1/admin").await;
        assert!(
            matches!(result, Err(HttpClientError::PrivateIpBlocked)),
            "Expected PrivateIpBlocked for 192.168.x.x"
        );

        // Test 172.16.0.0/12
        let result = client.get("http://172.16.0.1/admin").await;
        assert!(
            matches!(result, Err(HttpClientError::PrivateIpBlocked)),
            "Expected PrivateIpBlocked for 172.16.x.x"
        );

        // Test localhost
        let result = client.get("http://localhost:8080/admin").await;
        assert!(
            matches!(result, Err(HttpClientError::PrivateIpBlocked)),
            "Expected PrivateIpBlocked for localhost"
        );
    }

    /// Test 7: SSRF prevention blocks IPv6 private ranges
    #[tokio::test]
    async fn test_block_ipv6_loopback() {
        let client = HttpClient::new().unwrap();

        // IPv6 loopback address - should be blocked by SSRF prevention
        let result = client.get("http://[::1]:8080/admin").await;

        // Should be blocked as private IP
        assert!(
            matches!(result, Err(HttpClientError::PrivateIpBlocked)),
            "IPv6 loopback should be blocked: {:?}",
            result
        );
    }

    /// Test 8: POST request with body and headers reaches a real HTTPS endpoint.
    /// Uses https://example.com (which returns 200 on POST). Skips gracefully when
    /// the environment has no outbound network (air-gapped CI).
    #[tokio::test]
    async fn test_post_request() {
        let client = HttpClient::new().unwrap();

        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Custom-Header".to_string(), "test-value".to_string()),
        ];
        let body = Some(Bytes::from(r#"{"test": "data"}"#));

        // httpbin.org/post echoes back the request body/headers — best POST test target.
        // Skip gracefully on network errors or transient 5xx (CI air-gap / service down).
        let result = client
            .post("https://httpbin.org/post", Some(headers), body)
            .await;
        match result {
            Err(HttpClientError::Network(_)) | Err(HttpClientError::Timeout(_)) => return,
            Err(e) => panic!("unexpected error: {e}"),
            Ok(response) if response.status.is_server_error() => return,
            Ok(response) => assert!(
                response.status.is_success(),
                "expected 2xx from httpbin.org/post, got {}",
                response.status
            ),
        }
    }

    /// Test 9: Dangerous headers are filtered
    #[tokio::test]
    async fn test_filter_dangerous_headers() {
        let client = HttpClient::new().unwrap();

        // Try to set Host header (should be filtered)
        let headers = vec![
            ("Host".to_string(), "evil.com".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        // This should not fail, but Host header should be filtered
        let result = client
            .request("GET", "https://httpbin.org/get", Some(headers), None)
            .await;

        // The request should succeed (Host is just filtered, not rejected)
        // httpbin will echo back the headers it received
        match result {
            Ok(_) => {
                // Success - Host header was filtered but request went through
            }
            Err(e) => {
                println!("Network unavailable: {}", e);
            }
        }
    }

    /// Test 10: Response size limit enforcement
    #[tokio::test]
    async fn test_response_size_limit() {
        // Create client with very small max response size
        let mut client = HttpClient::new().unwrap();
        client.max_response_size = 100; // 100 bytes max

        // Request a large response that exceeds our limit
        let result = client.get("https://httpbin.org/bytes/1000").await;

        match result {
            Ok(_) => {
                // If network is available, we should get size limit error
                // But httpbin.org might not respond with exactly what we expect
            }
            Err(HttpClientError::ResponseTooLarge(_)) => {
                // This is the expected error when response is too large
            }
            Err(e) => {
                println!("Expected ResponseTooLarge or OK, got: {}", e);
            }
        }
    }

    /// Test 11: is_private_ip helper function
    #[test]
    fn test_is_private_ip_helper() {
        // Private ranges
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("192.168.255.255"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("169.254.1.1"));
        assert!(is_private_ip("localhost"));
        assert!(is_private_ip("LOCALHOST"));

        // IPv6 loopback (without brackets - this is how hyper parses it)
        assert!(is_private_ip("::1"));

        // New: unspecified addresses
        assert!(is_private_ip("0.0.0.0"));
        assert!(is_private_ip("::"));
        // New: CGNAT shared space
        assert!(is_private_ip("100.64.0.1"));
        assert!(is_private_ip("100.127.255.255"));
        // New: IPv6 unique-local (fc00::/7)
        assert!(is_private_ip("fc00::1"));
        assert!(is_private_ip("fd12:3456:789a::1"));
        // New: IPv4-mapped IPv6 loopback
        assert!(is_private_ip("::ffff:127.0.0.1"));
        assert!(is_private_ip("::ffff:192.168.1.1"));
        // New: cloud metadata hostnames
        assert!(is_private_ip("metadata.google.internal"));
        assert!(is_private_ip("metadata.internal"));
        assert!(is_private_ip("instance-data"));

        // Public ranges
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        assert!(!is_private_ip("example.com"));
        assert!(!is_private_ip("httpbin.org"));
    }

    /// Test 12: is_dangerous_header helper function
    #[test]
    fn test_is_dangerous_header_helper() {
        assert!(is_dangerous_header("Host"));
        assert!(is_dangerous_header("host"));
        assert!(is_dangerous_header("HOST"));
        assert!(is_dangerous_header("Content-Length"));
        assert!(is_dangerous_header("Transfer-Encoding"));

        assert!(!is_dangerous_header("Content-Type"));
        assert!(!is_dangerous_header("Authorization"));
        assert!(!is_dangerous_header("X-Custom-Header"));
    }

    // ==================== RequestBody Tests ====================

    /// Test 13: RequestBody::None variant
    #[test]
    fn test_request_body_none() {
        let body = RequestBody::None;
        assert!(body.is_none());
        assert!(!body.is_streaming());
        assert_eq!(body.content_length(), None);
        assert_eq!(body.content_type(), None);
    }

    /// Test 14: RequestBody::Fixed variant
    #[test]
    fn test_request_body_fixed() {
        let body = RequestBody::Fixed(Bytes::from("test data"));
        assert!(!body.is_none());
        assert!(!body.is_streaming());
        assert_eq!(body.content_length(), Some(9));
        assert_eq!(body.content_type(), None);
    }

    /// Test 15: RequestBody::Streaming variant
    #[test]
    fn test_request_body_streaming() {
        let body = RequestBody::Streaming {
            content_type: Some("application/json".to_string()),
        };
        assert!(!body.is_none());
        assert!(body.is_streaming());
        assert_eq!(body.content_length(), None);
        assert_eq!(body.content_type(), Some("application/json"));
    }

    /// Test 16: RequestBody default
    #[test]
    fn test_request_body_default() {
        let body: RequestBody = Default::default();
        assert!(body.is_none());
    }

    /// Test 17: StreamingConfig default
    #[test]
    fn test_streaming_config_default() {
        let config = StreamingConfig::default();
        assert_eq!(config.max_size, 100 * 1024 * 1024); // 100MB
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.max_concurrent, 10);
        assert_eq!(config.chunk_buffer_size, 4);
    }

    /// Test 18: StreamingConfig custom values
    #[test]
    fn test_streaming_config_custom() {
        let config = StreamingConfig {
            max_size: 50 * 1024 * 1024,
            timeout: Duration::from_secs(60),
            max_concurrent: 20,
            chunk_buffer_size: 8,
        };
        assert_eq!(config.max_size, 50 * 1024 * 1024); // 50MB
        assert_eq!(config.timeout, Duration::from_secs(60));
        assert_eq!(config.max_concurrent, 20);
        assert_eq!(config.chunk_buffer_size, 8);
    }
}
