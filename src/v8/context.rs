//! V8 context management with proper HandleScope nesting
//!
//! This module provides context creation and scope management following
//! the nested HandleScope pattern (D-04 from PITFALLS.md) to prevent
//! memory leaks during script execution.
//!
//! # HandleScope Nesting Pattern
//!
//! The critical pattern for V8 memory safety:
//! 1. Create HandleScope for the operation (using pin! + init)
//! 2. Create context within that scope
//! 3. Create ContextScope to enter the context (ContextScope::new - NO init needed!)
//! 4. Perform script operations
//! 5. Scopes drop automatically (RAII), freeing temporary handles
//!
//! Reference: PITFALLS.md §2 - Handle Scope Misuse Causing Memory Leaks

/// Create a new V8 context within a HandleScope
///
/// This function creates a context with default global template.
/// In v150+, you must use the pin!() + init() pattern to create scopes.
///
/// # Example
/// ```rust,ignore
/// use nano::v8::{initialize_platform, NanoIsolate};
/// use nano::v8::context::create_context;
///
/// initialize_platform().unwrap();
/// let mut isolate = NanoIsolate::new().unwrap();
/// let scope = std::pin::pin!(v8::HandleScope::new(isolate.isolate()));
/// let scope = scope.init();
/// let context = create_context(&scope);
/// // Context is now ready for script execution
/// ```
pub fn create_context<'s>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
) -> v8::Local<'s, v8::Context> {
    // Create context with default global template
    // v150 API: Context::new takes &PinnedRef
    v8::Context::new(scope, Default::default())
}

#[cfg(test)]
mod tests {
    use crate::v8::{platform, NanoIsolate};

    /// Helper to ensure platform is initialized for tests
    fn init_platform() {
        platform::initialize_platform().expect("Failed to initialize V8 platform");
    }

    /// Test creating context via NanoIsolate helper
    #[test]
    fn test_isolate_create_context() {
        init_platform();

        let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
        let _context = isolate.create_context();

        // Context created successfully
    }
}
