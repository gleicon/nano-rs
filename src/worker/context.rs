//! V8 Context Manager for Isolate Reuse
//!
//! This module provides fast request handling by:
//! 1. Creating an isolate and context once
//! 2. Executing the handler script (handler lives in global scope)
//! 3. For each request: call the handler from global scope
//!
//! The key: We NEVER drop the context, so the handler remains valid.

use crate::v8::NanoIsolate;
use crate::worker::eviction::IsolateId;
use anyhow::Result;

/// Manages V8 context lifecycle for a worker thread.
pub struct ContextManager {
    isolate: NanoIsolate,
    initialized_entrypoint: Option<String>,
    isolate_id: IsolateId,
    request_count: u64,
}

impl ContextManager {
    /// Create a new ContextManager.
    pub fn new() -> Result<Self> {
        let isolate_id = IsolateId::generate();
        let nano_isolate = NanoIsolate::new()?;
        tracing::debug!("Created isolate {} from scratch", isolate_id);

        Ok(Self {
            isolate: nano_isolate,
            initialized_entrypoint: None,
            isolate_id,
            request_count: 0,
        })
    }

    /// Check if handler has been initialized for this entrypoint.
    pub fn is_handler_initialized(&self, entrypoint: &str) -> bool {
        self.initialized_entrypoint.as_deref() == Some(entrypoint)
    }

    /// Initialize the handler for an entrypoint.
    ///
    /// This executes the script, defining the handler in the isolate's global scope.
    /// The handler will remain valid for all subsequent requests as long as the
    /// isolate and context are kept alive.
    pub fn initialize_handler(&mut self, entrypoint: &str) -> Result<()> {
        use crate::data_plane::read_code_cached;
        use crate::runtime::apis::RuntimeAPIs;
        use crate::v8::module::{is_esm_module, transform_module_code};

        if self.initialized_entrypoint.as_deref() == Some(entrypoint) {
            tracing::debug!("Handler already initialized for entrypoint: {}", entrypoint);
            return Ok(());
        }

        tracing::info!(
            "Initializing handler for entrypoint: {} (isolate: {})",
            entrypoint,
            self.isolate_id
        );

        // Read and transform code
        let code = read_code_cached(entrypoint)?;
        let transformed_code = if is_esm_module(&code) {
            transform_module_code(&code)
        } else {
            code.to_string()
        };

        // Execute script - handler is now in global scope
        let scope_storage = std::pin::pin!(v8::HandleScope::new(self.isolate.isolate()));
        let mut handle_scope = scope_storage.init();
        let context = v8::Context::new(&handle_scope, Default::default());
        let mut context_scope = v8::ContextScope::new(&mut handle_scope, context);

        // Clear stale CURRENT_ENV before binding so Nano.env does not leak secrets
        // from a prior app that ran on this thread.
        crate::runtime::vfs_bindings::set_current_env(std::collections::HashMap::new());
        // Bind APIs
        RuntimeAPIs::bind_all(&mut context_scope, context);

        // Compile and execute script
        let code_str = v8::String::new(&mut context_scope, &transformed_code)
            .ok_or_else(|| anyhow::anyhow!("Failed to create code string"))?;
        let script = v8::Script::compile(&context_scope, code_str, None)
            .ok_or_else(|| anyhow::anyhow!("Script compilation failed"))?;

        let _script_result = script
            .run(&context_scope)
            .ok_or_else(|| anyhow::anyhow!("Script execution failed"))?;

        // Verify handler exists
        let global = context.global(&mut context_scope);
        let handler_key = v8::String::new(&mut context_scope, "__nano_user_fetch")
            .ok_or_else(|| anyhow::anyhow!("Failed to create handler key"))?;

        let handler_exists = match global.get(&mut context_scope, handler_key.into()) {
            Some(val) if val.is_function() => true,
            _ => {
                let fetch_key = v8::String::new(&mut context_scope, "fetch")
                    .ok_or_else(|| anyhow::anyhow!("Failed to create fetch key"))?;
                matches!(global.get(&mut context_scope, fetch_key.into()), Some(val) if val.is_function())
            }
        };

        if !handler_exists {
            return Err(anyhow::anyhow!(
                "No handler function found after script execution. Entrypoint must export a 'fetch' function."
            ));
        }

        self.initialized_entrypoint = Some(entrypoint.to_string());

        tracing::info!(
            "Handler initialized for entrypoint: {} (isolate: {})",
            entrypoint,
            self.isolate_id
        );

        // Scopes are dropped here, BUT the isolate keeps the context alive internally
        Ok(())
    }

    /// Get a mutable reference to the isolate.
    pub fn isolate_mut(&mut self) -> &mut NanoIsolate {
        &mut self.isolate
    }

    /// Get a reference to the VFS.
    pub fn vfs(&self) -> Option<&crate::vfs::IsolateVfs> {
        Some(self.isolate.vfs())
    }

    /// Get the unique identifier for this isolate instance.
    pub fn isolate_id(&self) -> &IsolateId {
        &self.isolate_id
    }

    /// Get the request count.
    pub fn request_count(&self) -> u64 {
        self.request_count
    }

    /// Increment request count.
    pub fn increment_request_count(&mut self) {
        self.request_count += 1;
    }

    // Backward compatibility
    pub fn create_initial_context(&mut self) -> Result<()> {
        Ok(())
    }
    pub fn reset_context(&mut self) -> Result<std::time::Duration> {
        Ok(std::time::Duration::from_millis(0))
    }
    pub fn average_reset_time_ms(&self) -> f64 {
        0.0
    }
    pub fn reset_count(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v8::platform;

    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    #[test]
    fn test_context_manager_creation() {
        init_platform();

        let manager = ContextManager::new().expect("Failed to create manager");
        assert_eq!(manager.request_count(), 0);
    }
}
