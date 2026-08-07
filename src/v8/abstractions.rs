//! V8 API Abstractions for v147+ Compatibility
//!
//! This module provides helper types and functions to work with the new V8 v147
//! API which uses Pin<ScopeStorage> and PinnedRef patterns instead of direct
//! HandleScope references.
//!
//! # V147 API Pattern
//!
//! The key pattern in v147:
//! ```rust,ignore
//! let scope = std::pin::pin!(v8::HandleScope::new(isolate));
//! let mut scope = scope.init();
//! // scope is now PinnedRef<HandleScope> which can be used with V8 APIs
//! ```
//!
//! Note: Many internal traits are private in v147, so we use concrete types
//! and inline the pin! + init() pattern rather than trying to abstract over
//! scope types with generics.

/// Type alias for the pinned HandleScope (post-init)
///
/// This is the type you get after calling `.init()` on the pinned ScopeStorage.
/// It's a PinnedRef that can be used directly with V8 APIs.
pub type PinnedHandleScope<'a, 'b> = v8::PinnedRef<'a, v8::HandleScope<'b, ()>>;

// NOTE: Functions that create HandleScope and return PinnedRef cannot work
// because pin! creates a stack value that gets dropped at end of function.
// Use the inline pin! + init() pattern instead.

/// Helper to create a context from a pinned scope
///
/// # Example
/// ```rust,ignore
/// let context = create_context_from_scope(&scope);
/// ```
pub fn create_context_from_scope<'s>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
) -> v8::Local<'s, v8::Context> {
    // In v147, Context::new takes &PinnedRef
    v8::Context::new(scope, Default::default())
}

/// Helper to create a Global from a pinned scope
///
/// In v147, Global::new takes &Isolate directly.
///
/// # Example
/// ```rust,ignore
/// let global = create_global_from_scope(&scope, local_value);
/// ```
pub fn create_global_from_scope<'s, T>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
    handle: v8::Local<T>,
) -> v8::Global<T> {
    // Global::new needs &Isolate - we get it from the HandleScope
    // The HandleScope derefs to &Isolate
    v8::Global::new(&**scope, handle)
}

/// Helper to get undefined from a pinned scope
///
/// # Example
/// ```rust,ignore
/// let undefined = undefined_from_scope(&scope);
/// ```
pub fn undefined_from_scope<'s>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
) -> v8::Local<'s, v8::Primitive> {
    v8::undefined(scope)
}

/// Helper to get null from a pinned scope
///
/// # Example
/// ```rust,ignore
/// let null = null_from_scope(&scope);
/// ```
pub fn null_from_scope<'s>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
) -> v8::Local<'s, v8::Primitive> {
    v8::null(scope)
}

/// Helper to create a String from a pinned scope
///
/// # Example
/// ```rust,ignore
/// let str = string_from_scope(&scope, "hello");
/// ```
pub fn string_from_scope<'s>(
    scope: &v8::PinnedRef<'s, v8::HandleScope<'s, ()>>,
    value: &str,
) -> Option<v8::Local<'s, v8::String>> {
    v8::String::new(scope, value)
}

#[cfg(test)]
mod tests {
    use crate::v8::platform;

    #[test]
    fn test_pin_init_pattern() {
        platform::initialize_platform().unwrap();

        let mut isolate = v8::Isolate::new(Default::default());

        // v147 pattern: pin! + init()
        let scope_storage = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let _scope = scope_storage.init();
        // Test that we can create a scope without crashing
    }
}
