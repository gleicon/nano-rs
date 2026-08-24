//! Tests for the nano:gas Google Apps Script compatibility shim.
//!
//! Covers:
//! - GAS_SHIM_PREFIX compiles without V8 syntax errors
//! - GAS_SHIM_SUFFIX installs `__nano_user_fetch` as a function
//! - PropertiesService round-trip (synchronous via __nano_kv_*)
//! - CacheService put/get (synchronous)
//! - Utilities: base64, UUID, parseCsv
//! - Logger: doesn't throw
//! - Stubs (GmailApp etc.) throw descriptive errors
//! - get_nano_module_code("nano:gas") is registered
//! - is_esm_module() correctly identifies GAS scripts as classic

use nano::runtime::apis::RuntimeAPIs;
use nano::runtime::kv::get_nano_module_code;
use nano::runtime::vfs_bindings::set_current_env;
use nano::v8::{initialize_platform, is_esm_module};

fn init() {
    let _ = initialize_platform();
}

/// Run synchronous JS with all RuntimeAPIs bound (includes __nano_kv_*, fetch, Nano.env, etc.)
fn run_js(code: &str) -> Option<String> {
    run_js_with_env(code, std::collections::HashMap::new())
}

fn run_js_with_env(code: &str, env: std::collections::HashMap<String, String>) -> Option<String> {
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

/// Inject the full GAS shim (prefix + optional user code + suffix) and run a JS assertion.
fn run_with_shim(user_code: &str, assertion: &str) -> Option<String> {
    let full = format!(
        "{}\n{}\n{}\n{}",
        nano::runtime::gas::GAS_SHIM_PREFIX,
        user_code,
        nano::runtime::gas::GAS_SHIM_SUFFIX,
        assertion
    );
    run_js(&full)
}

// ── Module registration ───────────────────────────────────────────────────────

#[test]
fn test_gas_module_registered() {
    let code = get_nano_module_code("nano:gas");
    assert!(
        code.is_some(),
        "nano:gas must be registered in get_nano_module_code"
    );
    assert!(
        code.unwrap().contains("SpreadsheetApp"),
        "nano:gas module must contain SpreadsheetApp"
    );
}

#[test]
fn test_gas_module_code_has_dispatch_export() {
    let code = get_nano_module_code("nano:gas").unwrap();
    assert!(
        code.contains("export async function dispatch"),
        "nano:gas module must export dispatch()"
    );
}

#[test]
fn test_nano_kv_module_still_registered() {
    assert!(
        get_nano_module_code("nano:kv").is_some(),
        "nano:kv registration must not be broken by nano:gas addition"
    );
}

// ── GAS shim constants ────────────────────────────────────────────────────────

#[test]
fn test_gas_constants_non_empty() {
    assert!(!nano::runtime::gas::GAS_SHIM_PREFIX.is_empty());
    assert!(!nano::runtime::gas::GAS_SHIM_SUFFIX.is_empty());
    assert!(!nano::runtime::gas::GAS_MODULE_CODE.is_empty());
}

#[test]
fn test_gas_prefix_contains_required_globals() {
    let prefix = nano::runtime::gas::GAS_SHIM_PREFIX;
    assert!(
        prefix.contains("SpreadsheetApp"),
        "prefix must define SpreadsheetApp"
    );
    assert!(prefix.contains("Logger"), "prefix must define Logger");
    assert!(
        prefix.contains("PropertiesService"),
        "prefix must define PropertiesService"
    );
    assert!(
        prefix.contains("CacheService"),
        "prefix must define CacheService"
    );
    assert!(prefix.contains("Utilities"), "prefix must define Utilities");
    assert!(
        prefix.contains("DocumentApp"),
        "prefix must define DocumentApp"
    );
    assert!(prefix.contains("DriveApp"), "prefix must define DriveApp");
    assert!(
        prefix.contains("UrlFetchApp"),
        "prefix must define UrlFetchApp"
    );
    assert!(
        prefix.contains("__gasGetToken"),
        "prefix must define __gasGetToken"
    );
    assert!(
        prefix.contains("__gasFlush"),
        "prefix must define __gasFlush"
    );
}

#[test]
fn test_gas_suffix_installs_handler() {
    let prefix = nano::runtime::gas::GAS_SHIM_PREFIX;
    assert!(
        prefix.contains("__gasFetch = fetch"),
        "prefix must capture original fetch"
    );
    let suffix = nano::runtime::gas::GAS_SHIM_SUFFIX;
    assert!(
        suffix.contains("__nano_user_fetch"),
        "suffix must install __nano_user_fetch"
    );
}

// ── is_esm_module detection ───────────────────────────────────────────────────

#[test]
fn test_gas_classic_script_not_detected_as_esm() {
    // Typical GAS scripts have no import/export
    let gas_scripts = [
        "function doGet(e) { return ContentService.createTextOutput('ok'); }",
        "function main() { var sheet = SpreadsheetApp.getActiveSpreadsheet(); }",
        "var SHEET_ID = 'abc123';\nfunction processData() {}",
        "function doPost(e) { var data = JSON.parse(e.postData.contents); }",
    ];
    for script in &gas_scripts {
        assert!(
            !is_esm_module(script),
            "GAS script should NOT be detected as ESM: {:.60}",
            script
        );
    }
}

#[test]
fn test_gas_esm_script_detected_correctly() {
    // ESM scripts with import 'nano:gas' should be detected as ESM
    let esm = "import 'nano:gas';\nexport default { fetch: req => dispatch(req, {}) };";
    assert!(
        is_esm_module(esm),
        "ESM gas script should be detected as ESM"
    );
}

// ── V8 compilation ────────────────────────────────────────────────────────────

#[test]
fn test_gas_shim_prefix_compiles_without_errors() {
    // If the prefix has a syntax error, v8::Script::compile returns None
    let result = run_js(&format!(
        "{}\n'shim_ok'",
        nano::runtime::gas::GAS_SHIM_PREFIX
    ));
    assert_eq!(
        result.as_deref(),
        Some("shim_ok"),
        "GAS_SHIM_PREFIX must compile without V8 errors"
    );
}

#[test]
fn test_gas_full_shim_compiles_and_installs_handler() {
    let result = run_with_shim("", "typeof __nano_user_fetch");
    assert_eq!(
        result.as_deref(),
        Some("function"),
        "__nano_user_fetch must be a function after prefix+suffix injection"
    );
}

#[test]
fn test_gas_shim_globals_are_available() {
    let code = format!(
        "{}\n{}",
        nano::runtime::gas::GAS_SHIM_PREFIX,
        r#"[
          typeof SpreadsheetApp,
          typeof Logger,
          typeof PropertiesService,
          typeof CacheService,
          typeof Utilities,
          typeof DocumentApp,
          typeof DriveApp,
          typeof UrlFetchApp,
          typeof GmailApp
        ].join(',')"#
    );
    let result = run_js(&code);
    assert_eq!(
        result.as_deref(),
        Some("object,object,object,object,object,object,object,object,object"),
        "all GAS globals must be defined objects after prefix injection"
    );
}

// ── PropertiesService (synchronous) ──────────────────────────────────────────

#[test]
fn test_gas_properties_service_set_and_get() {
    let result = run_with_shim(
        "",
        r#"
        PropertiesService.getScriptProperties().setProperty('test_key_1', 'hello_gas');
        PropertiesService.getScriptProperties().getProperty('test_key_1')
        "#,
    );
    assert_eq!(result.as_deref(), Some("hello_gas"));
}

#[test]
fn test_gas_properties_service_get_missing_returns_null() {
    let result = run_with_shim(
        "",
        r#"String(PropertiesService.getScriptProperties().getProperty('__no_such_key_gas__'))"#,
    );
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn test_gas_properties_service_delete() {
    let result = run_with_shim(
        "",
        r#"
        PropertiesService.getScriptProperties().setProperty('del_key_gas', 'to_delete');
        PropertiesService.getScriptProperties().deleteProperty('del_key_gas');
        String(PropertiesService.getScriptProperties().getProperty('del_key_gas'))
        "#,
    );
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn test_gas_properties_service_namespaces_isolated() {
    // Script and User props should not share keys
    let result = run_with_shim(
        "",
        r#"
        PropertiesService.getScriptProperties().setProperty('shared_key_iso', 'script_val');
        PropertiesService.getUserProperties().setProperty('shared_key_iso', 'user_val');
        PropertiesService.getScriptProperties().getProperty('shared_key_iso') + ':' +
        PropertiesService.getUserProperties().getProperty('shared_key_iso')
        "#,
    );
    assert_eq!(result.as_deref(), Some("script_val:user_val"));
}

#[test]
fn test_gas_properties_service_get_all() {
    let result = run_with_shim(
        "",
        r#"
        var ps = PropertiesService.getScriptProperties();
        ps.setProperty('gas_gp_a', 'val_a');
        ps.setProperty('gas_gp_b', 'val_b');
        var all = ps.getProperties();
        all['gas_gp_a'] + ':' + all['gas_gp_b']
        "#,
    );
    // The result should contain both values in some order
    let s = result.unwrap_or_default();
    assert!(
        s.contains("val_a"),
        "getProperties must return gas_gp_a value"
    );
    assert!(
        s.contains("val_b"),
        "getProperties must return gas_gp_b value"
    );
}

// ── CacheService (synchronous) ────────────────────────────────────────────────

#[test]
fn test_gas_cache_service_put_and_get() {
    let result = run_with_shim(
        "",
        r#"
        CacheService.getScriptCache().put('cache_key_1', 'cached_value', 3600);
        CacheService.getScriptCache().get('cache_key_1')
        "#,
    );
    assert_eq!(result.as_deref(), Some("cached_value"));
}

#[test]
fn test_gas_cache_service_get_missing_returns_null() {
    let result = run_with_shim(
        "",
        r#"String(CacheService.getScriptCache().get('__no_such_cache_key__'))"#,
    );
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn test_gas_cache_service_remove() {
    let result = run_with_shim(
        "",
        r#"
        CacheService.getScriptCache().put('rm_cache_key', 'to_remove', 3600);
        CacheService.getScriptCache().remove('rm_cache_key');
        String(CacheService.getScriptCache().get('rm_cache_key'))
        "#,
    );
    assert_eq!(result.as_deref(), Some("null"));
}

#[test]
fn test_gas_cache_service_namespaces_isolated() {
    let result = run_with_shim(
        "",
        r#"
        CacheService.getScriptCache().put('iso_key', 'script_cache', 3600);
        CacheService.getUserCache().put('iso_key', 'user_cache', 3600);
        CacheService.getScriptCache().get('iso_key') + ':' + CacheService.getUserCache().get('iso_key')
        "#,
    );
    assert_eq!(result.as_deref(), Some("script_cache:user_cache"));
}

// ── Utilities (synchronous) ───────────────────────────────────────────────────

#[test]
fn test_gas_utilities_base64_encode() {
    let result = run_with_shim("", r#"Utilities.base64Encode('hello')"#);
    assert_eq!(result.as_deref(), Some("aGVsbG8="));
}

#[test]
fn test_gas_utilities_base64_decode() {
    let result = run_with_shim("", r#"JSON.stringify(Utilities.base64Decode('aGVsbG8='))"#);
    // [104, 101, 108, 108, 111] = "hello"
    assert_eq!(result.as_deref(), Some("[104,101,108,108,111]"));
}

#[test]
fn test_gas_utilities_base64_roundtrip() {
    let result = run_with_shim(
        "",
        r#"
        var encoded = Utilities.base64Encode('roundtrip test!');
        var decoded = Utilities.base64Decode(encoded);
        new TextDecoder().decode(new Uint8Array(decoded))
        "#,
    );
    assert_eq!(result.as_deref(), Some("roundtrip test!"));
}

#[test]
fn test_gas_utilities_base64_web_safe() {
    let result = run_with_shim(
        "",
        r#"
        var encoded = Utilities.base64EncodeWebSafe('hello+world/test=');
        // Web-safe should not contain +, /, or =
        (encoded.indexOf('+') === -1 && encoded.indexOf('/') === -1 && encoded.indexOf('=') === -1).toString()
        "#,
    );
    assert_eq!(result.as_deref(), Some("true"));
}

#[test]
fn test_gas_utilities_get_uuid_format() {
    let result = run_with_shim(
        "",
        r#"
        var uuid = Utilities.getUuid();
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid).toString()
        "#,
    );
    assert_eq!(
        result.as_deref(),
        Some("true"),
        "Utilities.getUuid() must return a valid v4 UUID"
    );
}

#[test]
fn test_gas_utilities_parse_csv() {
    let result = run_with_shim(
        "",
        r#"
        var rows = Utilities.parseCsv('a,b,c\nd,e,f');
        rows[0][0] + rows[0][2] + rows[1][1]
        "#,
    );
    assert_eq!(result.as_deref(), Some("ace"));
}

#[test]
fn test_gas_utilities_new_blob() {
    let result = run_with_shim(
        "",
        r#"
        var blob = Utilities.newBlob('hello world', 'text/plain', 'test.txt');
        blob.getName() + ':' + blob.getContentType() + ':' + blob.getDataAsString()
        "#,
    );
    assert_eq!(result.as_deref(), Some("test.txt:text/plain:hello world"));
}

// ── Logger (synchronous) ──────────────────────────────────────────────────────

#[test]
fn test_gas_logger_does_not_throw() {
    let result = run_with_shim(
        "",
        r#"Logger.log('test message'); Logger.info('info'); Logger.warning('warn'); Logger.error('err'); 'logger_ok'"#,
    );
    assert_eq!(
        result.as_deref(),
        Some("logger_ok"),
        "Logger methods must not throw"
    );
}

// ── Stubs (unavailable services) ─────────────────────────────────────────────

#[test]
fn test_gas_gmail_stub_throws_descriptive_error() {
    let result = run_with_shim(
        "",
        r#"
        var err = '';
        try { GmailApp.sendEmail('a@b.com', 'subject', 'body'); }
        catch(e) { err = e.message; }
        err.indexOf('GmailApp') !== -1 ? 'caught_gas_error' : 'wrong_error:' + err
        "#,
    );
    assert_eq!(
        result.as_deref(),
        Some("caught_gas_error"),
        "GmailApp must throw with 'GmailApp' in message"
    );
}

#[test]
fn test_gas_calendar_stub_throws() {
    let result = run_with_shim(
        "",
        r#"
        var err = '';
        try { CalendarApp.getDefaultCalendar(); }
        catch(e) { err = e.message; }
        err.indexOf('CalendarApp') !== -1 ? 'caught' : 'wrong:' + err
        "#,
    );
    assert_eq!(result.as_deref(), Some("caught"));
}

// ── HtmlService ───────────────────────────────────────────────────────────────

#[test]
fn test_gas_html_service_create_output() {
    let result = run_with_shim(
        "",
        r#"
        var output = HtmlService.createHtmlOutput('<h1>Hello</h1>');
        output.getContent()
        "#,
    );
    assert_eq!(result.as_deref(), Some("<h1>Hello</h1>"));
}

// ── Session ───────────────────────────────────────────────────────────────────

#[test]
fn test_gas_session_default_email() {
    let result = run_with_shim("", r#"Session.getEffectiveUser().getEmail()"#);
    // Default when GAS_USER_EMAIL env var is not set
    assert_eq!(result.as_deref(), Some("service-account@nano.local"));
}

#[test]
fn test_gas_session_custom_email_from_env() {
    let mut env = std::collections::HashMap::new();
    env.insert("GAS_USER_EMAIL".to_string(), "myuser@corp.com".to_string());
    let full = format!(
        "{}\n{}\n{}",
        nano::runtime::gas::GAS_SHIM_PREFIX,
        nano::runtime::gas::GAS_SHIM_SUFFIX,
        "Session.getEffectiveUser().getEmail()"
    );
    let result = run_js_with_env(&full, env);
    assert_eq!(result.as_deref(), Some("myuser@corp.com"));
}

// ── ScriptApp ─────────────────────────────────────────────────────────────────

#[test]
fn test_gas_script_app_get_token_null_initially() {
    let result = run_with_shim("", r#"String(ScriptApp.getOAuthToken())"#);
    // Token is null before any auth call
    assert_eq!(result.as_deref(), Some("null"));
}

// ── SpreadsheetApp error before auth ─────────────────────────────────────────

#[test]
fn test_gas_spreadsheet_app_throws_without_spreadsheet_id() {
    let result = run_with_shim(
        "",
        r#"
        var err = '';
        try { SpreadsheetApp.getActiveSpreadsheet(); }
        catch(e) { err = e.message; }
        err.indexOf('SPREADSHEET_ID') !== -1 ? 'caught' : 'wrong:' + err
        "#,
    );
    assert_eq!(
        result.as_deref(),
        Some("caught"),
        "must throw with SPREADSHEET_ID in message when not set"
    );
}

#[test]
fn test_gas_spreadsheet_app_open_by_id_returns_object() {
    // openById doesn't make a network call synchronously — it just creates a proxy object
    let result = run_with_shim("", r#"typeof SpreadsheetApp.openById('fake-sheet-id')"#);
    assert_eq!(
        result.as_deref(),
        Some("object"),
        "openById must return a spreadsheet proxy object"
    );
}

// ── User code survives shim injection ────────────────────────────────────────

#[test]
fn test_gas_user_functions_survive_shim_injection() {
    let result = run_with_shim(
        r#"
        function doGet(e) { return 'hello from doGet'; }
        function processData() { return 42; }
        function main() { return 'main called'; }
        "#,
        r#"
        typeof doGet + ':' + typeof processData + ':' + typeof main
        "#,
    );
    assert_eq!(
        result.as_deref(),
        Some("function:function:function"),
        "user-defined functions must survive shim injection"
    );
}

#[test]
fn test_gas_shim_does_not_override_user_spreadsheet_id() {
    // When env var IS set, getActiveSpreadsheet should use it
    let mut env = std::collections::HashMap::new();
    env.insert("SPREADSHEET_ID".to_string(), "my-sheet-id".to_string());
    let full = format!(
        "{}\n{}\n{}",
        nano::runtime::gas::GAS_SHIM_PREFIX,
        nano::runtime::gas::GAS_SHIM_SUFFIX,
        "SpreadsheetApp.getActiveSpreadsheet().getId()"
    );
    let result = run_js_with_env(&full, env);
    assert_eq!(result.as_deref(), Some("my-sheet-id"));
}
