//! Node `assert` module callbacks.

use super::extract_string_arg;

// ============== Assert module callbacks ==============

pub(super) fn assert_ok(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let passed = args.length() > 0 && {
        let val = args.get(0);
        // Truthy check: reject null, undefined, false, 0, ""
        !val.is_null_or_undefined()
            && !val.is_false()
            && val
                .to_string(scope)
                .map(|s| !s.to_rust_string_lossy(scope).is_empty())
                .unwrap_or(true)
            && val
                .to_number(scope)
                .map(|n| n.value() != 0.0)
                .unwrap_or(true)
    };
    if !passed {
        let msg =
            extract_string_arg(scope, &args, 1).unwrap_or_else(|| "Assertion failed".to_string());
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

pub(super) fn assert_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 {
        return;
    }
    let a = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let b = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if a != b {
        let msg = extract_string_arg(scope, &args, 2).unwrap_or_else(|| format!("{} != {}", a, b));
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

pub(super) fn assert_strict_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 {
        return;
    }
    if !args.get(0).strict_equals(args.get(1)) {
        let msg = extract_string_arg(scope, &args, 2)
            .unwrap_or_else(|| "Strict equality assertion failed".to_string());
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

pub(super) fn assert_not_equal(
    scope: &mut v8::PinnedRef<v8::HandleScope>,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    if args.length() < 2 {
        return;
    }
    let a = args
        .get(0)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let b = args
        .get(1)
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if a == b {
        let msg = extract_string_arg(scope, &args, 2).unwrap_or_else(|| format!("{} == {}", a, b));
        if let Some(msg_str) = v8::String::new(scope, &msg) {
            let error = v8::Exception::error(scope, msg_str);
            scope.throw_exception(error);
        }
    }
}

