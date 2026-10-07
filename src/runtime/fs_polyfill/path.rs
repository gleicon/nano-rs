//! Node `path` module callbacks (pure path-string operations).

use super::extract_string_arg;

// ============== Path normalization (pure Rust) ==============

pub(super) fn normalize_path(path: &str) -> String {
    let is_absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let joined = parts.join("/");
    if is_absolute {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

// ============== Path module callbacks ==============

pub(super) fn path_join(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..args.length() {
        if let Some(s) = args.get(i).to_string(scope) {
            let s = s.to_rust_string_lossy(scope);
            if !s.is_empty() {
                parts.push(s);
            }
        }
    }
    let result = normalize_path(&parts.join("/"));
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

pub(super) fn path_dirname(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let result = match p.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
        None => ".".to_string(),
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

pub(super) fn path_basename(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let ext = extract_string_arg(scope, &args, 1);
    let base = p.split('/').last().unwrap_or("").to_string();
    let result = if let Some(ref e) = ext {
        if base.ends_with(e.as_str()) {
            base[..base.len() - e.len()].to_string()
        } else {
            base
        }
    } else {
        base
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

pub(super) fn path_extname(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let base = p.split('/').last().unwrap_or("");
    let result = match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_string(),
        _ => String::new(),
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

pub(super) fn path_resolve(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let mut parts: Vec<String> = Vec::new();
    for i in 0..args.length() {
        if let Some(s) = args.get(i).to_string(scope) {
            parts.push(s.to_rust_string_lossy(scope));
        }
    }
    let joined = parts.join("/");
    let normalized = normalize_path(&joined);
    let result = if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{}", normalized)
    };
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

pub(super) fn path_is_absolute(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    retval.set(v8::Boolean::new(scope, p.starts_with('/')).into());
}

pub(super) fn path_normalize_fn(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let p = extract_string_arg(scope, &args, 0).unwrap_or_default();
    let result = normalize_path(&p);
    if let Some(s) = v8::String::new(scope, &result) {
        retval.set(s.into());
    }
}

