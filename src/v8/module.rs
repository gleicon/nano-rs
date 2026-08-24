//! ESM Module Loader for V8 Module API (v150 Compatible)
//!
//! This module provides the infrastructure for executing ECMAScript Modules (ESM)
//! using V8's Module API instead of the classic Script API. This enables proper
//! support for `export default { fetch }` and `import` statements.
//!
//! The module loader integrates with the VFS for resolving relative imports
//! within the isolate's namespace.
//!
//! # V150 API Changes
//!
//! In v150:
//! - ContextScope requires 2 lifetime parameters: `ContextScope<'borrow, 'scope, P>`
//! - ContextScope implements Deref/DerefMut to PinnedRef<HandleScope>
//! - When passing scope to V8 APIs, use `&**scope` to dereference through the ContextScope

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
/// Scans non-comment lines for `export`/`import` keywords. Each line is split on `;`
/// so that handlers like `let counter = 0; export default { fetch }` are correctly
/// detected even when the export keyword isn't at the very start of the first line.
///
/// Block comments (`/* ... */`): if `/*` and `*/` both appear on the same line the
/// comment is treated as self-contained and that line is still scanned. A bare `/*`
/// with no matching `*/` on the same line enters block-comment mode; subsequent lines
/// are skipped until a line containing `*/` is found.
///
/// Dynamic `import()` mid-expression (`const x = await import('./m')`) is intentionally
/// **not** detected. Dynamic import is valid in classic script mode too, so a file with
/// only mid-line `import()` calls executes correctly whether treated as ESM or Script.
/// Only `import(` at the start of a fragment (after trimming) is treated as an ESM marker.
///
/// This is a heuristic, not a full parser. Rare false positives (a string literal that
/// starts with `export`) are accepted because misclassifying ESM as Script causes a V8
/// SyntaxError ("Unexpected token 'export'"), while misclassifying Script as ESM is
/// harmless — ESM mode is a strict superset for non-exporting code.
pub fn detect_module_type(code: &str) -> ModuleType {
    let mut in_block_comment = false;
    for line in code.lines() {
        let trimmed = line.trim();
        if in_block_comment {
            if trimmed.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("/*") {
            let open = trimmed.find("/*").unwrap();
            let close = trimmed.find("*/");
            // Only enter block-comment mode when /* is not closed on the same line.
            // This prevents a string like '/* comment */ code' from swallowing
            // all subsequent lines (the string literal case).
            if close.map(|c| c <= open).unwrap_or(true) {
                in_block_comment = true;
            }
        }
        // Split on `;` so that `let x = 0; export default {...}` is detected.
        // Each fragment is checked for ESM keywords at fragment start (after trim).
        for fragment in trimmed.split(';') {
            let frag = fragment.trim();
            if frag.starts_with("export ")
                || frag.starts_with("export{")
                || frag.starts_with("export default")
                || frag.starts_with("import ")
                || frag.starts_with("import{")
                || frag.starts_with("import(")
            {
                return ModuleType::ESM;
            }
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
        // nano: built-in modules — bypass VFS entirely
        if specifier.starts_with("nano:") {
            return Ok(specifier.to_string());
        }

        // Absolute VFS path
        if specifier.starts_with('/') {
            return Ok(specifier.to_string());
        }

        // Bare specifier — not relative, not absolute
        if !specifier.starts_with('.') {
            // Try VFS /node_modules first
            let vfs_nm = format!("/node_modules/{}/index.js", specifier);
            let vfs_check =
                crate::data_plane::with_worker_runtime(|h| h.block_on(self.vfs.exists(&vfs_nm)))
                    .unwrap_or_else(|| {
                        let rt = tokio::runtime::Runtime::new().expect("rt");
                        rt.block_on(self.vfs.exists(&vfs_nm))
                    })
                    .unwrap_or(false);
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
                    Err(_) => {
                        crate::data_plane::with_worker_runtime(|h| h.block_on(vfs.exists(&p)))
                            .unwrap_or_else(|| {
                                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                                rt.block_on(vfs.exists(&p))
                            })
                            .unwrap_or(false)
                    }
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

/// Module resolution callback for V8
///
/// This callback is invoked by V8 when a module has import statements.
/// It resolves the import specifier against the VFS and returns the
/// compiled module.
///
/// The signature matches V8's ResolveModuleCallback which is automatically
/// converted via MapFnFrom trait.
///
/// # V150 API Note
/// CallbackScope uses the same pin! + init() pattern as HandleScope.
pub(crate) fn module_resolve_callback<'a>(
    context: v8::Local<'a, v8::Context>,
    specifier: v8::Local<'a, v8::String>,
    _import_attributes: v8::Local<'a, v8::FixedArray>,
    _referrer: v8::Local<'a, v8::Module>,
) -> Option<v8::Local<'a, v8::Module>> {
    // Get the module loader from thread-local storage
    let loader_option = with_current_loader(|loader| loader.map(|l| l as *mut ModuleLoader));

    let loader_ptr = loader_option?;
    let loader = unsafe { &mut *loader_ptr };

    // Convert specifier to Rust string
    // v150 API: CallbackScope uses pin! + init() pattern
    let callback_scope = unsafe { v8::CallbackScope::new(context) };
    let callback_scope = std::pin::pin!(callback_scope);
    let callback_scope = callback_scope.init();
    // v150 API: to_rust_string_lossy expects &Isolate, get via Deref from PinnedRef
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
    if let Some(cached) = loader.get_cached(&resolved_path) {
        return Some(v8::Local::new(&*callback_scope, &cached));
    }

    // nano: built-in synthetic modules
    if resolved_path.starts_with("nano:") {
        use crate::runtime::kv::get_nano_module_code;
        let code = get_nano_module_code(&resolved_path)?;
        let resource_name = v8::String::new(&*callback_scope, &resolved_path)?;
        let source_map_url: Option<v8::Local<v8::Value>> =
            Some(v8::undefined(&*callback_scope).into());
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
        let code_str = v8::String::new(&*callback_scope, code)?;
        let mut source = v8::script_compiler::Source::new(code_str, Some(&origin));
        let module = v8::script_compiler::compile_module(&*callback_scope, &mut source)?;
        let global_module = v8::Global::new(&**callback_scope, module);
        loader.cache_module(&resolved_path, global_module.clone());
        return Some(v8::Local::new(&*callback_scope, &global_module));
    }

    // Load module from VFS
    let code = match loader.load_module_from_vfs(&resolved_path) {
        Ok(code) => code,
        Err(_) => return None,
    };

    // Track that we're loading this module
    loader.push_loading(&resolved_path);

    // Create origin for the module
    // v150 API: All V8 APIs that expect &PinnedRef<HandleScope> work with CallbackScope
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
    // v150 API: compile_module expects &PinnedRef<HandleScope>
    let module = match v8::script_compiler::compile_module(&*callback_scope, &mut source) {
        Some(m) => m,
        None => {
            loader.pop_loading();
            return None;
        }
    };

    // Cache the module
    // v150 API: Global::new expects &Isolate (accessed via Deref from PinnedRef)
    let global_module = v8::Global::new(&**callback_scope, module);
    loader.cache_module(&resolved_path, global_module.clone());

    // Pop from loading stack
    loader.pop_loading();

    // Return the module
    // v150 API: v8::Local::new expects &PinnedRef<HandleScope>
    Some(v8::Local::new(&*callback_scope, &global_module))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::{MemoryBackend, VfsNamespace};

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
            loader
                .resolve_import_path("/app/handler.js", "./utils")
                .unwrap(),
            "/app/utils"
        );
        assert_eq!(
            loader
                .resolve_import_path("/app/handler.js", "../helpers")
                .unwrap(),
            "/helpers"
        );
        assert_eq!(
            loader
                .resolve_import_path("/app/handler.js", "./lib/helper")
                .unwrap(),
            "/app/lib/helper"
        );
        assert_eq!(
            loader
                .resolve_import_path("/handler.js", "./utils.js")
                .unwrap(),
            "/utils.js"
        );

        // Test absolute paths
        assert_eq!(
            loader
                .resolve_import_path("/app/handler.js", "/absolute")
                .unwrap(),
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
        assert_eq!(
            detect_module_type("export default { fetch(req) {} }"),
            ModuleType::ESM
        );
        assert_eq!(
            detect_module_type("export function fetch(req) {}"),
            ModuleType::ESM
        );
        assert_eq!(
            detect_module_type("import { x } from './mod.js';\nconst y = 1;"),
            ModuleType::ESM
        );
    }

    #[test]
    fn test_detect_module_type_classic() {
        assert_eq!(
            detect_module_type("function fetch(req) { return { status: 200 }; }"),
            ModuleType::Script
        );
        // string literal containing "export " must not trigger ESM detection
        assert_eq!(
            detect_module_type("const msg = \"please export your config\";"),
            ModuleType::Script
        );
        assert_eq!(detect_module_type("var x = 1;"), ModuleType::Script);
    }

    #[test]
    fn test_detect_module_type_comment_skip() {
        // export in a // comment should not trigger ESM
        assert_eq!(
            detect_module_type("// export function fetch() {}\nfunction fetch(req) {}"),
            ModuleType::Script
        );
    }

    #[test]
    fn test_block_comment_same_line_does_not_swallow_export() {
        // /* */ open and close on the same line: block comment is self-contained,
        // so subsequent lines must still be scanned.
        // Regression: a string like `css: '/* styles */ body {}'` used to set
        // in_block_comment=true and skip all following lines including `export default`.
        let code = "const x = { css: '/* Next.js styles */ body {}' };\nexport default {};";
        assert_eq!(detect_module_type(code), ModuleType::ESM);

        // Real block comment spanning two lines still suppresses the export in between
        let suppressed = "/*\nexport default {}\n*/\nfunction fetch() {}";
        assert_eq!(detect_module_type(suppressed), ModuleType::Script);
    }

    #[test]
    fn test_dynamic_import_mid_expression_not_detected() {
        // await import() inside an assignment is intentionally NOT detected as ESM —
        // dynamic import() is valid in classic script mode so the file runs correctly
        // either way. This keeps the heuristic free of false positives from string
        // literals like `const url = "see import() docs"`.
        assert_eq!(
            detect_module_type("const lazy = await import('./lazy');"),
            ModuleType::Script
        );
        assert_eq!(
            detect_module_type("const url = \"see import() docs\";"),
            ModuleType::Script
        );
        // import() at the start of a line IS an ESM marker
        assert_eq!(
            detect_module_type("import('./lazy').then(m => use(m))"),
            ModuleType::ESM
        );
    }

    #[test]
    fn test_mid_line_export_detected() {
        // export default after a semicolon on the same line — the case that caused
        // 500 errors because the handler was classified as Script and V8 threw
        // "Unexpected token 'export'" in classic mode.
        assert_eq!(
            detect_module_type("let counter = 0; export default { async fetch() {} }"),
            ModuleType::ESM
        );
        assert_eq!(
            detect_module_type("const x = 1; const y = 2; export default { fetch() {} }"),
            ModuleType::ESM
        );
        // Genuinely no ESM keywords — still Script
        assert_eq!(
            detect_module_type("function fetch() { return 'hello'; }"),
            ModuleType::Script
        );
    }

    #[test]
    fn test_each_esm_keyword_form_detected_independently() {
        // Every alternative in the detect_module_type `||` chain must be a
        // sufficient ESM marker on its own. If any `||` flips to `&&`, one of
        // these inputs stops being detected and this test fails.
        for src in [
            "export const x = 1", // export<space>
            "export{x}",          // export{
            "export default {}",  // export default
            "import x from 'y'",  // import<space>
            "import{x}from'y'",   // import{
            "import('./m')",      // import(
        ] {
            assert_eq!(
                detect_module_type(src),
                ModuleType::ESM,
                "should be ESM: {src}"
            );
        }
    }

    #[test]
    fn test_is_esm_module_wrapper() {
        // Directly exercise the public bool wrapper so a constant-return mutant
        // (always true / always false) is caught.
        assert!(is_esm_module("export default {}"));
        assert!(!is_esm_module("function fetch() {}"));
    }

    #[test]
    fn test_transform_module_code_lib() {
        // Pure string transform — lib-covered so a `-> String::new()` mutant fails.
        let transformed = transform_module_code("export default { fetch: function() {} }");
        assert!(transformed.contains("__nano_handler"), "got: {transformed}");
        assert!(
            transformed.contains("__nano_user_fetch"),
            "got: {transformed}"
        );
        // A classic script is returned unchanged.
        let classic = "function fetch() { return 1; }";
        assert_eq!(transform_module_code(classic), classic);
    }

    // ── ModuleLoader VFS/path tests (previously uncovered) ───────────────────

    fn test_vfs() -> IsolateVfs {
        IsolateVfs::new(
            VfsNamespace::from_hostname("test.example.com"),
            crate::vfs::VfsBackendEnum::memory(MemoryBackend::default()),
        )
    }

    #[test]
    fn load_module_from_vfs_reads_and_errors() {
        let vfs = test_vfs();
        pollster::block_on(vfs.write("/mod.js", b"export const x = 1;")).unwrap();
        let loader = ModuleLoader::new(vfs);

        // Present → returns content.
        let content = loader.load_module_from_vfs("/mod.js").unwrap();
        assert!(content.contains("export const x"), "got: {content}");

        // Absent (not in VFS or on disk) → error, not a silent empty string.
        assert!(loader
            .load_module_from_vfs("/does-not-exist-xyz.js")
            .is_err());
    }

    #[test]
    fn resolve_import_path_appends_js_when_file_exists() {
        let vfs = test_vfs();
        pollster::block_on(vfs.write("/app/utils.js", b"export const y = 2;")).unwrap();
        let loader = ModuleLoader::new(vfs);

        // Bare "./utils" (no extension) resolves to the existing "/app/utils.js".
        // Pins the `.js`-extension branch (the `if !resolved.contains('.')` logic).
        assert_eq!(
            loader
                .resolve_import_path("/app/handler.js", "./utils")
                .unwrap(),
            "/app/utils.js"
        );
        // Path traversal past root is rejected.
        assert!(loader
            .resolve_import_path("/handler.js", "../../etc/passwd")
            .is_err());
    }
}
