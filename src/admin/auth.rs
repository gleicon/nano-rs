//! Admin API authentication middleware
//!
//! Provides API key-based authentication for the Admin API endpoints.
//! API keys are expected in the `X-Admin-Key` header. Health and readiness
//! endpoints are publicly accessible without authentication.
//!
//! # Security Considerations
//!
//! - API keys should be at least 32 characters long (configurable)
//! - Keys should be cryptographically random (use `openssl rand -hex 32`)
//! - Failed auth attempts are logged at WARN level for intrusion detection
//! - Constant-time XOR-fold comparison prevents timing attacks

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use std::sync::Arc;

/// Admin API authentication state
///
/// Contains the configured API key for request validation.
/// Shared across all admin endpoints via State.
#[derive(Debug, Clone)]
pub struct AdminAuth {
    /// The valid API key for authentication
    api_key: String,
}

impl AdminAuth {
    /// Create a new AdminAuth with the specified API key
    ///
    /// # Arguments
    ///
    /// * `api_key` - The API key for authentication
    ///
    /// # Example
    ///
    /// ```rust
    /// use nano::admin::auth::AdminAuth;
    ///
    /// let auth = AdminAuth::new("my-secret-api-key-12345");
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }

    /// Validate an API key against the configured key
    ///
    /// Returns true if the provided key matches the configured key.
    /// Empty keys never match.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to validate
    ///
    /// # Returns
    ///
    /// `true` if the key is valid, `false` otherwise
    pub fn validate(&self, key: &str) -> bool {
        if key.is_empty() || self.api_key.is_empty() {
            return false;
        }
        let a = key.as_bytes();
        let b = self.api_key.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    /// Check if an API key is configured
    ///
    /// Returns true if the API key is non-empty.
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }
}

/// Error response for authentication failures
#[derive(Debug, Serialize)]
pub struct AuthError {
    error: String,
    message: String,
    code: u16,
}

impl AuthError {
    /// Create a new authentication error
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            error: "Unauthorized".to_string(),
            message: message.into(),
            code: 401,
        }
    }

    /// Create a JSON response for this error
    pub fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, Json(self)).into_response()
    }
}

/// API key authentication middleware
///
/// Extracts the `X-Admin-Key` header and validates it against the configured API key.
/// If authentication succeeds, the request continues. If it fails, returns a 401 error.
///
/// # Arguments
///
/// * `State(auth)` - The AdminAuth state containing the valid API key
/// * `req` - The incoming request
/// * `next` - The next middleware/handler in the chain
///
/// # Returns
///
/// Either the response from the next handler (if auth succeeds) or a 401 error response.
///
/// # Example
///
/// ```ignore
/// use axum::Router;
/// use axum::routing::get;
/// use nano::admin::auth::{api_key_middleware, AdminAuth};
/// use std::sync::Arc;
///
/// let auth = AdminAuth::new("my-secret-key");
/// let app: Router = Router::new()
///     .route("/admin/protected", get(|| async { "Hello" }))
///     .layer(axum::middleware::from_fn_with_state(
///         Arc::new(auth),
///         api_key_middleware,
///     ));
/// ```
pub async fn api_key_middleware(
    State(auth): State<Arc<AdminAuth>>,
    req: Request,
    next: Next,
) -> Response {
    // Skip auth if no key is configured (shouldn't happen in production)
    if !auth.is_configured() {
        tracing::warn!("Admin API key not configured, allowing request (development mode?)");
        return next.run(req).await;
    }

    // Extract the X-Admin-Key header
    let key = req
        .headers()
        .get("X-Admin-Key")
        .and_then(|v| v.to_str().ok());

    match key {
        Some(k) if auth.validate(k) => {
            // Authentication successful
            tracing::debug!("Admin API authentication successful");
            next.run(req).await
        }
        Some(_) => {
            // Invalid key provided
            tracing::warn!("Admin API authentication failed: invalid API key");
            AuthError::new("Invalid API key").into_response()
        }
        None => {
            // No key provided
            tracing::warn!("Admin API authentication failed: missing X-Admin-Key header");
            AuthError::new("Missing X-Admin-Key header").into_response()
        }
    }
}

/// API key authentication middleware that returns 403 Forbidden
///
/// Similar to api_key_middleware but returns 403 instead of 401.
/// Used when authentication is required but the user is already authenticated
/// (e.g., valid API key but insufficient permissions).
pub async fn api_key_middleware_forbidden(
    State(auth): State<Arc<AdminAuth>>,
    req: Request,
    next: Next,
) -> Response {
    let key = req
        .headers()
        .get("X-Admin-Key")
        .and_then(|v| v.to_str().ok());

    match key {
        Some(k) if auth.validate(k) => {
            next.run(req).await
        }
        _ => {
            let error = AuthError {
                error: "Forbidden".to_string(),
                message: "Access denied".to_string(),
                code: 403,
            };
            (StatusCode::FORBIDDEN, Json(error)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    use axum::http::StatusCode;

    #[test]
    fn test_admin_auth_new() {
        let auth = AdminAuth::new("test-key-12345");
        assert!(auth.is_configured());
        assert!(auth.validate("test-key-12345"));
        assert!(!auth.validate("wrong-key"));
    }

    #[test]
    fn test_admin_auth_empty() {
        let auth = AdminAuth::new("");
        assert!(!auth.is_configured());
        assert!(!auth.validate(""));
        assert!(!auth.validate("any-key"));
    }

    #[test]
    fn test_admin_auth_validate_case_sensitive() {
        let auth = AdminAuth::new("SecretKey123");
        assert!(auth.validate("SecretKey123"));
        assert!(!auth.validate("secretkey123")); // Case sensitive
        assert!(!auth.validate("SECRETKEY123"));
    }

    #[test]
    fn test_auth_error_creation() {
        let error = AuthError::new("Test error message");
        assert_eq!(error.error, "Unauthorized");
        assert_eq!(error.message, "Test error message");
        assert_eq!(error.code, 401);
    }

    #[tokio::test]
    async fn test_auth_error_response() {
        let error = AuthError::new("Invalid key");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_admin_auth_long_key() {
        // Simulate a 32+ character key (recommended minimum)
        let long_key = "a".repeat(32);
        let auth = AdminAuth::new(&long_key);
        assert!(auth.validate(&long_key));
        assert!(!auth.validate(&"a".repeat(31)));
    }

    #[test]
    fn test_validate_empty_key() {
        let auth = AdminAuth::new("valid-key");
        assert!(!auth.validate(""));
    }

    #[test]
    fn test_validate_with_whitespace() {
        let auth = AdminAuth::new("my-key");
        // Whitespace is significant in the comparison
        assert!(!auth.validate(" my-key"));
        assert!(!auth.validate("my-key "));
        assert!(!auth.validate("my-key\n"));
    }
}
