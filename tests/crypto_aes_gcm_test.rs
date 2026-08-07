//! Integration tests for AES-GCM encryption/decryption
//!
//! Tests AES-GCM key generation, encrypt/decrypt roundtrip,
//! JWK import/export, and authentication tag verification.

use nano::runtime::apis::RuntimeAPIs;
use nano::v8::{initialize_platform, NanoIsolate};

/// Helper to execute code with V8 v147 scope pattern
fn with_v8_context<F, R>(isolate: &mut v8::Isolate, f: F) -> R
where
    F: FnOnce(&mut v8::ContextScope<v8::HandleScope>, v8::Local<v8::Context>) -> R,
{
    v8::scope!(handle_scope, isolate);
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);
    f(ctx_scope, context)
}

fn init_platform() {
    initialize_platform().expect("Failed to initialize V8 platform");
}

#[test]
fn test_aes_gcm_encrypt_decrypt_roundtrip() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        (function() {
            try {
                // Generate key
                const key = crypto.subtle.generateKey(
                    { name: "AES-GCM", length: 256 },
                    true,
                    ["encrypt", "decrypt"]
                );
                if (key instanceof Error) {
                    return "Key generation error: " + key.message;
                }
                
                // Create IV
                const iv = crypto.getRandomValues(new Uint8Array(12));
                
                // Encrypt
                const plaintext = new TextEncoder().encode("Hello, AES-GCM World!");
                const ciphertext = crypto.subtle.encrypt(
                    { name: "AES-GCM", iv: iv },
                    key,
                    plaintext
                );
                
                if (ciphertext === undefined || ciphertext === null) {
                    return "ciphertext is " + ciphertext;
                }
                if (ciphertext instanceof Error) {
                    return "Encrypt error: " + ciphertext.message;
                }
                
                // Decrypt
                const decrypted = crypto.subtle.decrypt(
                    { name: "AES-GCM", iv: iv },
                    key,
                    ciphertext
                );
                
                if (decrypted === undefined || decrypted === null) {
                    return "decrypted is " + decrypted;
                }
                if (decrypted instanceof Error) {
                    return "Decrypt error: " + decrypted.message;
                }
                
                // Verify decrypted matches original
                const decoder = new TextDecoder();
                const decryptedText = decoder.decode(decrypted);
                const plaintextText = decoder.decode(plaintext);
                
                if (decryptedText === plaintextText) {
                    return "true";
                } else {
                    return "Mismatch: expected '" + plaintextText + "' got '" + decryptedText + "'";
                }
            } catch (e) {
                return "Exception: " + e.message;
            }
        })()
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    // Debug: print the result
    eprintln!("DEBUG AES-GCM roundtrip result: {}", result_str);

    assert_eq!(
        result_str, "true",
        "AES-GCM encrypt/decrypt roundtrip should work"
    );
}

#[test]
fn test_aes_gcm_different_key_lengths() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        try {
            // Test 128-bit key
            const key128 = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 128 },
                true,
                ["encrypt", "decrypt"]
            );
            const iv128 = crypto.getRandomValues(new Uint8Array(12));
            const data128 = new Uint8Array([1, 2, 3, 4, 5]);
            const ct128 = crypto.subtle.encrypt(
                { name: "AES-GCM", iv: iv128 },
                key128,
                data128
            );
            const pt128 = crypto.subtle.decrypt(
                { name: "AES-GCM", iv: iv128 },
                key128,
                ct128
            );
            const ok128 = pt128.byteLength === data128.byteLength;
            
            // Test 256-bit key
            const key256 = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 256 },
                true,
                ["encrypt", "decrypt"]
            );
            const iv256 = crypto.getRandomValues(new Uint8Array(12));
            const data256 = new Uint8Array([1, 2, 3, 4, 5]);
            const ct256 = crypto.subtle.encrypt(
                { name: "AES-GCM", iv: iv256 },
                key256,
                data256
            );
            const pt256 = crypto.subtle.decrypt(
                { name: "AES-GCM", iv: iv256 },
                key256,
                ct256
            );
            const ok256 = pt256.byteLength === data256.byteLength;
            
            ok128 && ok256 ? "true" : "false"
        } catch (e) {
            "Error: " + e.message
        }
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "true",
        "Both 128-bit and 256-bit AES-GCM should work"
    );
}

/// Test that AES-GCM correctly detects tampered ciphertext
/// This verifies the authentication tag is properly checked during decryption
#[test]
fn test_aes_gcm_tampered_ciphertext_fails() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        try {
            // Generate key and encrypt
            const key = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 256 },
                true,
                ["encrypt", "decrypt"]
            );
            const iv = crypto.getRandomValues(new Uint8Array(12));
            const plaintext = new TextEncoder().encode("Secret message");
            const ciphertext = crypto.subtle.encrypt(
                { name: "AES-GCM", iv: iv },
                key,
                plaintext
            );
            
            // Tamper with last byte (auth tag)
            const ctView = new Uint8Array(ciphertext);
            ctView[ctView.byteLength - 1] ^= 0xFF;
            
            // Decrypt should fail
            try {
                crypto.subtle.decrypt(
                    { name: "AES-GCM", iv: iv },
                    key,
                    ciphertext
                );
                "Should have failed";
            } catch (e) {
                "Tampering detected"
            }
        } catch (e) {
            "Error: " + e.message
        }
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    eprintln!("DEBUG tampered test result: {}", result_str);
    assert!(
        result_str.contains("Tampering detected"),
        "Should detect tampered ciphertext, got: {}",
        result_str
    );
}

#[test]
fn test_aes_gcm_with_additional_data() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        try {
            const key = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 256 },
                true,
                ["encrypt", "decrypt"]
            );
            const iv = crypto.getRandomValues(new Uint8Array(12));
            const plaintext = new TextEncoder().encode("Data with AAD");
            const aad = new TextEncoder().encode("authenticated metadata");
            
            // Encrypt with AAD
            const ciphertext = crypto.subtle.encrypt(
                { name: "AES-GCM", iv: iv, additionalData: aad },
                key,
                plaintext
            );
            
            // Decrypt with correct AAD should work
            const decrypted = crypto.subtle.decrypt(
                { name: "AES-GCM", iv: iv, additionalData: aad },
                key,
                ciphertext
            );
            const decoder = new TextDecoder();
            const ok = decoder.decode(decrypted) === "Data with AAD";
            
            ok
        } catch (e) {
            "Error: " + e.message
        }
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "true", "AES-GCM with AAD should work");
}

#[test]
fn test_aes_gcm_jwk_import_export() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        try {
            // Generate a key
            const key = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 256 },
                true,
                ["encrypt", "decrypt"]
            );
            
            // Export to JWK
            const jwk = crypto.subtle.exportKey("jwk", key);
            
            // Verify JWK properties
            const ok1 = jwk.kty === "oct";
            const ok2 = jwk.alg === "A256GCM";
            const ok3 = jwk.ext === true;
            const ok4 = Array.isArray(jwk.key_ops);
            const ok5 = typeof jwk.k === "string";
            
            // Import the JWK back
            const importedKey = crypto.subtle.importKey(
                "jwk",
                jwk,
                { name: "AES-GCM" },
                true,
                ["encrypt", "decrypt"]
            );
            
            // Test that imported key works
            const iv = crypto.getRandomValues(new Uint8Array(12));
            const plaintext = new TextEncoder().encode("Test message");
            const ciphertext = crypto.subtle.encrypt(
                { name: "AES-GCM", iv: iv },
                importedKey,
                plaintext
            );
            const decrypted = crypto.subtle.decrypt(
                { name: "AES-GCM", iv: iv },
                importedKey,
                ciphertext
            );
            const decoder = new TextDecoder();
            const ok6 = decoder.decode(decrypted) === "Test message";
            
            ok1 && ok2 && ok3 && ok4 && ok5 && ok6
        } catch (e) {
            "Error: " + e.message
        }
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "true", "AES-GCM JWK import/export should work");
}

/// Test that non-extractable keys cannot be exported
/// Verifies the extractable flag is enforced by exportKey
#[test]
fn test_non_extractable_key_cannot_export() {
    init_platform();

    let mut isolate = NanoIsolate::new().expect("Failed to create isolate");
    v8::scope!(handle_scope, isolate.isolate());
    let context = v8::Context::new(handle_scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(handle_scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = r#"
        try {
            // Generate non-extractable key
            const key = crypto.subtle.generateKey(
                { name: "AES-GCM", length: 256 },
                false, // non-extractable
                ["encrypt", "decrypt"]
            );
            
            // Try to export
            try {
                crypto.subtle.exportKey("jwk", key);
                "Should have failed";
            } catch (e) {
                "Export blocked: " + e.message
            }
        } catch (e) {
            "Error: " + e.message
        }
    "#;

    let code_string = v8::String::new(ctx_scope, code).unwrap();
    let script =
        v8::Script::compile(ctx_scope, code_string, None).expect("Script compilation failed");

    let result = script.run(ctx_scope).expect("Script execution failed");
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    eprintln!("DEBUG export test result: {}", result_str);
    assert!(
        result_str.contains("Export blocked"),
        "Should block export of non-extractable key, got: {}",
        result_str
    );
}
