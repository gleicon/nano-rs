//! Field extraction helpers for structured JSON logging
//!
//! Provides utilities for extracting and formatting field values from tracing events
//! and span contexts. Used by the JSON logging layer to build structured log output.

use serde_json::Value;
use std::collections::BTreeMap;
use tracing::field::{Field, Visit};

/// Visitor that extracts field values into a BTreeMap for JSON serialization
///
/// This visitor is used by the JSON layer to collect all fields from a tracing
/// event into a serializable format. It handles all standard field types
/// (booleans, integers, floats, strings, errors).
pub struct JsonVisitor<'a>(pub &'a mut BTreeMap<String, Value>);

impl<'a> Visit for JsonVisitor<'a> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // Format debug representation as string
        let key = field.name().to_string();
        let value_str = format!("{:?}", value);
        self.0.insert(key, Value::String(value_str));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(num) = serde_json::Number::from_f64(value) {
            self.0.insert(field.name().to_string(), Value::Number(num));
        } else {
            // Fallback to string representation if f64 is not a valid JSON number
            self.0
                .insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_string(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        // Handle values that might not fit in i64
        if let Ok(val) = i64::try_from(value) {
            self.0
                .insert(field.name().to_string(), Value::Number(val.into()));
        } else {
            // For values too large, store as string
            self.0
                .insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_string(), Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.0
            .insert(field.name().to_string(), Value::String(value.to_string()));
    }
}

/// Extract a string field from the visitor's collected fields
pub fn extract_string(fields: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// Extract an integer field from the visitor's collected fields
pub fn extract_i64(fields: &BTreeMap<String, Value>, key: &str) -> Option<i64> {
    fields.get(key).and_then(|v| v.as_i64())
}

/// Extract a boolean field from the visitor's collected fields
pub fn extract_bool(fields: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| v.as_bool())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_visitor_string() {
        let mut fields = BTreeMap::new();
        let _visitor = JsonVisitor(&mut fields);

        // Simulate recording a string field

        // We can't easily create a real Field, so we test through the public API

        // Test extraction helpers with manually inserted data
        fields.insert("test".to_string(), Value::String("value".to_string()));

        assert_eq!(extract_string(&fields, "test"), Some("value".to_string()));
        assert_eq!(extract_string(&fields, "missing"), None);
    }

    #[test]
    fn test_json_visitor_integer() {
        let mut fields = BTreeMap::new();
        fields.insert("count".to_string(), Value::Number(42i64.into()));
        fields.insert("large".to_string(), Value::String(u64::MAX.to_string()));

        assert_eq!(extract_i64(&fields, "count"), Some(42));
        assert_eq!(extract_i64(&fields, "missing"), None);
    }

    #[test]
    fn test_json_visitor_bool() {
        let mut fields = BTreeMap::new();
        fields.insert("enabled".to_string(), Value::Bool(true));

        assert_eq!(extract_bool(&fields, "enabled"), Some(true));
        assert_eq!(extract_bool(&fields, "missing"), None);
    }
}
