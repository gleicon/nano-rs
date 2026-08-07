//! CLI Input Validation
//!
//! Provides comprehensive input validation with helpful error messages
//! and suggestions for fixing common mistakes.

use crate::cli::error::{CliError, CliResult};

/// Maximum length for sliver names
const MAX_SLIVER_NAME_LEN: usize = 64;

/// Valid hostname characters (simplified check)
const VALID_HOSTNAME_CHARS: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-.";

/// Validate a hostname
pub fn validate_hostname(input: &str) -> CliResult<()> {
    if input.is_empty() {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: None,
        });
    }

    // Check length
    if input.len() > 253 {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: Some("Use a shorter hostname".to_string()),
        });
    }

    // Check for valid characters
    let invalid_chars: Vec<char> = input
        .chars()
        .filter(|c| !VALID_HOSTNAME_CHARS.contains(*c))
        .collect();

    if !invalid_chars.is_empty() {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: Some(format!(
                "Remove invalid characters: {}",
                invalid_chars.iter().collect::<String>()
            )),
        });
    }

    // Check for consecutive dots
    if input.contains("..") {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: Some("Remove consecutive dots".to_string()),
        });
    }

    // Check for leading/trailing dots or hyphens
    if input.starts_with('.') || input.starts_with('-') {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: Some("Remove leading dot or hyphen".to_string()),
        });
    }

    if input.ends_with('.') || input.ends_with('-') {
        return Err(CliError::InvalidHostname {
            input: input.to_string(),
            suggestion: Some("Remove trailing dot or hyphen".to_string()),
        });
    }

    // Check for at least one dot (fully qualified domain)
    // Note: Allow simple names for local testing, but warn
    // This is a relaxed validation suitable for the tool

    Ok(())
}

/// Validate a sliver name
pub fn validate_sliver_name(input: &str) -> CliResult<()> {
    if input.is_empty() {
        return Err(CliError::InvalidSliverName {
            name: input.to_string(),
            reason: "Name cannot be empty".to_string(),
        });
    }

    if input.len() > MAX_SLIVER_NAME_LEN {
        return Err(CliError::InvalidSliverName {
            name: input.to_string(),
            reason: format!("Name too long (max {} characters)", MAX_SLIVER_NAME_LEN),
        });
    }

    // Check first character
    let first = input.chars().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(CliError::InvalidSliverName {
            name: input.to_string(),
            reason: "Name must start with a letter or number".to_string(),
        });
    }

    // Check valid characters
    let invalid: Vec<char> = input
        .chars()
        .filter(|c| !c.is_ascii_alphanumeric() && *c != '-' && *c != '_')
        .collect();

    if !invalid.is_empty() {
        return Err(CliError::InvalidSliverName {
            name: input.to_string(),
            reason: format!(
                "Invalid characters: {} (only letters, numbers, hyphens, underscores allowed)",
                invalid.iter().collect::<String>()
            ),
        });
    }

    Ok(())
}

/// Check if a string is a valid tag
pub fn validate_tag(tag: &str) -> CliResult<()> {
    if tag.len() > 32 {
        return Err(CliError::OperationFailed {
            operation: "Tag validation".to_string(),
            reason: format!("Tag too long: {} (max 32 characters)", tag.len()),
            suggestion: Some("Use a shorter tag".to_string()),
        });
    }

    // Tags can be more permissive than names
    let invalid: Vec<char> = tag
        .chars()
        .filter(|c| !c.is_ascii_alphanumeric() && !"._-".contains(*c))
        .collect();

    if !invalid.is_empty() {
        return Err(CliError::OperationFailed {
            operation: "Tag validation".to_string(),
            reason: format!(
                "Invalid characters in tag: {}",
                invalid.iter().collect::<String>()
            ),
            suggestion: Some(
                "Use only letters, numbers, dots, underscores, and hyphens".to_string(),
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_hostname_valid() {
        assert!(validate_hostname("api.example.com").is_ok());
        assert!(validate_hostname("localhost").is_ok());
        assert!(validate_hostname("my-app.example.com").is_ok());
        assert!(validate_hostname("a.b").is_ok());
    }

    #[test]
    fn test_validate_hostname_invalid() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("..").is_err());
        assert!(validate_hostname("test..com").is_err());
        assert!(validate_hostname("-example.com").is_err());
        assert!(validate_hostname("example.com-").is_err());
    }

    #[test]
    fn test_validate_sliver_name_valid() {
        assert!(validate_sliver_name("api-prod").is_ok());
        assert!(validate_sliver_name("my_app").is_ok());
        assert!(validate_sliver_name("v123").is_ok());
        assert!(validate_sliver_name("a").is_ok());
    }

    #[test]
    fn test_validate_sliver_name_invalid() {
        assert!(validate_sliver_name("").is_err());
        assert!(validate_sliver_name("-invalid").is_err());
        assert!(validate_sliver_name("_invalid").is_err());
        assert!(validate_sliver_name("invalid!").is_err());
        assert!(validate_sliver_name("invalid space").is_err());

        // Too long
        let long_name = "a".repeat(65);
        assert!(validate_sliver_name(&long_name).is_err());
    }

    #[test]
    fn test_validate_tag() {
        assert!(validate_tag("v1.0").is_ok());
        assert!(validate_tag("1.0.0").is_ok());
        assert!(validate_tag("beta-1").is_ok());
        assert!(validate_tag("rc.1").is_ok());

        assert!(validate_tag("v1.0!").is_err());
        assert!(validate_tag(&"a".repeat(33)).is_err());
    }
}
