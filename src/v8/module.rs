//! ESM Module Loader for V8 Module API (v147 Compatible)
//!
//! This module provides the infrastructure for executing ECMAScript Modules (ESM)
//! using V8's Module API instead of the classic Script API. This enables proper
//! support for `export default { fetch }` and `import` statements.
//!
//! The module loader integrates with the VFS for resolving relative imports
//! within the isolate's namespace.
//!
//! # V147 API Changes
//!
//! In v147:
//! - ContextScope requires 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
//! - ContextScope implements Deref/DerefMut to PinnedRef<HandleScope>
//! - When passing scope to V8 APIs, use `&**scope` to dereference through the ContextScope

use crate::http::NanoResponse;
use crate::http::v8_bridge::serialize_request_to_json;
use crate::runtime::{HandlerContext, async_support};
use crate::vfs::IsolateVfs;
use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::HashMap;

// Thread-local storage for the current module loader during ESM execution
thread_local! {
    static CURRENT_LOADER: RefCell<Option<*mut ModuleLoader>> = RefCell::new(None);
}

/// Set the current module loader for import resolution callbacks.
///
/// # Safety
/// The caller must ensure the loader pointer remains valid for the duration of
/// V8 module instantiation. Call with `None` immediately after instantiate_module returns.
pub(crate) unsafe fn set_current_loader(loader: Option<*mut ModuleLoader>) {
    CURRENT_LOADER.with(|cell| {
        *cell.borrow_mut() = loader;
    });
}

/// Get the current module loader if available
fn with_current_loader<F, R>(f: F) -> R
where
    F: FnOnce(Option<&mut ModuleLoader>) -> R,
{
    CURRENT_LOADER.with(|cell| {
        let loader_ptr = cell.borrow();
        if let Some(ptr) = *loader_ptr {
            unsafe { f(Some(&mut *ptr)) }
        } else {
            f(None)
        }
    })
}

/// Type of JavaScript module being executed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleType {
    /// ES Module with import/export syntax
    ESM,
    /// Classic script without module syntax
    Script,
}

/// Detect the type of module based on code content.
///
/// Scans non-comment lines for `export` or `import` at the start (after
/// optional whitespace). This avoids false positives from string literals
/// like `const msg = "please export your config"`.
pub fn detect_module_type(code: &str) -> ModuleType {
    let mut in_block_comment = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if trimmed.contains("*/") { in_block_comment = false; }
            continue;
        }
        if trimmed.starts_with("//") { continue; }
        if trimmed.contains("/*") { in_block_comment = true; }
        if trimmed.starts_with("export ") || trimmed.starts_with("export{")
            || trimmed.starts_with("export default")
            || trimmed.starts_with("import ") || trimmed.starts_with("import{")
            || trimmed.starts_with("import(")
        {
            return ModuleType::ESM;
        }
    }
    ModuleType::Script
}

/// Check if code is an ESM module
pub fn is_esm_module(code: &str) -> bool {
    matches!(detect_module_type(code), ModuleType::ESM)
}

/// Module loader that handles ESM compilation and import resolution
///
/// The ModuleLoader maintains a cache of compiled modules and provides
/// the import resolution callback for V8's Module API.
pub struct ModuleLoader {
    /// VFS for reading imported modules
    vfs: IsolateVfs,
    /// Cache of compiled modules by path
    module_cache: HashMap<String, v8::Global<v8::Module>>,
    /// Stack of currently loading modules (for circular import detection)
    loading_stack: Vec<String>,
}

impl ModuleLoader {
    /// Create a new ModuleLoader with the given VFS
    pub fn new(vfs: IsolateVfs) -> Self {
        Self {
            vfs,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
        }
    }

    /// Load a module: VFS first, disk fallback.
    fn load_module_from_vfs(&self, path: &str) -> Result<String> {
        let vfs = &self.vfs;
        let vfs_result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(vfs.read(path)),
            Err(_) => crate::data_plane::with_worker_runtime(|h| h.block_on(vfs.read(path)))
                .unwrap_or_else(|| {
                    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                    rt.block_on(vfs.read(path))
                }),
        };
        match vfs_result {
            Ok(content) => return Ok(String::from_utf8_lossy(&content).to_string()),
            Err(_) => {} // fall through to disk
        }
        // Disk fallback: strip leading slash for relative disk reads
        let disk_path = path.trim_start_matches('/');
        std::fs::read_to_string(disk_path)
            .map_err(|e| anyhow!("Module '{}' not found in VFS or on disk: {}", path, e))
    }

    /// Resolve an import specifier to a VFS (or disk) path.
    ///
    /// - Absolute paths (`/foo`): returned as-is
    /// - Relative paths (`./foo`, `../foo`): resolved against base_path
    /// - Bare specifiers (`lodash`): try `/node_modules/<spec>/index.js` in VFS,
    ///   then disk-walk (node_modules/<spec>/index.js relative to cwd)
    fn resolve_import_path(&self, base_path: &str, specifier: &str) -> Result<String> {
        // Absolute VFS path
        if specifier.starts_with('/') {
            return Ok(specifier.to_string());
        }

        // Bare specifier — not relative, not absolute
        if !specifier.starts_with('.') {
            // Try VFS /node_modules first
            let vfs_nm = format!("/node_modules/{}/index.js", specifier);
            let vfs_check = crate::data_plane::with_worker_runtime(|h| {
                h.block_on(self.vfs.exists(&vfs_nm))
            }).unwrap_or_else(|| {
                let rt = tokio::runtime::Runtime::new().expect("rt");
                rt.block_on(self.vfs.exists(&vfs_nm))
            }).unwrap_or(false);
            if vfs_check {
                return Ok(vfs_nm);
            }
            // Disk-walk fallback
            let disk_nm = format!("node_modules/{}/index.js", specifier);
            if std::path::Path::new(&disk_nm).exists() {
                return Ok(format!("/{}", disk_nm));
            }
            return Err(anyhow!(
                "Cannot resolve bare specifier '{}': not found in VFS /node_modules/ or disk node_modules/",
                specifier
            ));
        }

        // Get the directory of the base path
        let base_dir = if base_path.contains('/') {
            let parts: Vec<&str> = base_path.rsplitn(2, '/').collect();
            parts[1]
        } else {
            "."
        };

        // Normalize the path by processing . and ..
        let mut components: Vec<&str> = Vec::new();

        // Start with base directory components
        for component in base_dir.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            components.push(component);
        }

        // Process the import specifier
        for component in specifier.split('/') {
            if component.is_empty() || component == "." {
                continue;
            } else if component == ".." {
                // Go up one directory
                if components.pop().is_none() {
                    return Err(anyhow!(
                        "Path traversal out of bounds: {} from {}",
                        specifier,
                        base_path
                    ));
                }
            } else {
                components.push(component);
            }
        }

        // Reconstruct the path
        let resolved = if components.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", components.join("/"))
        };

        // Try to add .js extension if no extension present
        if !resolved.contains('.') {
            let vfs_exists = |path: &str| -> bool {
                let p = path.to_string();
                let vfs = &self.vfs;
                match tokio::runtime::Handle::try_current() {
                    Ok(handle) => handle.block_on(vfs.exists(&p)).unwrap_or(false),
                    Err(_) => crate::data_plane::with_worker_runtime(|h| h.block_on(vfs.exists(&p)))
                        .unwrap_or_else(|| {
                            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                            rt.block_on(vfs.exists(&p))
                        })
                        .unwrap_or(false),
                }
            };
            let with_js = format!("{}.js", resolved);
            if vfs_exists(&with_js) {
                return Ok(with_js);
            }
            let with_mjs = format!("{}.mjs", resolved);
            if vfs_exists(&with_mjs) {
                return Ok(with_mjs);
            }
        }

        Ok(resolved)
    }

    /// Check if a module is already being loaded (circular import detection)
    fn is_circular_import(&self, path: &str) -> bool {
        self.loading_stack.contains(&path.to_string())
    }

    /// Push a module onto the loading stack
    fn push_loading(&mut self, path: &str) {
        self.loading_stack.push(path.to_string());
    }

    /// Pop a module from the loading stack
    fn pop_loading(&mut self) {
        self.loading_stack.pop();
    }

    /// Get a cached module if available
    fn get_cached(&self, path: &str) -> Option<v8::Global<v8::Module>> {
        self.module_cache.get(path).cloned()
    }

    /// Cache a compiled module
    fn cache_module(&mut self, path: &str, module: v8::Global<v8::Module>) {
        self.module_cache.insert(path.to_string(), module);
    }
}

/// Execute an ESM module with proper import resolution
///
/// This is the main entry point for executing JavaScript handlers.
/// It detects whether the code is ESM or classic script and routes
/// accordingly.
///
/// # Arguments
/// * `scope` - The V8 context scope (v147: ContextScope with 2 lifetimes)
/// * `v8_context` - The V8 context to execute in
/// * `code` - The JavaScript code to execute
/// * `entrypoint` - The path to the entrypoint (for import resolution)
/// * `handler_ctx` - The handler context with request information
/// * `vfs` - The VFS for resolving module imports within the isolate's namespace
///
/// # V147 API Note
/// ContextScope now has 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
/// When calling V8 APIs, use `&**scope` to dereference through the ContextScope to the PinnedRef.
///
/// Note: After entering a context, the parent HandleScope type changes from `HandleScope<'a, ()>`
/// to `HandleScope<'a, Context>`. The type parameter reflects this.
pub fn execute_esm_or_script<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    v8_context: v8::Local<'a, v8::Context>,
    code: &str,
    entrypoint: &str,
    handler_ctx: &HandlerContext,
    vfs: IsolateVfs,
) -> Result<NanoResponse> {
    // Detect module type
    if is_esm_module(code) {
        // ESM path - use module loader with actual VFS
        execute_esm_module(scope, v8_context, code, entrypoint, handler_ctx, vfs)
    } else {
        // Classic script path
        execute_classic_script(scope, v8_context, code, handler_ctx)
    }
}

/// Execute a classic script using Script API
///
/// This provides backward compatibility for existing handlers that
/// don't use ESM syntax.
///
/// # V147 API Note
/// ContextScope now has 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
/// When calling V8 APIs, use `&**scope` to dereference through the ContextScope.
/// 
/// Note: After entering a context, the parent HandleScope type changes from `HandleScope<'a, ()>`
/// to `HandleScope<'a, Context>`.
pub fn execute_classic_script<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    v8_context: v8::Local<'a, v8::Context>,
    code: &str,
    handler_ctx: &HandlerContext,
) -> Result<NanoResponse> {
    // Transform ES6 module syntax for classic scripts
    let transformed_code = transform_module_code(code);

    // Compile and run script to define fetch function
    // v147 API: Dereference ContextScope to get PinnedRef via &**scope
    let code_str = v8::String::new(&**scope, &transformed_code)
        .ok_or_else(|| anyhow!("Failed to create code string"))?;
    let script = v8::Script::compile(&**scope, code_str, None)
        .ok_or_else(|| anyhow!("Script compilation failed"))?;
    script.run(&**scope);

    // Get global and look for fetch function
    let global = v8_context.global(&**scope);
    let fetch_key = v8::String::new(&**scope, "fetch").unwrap();
    let fetch_val = match global.get(&**scope, fetch_key.into()) {
        Some(val) => val,
        None => {
            return Ok(NanoResponse::ok()
                .with_header("Content-Type", "text/plain")
                .with_body("Handler executed (no fetch function defined)"));
        }
    };

    if !fetch_val.is_function() {
        return Ok(NanoResponse::ok()
            .with_header("Content-Type", "text/plain")
            .with_body("Handler executed (fetch is not a function)"));
    }
    let fetch_fn = fetch_val.cast::<v8::Function>();

    // Create request object using full WinterTC serialization
    let request_json = serialize_request_to_json(&handler_ctx.request);
    let request_str = v8::String::new(&**scope, &request_json)
        .ok_or_else(|| anyhow!("Failed to create request JSON string"))?;

    // Parse JSON to create proper JS object
    let json_key = v8::String::new(&**scope, "JSON").unwrap();
    let json_val = global.get(&**scope, json_key.into())
        .ok_or_else(|| anyhow!("JSON not found"))?;
    let json_obj = json_val.to_object(&**scope)
        .ok_or_else(|| anyhow!("JSON is not an object"))?;
    let parse_key = v8::String::new(&**scope, "parse").unwrap();
    let parse_val = json_obj.get(&**scope, parse_key.into())
        .filter(|v| v.is_function())
        .ok_or_else(|| anyhow!("JSON.parse not found or not a function"))?;
    let parse_fn = parse_val.cast::<v8::Function>();

    let js_request = parse_fn.call(&**scope, json_val.into(), &[request_str.into()])
        .ok_or_else(|| anyhow!("Failed to parse request JSON"))?;

    // Call fetch function with parsed JS object
    let result = fetch_fn.call(&**scope, global.into(), &[js_request.into()]);

    // Perform microtask checkpoint to resolve any Promises
    scope.perform_microtask_checkpoint();

    // Check if result is a Promise and resolve if needed
    // Resolve using async event loop for Promises
    let resolved = if let Some(response) = result {
        if response.is_promise() {
            let promise = response.cast::<v8::Promise>();
            match async_support::resolve_promise_with_async(scope, promise) {
                Ok(value) => Some(value),
                Err(e) => return Err(e),
            }
        } else {
            Some(response)
        }
    } else {
        None
    };

    // Extract response
    match resolved {
        Some(response) => {
            extract_js_response(scope, response)
        }
        None => Err(anyhow!("Handler returned None")),
    }
}

/// Transform ES6 module syntax to be compatible with V8 Script execution
///
/// Converts `export default { fetch: ... }` to `var __nano_handler = { ... };`
/// and extracts the fetch function to a separate global variable without
/// overwriting the native fetch() API.
pub fn transform_module_code(code: &str) -> String {
    // Check if this looks like ES6 module syntax with export default
    if code.contains("export default") {
        // Replace export default with var declaration
        let transformed = code.replace("export default", "var __nano_handler =");

        // Add code to extract handler function to a SEPARATE global variable
        // This preserves the native fetch() for external HTTP requests
        format!("{}\n\n// Extract handler function from export\nvar __nano_user_fetch = undefined;\nif (typeof __nano_handler === 'object' && __nano_handler.fetch) {{\n    __nano_user_fetch = __nano_handler.fetch;\n}}", transformed)
    } else {
        // No transformation needed
        code.to_string()
    }
}

/// Extract a NanoResponse from a V8 JavaScript object
///
/// # V147 API Note
/// ContextScope now has 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
/// When calling V8 APIs, use `&**scope` to dereference through the ContextScope.
/// 
/// Note: After entering a context, the parent HandleScope type changes from `HandleScope<'a, ()>`
/// to `HandleScope<'a, Context>`.
fn extract_js_response<'s>(
    scope: &mut v8::ContextScope<'s, 's, v8::HandleScope<'s, v8::Context>>,
    js_response: v8::Local<'s, v8::Value>,
) -> Result<NanoResponse> {
    use crate::http::NanoHeaders;
    use bytes::Bytes;

    // v147 API: Dereference ContextScope to get PinnedRef via &**scope

    // Verify the response is an object
    let obj = match js_response.to_object(&**scope) {
        Some(o) => o,
        None => return Err(anyhow!("Response is not an object")),
    };

    // Extract status property (default to 200)
    let status_key = v8::String::new(&**scope, "status").unwrap();
    let status = match obj.get(&**scope, status_key.into()) {
        Some(val) if !val.is_null() && !val.is_undefined() => match val.to_integer(&**scope) {
            Some(int) => int.value() as u16,
            None => 200,
        },
        _ => 200,
    };

    // Extract headers property
    let mut nano_headers = NanoHeaders::new();
    let headers_key = v8::String::new(&**scope, "headers").unwrap();

    if let Some(headers_val) = obj.get(&**scope, headers_key.into()) {
        if let Some(headers_obj) = headers_val.to_object(&**scope) {
            // Headers may be stored internally in __headers__ property (for Headers class instances)
            // or directly on the object (for plain objects used by Response)
            let internal_headers_key = v8::String::new(&**scope, "__headers__").unwrap();
            let headers_source = headers_obj
                .get(&**scope, internal_headers_key.into())
                .and_then(|v| v.to_object(&**scope))
                .unwrap_or(headers_obj);

            if let Some(names) = headers_source.get_own_property_names(&**scope, Default::default()) {
                let len = names.length();
                for i in 0..len {
                    if let Some(key) = names.get_index(&**scope, i) {
                        if let Some(key_str) = key.to_string(&**scope) {
                            let key_name = key_str.to_rust_string_lossy(&**scope);
                            // Skip internal properties and methods (functions)
                            if key_name.starts_with("__")
                                || key_name == "set"
                                || key_name == "get"
                                || key_name == "forEach"
                            {
                                continue;
                            }
                            if let Some(value) = headers_source.get(&**scope, key.into()) {
                                // Only include string values (not functions)
                                if !value.is_function() {
                                    if let Some(value_str) = value.to_string(&**scope) {
                                        let value_string = value_str.to_rust_string_lossy(&**scope);
                                        nano_headers.set(&key_name, &value_string);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Extract body property
    let body_key = v8::String::new(&**scope, "body").unwrap();
    let body = match obj.get(&**scope, body_key.into()) {
        Some(val) if !val.is_null() && !val.is_undefined() => match val.to_string(&**scope) {
            Some(s) => Some(Bytes::from(s.to_rust_string_lossy(&**scope))),
            None => None,
        },
        _ => None,
    };

    Ok(NanoResponse::new(status, nano_headers, body))
}

/// Execute an ESM module
///
/// Uses V8's Module API to compile and execute the module with proper
/// import resolution.
///
/// # V147 API Note
/// ContextScope now has 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
/// When calling V8 APIs, use `&**scope` to dereference through the ContextScope.
/// 
/// Note: After entering a context, the parent HandleScope type changes from `HandleScope<'a, ()>`
/// to `HandleScope<'a, Context>`.
fn execute_esm_module<'a>(
    scope: &mut v8::ContextScope<'a, 'a, v8::HandleScope<'a, v8::Context>>,
    _v8_context: v8::Local<'a, v8::Context>,
    code: &str,
    entrypoint: &str,
    handler_ctx: &HandlerContext,
    vfs: IsolateVfs,
) -> Result<NanoResponse> {
    // v147 API: Dereference ContextScope to get PinnedRef via &**scope

    // Create module origin
    let resource_name = v8::String::new(&**scope, entrypoint).unwrap();
    let source_map_url: Option<v8::Local<v8::Value>> = Some(v8::undefined(&**scope).into());
    let origin = v8::ScriptOrigin::new(
        &**scope,
        resource_name.into(),
        0,      // line offset
        0,      // column offset
        true,   // is cross origin
        -1,     // script id
        source_map_url,
        false,  // resource_is_opaque
        false,  // is_wasm
        true,   // is_module
        None,   // host_defined_options
    );

    // Create source
    let code_str = v8::String::new(&**scope, code)
        .ok_or_else(|| anyhow!("Failed to create code string"))?;
    let mut source = v8::script_compiler::Source::new(code_str, Some(&origin));

    // Compile module
    let module = v8::script_compiler::compile_module(&**scope, &mut source)
        .ok_or_else(|| anyhow!("Module compilation failed"))?;

    // Create module loader for import resolution using the isolate's VFS
    // The VFS is passed from the caller (WorkerPool via HandlerContext -> execute_handler)
    let mut loader = ModuleLoader::new(vfs);

    // Store loader in thread-local storage for the callback to access
    let loader_ptr = &mut loader as *mut ModuleLoader;
    unsafe {
        set_current_loader(Some(loader_ptr));
    }

    // Instantiate module with import resolution callback
    let instantiate_result = module.instantiate_module(scope, module_resolve_callback);

    // Clear the loader after instantiation
    unsafe {
        set_current_loader(None);
    }

    if instantiate_result.is_none() {
        return Err(anyhow!("Module instantiation failed"));
    }

    // Evaluate module
    let eval_result = module.evaluate(scope);
    if eval_result.is_none() {
        return Err(anyhow!("Module evaluation failed"));
    }

    // Perform microtask checkpoint
    scope.perform_microtask_checkpoint();

    // PROPER ESM EXECUTION: Extract and call default export directly
    // Using v8::Global to escape scope lifetime limitations
    //
    // Step 1: Extract fetch function and default object as v8::Global
    let (fetch_global, default_global) = {
        let namespace = module.get_module_namespace();
        let obj = namespace
            .to_object(scope)
            .ok_or_else(|| anyhow!("Module namespace is not an object"))?;

        // Get 'default' export
        let default_key = v8::String::new(scope, "default").unwrap();
        let default_val = obj
            .get(scope, default_key.into())
            .ok_or_else(|| anyhow!("No default export found"))?;

        // Check if default is an object with fetch method
        let result = if let Some(default_obj) = default_val.to_object(scope) {
            let fetch_key = v8::String::new(scope, "fetch").unwrap();
            if let Some(fetch_val) = default_obj.get(scope, fetch_key.into()) {
                if fetch_val.is_function() {
                    let fetch_fn = fetch_val.cast::<v8::Function>();
                    Some((
                        v8::Global::new(scope, fetch_fn),
                        Some(v8::Global::new(scope, default_obj)),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some(r) = result {
            r
        } else if default_val.is_function() {
            // Check if default is directly a function
            let fetch_fn = default_val.cast::<v8::Function>();
            (v8::Global::new(scope, fetch_fn), None)
        } else {
            return Err(anyhow!(
                "Default export must be an object with fetch method or a function"
            ));
        }
    };

    // Step 2: Create JS Request object
    let js_request = {
        let global = scope.get_current_context().global(scope);
        let request_json = serialize_request_to_json(&handler_ctx.request);
        let request_str = v8::String::new(scope, &request_json)
            .ok_or_else(|| anyhow!("Failed to create request JSON string"))?;

        // Get JSON.parse
        let json_key = v8::String::new(scope, "JSON").unwrap();
        let json_val = global
            .get(scope, json_key.into())
            .ok_or_else(|| anyhow!("JSON not found"))?;
        let json_obj = json_val
            .to_object(scope)
            .ok_or_else(|| anyhow!("JSON is not an object"))?;
        let parse_key = v8::String::new(scope, "parse").unwrap();
        let parse_val = json_obj
            .get(scope, parse_key.into())
            .filter(|v| v.is_function())
            .ok_or_else(|| anyhow!("JSON.parse not found or not a function"))?;
        let parse_fn = parse_val.cast::<v8::Function>();

        parse_fn
            .call(scope, json_val.into(), &[request_str.into()])
            .ok_or_else(|| anyhow!("Failed to parse request JSON"))?
    };

    // Step 3: Call fetch function
    let response_val = {
        let fetch_fn = v8::Local::new(scope, fetch_global);
        let recv: v8::Local<v8::Value> = if let Some(default_global) = default_global {
            let default_obj = v8::Local::new(scope, default_global);
            default_obj.into()
        } else {
            v8::undefined(scope).into()
        };

        fetch_fn.call(scope, recv, &[js_request.into()])
    };

    // Step 4 & 5: Resolve Promise if needed and extract response
    // Must inline promise resolution to avoid intermediate v8::Local borrows
    {
        let response = match response_val {
            Some(val) => val,
            None => return Err(anyhow!("Handler returned None")),
        };

        // Resolve using async event loop for Promises
        let resolved_val = if response.is_promise() {
            let promise = response.cast::<v8::Promise>();
            match async_support::resolve_promise_with_async(scope, promise) {
                Ok(value) => value,
                Err(e) => return Err(e),
            }
        } else {
            response
        };

        // Convert to Global to escape borrow, then back to Local for extraction
        let resolved_global = v8::Global::new(scope, resolved_val);
        let resolved_local = v8::Local::new(scope, resolved_global);
        extract_js_response(scope, resolved_local)
    }
}

/// Module resolution callback for V8
///
/// This callback is invoked by V8 when a module has import statements.
/// It resolves the import specifier against the VFS and returns the
/// compiled module.
///
/// The signature matches V8's ResolveModuleCallback which is automatically
/// converted via MapFnFrom trait.
///
/// # V147 API Note
/// CallbackScope uses the same pin! + init() pattern as HandleScope.
pub(crate) fn module_resolve_callback<'a>(
    context: v8::Local<'a, v8::Context>,
    specifier: v8::Local<'a, v8::String>,
    _import_attributes: v8::Local<'a, v8::FixedArray>,
    _referrer: v8::Local<'a, v8::Module>,
) -> Option<v8::Local<'a, v8::Module>> {
    // Get the module loader from thread-local storage
    let loader_option = with_current_loader(|loader| {
        loader.map(|l| l as *mut ModuleLoader)
    });

    let loader_ptr = loader_option?;
    let loader = unsafe { &mut *loader_ptr };

    // Convert specifier to Rust string
    // v147 API: CallbackScope uses pin! + init() pattern
    let callback_scope = unsafe { v8::CallbackScope::new(context) };
    let callback_scope = std::pin::pin!(callback_scope);
    let callback_scope = callback_scope.init();
    // v147 API: to_rust_string_lossy expects &Isolate, get via Deref from PinnedRef
    // Note: CallbackScope derefs to PinnedRef<HandleScope>, which derefs to Isolate
    let specifier_str = specifier.to_rust_string_lossy(&**callback_scope);

    // Resolve the import path
    // The base path defaults to the typical entrypoint location.
    // Full referrer-based resolution would use the referrer module's path
    // from the V8 Module API, but the default handles the common case
    // of imports relative to the app entrypoint.
    let base_path = "/handler.js";

    let resolved_path = match loader.resolve_import_path(base_path, &specifier_str) {
        Ok(path) => path,
        Err(_) => return None,
    };

    // Check for circular imports
    if loader.is_circular_import(&resolved_path) {
        return None;
    }

    // Check cache
    // v147 API: v8::Local::new expects &PinnedRef<HandleScope>
    // Note: CallbackScope derefs to PinnedRef<HandleScope>, which is compatible
    if let Some(cached) = loader.get_cached(&resolved_path) {
        return Some(v8::Local::new(&*callback_scope, &cached));
    }

    // Load module from VFS
    let code = match loader.load_module_from_vfs(&resolved_path) {
        Ok(code) => code,
        Err(_) => return None,
    };

    // Track that we're loading this module
    loader.push_loading(&resolved_path);

    // Create origin for the module
    // v147 API: All V8 APIs that expect &PinnedRef<HandleScope> work with CallbackScope
    // via Deref (CallbackScope -> PinnedRef<HandleScope>)
    let resource_name = v8::String::new(&*callback_scope, &resolved_path).unwrap();
    let source_map_url: Option<v8::Local<v8::Value>> = Some(v8::undefined(&*callback_scope).into());
    let origin = v8::ScriptOrigin::new(
        &*callback_scope,
        resource_name.into(),
        0,
        0,
        true,
        -1,
        source_map_url,
        false,
        false,
        true,
        None,
    );

    // Create source
    let code_str = match v8::String::new(&*callback_scope, &code) {
        Some(s) => s,
        None => {
            loader.pop_loading();
            return None;
        }
    };
    let mut source = v8::script_compiler::Source::new(code_str, Some(&origin));

    // Compile module
    // v147 API: compile_module expects &PinnedRef<HandleScope>
    let module = match v8::script_compiler::compile_module(&*callback_scope, &mut source) {
        Some(m) => m,
        None => {
            loader.pop_loading();
            return None;
        }
    };

    // Cache the module
    // v147 API: Global::new expects &Isolate (accessed via Deref from PinnedRef)
    let global_module = v8::Global::new(&**callback_scope, module);
    loader.cache_module(&resolved_path, global_module.clone());

    // Pop from loading stack
    loader.pop_loading();

    // Return the module
    // v147 API: v8::Local::new expects &PinnedRef<HandleScope>
    Some(v8::Local::new(&*callback_scope, &global_module))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{VfsNamespace, MemoryBackend};

    #[test]
    fn test_module_loader_creation() {
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let loader = ModuleLoader::new(vfs);
        assert!(loader.module_cache.is_empty());
        assert!(loader.loading_stack.is_empty());
    }

    #[test]
    fn test_resolve_import_path() {
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let loader = ModuleLoader::new(vfs);

        // Test relative path resolution
        assert_eq!(
            loader.resolve_import_path("/app/handler.js", "./utils").unwrap(),
            "/app/utils"
        );
        assert_eq!(
            loader.resolve_import_path("/app/handler.js", "../helpers").unwrap(),
            "/helpers"
        );
        assert_eq!(
            loader.resolve_import_path("/app/handler.js", "./lib/helper").unwrap(),
            "/app/lib/helper"
        );
        assert_eq!(
            loader.resolve_import_path("/handler.js", "./utils.js").unwrap(),
            "/utils.js"
        );

        // Test absolute paths
        assert_eq!(
            loader.resolve_import_path("/app/handler.js", "/absolute").unwrap(),
            "/absolute"
        );
    }

    #[test]
    fn test_circular_import_detection() {
        let vfs = IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        );
        let mut loader = ModuleLoader::new(vfs);

        // Initially not circular
        assert!(!loader.is_circular_import("/a.js"));

        // Push modules onto loading stack
        loader.push_loading("/a.js");
        loader.push_loading("/b.js");

        // Now a.js is circular (it's in the stack)
        assert!(loader.is_circular_import("/a.js"));
        // c.js is not
        assert!(!loader.is_circular_import("/c.js"));

        // Pop and verify
        loader.pop_loading();
        loader.pop_loading();
        assert!(!loader.is_circular_import("/a.js"));
    }

    #[test]
    fn test_detect_module_type_esm() {
        assert_eq!(detect_module_type("export default { fetch(req) {} }"), ModuleType::ESM);
        assert_eq!(detect_module_type("export function fetch(req) {}"), ModuleType::ESM);
        assert_eq!(detect_module_type("import { x } from './mod.js';\nconst y = 1;"), ModuleType::ESM);
    }

    #[test]
    fn test_detect_module_type_classic() {
        assert_eq!(detect_module_type("function fetch(req) { return { status: 200 }; }"), ModuleType::Script);
        // string literal containing "export " must not trigger ESM detection
        assert_eq!(detect_module_type("const msg = \"please export your config\";"), ModuleType::Script);
        assert_eq!(detect_module_type("var x = 1;"), ModuleType::Script);
    }

    #[test]
    fn test_detect_module_type_comment_skip() {
        // export in a comment should not trigger ESM
        assert_eq!(detect_module_type("// export function fetch() {}\nfunction fetch(req) {}"), ModuleType::Script);
    }
}
