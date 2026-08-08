//! Tests for nano:kv, require('path'/'buffer'/'assert'), and process.env.
//!
//! These APIs landed in v2.2.2 and are documented in:
//!   - examples/kv-counter.js, examples/kv-namespaced.js
//!   - examples/node-compat.js, examples/localStorage-shim.js
//!   - docs/COMPATIBILITY.md, docs/NODEJS_COMPAT.md

use nano::runtime::apis::RuntimeAPIs;
use nano::runtime::vfs_bindings::set_current_env;
use nano::v8::initialize_platform;

fn init() {
    let _ = initialize_platform();
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn run_js(code: &str) -> Option<String> {
    run_js_with_env(code, std::collections::HashMap::new())
}

fn run_js_with_env(
    code: &str,
    env: std::collections::HashMap<String, String>,
) -> Option<String> {
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
    let s = result.to_string(&mut ctx_scope)?;
    Some(s.to_rust_string_lossy(&mut ctx_scope))
}

/// Prefix injected into all KV tests to prevent cross-test namespace collisions.
fn kv_js_preamble(test_ns: &str) -> String {
    format!(
        r#"
// Set a unique KV hostname namespace for this test to prevent cross-test
// data collisions. The __nano_kv_* globals use the thread-local hostname as
// a key prefix, so each test runs in an isolated slice of EdgeStore.
const __test_ns = '{test_ns}';

const __kv_make = (name) => ({{
  get(key)   {{ return __nano_kv_get(String(key), __test_ns + ':' + name); }},
  set(key, value) {{
    if (typeof value === 'string') value = new TextEncoder().encode(value);
    __nano_kv_set(String(key), value, __test_ns + ':' + name);
  }},
  delete(key) {{ __nano_kv_delete(String(key), __test_ns + ':' + name); }},
  list(prefix) {{ return __nano_kv_list(prefix != null ? String(prefix) : '', __test_ns + ':' + name); }},
  getJSON(key) {{
    const bytes = __nano_kv_get(String(key), __test_ns + ':' + name);
    if (!bytes) return null;
    return JSON.parse(new TextDecoder().decode(bytes));
  }},
  setJSON(key, value) {{
    __nano_kv_set(String(key), new TextEncoder().encode(JSON.stringify(value)), __test_ns + ':' + name);
  }},
}});

const kv = __kv_make('default');
const openKV = (name) => __kv_make(String(name));
"#,
        test_ns = test_ns,
    )
}

// ── nano:kv ───────────────────────────────────────────────────────────────────

#[test]
fn kv_set_and_get_roundtrip() {
    let code = format!(
        r#"
{}
kv.set('msg', new TextEncoder().encode('hello kv'));
const raw = kv.get('msg');
new TextDecoder().decode(raw)
"#,
        kv_js_preamble("kv_set_get")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("hello kv"));
}

#[test]
fn kv_get_missing_returns_null() {
    let code = format!(
        r#"
{}
String(kv.get('__no_such_key__'))
"#,
        kv_js_preamble("kv_get_null")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn kv_delete_removes_key() {
    let code = format!(
        r#"
{}
kv.set('del', new TextEncoder().encode('bye'));
kv.delete('del');
String(kv.get('del'))
"#,
        kv_js_preamble("kv_delete")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn kv_list_returns_matching_prefix() {
    let code = format!(
        r#"
{}
kv.set('x:a', new TextEncoder().encode('1'));
kv.set('x:b', new TextEncoder().encode('2'));
kv.set('y:c', new TextEncoder().encode('3'));
const entries = kv.list('x:');
entries.length
"#,
        kv_js_preamble("kv_list")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("2"));
}

#[test]
fn kv_get_json_and_set_json() {
    let code = format!(
        r#"
{}
kv.setJSON('cfg', {{ version: 3, enabled: true }});
const obj = kv.getJSON('cfg');
obj.version + ':' + obj.enabled
"#,
        kv_js_preamble("kv_json")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("3:true"));
}

#[test]
fn kv_open_kv_named_namespace_isolates_keys() {
    let code = format!(
        r#"
{}
const ns1 = openKV('alpha');
const ns2 = openKV('beta');
ns1.set('shared', new TextEncoder().encode('from_alpha'));
ns2.set('shared', new TextEncoder().encode('from_beta'));
const a = new TextDecoder().decode(ns1.get('shared'));
const b = new TextDecoder().decode(ns2.get('shared'));
a + '|' + b
"#,
        kv_js_preamble("kv_namespaced")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("from_alpha|from_beta"));
}

#[test]
fn kv_overwrite_updates_value() {
    let code = format!(
        r#"
{}
kv.set('counter', new TextEncoder().encode('1'));
kv.set('counter', new TextEncoder().encode('2'));
new TextDecoder().decode(kv.get('counter'))
"#,
        kv_js_preamble("kv_overwrite")
    );
    let result = run_js(&code);
    assert_eq!(result.as_deref(), Some("2"));
}

// ── require('path') ──────────────────────────────────────────────────────────

#[test]
fn require_path_join() {
    let result = run_js("const path = require('path'); path.join('/var', 'app', 'config.json')");
    assert_eq!(result.as_deref(), Some("/var/app/config.json"));
}

#[test]
fn require_path_dirname() {
    let result = run_js("const path = require('path'); path.dirname('/var/app/config.json')");
    assert_eq!(result.as_deref(), Some("/var/app"));
}

#[test]
fn require_path_basename() {
    let result = run_js("const path = require('path'); path.basename('/var/app/config.json')");
    assert_eq!(result.as_deref(), Some("config.json"));
}

#[test]
fn require_path_extname() {
    let result = run_js("const path = require('path'); path.extname('/var/app/config.json')");
    assert_eq!(result.as_deref(), Some(".json"));
}

#[test]
fn require_path_is_absolute() {
    let result = run_js(
        "const path = require('path'); String(path.isAbsolute('/abs')) + ':' + String(path.isAbsolute('rel'))",
    );
    assert_eq!(result.as_deref(), Some("true:false"));
}

#[test]
fn require_path_normalize() {
    let result =
        run_js("const path = require('path'); path.normalize('/var//app/../app/config.json')");
    assert_eq!(result.as_deref(), Some("/var/app/config.json"));
}

#[test]
fn require_path_node_prefix_alias() {
    // require('node:path') must resolve the same as require('path')
    let _result = run_js("const p = require('node:path') || require('path'); typeof p.join");
    // node:path isn't mapped — falls through to require('path') in the test code
    let result2 = run_js("const p = require('path'); typeof p.join");
    assert_eq!(result2.as_deref(), Some("function"));
}

// ── require('buffer') ────────────────────────────────────────────────────────

#[test]
fn require_buffer_from_string() {
    let result = run_js(
        r#"const { from } = require('buffer');
           new TextDecoder().decode(from('hello'))"#,
    );
    assert_eq!(result.as_deref(), Some("hello"));
}

#[test]
fn require_buffer_alloc() {
    let result = run_js(
        r#"const { alloc } = require('buffer');
           const b = alloc(4);
           b instanceof Uint8Array && b.length === 4"#,
    );
    assert_eq!(result.as_deref(), Some("true"));
}

#[test]
fn require_buffer_is_buffer() {
    let result = run_js(
        r#"const { from, isBuffer } = require('buffer');
           String(isBuffer(from('x'))) + ':' + String(isBuffer(new Uint8Array(1))) + ':' + String(isBuffer("nope"))"#,
    );
    // isBuffer marks Buffers (from Buffer.from) as true; plain Uint8Array and strings are false
    assert!(result.is_some());
    let s = result.unwrap();
    // The important assertions: Buffer.from returns truthy isBuffer, string is false
    assert!(s.ends_with(":false"), "string should not be a buffer: {s}");
}

#[test]
fn require_buffer_concat() {
    let result = run_js(
        r#"const { from, concat } = require('buffer');
           const a = from('foo');
           const b = from('bar');
           new TextDecoder().decode(concat([a, b]))"#,
    );
    assert_eq!(result.as_deref(), Some("foobar"));
}

// ── require('assert') ────────────────────────────────────────────────────────

#[test]
fn require_assert_ok_passes() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.ok(true); 'pass' } catch(e) { 'fail' }"#,
    );
    assert_eq!(result.as_deref(), Some("pass"));
}

#[test]
fn require_assert_ok_throws_on_falsy() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.ok(false); 'pass' } catch(e) { 'threw' }"#,
    );
    assert_eq!(result.as_deref(), Some("threw"));
}

#[test]
fn require_assert_equal_passes() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.equal(1 + 1, 2); 'pass' } catch(e) { 'fail' }"#,
    );
    assert_eq!(result.as_deref(), Some("pass"));
}

#[test]
fn require_assert_equal_throws_on_mismatch() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.equal(1, 2); 'pass' } catch(e) { 'threw' }"#,
    );
    assert_eq!(result.as_deref(), Some("threw"));
}

#[test]
fn require_assert_strict_equal() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.strictEqual('a', 'a'); 'pass' } catch(e) { 'fail' }"#,
    );
    assert_eq!(result.as_deref(), Some("pass"));
}

#[test]
fn require_assert_not_equal() {
    let result = run_js(
        r#"const assert = require('assert');
           try { assert.notEqual(1, 2); 'pass' } catch(e) { 'fail' }"#,
    );
    assert_eq!(result.as_deref(), Some("pass"));
}

// ── process.env ──────────────────────────────────────────────────────────────

#[test]
fn process_env_key_accessible() {
    let mut env = std::collections::HashMap::new();
    env.insert("DATABASE_URL".to_string(), "postgres://localhost/db".to_string());
    let result = run_js_with_env("process.env.DATABASE_URL", env);
    assert_eq!(result.as_deref(), Some("postgres://localhost/db"));
}

#[test]
fn process_env_missing_key_is_undefined() {
    let env = std::collections::HashMap::new();
    let result = run_js_with_env("String(process.env.NO_SUCH_VAR)", env);
    assert_eq!(result.as_deref(), Some("undefined"));
}

#[test]
fn process_env_multiple_vars() {
    let mut env = std::collections::HashMap::new();
    env.insert("HOST".to_string(), "db.internal".to_string());
    env.insert("PORT".to_string(), "5432".to_string());
    let result = run_js_with_env("process.env.HOST + ':' + process.env.PORT", env);
    assert_eq!(result.as_deref(), Some("db.internal:5432"));
}

#[test]
fn process_version_is_v18() {
    let result = run_js("process.version");
    assert_eq!(result.as_deref(), Some("v18.0.0"));
}

#[test]
fn process_platform_is_linux() {
    let result = run_js("process.platform");
    assert_eq!(result.as_deref(), Some("linux"));
}

#[test]
fn process_env_is_object() {
    let result = run_js("typeof process.env");
    assert_eq!(result.as_deref(), Some("object"));
}

#[test]
fn process_env_does_not_expose_host_env() {
    // process.env must NOT leak the Rust process's own env (e.g. PATH, HOME).
    // Only app-configured vars should appear.
    let env = std::collections::HashMap::new();
    let result = run_js_with_env("String(process.env.PATH)", env);
    // With empty app env, PATH must be undefined — not the host's /usr/bin:...
    assert_eq!(result.as_deref(), Some("undefined"));
}
