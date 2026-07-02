//! Console API bindings for JavaScript runtime
//!
//! Provides console.log/warn/error with structured logging via tracing,
//! and the security hardening callback that blocks the Function constructor.

/// Format console arguments into a single string
pub(crate) fn format_console_args(scope: &mut v8::PinnedRef<v8::HandleScope>, args: v8::FunctionCallbackArguments) -> String {
    let mut parts = Vec::new();
    for i in 0..args.length() {
        let arg = args.get(i);
        if let Some(s) = arg.to_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    parts.join(" ")
}

/// V8 callback that blocks dynamic code generation via the Function constructor
/// Throws TypeError unconditionally. Replaces globalThis.Function in hardened contexts.
pub(crate) fn function_constructor_blocked(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    _args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let msg = v8::String::new(scope, "Function constructor is not allowed in this context").unwrap();
    let err = v8::Exception::type_error(scope, msg);
    scope.throw_exception(err);
    retval.set_undefined();
}

/// V8 callback for console.log
fn console_log_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::info!(target: "js_console", "{}", message);
}

/// V8 callback for console.warn
fn console_warn_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::warn!(target: "js_console", "{}", message);
}

/// V8 callback for console.error
fn console_error_callback(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let message = format_console_args(scope, args);
    tracing::error!(target: "js_console", "{}", message);
}

/// Bind console API (log/warn/error) to global scope
pub(crate) fn bind_console(
    scope: &mut v8::PinnedRef<v8::HandleScope<'_, ()>>,
    context: v8::Local<v8::Context>,
) {
    let global = context.global(scope);

    // Enter context scope for operations that need HandleScope<Context>
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    let console = v8::Object::new(&mut &mut ctx_scope);

    // Bind log method
    if let Some(log_fn) = v8::Function::new(&mut ctx_scope, console_log_callback) {
        let key = v8::String::new(&mut ctx_scope, "log").unwrap();
        console.set(&mut ctx_scope, key.into(), log_fn.into());
    }

    // Bind warn method
    if let Some(warn_fn) = v8::Function::new(&mut ctx_scope, console_warn_callback) {
        let key = v8::String::new(&mut ctx_scope, "warn").unwrap();
        console.set(&mut ctx_scope, key.into(), warn_fn.into());
    }

    // Bind error method
    if let Some(error_fn) = v8::Function::new(&mut ctx_scope, console_error_callback) {
        let key = v8::String::new(&mut ctx_scope, "error").unwrap();
        console.set(&mut ctx_scope, key.into(), error_fn.into());
    }

    // Attach console to global
    let console_key = v8::String::new(&mut ctx_scope, "console").unwrap();
    global.set(&mut ctx_scope, console_key.into(), console.into());
}
