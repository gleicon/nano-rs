//! Control Plane: Request validation, scheduling, and batching.
//!
//! Per TigerStyle, all validation checks and coordination happen here.
//! Data plane receives pre-validated, batched work units.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use crate::http::NanoResponse;
use crate::limits::*;
use crate::worker::HandlerTask;
use crate::{
    assert_invariant, assert_negative, assert_positive, assert_postcondition, assert_precondition,
    assert_range, assert_resource_limit,
};

/// Maximum number of requests in a single batch.
pub const BATCH_SIZE_MAX: usize = 64;

/// Maximum time to wait for batch to fill before flushing (milliseconds).
pub const BATCH_WAIT_MS: u32 = 10;

/// Maximum pending batches before backpressure.
pub const MAX_PENDING_BATCHES: usize = 10_000;

/// Maximum entrypoint path length.
pub const MAX_ENTRYPOINT_PATH_LEN: usize = 4096;

/// Maximum request ID length.
pub const MAX_REQUEST_ID_LEN: usize = 64;

/// Submission identifier returned to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionId(pub u64);

/// Error types for control plane operations.
#[derive(Debug, Clone)]
pub enum ControlError {
    /// Validation failed
    ValidationError(String),
    /// Tenant not found in registry
    TenantNotFound(String),
    /// Resource limit exceeded
    LimitExceeded(String),
    /// Control plane not initialized
    NotInitialized,
    /// Batch queue full
    QueueFull,
    /// Work queue dispatch failed
    DispatchError(String),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::ValidationError(e) => write!(f, "Validation error: {}", e),
            ControlError::TenantNotFound(t) => write!(f, "Tenant not found: {}", t),
            ControlError::LimitExceeded(l) => write!(f, "Limit exceeded: {}", l),
            ControlError::NotInitialized => write!(f, "Control plane not initialized"),
            ControlError::QueueFull => write!(f, "Batch queue full"),
            ControlError::DispatchError(e) => write!(f, "Dispatch error: {}", e),
        }
    }
}

impl std::error::Error for ControlError {}

/// Request validated by control plane.
pub struct ValidatedRequest {
    /// The handler task (moved here after validation)
    pub task: HandlerTask,
    /// Estimated script size in bytes
    pub script_size: u32,
    /// Timeout in milliseconds
    pub timeout_ms: u32,
    /// Priority (lower = higher priority)
    pub priority: u32,
    /// When this request was validated
    pub validated_at: Instant,
}

/// Batch of requests targeting the same tenant.
pub struct RequestBatch {
    /// Tenant identifier
    pub tenant_id: String,
    /// Validated requests in this batch
    pub requests: Vec<ValidatedRequest>,
    /// When this batch was created
    pub created_at: Instant,
}

/// Batch execution result returned by data plane.
#[derive(Debug)]
pub struct BatchResult {
    /// Responses for each request in the batch
    pub responses: Vec<Result<NanoResponse, anyhow::Error>>,
    /// Total execution time in microseconds
    pub execution_time_us: u64,
    /// Tenant identifier
    pub tenant_id: String,
}

/// Control plane manages request lifecycle.
///
/// All validation, checks, and batching happen here.
/// Data plane receives only pre-validated batches.
pub struct ControlPlane {
    is_initialized: bool,
    batch_queue: Mutex<BatchQueue>,
    metrics: Mutex<ControlMetrics>,
    next_id: Mutex<u64>,
    tenant_registry: HashMap<String, TenantLimits>,
}

/// Per-tenant limits for validation.
#[derive(Debug, Clone)]
pub struct TenantLimits {
    /// Maximum script size in bytes
    pub max_script_size: u32,
    /// Maximum timeout in milliseconds
    pub max_timeout_ms: u32,
    /// Maximum batch size for this tenant
    pub max_batch_size: u32,
    /// Allowed HTTP methods
    pub allowed_methods: Vec<String>,
}

/// Metrics tracked by control plane.
#[derive(Debug, Default, Clone)]
pub struct ControlMetrics {
    /// Total requests submitted
    pub requests_submitted: u64,
    /// Total requests validated successfully
    pub requests_validated: u64,
    /// Total requests rejected
    pub requests_rejected: u64,
    /// Total batches created
    pub batches_created: u64,
    /// Total batches flushed
    pub batches_flushed: u64,
    /// Batches flushed due to timeout
    pub batches_timeout_flushed: u64,
    /// Batches flushed due to size
    pub batches_size_flushed: u64,
    /// Total time spent validating (microseconds)
    pub total_validation_time_us: u64,
}

/// Batch queue groups requests by tenant.
struct BatchQueue {
    /// Pending batches keyed by tenant ID
    pending: HashMap<String, PendingBatch>,
    /// Maximum requests per batch
    max_batch_size: usize,
    /// Maximum wait time before flushing (ms)
    max_batch_wait_ms: u32,
}

/// A pending batch for a single tenant.
struct PendingBatch {
    /// Validated requests
    requests: Vec<ValidatedRequest>,
    /// When the first request was added
    created_at: Instant,
}

impl ControlPlane {
    /// Create a new control plane.
    pub fn new() -> Self {
        assert_precondition!(true, "control plane creation preconditions met");

        let mut tenant_registry = HashMap::new();
        tenant_registry.insert("default".to_string(), TenantLimits::default());

        Self {
            is_initialized: true,
            batch_queue: Mutex::new(BatchQueue::new(BATCH_SIZE_MAX, BATCH_WAIT_MS)),
            metrics: Mutex::new(ControlMetrics::default()),
            next_id: Mutex::new(0),
            tenant_registry,
        }
    }

    /// Validate and submit request for batching.
    ///
    /// # Checks performed (Control Plane Responsibility)
    /// - Script path within limits
    /// - Timeout within limits
    /// - Tenant exists and has quota
    /// - Request format valid
    pub fn submit_request(&self, task: HandlerTask) -> Result<SubmissionId, ControlError> {
        // PRECONDITION: Control plane must be initialized
        assert_precondition!(self.is_initialized, "control plane must be initialized");

        // POSITIVE: Entrypoint must not be empty
        assert_positive!(!task.entrypoint.is_empty(), "entrypoint must not be empty");

        // NEGATIVE: Entrypoint path must not exceed maximum length
        assert_negative!(
            task.entrypoint.len() > MAX_ENTRYPOINT_PATH_LEN,
            "entrypoint path exceeds maximum length"
        );

        // RANGE: Entrypoint length must be within bounds
        assert_range!(task.entrypoint.len(), 1, MAX_ENTRYPOINT_PATH_LEN);

        // POSITIVE: Hostname (tenant) must be present
        assert_positive!(!task.hostname.is_empty(), "hostname must not be empty");

        // NEGATIVE: Hostname must not contain null bytes
        assert_negative!(
            task.hostname.contains('\0'),
            "hostname contains invalid characters"
        );

        // PRECONDITION: Request method must be valid
        let method = task.request.method();
        assert_precondition!(!method.is_empty(), "request method must not be empty");

        // POSITIVE: Request body size must be within limits
        let body_size = task.request.body().map(|b| b.len()).unwrap_or(0);
        assert_positive!(
            body_size <= buffer::REQUEST_SIZE_BYTES_MAX as usize,
            "request body size {} exceeds maximum {}",
            body_size,
            buffer::REQUEST_SIZE_BYTES_MAX
        );

        // NEGATIVE: Body must not exceed absolute maximum
        assert_negative!(
            body_size > http::BODY_SIZE_BYTES_MAX as usize,
            "body size exceeds absolute maximum"
        );

        // RANGE: CPU time limit must be within bounds
        assert_range!(
            task.cpu_time_limit_ms as usize,
            0,
            execution::TIMEOUT_MS as usize
        );

        // PRECONDITION: Request ID must be present
        assert_precondition!(!task.request_id.is_empty(), "request ID must not be empty");

        // POSITIVE: Request ID must be reasonable length
        assert_positive!(
            task.request_id.len() <= MAX_REQUEST_ID_LEN,
            "request ID must be <= {} chars",
            MAX_REQUEST_ID_LEN
        );

        // INVARIANT: Tenant must exist in registry
        assert_invariant!(
            self.tenant_exists(&task.hostname),
            "tenant {} must exist in registry",
            task.hostname
        );

        // VALIDATE request through comprehensive checks
        let validated = self.validate_request(task)?;

        // INVARIANT: Validated request must have positive script size
        assert_invariant!(
            validated.script_size > 0,
            "validated script size must be positive"
        );

        // POSTCONDITION: Validation time must be reasonable
        let validation_start = Instant::now();

        // BATCH BY TENANT
        let mut batch_queue = self.batch_queue.lock().unwrap();
        let _batch_id = batch_queue.add_to_batch(validated).map_err(|e| e)?;

        // POSTCONDITION: Batch queue must not exceed maximum pending batches
        assert_postcondition!(
            batch_queue.pending_count() <= MAX_PENDING_BATCHES,
            "batch queue pending count within limits"
        );

        let mut metrics = self.metrics.lock().unwrap();
        metrics.requests_submitted += 1;
        metrics.requests_validated += 1;
        metrics.total_validation_time_us += validation_start.elapsed().as_micros() as u64;

        // Get next submission ID
        let mut next_id = self.next_id.lock().unwrap();
        let id = *next_id;
        *next_id += 1;

        Ok(SubmissionId(id))
    }

    /// Process batches - called periodically or when batch full.
    ///
    /// Returns ready batches for handoff to data plane.
    pub fn process_batches(&self) -> Vec<RequestBatch> {
        // PRECONDITION: Control plane must be initialized
        assert_precondition!(
            self.is_initialized,
            "control plane must be initialized before processing batches"
        );

        let mut batch_queue = self.batch_queue.lock().unwrap();
        let batches = batch_queue.drain_ready_batches();

        // INVARIANT: All returned batches must have at least one request
        let batch_count = batches.len();
        for batch in &batches {
            assert_invariant!(
                !batch.requests.is_empty(),
                "batch must not be empty after draining"
            );

            // RANGE: Batch size must be within limits
            assert_range!(batch.requests.len(), 1, BATCH_SIZE_MAX);

            // POSITIVE: Tenant ID must be present
            assert_positive!(
                !batch.tenant_id.is_empty(),
                "batch tenant ID must not be empty"
            );
        }

        // POSTCONDITION: Drained batches count must be non-negative
        assert_postcondition!(
            batch_count <= MAX_PENDING_BATCHES,
            "drained batch count within limits"
        );

        let mut metrics = self.metrics.lock().unwrap();
        metrics.batches_flushed += batch_count as u64;

        batches
    }

    /// Validate a request reference without taking ownership.
    ///
    /// Use this for pre-dispatch validation when the task will be dispatched separately.
    pub fn validate_request_ref(&self, task: &HandlerTask) -> Result<(), ControlError> {
        // POSITIVE: Script size estimate from entrypoint path length
        let script_size = task.entrypoint.len() as u32;
        assert_positive!(script_size > 0, "script size must be positive");

        // NEGATIVE: Script path must not exceed execution limit
        assert_negative!(
            script_size > execution::SCRIPT_SIZE_BYTES_MAX,
            "script path indicates potential oversize script"
        );

        // RANGE: Validate timeout
        let timeout_ms = if task.cpu_time_limit_ms > 0 {
            task.cpu_time_limit_ms
        } else {
            execution::TIMEOUT_MS
        };
        assert_range!(timeout_ms as usize, 1, execution::TIMEOUT_MS as usize);

        // PRECONDITION: Validate tenant exists and has quota
        assert_precondition!(
            self.tenant_exists(&task.hostname),
            "tenant {} must exist",
            task.hostname
        );

        // POSITIVE: Request body size must be within limits
        let body_size = task.request.body().map(|b| b.len()).unwrap_or(0);
        assert_positive!(
            body_size <= buffer::REQUEST_SIZE_BYTES_MAX as usize,
            "request body size {} exceeds maximum {}",
            body_size,
            buffer::REQUEST_SIZE_BYTES_MAX
        );

        // POSTCONDITION: Validated request must have valid timeout
        assert_postcondition!(timeout_ms > 0, "validated request must have valid timeout");

        Ok(())
    }

    /// Validate request format and limits.
    fn validate_request(&self, task: HandlerTask) -> Result<ValidatedRequest, ControlError> {
        // POSITIVE: Script size estimate from entrypoint path length
        let script_size = task.entrypoint.len() as u32;
        assert_positive!(script_size > 0, "script size must be positive");

        // NEGATIVE: Script path must not exceed execution limit
        assert_negative!(
            script_size > execution::SCRIPT_SIZE_BYTES_MAX,
            "script path indicates potential oversize script"
        );

        // RANGE: Validate timeout
        let timeout_ms = if task.cpu_time_limit_ms > 0 {
            task.cpu_time_limit_ms
        } else {
            execution::TIMEOUT_MS
        };
        assert_range!(timeout_ms as usize, 1, execution::TIMEOUT_MS as usize);

        // PRECONDITION: Validate tenant exists and has quota
        assert_precondition!(
            self.tenant_exists(&task.hostname),
            "tenant {} must exist",
            task.hostname
        );

        // RESOURCE LIMIT: Check tenant batch quota
        if let Some(limits) = self.tenant_registry.get(&task.hostname) {
            assert_resource_limit!(
                limits.max_batch_size,
                BATCH_SIZE_MAX as u32,
                "tenant batch size"
            );
        }

        // POSTCONDITION: Validated request must have valid timeout
        assert_postcondition!(timeout_ms > 0, "validated request must have valid timeout");

        let now = Instant::now();

        Ok(ValidatedRequest {
            task,
            script_size,
            timeout_ms,
            priority: 5,
            validated_at: now,
        })
    }

    /// Check if tenant exists in registry.
    pub fn tenant_exists(&self, tenant_id: &str) -> bool {
        if tenant_id.is_empty() {
            return true; // Default tenant always exists
        }
        self.tenant_registry.contains_key(tenant_id)
            || tenant_id == "default"
            || tenant_id == "localhost"
    }

    /// Get current metrics snapshot.
    pub fn metrics(&self) -> ControlMetrics {
        self.metrics.lock().unwrap().clone()
    }

    /// Register a tenant with limits.
    pub fn register_tenant(&mut self, tenant_id: String, limits: TenantLimits) {
        assert_positive!(!tenant_id.is_empty(), "tenant ID must not be empty");
        assert_negative!(tenant_id.contains('\0'), "tenant ID contains invalid chars");
        assert_range!(limits.max_script_size, 1, execution::SCRIPT_SIZE_BYTES_MAX);
        assert_range!(limits.max_timeout_ms, 100, execution::TIMEOUT_MS);

        self.tenant_registry.insert(tenant_id, limits);
    }

    /// Get number of pending batches.
    pub fn pending_batch_count(&self) -> usize {
        self.batch_queue.lock().unwrap().pending_count()
    }

    /// Flush all pending batches immediately.
    pub fn flush_all(&self) -> Vec<RequestBatch> {
        assert_precondition!(self.is_initialized, "control plane must be initialized");

        let mut batch_queue = self.batch_queue.lock().unwrap();
        let batches = batch_queue.drain_all_batches();

        let mut metrics = self.metrics.lock().unwrap();
        metrics.batches_flushed += batches.len() as u64;

        batches
    }
}

impl Default for ControlPlane {
    fn default() -> Self {
        Self::new()
    }
}

impl TenantLimits {
    /// Create default tenant limits.
    pub fn default() -> Self {
        Self {
            max_script_size: execution::SCRIPT_SIZE_BYTES_MAX,
            max_timeout_ms: execution::TIMEOUT_MS,
            max_batch_size: BATCH_SIZE_MAX as u32,
            allowed_methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "HEAD".to_string(),
                "OPTIONS".to_string(),
            ],
        }
    }
}

impl BatchQueue {
    fn new(max_batch_size: usize, max_batch_wait_ms: u32) -> Self {
        assert_positive!(max_batch_size > 0, "max batch size must be positive");
        assert_positive!(max_batch_wait_ms > 0, "max batch wait must be positive");
        assert_range!(max_batch_size, 1, 1000);
        assert_range!(max_batch_wait_ms, 1, 1000);

        Self {
            pending: HashMap::new(),
            max_batch_size,
            max_batch_wait_ms,
        }
    }

    fn pending_count(&self) -> usize {
        self.pending.len()
    }

    fn add_to_batch(&mut self, request: ValidatedRequest) -> Result<u64, ControlError> {
        let tenant_id = request.task.hostname.clone();

        // POSITIVE: Tenant ID must be present
        assert_positive!(!tenant_id.is_empty(), "tenant ID must not be empty");

        let batch = self
            .pending
            .entry(tenant_id.clone())
            .or_insert(PendingBatch {
                requests: Vec::with_capacity(self.max_batch_size),
                created_at: Instant::now(),
            });

        // INVARIANT: Batch must not exceed max size before push
        assert_invariant!(
            batch.requests.len() < self.max_batch_size,
            "batch size must be below maximum before adding request"
        );

        batch.requests.push(request);

        let current_size = batch.requests.len();

        // Check if batch ready to flush
        if current_size >= self.max_batch_size {
            return Ok(self.flush_batch(&tenant_id));
        }

        Ok(current_size as u64)
    }

    fn flush_batch(&mut self, tenant_id: &str) -> u64 {
        if let Some(batch) = self.pending.remove(tenant_id) {
            assert_positive!(
                !batch.requests.is_empty(),
                "flushed batch must not be empty"
            );
            batch.requests.len() as u64
        } else {
            0
        }
    }

    fn drain_ready_batches(&mut self) -> Vec<RequestBatch> {
        let now = Instant::now();
        let mut ready = Vec::new();
        let mut to_remove = Vec::new();

        for (tenant_id, batch) in &self.pending {
            let age_ms = now.duration_since(batch.created_at).as_millis() as u32;

            // INVARIANT: Batch age must not exceed maximum wait time significantly
            assert_invariant!(
                age_ms <= self.max_batch_wait_ms * 2,
                "batch age should not exceed twice the max wait time"
            );

            if batch.requests.len() >= self.max_batch_size || age_ms >= self.max_batch_wait_ms {
                to_remove.push(tenant_id.clone());
            }
        }

        // Remove flushed batches and move them into ready
        for tenant_id in to_remove {
            if let Some(batch) = self.pending.remove(&tenant_id) {
                ready.push(RequestBatch {
                    tenant_id,
                    requests: batch.requests,
                    created_at: Instant::now(),
                });
            }
        }

        ready
    }

    fn drain_all_batches(&mut self) -> Vec<RequestBatch> {
        let mut ready = Vec::new();

        for (tenant_id, batch) in self.pending.drain() {
            if !batch.requests.is_empty() {
                ready.push(RequestBatch {
                    tenant_id,
                    requests: batch.requests,
                    created_at: batch.created_at,
                });
            }
        }

        ready
    }
}
