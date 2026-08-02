//! Tests for Nano.env injection into V8 context.
//!
//! Verifies that env vars set via `set_current_env` are:
//! - accessible as `Nano.env.KEY` in JavaScript
//! - read-only (frozen object — write attempts are silently ignored in non-strict mode)
//! - not present when no env vars are configured

use nano::runtime::apis::RuntimeAPIs;
use nano::runtime::vfs_bindings::set_current_env;
use nano::v8::initialize_platform;

fn init() {
    let _ = initialize_platform();
}

fn run_js(code: &str, env: std::collections::HashMap<String, String>) -> Option<String> {
    init();

    set_current_env(env);

    let mut isolate = v8::Isolate::new(Default::default());
    let handle_scope = v8::HandleScope::new(&mut isolate);
    let pinned = std::pin::pin!(handle_scope);
    let mut scope = pinned.init();

    let context = v8::Context::new(&mut scope, Default::default());
    RuntimeAPIs::bind_all(&mut scope, context);

    let mut ctx_scope = v8::ContextScope::new(&mut scope, context);
    let code_v8 = v8::String::new(&mut ctx_scope, code)?;
    let script = v8::Script::compile(&mut ctx_scope, code_v8, None)?;
    let result = script.run(&mut ctx_scope)?;
    let result_str = result.to_string(&mut ctx_scope)?;
    Some(result_str.to_rust_string_lossy(&mut ctx_scope))
}

#[test]
fn env_key_accessible() {
    let mut env = std::collections::HashMap::new();
    env.insert("API_KEY".to_string(), "secret-value".to_string());

    let result = run_js("Nano.env.API_KEY", env);
    assert_eq!(result.as_deref(), Some("secret-value"));
}

#[test]
fn env_missing_key_is_undefined() {
    let env = std::collections::HashMap::new();
    let result = run_js("String(Nano.env.MISSING_KEY)", env);
    assert_eq!(result.as_deref(), Some("undefined"));
}

#[test]
fn env_multiple_keys() {
    let mut env = std::collections::HashMap::new();
    env.insert("HOST".to_string(), "db.internal".to_string());
    env.insert("PORT".to_string(), "5432".to_string());

    let result = run_js("Nano.env.HOST + ':' + Nano.env.PORT", env);
    assert_eq!(result.as_deref(), Some("db.internal:5432"));
}

#[test]
fn env_is_frozen_write_ignored() {
    let mut env = std::collections::HashMap::new();
    env.insert("KEY".to_string(), "original".to_string());

    // Non-strict mode: assignment to frozen property is silently ignored.
    let result = run_js(
        "(function() { Nano.env.KEY = 'modified'; return Nano.env.KEY; })()",
        env,
    );
    assert_eq!(result.as_deref(), Some("original"));
}

#[test]
fn env_is_frozen_new_key_ignored() {
    let env = std::collections::HashMap::new();
    // Adding new key to frozen object is silently ignored.
    let result = run_js(
        "(function() { Nano.env.NEW = 'value'; return String(Nano.env.NEW); })()",
        env,
    );
    assert_eq!(result.as_deref(), Some("undefined"));
}

#[test]
fn empty_env_does_not_crash() {
    let env = std::collections::HashMap::new();
    let result = run_js("typeof Nano.env", env);
    assert_eq!(result.as_deref(), Some("object"));
}
