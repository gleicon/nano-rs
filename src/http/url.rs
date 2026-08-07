//! URL and URLSearchParams implementation
//!
//! WinterTC-compliant URL parsing and query parameter handling.
//! Implements all WinterTC URL properties and URLSearchParams API.
//!
//! # Decisions
//!
//! - **D-09:** Full WinterTC URL compliance (not basic parsing only)
//! - **D-10:** Percent-decode with lossy UTF-8 replacement (U+FFFD for invalid sequences)

use percent_encoding::percent_decode_str;
use std::collections::HashMap;
use url::Url;

/// WinterTC-compliant URL type
///
/// Wraps the `url` crate with WinterTC-specific properties and methods.
/// All URL operations are per the WinterTC specification.
#[derive(Debug, Clone)]
pub struct NanoUrl {
    inner: Url,
    search_params: NanoUrlSearchParams,
}

impl NanoUrl {
    /// Parse a URL string into a NanoUrl
    ///
    /// # Arguments
    ///
    /// * `url_str` - The URL string to parse
    ///
    /// # Returns
    ///
    /// `Ok(NanoUrl)` on success, or an error if parsing fails
    ///
    /// # Example
    ///
    /// ```
    /// use nano::http::NanoUrl;
    ///
    /// let url = NanoUrl::parse("https://example.com/path?foo=bar").unwrap();
    /// assert_eq!(url.protocol(), "https:");
    /// ```
    pub fn parse(url_str: &str) -> anyhow::Result<Self> {
        let inner = Url::parse(url_str)?;
        let search_params = NanoUrlSearchParams::from_query(inner.query());
        Ok(Self {
            inner,
            search_params,
        })
    }

    /// The full URL as a string
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/href
    pub fn href(&self) -> String {
        self.inner.to_string()
    }

    /// The origin of the URL (scheme + host + port)
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/origin
    pub fn origin(&self) -> String {
        self.inner.origin().ascii_serialization()
    }

    /// The protocol/scheme of the URL with colon
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/protocol
    pub fn protocol(&self) -> String {
        format!("{}:", self.inner.scheme())
    }

    /// The host (hostname + optional port)
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/host
    pub fn host(&self) -> String {
        self.inner.host_str().unwrap_or("").to_string()
    }

    /// The hostname without port
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/hostname
    pub fn hostname(&self) -> String {
        self.inner.host_str().unwrap_or("").to_string()
    }

    /// The port number (if any)
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/port
    pub fn port(&self) -> Option<u16> {
        self.inner.port()
    }

    /// The path component of the URL
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/pathname
    pub fn pathname(&self) -> String {
        self.inner.path().to_string()
    }

    /// The query string with leading ?
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/search
    pub fn search(&self) -> String {
        self.inner
            .query()
            .map(|q| format!("?{}", q))
            .unwrap_or_default()
    }

    /// The fragment/hash with leading #
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/hash
    pub fn hash(&self) -> String {
        self.inner
            .fragment()
            .map(|f| format!("#{}", f))
            .unwrap_or_default()
    }

    /// The search parameters for this URL
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URL/searchParams
    pub fn search_params(&self) -> &NanoUrlSearchParams {
        &self.search_params
    }
}

/// WinterTC-compliant URLSearchParams type
///
/// Handles query string parsing with percent-decoding per D-10.
/// All values are stored as decoded strings, with invalid UTF-8
/// sequences replaced with U+FFFD per the lossy decoding requirement.
#[derive(Debug, Clone, Default)]
pub struct NanoUrlSearchParams {
    params: HashMap<String, Vec<String>>, // name -> values
}

impl NanoUrlSearchParams {
    /// Create URLSearchParams from a query string
    ///
    /// Parses the query string, percent-decoding keys and values.
    /// Invalid UTF-8 sequences are replaced with U+FFFD per D-10.
    ///
    /// # Arguments
    ///
    /// * `query` - The query string (without leading ?) or None
    ///
    /// # Returns
    ///
    /// A new `NanoUrlSearchParams` with parsed parameters
    pub fn from_query(query: Option<&str>) -> Self {
        let mut params = HashMap::new();
        if let Some(q) = query {
            for pair in q.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    let decoded_key = percent_decode_str(key).decode_utf8_lossy().to_string();
                    let decoded_value = percent_decode_str(value).decode_utf8_lossy().to_string();
                    params
                        .entry(decoded_key)
                        .or_insert_with(Vec::new)
                        .push(decoded_value);
                }
            }
        }
        Self { params }
    }

    /// Get the first value for a parameter name
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/get
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name to look up
    ///
    /// # Returns
    ///
    /// `Some(value)` if found, `None` otherwise
    pub fn get(&self, name: &str) -> Option<String> {
        self.params.get(name).and_then(|v| v.first().cloned())
    }

    /// Get all values for a parameter name
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/getAll
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name to look up
    ///
    /// # Returns
    ///
    /// A vector of all values for this parameter (empty if not found)
    pub fn get_all(&self, name: &str) -> Vec<String> {
        self.params.get(name).cloned().unwrap_or_default()
    }

    /// Check if a parameter exists
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/has
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name to check
    ///
    /// # Returns
    ///
    /// `true` if the parameter exists, `false` otherwise
    pub fn has(&self, name: &str) -> bool {
        self.params.contains_key(name)
    }

    /// Set a parameter to a single value (replaces any existing values)
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/set
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name
    /// * `value` - The value to set
    pub fn set(&mut self, name: &str, value: &str) {
        self.params
            .insert(name.to_string(), vec![value.to_string()]);
    }

    /// Append a value to a parameter (preserving existing values)
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/append
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name
    /// * `value` - The value to append
    pub fn append(&mut self, name: &str, value: &str) {
        self.params
            .entry(name.to_string())
            .or_default()
            .push(value.to_string());
    }

    /// Delete all values for a parameter name
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/delete
    ///
    /// # Arguments
    ///
    /// * `name` - The parameter name to delete
    pub fn delete(&mut self, name: &str) {
        self.params.remove(name);
    }

    /// Serialize to query string format
    ///
    /// Per WinterTC: https://developer.mozilla.org/en-US/docs/Web/API/URLSearchParams/toString
    ///
    /// # Returns
    ///
    /// A query string representation (without leading ?)
    pub fn to_string(&self) -> String {
        let mut pairs = Vec::new();
        for (key, values) in &self.params {
            for value in values {
                pairs.push(format!("{}={}", key, value));
            }
        }
        pairs.join("&")
    }

    /// Iterate over all parameter entries
    ///
    /// Returns an iterator over (name, values) pairs.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &Vec<String>)> {
        self.params.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_parsing() {
        let url = NanoUrl::parse("https://example.com/path?foo=bar&baz=qux").unwrap();
        assert_eq!(url.href(), "https://example.com/path?foo=bar&baz=qux");
        assert_eq!(url.protocol(), "https:");
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.pathname(), "/path");
    }

    #[test]
    fn test_url_search_params() {
        let url = NanoUrl::parse("https://example.com/?foo=bar&foo=baz&qux=quux").unwrap();
        assert_eq!(url.search_params().get("foo"), Some("bar".to_string()));
        assert_eq!(url.search_params().get_all("foo"), vec!["bar", "baz"]);
        assert!(url.search_params().has("qux"));
    }

    #[test]
    fn test_percent_decoding() {
        // Test D-10: percent-decode with lossy UTF-8 replacement
        let url = NanoUrl::parse("https://example.com/?q=%FF%FE").unwrap();
        assert_eq!(
            url.search_params().get("q"),
            Some("\u{FFFD}\u{FFFD}".to_string())
        );
    }

    #[test]
    fn test_url_full_properties() {
        let url = NanoUrl::parse(
            "https://user:pass@example.com:8080/path/to/resource?foo=bar&baz=qux#section",
        )
        .unwrap();

        assert_eq!(url.protocol(), "https:");
        assert_eq!(url.host(), "example.com");
        assert_eq!(url.hostname(), "example.com");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.pathname(), "/path/to/resource");
        assert_eq!(url.search(), "?foo=bar&baz=qux");
        assert_eq!(url.hash(), "#section");
        assert!(url.href().contains("https://"));
        assert!(url.origin().contains("example.com"));
    }

    #[test]
    fn test_url_search_params_api() {
        let mut params = NanoUrlSearchParams::from_query(Some("foo=bar&foo=baz&qux=quux"));

        // get() returns first value
        assert_eq!(params.get("foo"), Some("bar".to_string()));

        // get_all() returns all values
        let all_foo = params.get_all("foo");
        assert_eq!(all_foo.len(), 2);
        assert!(all_foo.contains(&"bar".to_string()));
        assert!(all_foo.contains(&"baz".to_string()));

        // has()
        assert!(params.has("qux"));
        assert!(!params.has("unknown"));

        // set() replaces values
        params.set("foo", "new");
        assert_eq!(params.get("foo"), Some("new".to_string()));
        assert_eq!(params.get_all("foo"), vec!["new"]);

        // append() adds values
        params.append("foo", "extra");
        assert_eq!(params.get_all("foo"), vec!["new", "extra"]);

        // delete() removes
        params.delete("qux");
        assert!(!params.has("qux"));

        // to_string()
        let query = params.to_string();
        assert!(query.contains("foo=new"));
        assert!(query.contains("foo=extra"));
    }

    #[test]
    fn test_lossy_percent_decoding() {
        // D-10: Invalid UTF-8 sequences become U+FFFD
        let params = NanoUrlSearchParams::from_query(Some("invalid=%FF%FE%FD"));
        let value = params.get("invalid").unwrap();
        assert!(value.contains('\u{FFFD}')); // Replacement character
    }

    #[test]
    fn test_url_special_character_decoding() {
        // Test decoding of special characters
        let params = NanoUrlSearchParams::from_query(Some("special=%26%3D%2F"));
        assert_eq!(params.get("special"), Some("&=/".to_string()));
    }

    #[test]
    fn test_url_empty_query() {
        let url = NanoUrl::parse("https://example.com/path").unwrap();
        assert_eq!(url.search(), "");
        assert_eq!(url.search_params().entries().count(), 0);
    }

    #[test]
    fn test_url_no_fragment() {
        let url = NanoUrl::parse("https://example.com/path").unwrap();
        assert_eq!(url.hash(), "");
    }
}
