//! Integration tests for HMAC signing/verification
//!
//! Tests HMAC key generation, sign/verify roundtrip,
//! signature tampering detection, and JWK import/export.

use nano::runtime::apis::RuntimeAPIs;
use nano::v8::{initialize_platform, NanoIsolate};

fn init_platform() {
    initialize_platform().expect("Failed to initialize V8 platform");
}

/// Helper function to execute code with proper V8 v147 scopes
fn with_nano_context<F, R>(isolate: &mut NanoIsolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    let isolate_ptr = isolate.isolate();
    v8::scope!(handle_scope, isolate_ptr);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

#[test]
fn test_hmac_sign_verify_roundtrip() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    let result = with_nano_context(&mut isolate, |scope, context| {
        RuntimeAPIs::bind_all(scope, context);

        let code = r#"
            try {
                // Generate HMAC key
                const key = crypto.subtle.generateKey(
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign", "verify"]
                );
                
                // Sign data
                const message = new TextEncoder().encode("Hello, HMAC World!");
                const signature = crypto.subtle.sign("HMAC", key, message);
                
                // Verify signature length (32 bytes for SHA-256)
                const correctLength = signature.byteLength === 32;
                
                // Verify signature
                const valid = crypto.subtle.verify("HMAC", key, signature, message);
                
                correctLength && valid ? "true" : "false"
            } catch (e) {
                "Error: " + e.message
            }
        "#;

        let code_string = v8::String::new(scope, code).unwrap();
        let script =
            v8::Script::compile(scope, code_string, None).expect("Script compilation failed");

        let result = script.run(scope).expect("Script execution failed");
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "true", "HMAC sign/verify roundtrip should work");
}

#[test]
fn test_hmac_signature_tampering_detected() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    let result = with_nano_context(&mut isolate, |scope, context| {
        RuntimeAPIs::bind_all(scope, context);

        let code = r#"
            try {
                const key = crypto.subtle.generateKey(
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign", "verify"]
                );
                
                const message = new TextEncoder().encode("Test message");
                const signature = crypto.subtle.sign("HMAC", key, message);
                
                // Tamper with signature - need Uint8Array view
                const sigView = new Uint8Array(signature);
                sigView[0] ^= 0xFF;
                
                // Verify should return false, not throw
                const valid = crypto.subtle.verify("HMAC", key, signature, message);
                
                valid === false ? "true" : "false"
            } catch (e) {
                "Error: " + e.message
            }
        "#;

        let code_string = v8::String::new(scope, code).unwrap();
        let script =
            v8::Script::compile(scope, code_string, None).expect("Script compilation failed");

        let result = script.run(scope).expect("Script execution failed");
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(
        result, "true",
        "HMAC signature tampering should be detected"
    );
}

#[test]
fn test_hmac_key_export_jwk() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    let result = with_nano_context(&mut isolate, |scope, context| {
        RuntimeAPIs::bind_all(scope, context);

        let code = r#"
            try {
                const key = crypto.subtle.generateKey(
                    { name: "HMAC", hash: "SHA-256" },
                    true,
                    ["sign", "verify"]
                );
                
                // Export as JWK
                const jwk = crypto.subtle.exportKey("jwk", key);
                
                // Verify JWK properties
                const valid = jwk && 
                    jwk.kty === "oct" &&
                    jwk.alg === "HS256" &&
                    typeof jwk.k === "string";
                
                valid ? "true" : "false"
            } catch (e) {
                "Error: " + e.message
            }
        "#;

        let code_string = v8::String::new(scope, code).unwrap();
        let script =
            v8::Script::compile(scope, code_string, None).expect("Script compilation failed");

        let result = script.run(scope).expect("Script execution failed");
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "true", "HMAC key JWK export should work");
}

#[test]
fn test_hmac_key_import_jwk() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");

    let result = with_nano_context(&mut isolate, |scope, context| {
        RuntimeAPIs::bind_all(scope, context);

        let code = r#"
            try {
                // Import JWK key
                const jwk = {
                    kty: "oct",
                    alg: "HS256",
                    k: "dGVzdC1zZWNyZXQta2V5LTEyMw"
                };
                
                const key = crypto.subtle.importKey(
                    "jwk",
                    jwk,
                    { name: "HMAC", hash: "SHA-256" },
                    false,
                    ["sign", "verify"]
                );
                
                // Use the imported key
                const message = new TextEncoder().encode("test message");
                const signature = crypto.subtle.sign("HMAC", key, message);
                const valid = crypto.subtle.verify("HMAC", key, signature, message);
                
                valid ? "true" : "false"
            } catch (e) {
                "Error: " + e.message
            }
        "#;

        let code_string = v8::String::new(scope, code).unwrap();
        let script =
            v8::Script::compile(scope, code_string, None).expect("Script compilation failed");

        let result = script.run(scope).expect("Script execution failed");
        let result_str = result.to_string(scope).unwrap().to_rust_string_lossy(scope);
        result_str
    });

    assert_eq!(result, "true", "HMAC key JWK import should work");
}
