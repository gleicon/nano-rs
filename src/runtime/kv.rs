//! nano:kv — EdgeStore-backed key-value storage for nano-rs workers.
//!
//! Exposes the `nano:kv` ESM module:
//!
//! ```javascript
//! import { kv, openKV } from 'nano:kv';
//!
//! // Default namespace (auto-scoped to hostname)
//! await kv.set('counter', new TextEncoder().encode('42'));
//! const val = await kv.get('counter');
//!
//! // Named namespace
//! const cache = openKV('cache');
//! await cache.setJSON('config', { version: 1 });
//! const cfg = await cache.getJSON('config');
//! ```
//!
//! All keys are namespaced as `{hostname}::{kv_name}` to ensure
//! tenant isolation across a multi-app process.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use edgestore::{EdgestoreConfig, Engine};

// Global EdgeStore engine — initialized once, shared across worker threads via Mutex.
static KV_ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

thread_local! {
    // Hostname of the current worker's app — used for namespace isolation.
    static CURRENT_KV_HOSTNAME: RefCell<String> = RefCell::new(String::new());
}

/// Initialize the KV engine. Safe to call multiple times — only the first call takes effect.
pub fn init_kv_engine(data_dir: impl Into<PathBuf>) {
    KV_ENGINE.get_or_init(|| {
        let path = data_dir.into().join("kv");
        std::fs::create_dir_all(&path).ok();
        let config = EdgestoreConfig::new(&path);
        match Engine::open(config) {
            Ok(engine) => Mutex::new(engine),
            Err(e) => {
                tracing::warn!("KV engine open failed at {:?}: {}; falling back to temp dir", path, e);
                let tmp = std::env::temp_dir().join("nano-rs-kv-fallback");
                std::fs::create_dir_all(&tmp).ok();
                Mutex::new(Engine::open(EdgestoreConfig::new(tmp)).expect("KV fallback init failed"))
            }
        }
    });
}

/// Set the hostname for this thread's KV namespace. Called once per isolate lifetime.
pub fn set_kv_hostname(hostname: String) {
    CURRENT_KV_HOSTNAME.with(|cell| *cell.borrow_mut() = hostname);
}

/// Compute the full namespace bytes: `{hostname}::{kv_name}`.
fn kv_namespace(kv_name: &str) -> Vec<u8> {
    let hostname = CURRENT_KV_HOSTNAME.with(|cell| cell.borrow().clone());
    format!("{}::{}", hostname, kv_name).into_bytes()
}

// ─── V8 bindings ────────────────────────────────────────────────────────────

/// Bind `__nano_kv_*` globals to the V8 context.
///
/// Must be called during context setup (before any JS execution).
/// The `nano:kv` ESM module references these globals at import time.
pub fn bind_kv(scope: &mut v8::PinnedRef<v8::HandleScope<()>>, context: v8::Local<v8::Context>) {
    init_kv_engine(std::env::current_dir().unwrap_or_default().join(".nano-kv"));

    let global = context.global(scope);
    let mut ctx_scope = v8::ContextScope::new(scope, context);

    macro_rules! bind {
        ($name:expr, $cb:expr) => {
            if let (Some(f), Some(k)) = (
                v8::Function::new(&mut ctx_scope, $cb),
                v8::String::new(&mut ctx_scope, $name),
            ) {
                global.set(&mut ctx_scope, k.into(), f.into());
            }
        };
    }
    bind!("__nano_kv_get", kv_get);
    bind!("__nano_kv_set", kv_set);
    bind!("__nano_kv_delete", kv_delete);
    bind!("__nano_kv_list", kv_list);
}

/// `__nano_kv_get(key: string, namespace: string) -> Uint8Array | null`
fn kv_get(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let key = match args.get(0).to_string(scope) {
        Some(s) => s.to_rust_string_lossy(scope),
        None => return,
    };
    let ns_name = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "default".to_string());

    let ns = kv_namespace(&ns_name);

    let result = KV_ENGINE
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|e| e.get(&ns, key.as_bytes()).ok().flatten());

    match result {
        Some(bytes) => {
            let ab = v8::ArrayBuffer::new(scope, bytes.len());
            {
                let store = ab.get_backing_store();
                for (i, &byte) in bytes.iter().enumerate() {
                    if let Some(cell) = store.get(i) {
                        cell.set(byte);
                    }
                }
            }
            if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, bytes.len()) {
                retval.set(arr.into());
            }
        }
        None => retval.set(v8::null(scope).into()),
    }
}

/// `__nano_kv_set(key: string, value: Uint8Array | string, namespace: string)`
fn kv_set(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let key = match args.get(0).to_string(scope) {
        Some(s) => s.to_rust_string_lossy(scope),
        None => return,
    };

    let value: Vec<u8> = if args.length() > 1 {
        match crate::runtime::v8_helpers::extract_bytes_from_v8_value(scope, args.get(1)) {
            Some(b) => b,
            None => return,
        }
    } else {
        return;
    };

    let ns_name = args
        .get(2)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "default".to_string());

    let ns = kv_namespace(&ns_name);

    if let Some(engine) = KV_ENGINE.get() {
        if let Ok(mut e) = engine.lock() {
            let _ = e.put(&ns, key.as_bytes(), &value);
        }
    }
}

/// `__nano_kv_delete(key: string, namespace: string)`
fn kv_delete(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let key = match args.get(0).to_string(scope) {
        Some(s) => s.to_rust_string_lossy(scope),
        None => return,
    };
    let ns_name = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "default".to_string());

    let ns = kv_namespace(&ns_name);

    if let Some(engine) = KV_ENGINE.get() {
        if let Ok(mut e) = engine.lock() {
            let _ = e.delete(&ns, key.as_bytes());
        }
    }
}

/// `__nano_kv_list(prefix: string, namespace: string) -> Array<[string, Uint8Array]>`
fn kv_list(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let prefix = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let ns_name = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "default".to_string());

    let ns = kv_namespace(&ns_name);

    let pairs: Vec<(Vec<u8>, Vec<u8>)> = KV_ENGINE
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|e| e.prefix(&ns, prefix.as_bytes()).ok())
        .unwrap_or_default();

    let result_arr = v8::Array::new(scope, pairs.len() as i32);
    for (i, (key, val)) in pairs.into_iter().enumerate() {
        let key_str = String::from_utf8_lossy(&key).to_string();
        let entry = v8::Array::new(scope, 2);

        if let Some(k) = v8::String::new(scope, &key_str) {
            entry.set_index(scope, 0, k.into());
        }

        let ab = v8::ArrayBuffer::new(scope, val.len());
        {
            let store = ab.get_backing_store();
            for (j, &byte) in val.iter().enumerate() {
                if let Some(cell) = store.get(j) {
                    cell.set(byte);
                }
            }
        }
        if let Some(arr) = v8::Uint8Array::new(scope, ab, 0, val.len()) {
            entry.set_index(scope, 1, arr.into());
        }

        result_arr.set_index(scope, i as u32, entry.into());
    }

    retval.set(result_arr.into());
}

// ─── Synthetic ESM module code ──────────────────────────────────────────────

/// ESM code for `nano:kv`. References `__nano_kv_*` globals bound by `bind_kv`.
pub const NANO_KV_MODULE_CODE: &str = r#"
const __kv_make = (name) => ({
  async get(key) {
    return __nano_kv_get(String(key), name);
  },
  async set(key, value) {
    if (typeof value === 'string') {
      value = new TextEncoder().encode(value);
    }
    __nano_kv_set(String(key), value, name);
  },
  async delete(key) {
    __nano_kv_delete(String(key), name);
  },
  async list(prefix) {
    return __nano_kv_list(prefix != null ? String(prefix) : '', name);
  },
  async getJSON(key) {
    const bytes = __nano_kv_get(String(key), name);
    if (!bytes) return null;
    return JSON.parse(new TextDecoder().decode(bytes));
  },
  async setJSON(key, value) {
    __nano_kv_set(String(key), new TextEncoder().encode(JSON.stringify(value)), name);
  },
});

export const kv = __kv_make('default');

export function openKV(name) {
  return __kv_make(String(name));
}

export default { kv, openKV };
"#;

/// Return the ESM code for a built-in `nano:*` specifier.
///
/// Called from `v8/module.rs` `module_resolve_callback` to generate synthetic modules.
pub fn get_nano_module_code(specifier: &str) -> Option<&'static str> {
    match specifier {
        "nano:kv" => Some(NANO_KV_MODULE_CODE),
        _ => None,
    }
}
