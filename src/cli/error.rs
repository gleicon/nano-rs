//! CLI Error Types
//!
//! Defines human-readable error types for CLI operations with helpful
//! suggestions for fixing common issues.

use thiserror::Error;

/// CLI-specific errors with human-readable messages
#[derive(Error, Debug)]
pub enum CliError {
    /// Invalid hostname provided
    #[error("✗ Invalid hostname: '{input}'\n{}\n\nHostnames must be valid domain names (e.g., 'api.example.com')", 
        suggestion.as_ref().map(|s| format!("\nDid you mean: {}?", s)).unwrap_or_default())]
    InvalidHostname {
        input: String,
        suggestion: Option<String>,
    },

    /// Invalid sliver name
    #[error("✗ Invalid sliver name: '{name}'\n\nReason: {reason}\n\nSliver names must:\n  • Use only letters, numbers, hyphens, and underscores\n  • Be 1-64 characters long\n  • Start with a letter or number")]
    InvalidSliverName { name: String, reason: String },

    /// Generic operation error with suggestion
    #[error("✗ {operation} failed\n\nReason: {reason}\n\n{}", 
        suggestion.as_ref().unwrap_or(&"Check the error details and try again.".to_string()))]
    OperationFailed {
        operation: String,
        reason: String,
        suggestion: Option<String>,
    },
}

/// Result type alias for CLI operations
pub type CliResult<T> = Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = CliError::InvalidHostname {
            input: "not_a_valid_host".to_string(),
            suggestion: Some("not-a-valid-host".to_string()),
        };
        let display = format!("{}", err);
        assert!(display.contains("Invalid hostname"));
        assert!(display.contains("not_a_valid_host"));
    }

    #[test]
    fn test_invalid_sliver_name_error() {
        let err = CliError::InvalidSliverName {
            name: "bad name!".to_string(),
            reason: "Contains spaces and special characters".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("Invalid sliver name"));
        assert!(display.contains("bad name!"));
    }
}
