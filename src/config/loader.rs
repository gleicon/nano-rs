//! Configuration loader
//!
//! Provides JSON configuration file loading with environment variable
//! substitution. Supports the ${VAR} and ${VAR:-default} syntax for
//! dynamic configuration values.
//!
//! # Security Considerations
//!
//! - File size limits prevent memory exhaustion (per T-05-03)
//! - Path validation prevents directory traversal
//! - Environment variable substitution is explicit and controlled
//!
//! # Threat Model Coverage
//!
//! - T-05-01: JSON schema validation with #[serde(deny_unknown_fields)]
//! - T-05-03: Config file size limits (max 1MB)

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::config::{validate_nano_config, NanoConfig};

/// Maximum configuration file size (1MB per T-05-03)
const MAX_CONFIG_SIZE: u64 = 1024 * 1024;

/// Loads configuration from a JSON file
///
/// Reads the specified configuration file, performs environment variable
/// substitution, and validates the configuration.
///
/// # Arguments
///
/// * `path` - Path to the JSON configuration file
///
/// # Returns
///
/// `Ok(NanoConfig)` on success, `Err` with detailed error message on failure
///
/// # Errors
///
/// Returns error for:
/// - File not found or not readable
/// - File exceeds 1MB size limit
/// - Invalid JSON syntax
/// - Validation failures
///
/// # Examples
///
/// ```rust,no_run
/// use nano::config::loader::load_config;
/// use std::path::Path;
///
/// # async fn example() -> anyhow::Result<()> {
/// let config = load_config(Path::new("nano.json")).await?;
/// println!("Loaded {} apps", config.apps.len());
/// # Ok(())
/// # }
/// ```
pub async fn load_config(path: &Path) -> Result<NanoConfig> {
    // Check file exists and is readable
    if !path.exists() {
        return Err(anyhow!("Config file not found: {}", path.display()));
    }

    if !path.is_file() {
        return Err(anyhow!("Config path is not a file: {}", path.display()));
    }

    // Check file size limit (per T-05-03)
    let metadata = fs::metadata(path)
        .with_context(|| format!("Failed to read metadata for: {}", path.display()))?;

    if metadata.len() > MAX_CONFIG_SIZE {
        return Err(anyhow!(
            "Config file exceeds size limit: {} bytes (max {} bytes)",
            metadata.len(),
            MAX_CONFIG_SIZE
        ));
    }

    // Read file content
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    // Perform environment variable substitution
    let substituted = substitute_env_vars(&content)?;

    // Parse JSON
    let config: NanoConfig = serde_json::from_str(&substituted).map_err(|e| {
        let msg = format!("Invalid JSON at {}: {}", path.display(), e);
        anyhow!("{}", format_parse_error(&msg, &substituted, e.line()))
    })?;

    // Validate the configuration
    let base_path = path.parent();
    if let Err(errors) = validate_nano_config(&config, base_path) {
        return Err(anyhow!("Config validation failed:\n{}", errors));
    }

    tracing::info!(
        "Loaded configuration from {}: {} apps",
        path.display(),
        config.apps.len()
    );

    Ok(config)
}

/// Returns the default configuration file path
///
/// Returns "nano.json" in the current working directory.
pub fn default_config_path() -> PathBuf {
    PathBuf::from("nano.json")
}

/// Substitutes environment variables in configuration text
///
/// Supports two syntaxes:
/// - `${VAR}` - Replaced with value of environment variable VAR, or empty string if not set
/// - `${VAR:-default}` - Replaced with value of VAR, or "default" if VAR is not set
///
/// # Arguments
///
/// * `text` - The configuration text containing variable references
///
/// # Returns
///
/// `Ok(String)` with substitutions applied, `Err` on malformed syntax
fn substitute_env_vars(text: &str) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            // Consume the '{'
            chars.next();

            // Parse variable name and optional default
            let (var_name, default_value) = parse_var_ref(&mut chars)?;

            // Look up environment variable
            let value =
                std::env::var(&var_name).unwrap_or_else(|_| default_value.unwrap_or_default());

            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

/// Parses a variable reference like VAR or VAR:-default
///
/// Consumes characters from the iterator until '}' is found.
fn parse_var_ref<I>(chars: &mut std::iter::Peekable<I>) -> Result<(String, Option<String>)>
where
    I: Iterator<Item = char>,
{
    let mut var_name = String::new();

    // Read variable name (alphanumeric and underscore)
    while let Some(&ch) = chars.peek() {
        if ch == '}' {
            chars.next(); // consume '}'
            return Ok((var_name, None));
        } else if ch == ':' {
            // Found : separator, check for :-default syntax
            chars.next(); // consume ':'

            // Check for '-' after ':'
            if chars.peek() == Some(&'-') {
                chars.next(); // consume '-'

                // Read default value directly (avoid unused variable warning)
                let mut default = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == '}' {
                        chars.next(); // consume '}'
                        return Ok((var_name, Some(default)));
                    }
                    default.push(ch);
                    chars.next();
                }
                return Err(anyhow!("Unclosed default value in ${{VAR:-...}}"));
            } else {
                return Err(anyhow!(
                    "Invalid syntax after ':' in variable reference, expected ':-'"
                ));
            }
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            var_name.push(ch);
            chars.next();
        } else {
            return Err(anyhow!(
                "Invalid character '{}' in variable name '${{{}}}'",
                ch,
                var_name
            ));
        }
    }

    Err(anyhow!("Unclosed variable reference: ${{{}", var_name))
}

/// Formats a JSON parse error with line context
///
/// Adds line numbers and context around the error line for easier debugging.
fn format_parse_error(error_msg: &str, content: &str, error_line: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = error_line.saturating_sub(2).max(1);
    let end = (error_line + 1).min(lines.len());

    let mut formatted = format!("{}\n\nContext:\n", error_msg);

    for line_num in start..=end {
        let prefix = if line_num == error_line {
            ">>> "
        } else {
            "    "
        };
        formatted.push_str(&format!(
            "{}{}: {}\n",
            prefix,
            line_num,
            lines[line_num - 1]
        ));
    }

    formatted
}

/// Loads configuration from a string (for testing)
///
/// Parses and validates a configuration from an in-memory string.
/// Environment variable substitution is performed.
///
/// # Arguments
///
/// * `content` - The JSON configuration as a string
/// * `base_path` - Optional base path for validating entrypoint files
///
/// # Returns
///
/// `Ok(NanoConfig)` on success, `Err` on failure
#[cfg(test)]
pub fn load_config_from_str(content: &str, base_path: Option<&Path>) -> Result<NanoConfig> {
    let substituted = substitute_env_vars(content)?;

    let config: NanoConfig = serde_json::from_str(&substituted)
        .map_err(|e| anyhow!("Invalid JSON: {} at line {}", e, e.line()))?;

    if let Err(errors) = validate_nano_config(&config, base_path) {
        return Err(anyhow!("Config validation failed:\n{}", errors));
    }

    Ok(config)
}

/// Loads and validates a configuration from a string synchronously
///
/// This is a synchronous version of the config loader for use in
/// non-async contexts.
///
/// # Arguments
///
/// * `content` - JSON configuration string
/// * `base_path` - Optional base directory for relative path resolution
///
/// # Returns
/// `Ok(NanoConfig)` or `Err` with details
pub fn load_config_sync(content: &str, base_path: Option<&Path>) -> Result<NanoConfig> {
    let substituted = substitute_env_vars(content)?;

    let config: NanoConfig = serde_json::from_str(&substituted)
        .map_err(|e| anyhow!("Invalid JSON: {} at line {}", e, e.line()))?;

    if let Err(errors) = validate_nano_config(&config, base_path) {
        return Err(anyhow!("Config validation failed:\n{}", errors));
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_load_config_from_str_valid() {
        let json = r#"{
            "apps": [{
                "hostname": "api.example.com",
                "entrypoint": "/app/index.js",
                "env_vars": {"API_KEY": "secret"},
                "limits": {"memory_mb": 128, "timeout_secs": 30, "workers": 4}
            }],
            "server": {"port": 8080, "host": "0.0.0.0"}
        }"#;

        let config = load_config_from_str(json, None).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.apps[0].hostname, "api.example.com");
    }

    #[test]
    fn test_load_config_from_str_invalid_json() {
        let json = r#"{ invalid json }"#;

        let result = load_config_from_str(json, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid JSON"));
    }

    #[test]
    fn test_load_config_from_str_validation_failure() {
        // Empty hostname should fail validation
        let json = r#"{
            "apps": [{"hostname": "", "entrypoint": "/app.js"}],
            "server": {"port": 8080, "host": "0.0.0.0"}
        }"#;

        let result = load_config_from_str(json, None);
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        // Should get either JSON parse error or validation error
        assert!(
            err_str.contains("Config validation failed")
                || err_str.contains("missing")
                || err_str.contains("hostname"),
            "Expected validation error, got: {}",
            err_str
        );
    }

    #[tokio::test]
    async fn test_load_config_file_not_found() {
        let path = Path::new("/nonexistent/config.json");
        let result = load_config(path).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_substitute_env_vars_simple() {
        env::set_var("TEST_VAR", "test_value");

        let result = substitute_env_vars("Value is ${TEST_VAR}").unwrap();
        assert_eq!(result, "Value is test_value");

        env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_substitute_env_vars_default() {
        // TEST_VAR_DEFAULT is not set, should use default
        let result = substitute_env_vars("Value is ${TEST_VAR_DEFAULT:-default_value}").unwrap();
        assert_eq!(result, "Value is default_value");

        // Now set it and try again
        env::set_var("TEST_VAR_DEFAULT", "custom_value");
        let result = substitute_env_vars("Value is ${TEST_VAR_DEFAULT:-default_value}").unwrap();
        assert_eq!(result, "Value is custom_value");
        env::remove_var("TEST_VAR_DEFAULT");
    }

    #[test]
    fn test_substitute_env_vars_missing_no_default() {
        // UNSET_VAR is not set and no default, should be empty
        let result = substitute_env_vars("Value is '${UNSET_VAR}'").unwrap();
        assert_eq!(result, "Value is ''");
    }

    #[test]
    fn test_substitute_env_vars_multiple() {
        env::set_var("VAR_A", "alpha");
        env::set_var("VAR_B", "beta");

        let result = substitute_env_vars("${VAR_A} and ${VAR_B}").unwrap();
        assert_eq!(result, "alpha and beta");

        env::remove_var("VAR_A");
        env::remove_var("VAR_B");
    }

    #[test]
    fn test_substitute_env_vars_no_substitution() {
        let result = substitute_env_vars("No variables here").unwrap();
        assert_eq!(result, "No variables here");
    }

    #[test]
    fn test_substitute_env_vars_unclosed() {
        let result = substitute_env_vars("Unclosed ${VAR");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unclosed"));
    }

    #[test]
    fn test_substitute_env_vars_invalid_char() {
        let result = substitute_env_vars("Invalid ${VAR!NAME}");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid character"));
    }

    #[test]
    fn test_default_config_path() {
        let path = default_config_path();
        assert_eq!(path, PathBuf::from("nano.json"));
    }

    #[test]
    fn test_format_parse_error() {
        let content = "line1\nline2\nline3\nline4";
        let error = format_parse_error("Error message", content, 3);

        assert!(error.contains("Error message"));
        assert!(error.contains(">>> 3:")); // Error line highlighted
        assert!(error.contains("line2")); // Context lines included
        assert!(error.contains("line4")); // Context lines included
    }

    #[tokio::test]
    async fn test_load_config_with_env_substitution() {
        env::set_var("NANO_TEST_PORT", "9090");

        let temp_dir = tempfile::TempDir::new().unwrap();
        let config_path = temp_dir.path().join("test.json");
        let app_path = temp_dir.path().join("app.js");

        // Create a dummy app.js file so validation passes
        fs::write(&app_path, "// dummy app").unwrap();

        let json = r#"{
            "apps": [{
                "hostname": "api.example.com",
                "entrypoint": "app.js"
            }],
            "server": {"port": ${NANO_TEST_PORT}, "host": "0.0.0.0"}
        }"#;

        fs::write(&config_path, json).unwrap();

        let config = load_config(&config_path).await.unwrap();
        assert_eq!(config.server.port, 9090);

        env::remove_var("NANO_TEST_PORT");
    }

    #[test]
    fn test_load_config_sync() {
        let json = r#"{
            "apps": [{
                "hostname": "api.example.com",
                "entrypoint": "/app.js"
            }],
            "server": {"port": 8080, "host": "0.0.0.0"}
        }"#;

        let config = load_config_sync(json, None).unwrap();
        assert_eq!(config.apps.len(), 1);
        assert_eq!(config.server.port, 8080);
    }
}
