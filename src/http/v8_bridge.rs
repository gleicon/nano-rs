//! V8 Bridge for WinterTC types
//!
//! This module provides serialization for Request/Response objects used
//! with V8 JavaScript contexts.
//!
//! Per D-06: JSON serialization → V8 parse (simpler than direct V8 API creation).

use crate::http::NanoRequest;

/// Serialize a NanoRequest to JSON string
///
/// Creates a JSON representation matching the WinterTC Request interface.
/// This JSON can be parsed in V8 using JSON.parse() per D-06.
///
/// # Example
///
/// ```
/// use nano::http::{NanoRequest, NanoUrl, NanoHeaders};
/// use nano::http::v8_bridge::serialize_request_to_json;
///
/// let url = NanoUrl::parse("https://example.com/api").unwrap();
/// let request = NanoRequest::new(
///     "GET".to_string(),
///     url,
///     NanoHeaders::new(),
///     None,
/// );
/// let json = serialize_request_to_json(&request);
/// assert!(json.contains("\"method\":\"GET\""));
/// ```
pub fn serialize_request_to_json(request: &NanoRequest) -> String {
    use serde_json::{json, Map, Value};

    let mut headers = Map::new();
    request.headers().for_each(|k, v| {
        headers.insert(k.to_owned(), Value::String(v.to_owned()));
    });

    let (body_val, body_used) = match request.body() {
        Some(b) => (Value::String(base64_encode(b)), true),
        None => (Value::Null, false),
    };

    json!({
        "method": request.method(),
        "url": request.url_string(),
        "headers": headers,
        "body": body_val,
        "bodyUsed": body_used,
    })
    .to_string()
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{NanoHeaders, NanoUrl};

    #[test]
    fn test_request_serialization() {
        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let mut headers = NanoHeaders::new();
        headers.set("Content-Type", "application/json");
        let request = NanoRequest::new(
            "POST".to_string(),
            url,
            headers,
            Some(bytes::Bytes::from("test body")),
        );

        let json = serialize_request_to_json(&request);
        assert!(json.contains("\"method\":\"POST\""));
        assert!(json.contains("\"url\":\"https://example.com/api\""));
        // Headers are stored lowercase per D-07
        assert!(json.contains("\"content-type\""));
        assert!(json.contains("\"bodyUsed\":true"));
    }

    #[test]
    fn test_request_serialization_no_body() {
        let url = NanoUrl::parse("https://example.com/api").unwrap();
        let headers = NanoHeaders::new();
        let request = NanoRequest::new("GET".to_string(), url, headers, None);

        let json = serialize_request_to_json(&request);
        assert!(json.contains("\"body\":null"));
        assert!(json.contains("\"bodyUsed\":false"));
    }
}
