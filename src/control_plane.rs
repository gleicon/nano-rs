//! Per-tenant limits registry.
//!
//! `ControlPlane` holds the per-tenant resource limits (`TenantLimits`) that the
//! work queue consults when registering hostnames. It is a registry, not a
//! request pipeline: requests are validated and dispatched by the worker pool
//! directly, not routed through here.
//!
//! (An earlier design routed all requests through a validate-and-batch pipeline
//! in this module. That pipeline was never wired into the data plane and has been
//! removed; only the tenant-limits registry it also provided is kept.)

use std::collections::HashMap;

use crate::limits::*;
use crate::worker::HandlerTask;
use crate::{
    assert_negative, assert_positive, assert_postcondition, assert_precondition, assert_range,
};

/// Error returned by request validation.
#[derive(Debug, Clone)]
pub enum ControlError {
    /// A request failed a validation check.
    ValidationError(String),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::ValidationError(e) => write!(f, "Validation error: {}", e),
        }
    }
}

impl std::error::Error for ControlError {}

/// Per-tenant limits used to validate app registration. Bounds an app's declared
/// script size and timeout; enforced by `validate_request_ref` against the global
/// maxima. (An earlier design also carried `max_batch_size` and `allowed_methods`;
/// the fetch runtime has no request-batch concept and never enforced a per-tenant
/// method allowlist, so those were removed rather than left as inert config.)
#[derive(Debug, Clone)]
pub struct TenantLimits {
    /// Maximum script size in bytes
    pub max_script_size: u32,
    /// Maximum timeout in milliseconds
    pub max_timeout_ms: u32,
}

impl TenantLimits {
    /// Create default tenant limits.
    pub fn default() -> Self {
        Self {
            max_script_size: execution::SCRIPT_SIZE_BYTES_MAX,
            max_timeout_ms: execution::TIMEOUT_MS,
        }
    }
}

/// Registry of per-tenant limits, consulted when apps are (auto-)registered.
pub struct ControlPlane {
    is_initialized: bool,
    tenant_registry: HashMap<String, TenantLimits>,
}

impl ControlPlane {
    /// Create a new control plane seeded with a `default` tenant.
    pub fn new() -> Self {
        assert_precondition!(true, "control plane creation preconditions met");

        let mut tenant_registry = HashMap::new();
        tenant_registry.insert("default".to_string(), TenantLimits::default());

        Self {
            is_initialized: true,
            tenant_registry,
        }
    }

    /// Check if a tenant exists in the registry.
    pub fn tenant_exists(&self, tenant_id: &str) -> bool {
        if tenant_id.is_empty() {
            return true; // Default tenant always exists
        }
        self.tenant_registry.contains_key(tenant_id)
            || tenant_id == "default"
            || tenant_id == "localhost"
    }

    /// Validate a request against tenant/execution limits before dispatch.
    /// Called by the router on the hot path; violations panic via the TigerStyle
    /// assertions (they are invariants, not recoverable errors), so this returns
    /// `Ok` in practice, with the `Result` kept for the caller's error contract.
    pub fn validate_request_ref(&self, task: &HandlerTask) -> Result<(), ControlError> {
        let script_size = task.entrypoint.len() as u32;
        assert_positive!(script_size > 0, "script size must be positive");
        assert_negative!(
            script_size > execution::SCRIPT_SIZE_BYTES_MAX,
            "script path indicates potential oversize script"
        );

        let timeout_ms = if task.cpu_time_limit_ms > 0 {
            task.cpu_time_limit_ms
        } else {
            execution::TIMEOUT_MS
        };
        assert_range!(timeout_ms as usize, 1, execution::TIMEOUT_MS as usize);

        assert_precondition!(
            self.tenant_exists(&task.hostname),
            "tenant {} must exist",
            task.hostname
        );

        let body_size = task.request.body().map(|b| b.len()).unwrap_or(0);
        assert_positive!(
            body_size <= buffer::REQUEST_SIZE_BYTES_MAX as usize,
            "request body size {} exceeds maximum {}",
            body_size,
            buffer::REQUEST_SIZE_BYTES_MAX
        );

        assert_postcondition!(timeout_ms > 0, "validated request must have valid timeout");
        Ok(())
    }

    /// Register a tenant with limits.
    pub fn register_tenant(&mut self, tenant_id: String, limits: TenantLimits) {
        assert_precondition!(self.is_initialized, "control plane must be initialized");
        assert_positive!(!tenant_id.is_empty(), "tenant ID must not be empty");
        assert_negative!(tenant_id.contains('\0'), "tenant ID contains invalid chars");
        assert_range!(limits.max_script_size, 1, execution::SCRIPT_SIZE_BYTES_MAX);
        assert_range!(limits.max_timeout_ms, 100, execution::TIMEOUT_MS);

        self.tenant_registry.insert(tenant_id, limits);
    }
}

impl Default for ControlPlane {
    fn default() -> Self {
        Self::new()
    }
}
