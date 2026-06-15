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
/// # Arguments
///
/// * `request` - The NanoRequest to serialize
///
/// # Returns
///
/// A JSON string representation of the request
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
    // Build JSON manually to ensure correct WinterTC structure
    let mut json = String::from("{");

    // method
    json.push_str(&format!(
        "\"method\":\"{}\",",
        escape_json(request.method())
    ));

    // url
    json.push_str(&format!(
        "\"url\":\"{}\",",
        escape_json(&request.url_string())
    ));

    // headers
    json.push_str("\"headers\":{");
    let mut first = true;
    request.headers().for_each(|name, value| {
        if !first {
            json.push(',');
        }
        first = false;
        json.push_str(&format!(
            "\"{}\":\"{}\"",
            escape_json(name),
            escape_json(value)
        ));
    });
    json.push_str("},");

    // body (base64 encoded if present)
    if let Some(body) = request.body() {
        let base64 = base64_encode(body);
        json.push_str(&format!("\"body\":\"{}\",\"bodyUsed\":true", base64));
    } else {
        json.push_str("\"body\":null,\"bodyUsed\":false");
    }

    json.push('}');
    json
}

/// Serialize a NanoResponse to JSON string
///
/// Escape string for JSON safety
///
/// Escapes backslashes, quotes, and control characters for JSON.
///
/// # Arguments
///
/// * `s` - The string to escape
///
/// # Returns
///
/// The escaped string safe for JSON inclusion
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
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
    fn test_escape_json() {
        let token_a = format!("nano-{}", uuid::Uuid::new_v4());
        let token_b = format!("nano-{}", uuid::Uuid::new_v4());

        // Test escaping quotes
        let input = format!("{}\"{}", token_a, token_b);
        let result = escape_json(&input);
        assert!(result.contains(&token_a), "Escaped output missing token_a: {}", result);
        assert!(result.contains("\\\""), "Escaped output missing escaped quote: {}", result);
        assert!(result.contains(&token_b), "Escaped output missing token_b: {}", result);

        // Test escaping newlines
        let token_c = format!("nano-{}", uuid::Uuid::new_v4());
        let token_d = format!("nano-{}", uuid::Uuid::new_v4());
        let input = format!("{}\n{}", token_c, token_d);
        let result = escape_json(&input);
        assert!(result.contains(&token_c), "Escaped output missing token_c: {}", result);
        assert!(result.contains("\\n"), "Escaped output missing escaped newline: {}", result);
        assert!(result.contains(&token_d), "Escaped output missing token_d: {}", result);

        // Test escaping tabs
        let token_e = format!("nano-{}", uuid::Uuid::new_v4());
        let token_f = format!("nano-{}", uuid::Uuid::new_v4());
        let input = format!("{}\t{}", token_e, token_f);
        let result = escape_json(&input);
        assert!(result.contains(&token_e), "Escaped output missing token_e: {}", result);
        assert!(result.contains("\\t"), "Escaped output missing escaped tab: {}", result);
        assert!(result.contains(&token_f), "Escaped output missing token_f: {}", result);

        // Test escaping backslashes
        let token_g = format!("nano-{}", uuid::Uuid::new_v4());
        let token_h = format!("nano-{}", uuid::Uuid::new_v4());
        let input = format!("{}\\\\{}", token_g, token_h);
        let result = escape_json(&input);
        assert!(result.contains(&token_g), "Escaped output missing token_g: {}", result);
        assert!(result.contains("\\\\"), "Escaped output missing escaped backslash: {}", result);
        assert!(result.contains(&token_h), "Escaped output missing token_h: {}", result);
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
