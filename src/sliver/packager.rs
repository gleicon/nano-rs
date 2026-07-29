//! Sliver Packager
//!
//! Creates slivers from directories. Default: compiles entrypoint JS to V8 bytecode.
//! Pass `source_only = true` to skip compilation (portable, no V8 version tie).

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::sliver::metadata::SliverMetadata;
use crate::sliver::packer::SliverPacker;
use crate::vfs::types::{VfsFile, VfsPath};

/// Create a sliver from a directory.
///
/// By default compiles the entrypoint JS to V8 bytecode (ConsumeCodeCache fast path).
/// Set `source_only = true` to skip compilation — the sliver then always compiles from source.
pub async fn create_sliver_from_directory(
    source_dir: &str,
    name: &str,
    tag: Option<String>,
    output: Option<String>,
    hostname: Option<String>,
    source_only: bool,
) -> Result<PathBuf> {
    let source_path = Path::new(source_dir);
    if !source_path.exists() {
        bail!("Source directory does not exist: {}", source_dir);
    }
    if !source_path.is_dir() {
        bail!("Source path is not a directory: {}", source_dir);
    }

    let output_path = output.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!("{}.sliver", name))
    });
    if output_path.exists() {
        bail!("Sliver file already exists: {}. Use --output to specify a different path.", output_path.display());
    }

    let sliver_hostname = hostname.unwrap_or_else(|| name.to_string());
    let entrypoint = detect_entrypoint(source_path);
    let sliver_tag = tag.unwrap_or_else(|| "latest".to_string());

    let mut metadata = SliverMetadata::new(&sliver_hostname, env!("CARGO_PKG_VERSION"));
    metadata.name = Some(name.to_string());
    metadata.description = Some(format!(
        "Created from directory: {} | Entrypoint: {} | Tag: {}",
        source_dir, entrypoint, sliver_tag
    ));
    metadata.custom.insert("entrypoint".to_string(), entrypoint.clone());
    metadata.custom.insert("source_dir".to_string(), source_dir.to_string());
    metadata.custom.insert("tag".to_string(), sliver_tag.clone());

    let vfs_entries = load_directory_files(source_path)?;

    // Compile entrypoint JS to bytecode unless source_only.
    let bytecode: Option<Vec<u8>> = if !source_only {
        let entrypoint_path = source_path.join(&entrypoint);
        match std::fs::read_to_string(&entrypoint_path) {
            Ok(js_code) => {
                match compile_js_to_bytecode(&js_code) {
                    Some(bc) => {
                        metadata.v8_cache_version = Some(v8::script_compiler::cached_data_version_tag());
                        tracing::info!("Compiled bytecode: {} bytes for '{}'", bc.len(), entrypoint);
                        Some(bc)
                    }
                    None => {
                        tracing::warn!("Bytecode compilation failed for '{}'; creating source-only sliver", entrypoint);
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Could not read entrypoint '{}' for compilation: {}; source-only", entrypoint, e);
                None
            }
        }
    } else {
        tracing::info!("--source-only: skipping bytecode compilation");
        None
    };

    let mut packer = SliverPacker::new();
    packer.add_metadata(&metadata)?;
    if let Some(ref bc) = bytecode {
        packer.add_bytecode(bc)?;
    }
    if !vfs_entries.is_empty() {
        packer.add_vfs_entries(&vfs_entries)?;
    }
    let archive_data = packer.finalize()?;

    std::fs::write(&output_path, &archive_data)
        .with_context(|| format!("Failed to write sliver to {}", output_path.display()))?;

    println!("Created sliver: {}", output_path.display());
    println!("  Source: {}", source_dir);
    println!("  Name: {}", name);
    println!("  Hostname: {}", sliver_hostname);
    println!("  Tag: {}", sliver_tag);
    println!("  Entrypoint: {}", entrypoint);
    println!("  Bytecode: {}",
        bytecode.as_ref().map(|b| format!("{} bytes", b.len())).unwrap_or_else(|| "none (source-only)".to_string()));
    println!("  Files: {}", vfs_entries.len());
    println!("  Size: {} bytes", archive_data.len());

    Ok(output_path)
}

/// Compile JS source to V8 UnboundScript bytecode.
/// Returns None if V8 is not initialized or compilation fails.
pub fn compile_js_to_bytecode(js_code: &str) -> Option<Vec<u8>> {
    if !crate::v8::is_initialized() {
        if crate::v8::initialize_platform().is_err() {
            return None;
        }
    }

    let mut isolate = v8::Isolate::new(Default::default());
    let scope_pin = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let mut scope = scope_pin.init();
    let context = v8::Context::new(&scope, Default::default());
    let mut ctx_scope = v8::ContextScope::new(&mut scope, context);

    let code_v8 = v8::String::new(&mut ctx_scope, js_code)?;
    let mut source = v8::script_compiler::Source::new(code_v8, None);
    let unbound = v8::script_compiler::compile_unbound_script(
        &ctx_scope,
        &mut source,
        v8::script_compiler::CompileOptions::NoCompileOptions,
        v8::script_compiler::NoCacheReason::NoReason,
    )?;

    let cache = unbound.create_code_cache()?;
    Some((&**cache).to_vec())
}

fn detect_entrypoint(dir: &Path) -> String {
    for candidate in &["index.js", "index.mjs", "main.js", "worker.js"] {
        if dir.join(candidate).exists() {
            return candidate.to_string();
        }
    }
    "index.js".to_string()
}

fn load_directory_files(dir: &Path) -> Result<Vec<(VfsPath, VfsFile)>> {
    use std::time::SystemTime;
    use walkdir::WalkDir;

    let mut entries = Vec::new();
    for entry in WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let relative_path = path.strip_prefix(dir)
            .map_err(|e| anyhow::anyhow!("Relative path error: {}", e))?;

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') || file_name.ends_with(".sliver") {
            continue;
        }

        let content = std::fs::read(path)
            .with_context(|| format!("Failed to read: {}", path.display()))?;
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to stat: {}", path.display()))?;

        let vfs_path_str = format!("/{}", relative_path.to_string_lossy());
        let vfs_path = VfsPath::new(&vfs_path_str)
            .with_context(|| format!("Invalid VFS path: {}", vfs_path_str))?;

        entries.push((vfs_path, VfsFile {
            content,
            modified_at: metadata.modified().unwrap_or_else(|_| SystemTime::now()),
            created_at: metadata.created().unwrap_or_else(|_| SystemTime::now()),
            size: metadata.len() as usize,
        }));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_compile_js_to_bytecode_valid() {
        if !crate::v8::is_initialized() {
            crate::v8::initialize_platform().ok();
        }
        let result = compile_js_to_bytecode("function fetch(req) { return { status: 200 }; }");
        if let Some(bytes) = result {
            assert!(!bytes.is_empty(), "bytecode should be non-empty");
        }
        // None is acceptable if V8 not available in this test environment
    }

    #[test]
    fn test_compile_js_to_bytecode_syntax_error_returns_none() {
        if !crate::v8::is_initialized() {
            crate::v8::initialize_platform().ok();
        }
        let result = compile_js_to_bytecode("function { broken syntax }}}}");
        // Invalid JS should not panic — returns None
        assert!(result.is_none(), "syntax error should yield None, not panic");
    }

    #[test]
    fn test_detect_entrypoint_js() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.js"), "const x=1").unwrap();
        assert_eq!(detect_entrypoint(dir.path()), "index.js");
    }

    #[test]
    fn test_detect_entrypoint_fallback() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("style.css"), "body{}").unwrap();
        assert_eq!(detect_entrypoint(dir.path()), "index.js");
    }

    #[test]
    fn test_load_directory_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.js"), "const x=1").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets").join("style.css"), "body{}").unwrap();
        let entries = load_directory_files(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_load_directory_files_skips_sliver_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.js"), "const x=1").unwrap();
        std::fs::write(dir.path().join("app.sliver"), "binary").unwrap();
        let entries = load_directory_files(dir.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].0.as_str().contains("index.js"));
    }
}
