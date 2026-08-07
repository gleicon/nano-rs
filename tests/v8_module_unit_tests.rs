//! Unit tests for v8 module detection and transformation — extracted from src/v8/module.rs
//! Tests using private module internals remain embedded in the source file.
use nano::v8::module::{detect_module_type, ModuleType};
use nano::v8::{is_esm_module, transform_module_code};

#[test]
fn test_detect_module_type() {
    assert_eq!(
        detect_module_type("export default { fetch() {} }"),
        ModuleType::ESM
    );
    assert_eq!(detect_module_type("export const x = 1"), ModuleType::ESM);
    assert_eq!(
        detect_module_type("import { foo } from './bar'"),
        ModuleType::ESM
    );
    assert_eq!(detect_module_type("import('./dynamic')"), ModuleType::ESM);
    assert_eq!(detect_module_type("import{foo}from'bar'"), ModuleType::ESM);

    assert_eq!(
        detect_module_type("function fetch() {}"),
        ModuleType::Script
    );
    assert_eq!(detect_module_type("var x = 1"), ModuleType::Script);
    assert_eq!(
        detect_module_type("console.log('hello')"),
        ModuleType::Script
    );
}

#[test]
fn test_is_esm_module() {
    assert!(is_esm_module("export default {}"));
    assert!(!is_esm_module("function fetch() {}"));
}

#[test]
fn test_transform_module_code() {
    let esm = "export default { fetch: function() {} }";
    let transformed = transform_module_code(esm);
    assert!(transformed.contains("var __nano_handler ="));
    assert!(transformed.contains("var __nano_user_fetch = undefined"));
    assert!(transformed.contains("__nano_user_fetch = __nano_handler.fetch"));
    assert!(!transformed.contains("var fetch = __nano_handler.fetch"));

    let script = "function fetch() { return 1; }";
    let transformed = transform_module_code(script);
    assert_eq!(transformed, script);
}
