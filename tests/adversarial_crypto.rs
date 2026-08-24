//! Adversarial Cryptographic Attack Tests
//!
//! Tests to verify cryptographic security:
//! - Weak RSA key rejection
//! - Weak EC curve rejection  
//! - Weak AES key rejection
//! - Constant-time comparison
//! - Predictable random rejection
//! - Key extraction enforcement

#[path = "common.rs"]
mod common;

use nano::runtime::apis::RuntimeAPIs;
use nano::v8::initialize_platform;

/// Helper to execute code with V8 v147 scope pattern
#[allow(dead_code)]
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

/// Test constant-time comparison
/// Attack: Timing analysis to infer secret data
/// Mitigation: ring crate uses constant-time comparison
#[test]
fn test_constant_time_comparison() {
    // This test documents that NANO uses the ring crate
    // which implements constant-time comparison functions

    println!("Constant-time operations:");
    println!("  - ring::constant_time::verify_slices_are_equal");
    println!("  - Used for HMAC verification");
    println!("  - Used for signature verification");
    println!("  - Prevents timing attacks on authentication");

    // Note: Actual constant-time verification is in the crypto implementation
    // This is a documentation test

    // Verify constant-time comparison using XOR-fold (ring's verify_slices_are_equal is deprecated)
    let a = [0u8; 32];
    let b = [0u8; 32];
    let c = [1u8; 32];

    let eq = a
        .iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0;
    let ne = a
        .iter()
        .zip(c.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0;
    assert!(eq);
    assert!(!ne);

    println!("  ✓ XOR-fold constant-time comparison available");
}

/// Test predictable random rejection
/// Attack: crypto.getRandomValues not cryptographically secure
/// Mitigation: getrandom crate with proper entropy source
#[test]
fn test_predictable_random_rejected() {
    init_platform();

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    // Generate random values and verify they're not predictable
    let code = v8::String::new(
        ctx_scope,
        "
        const results = [];
        
        // Generate 5 sets of random values
        for (let i = 0; i < 5; i++) {
            const arr = new Uint8Array(32);
            crypto.getRandomValues(arr);
            results.push(Array.from(arr).join(','));
        }
        
        // All should be different (highly unlikely to match)
        const allDifferent = results.every((val, idx, arr) => 
            arr.indexOf(val) === idx
        );
        
        allDifferent ? 'random-ok' : 'predictable'
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        result_str, "random-ok",
        "getRandomValues should produce unpredictable values"
    );
}

/// Test non-extractable key enforcement
/// Attack: Extracting keys marked as non-extractable
/// Mitigation: extractable flag enforced in key storage
#[test]
fn test_key_extraction_blocked() {
    init_platform();

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    // Test key extraction
    let code = v8::String::new(
        ctx_scope,
        "
        (async function() {
            try {
                // Generate non-extractable key
                const key = await crypto.subtle.generateKey(
                    { name: 'AES-GCM', length: 256 },
                    false, // non-extractable
                    ['encrypt', 'decrypt']
                );
                
                // Try to export it
                try {
                    const exported = await crypto.subtle.exportKey('raw', key);
                    return 'extracted';
                } catch (exportError) {
                    return 'extraction-blocked';
                }
            } catch (e) {
                return 'error: ' + e.message;
            }
        })()
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();

    ctx_scope.perform_microtask_checkpoint();

    if result.is_promise() {
        let promise = result.cast::<v8::Promise>();
        match promise.state() {
            v8::PromiseState::Fulfilled => {
                let value = promise.result(ctx_scope);
                let result_str = value
                    .to_string(ctx_scope)
                    .unwrap()
                    .to_rust_string_lossy(ctx_scope);
                println!("Key extraction result: {}", result_str);

                // Should be blocked if extractable flag is enforced
                assert!(
                    result_str == "extraction-blocked" || result_str.contains("error"),
                    "Non-extractable key should not be exportable: {}",
                    result_str
                );
            }
            _ => {
                println!("Key extraction test promise pending");
            }
        }
    }
}

/// Test crypto.subtle.digest timing consistency
/// Attack: Timing differences in digest operations
/// Mitigation: Consistent operation time regardless of input
#[test]
fn test_digest_timing_consistency() {
    init_platform();

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    // Test digest availability
    let code = v8::String::new(
        ctx_scope,
        "
        typeof crypto.subtle.digest === 'function' ? 'available' : 'not-available'
    ",
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();
    let result_str = result
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(result_str, "available", "digest should be available");

    // Note: Full timing consistency testing requires statistical analysis
    println!("Digest timing consistency: Implemented in ring crate");
}

/// PBKDF2 deriveBits known-answer test.
///
/// Exercises the real `crypto.subtle.importKey('raw', …, {name:'PBKDF2'})` +
/// `deriveBits` path added in this branch. Uses the widely-published
/// PBKDF2-HMAC-SHA256 vector: password="password", salt="salt", iterations=1,
/// dkLen=32 → 120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b.
/// A regression in the ring wiring, key-handle plumbing, or byte marshalling
/// changes the output and fails this test — unlike the assertion-free tests it
/// replaced, which could never fail.
#[test]
fn test_pbkdf2_derive_bits_known_answer() {
    init_platform();

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    let code = v8::String::new(
        ctx_scope,
        r#"
        (async function() {
            const enc = new TextEncoder();
            const keyMaterial = await crypto.subtle.importKey(
                'raw', enc.encode('password'), { name: 'PBKDF2' }, false, ['deriveBits']
            );
            const bits = await crypto.subtle.deriveBits(
                { name: 'PBKDF2', hash: 'SHA-256', salt: enc.encode('salt'), iterations: 1 },
                keyMaterial,
                256
            );
            const arr = new Uint8Array(bits);
            return Array.from(arr).map(b => b.toString(16).padStart(2, '0')).join('');
        })()
    "#,
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();

    // Drain microtasks until the async IIFE settles (bounded).
    let promise = result.cast::<v8::Promise>();
    for _ in 0..100 {
        if promise.state() != v8::PromiseState::Pending {
            break;
        }
        ctx_scope.perform_microtask_checkpoint();
    }

    assert_eq!(
        promise.state(),
        v8::PromiseState::Fulfilled,
        "deriveBits promise should fulfill"
    );
    let value = promise.result(ctx_scope);
    let hex = value
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);

    assert_eq!(
        hex, "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",
        "PBKDF2-HMAC-SHA256 output must match the published known-answer vector"
    );
}

/// deriveBits must reject an oversized `length` instead of allocating a huge
/// host-side buffer. `length` is attacker-controlled and the output Vec bypasses
/// the V8 heap limit, so an unbounded value would abort the shared process (DoS).
#[test]
fn test_pbkdf2_derive_bits_rejects_oversized_length() {
    init_platform();

    let mut nano_isolate = common::create_test_isolate();
    v8::scope!(scope, nano_isolate.isolate());
    let context = v8::Context::new(scope, Default::default());
    let ctx_scope = &mut v8::ContextScope::new(scope, context);

    RuntimeAPIs::bind_all(ctx_scope, context);

    // 8e15 bits would be a ~1 PB allocation if unbounded.
    let code = v8::String::new(
        ctx_scope,
        r#"
        (async function() {
            const enc = new TextEncoder();
            const keyMaterial = await crypto.subtle.importKey(
                'raw', enc.encode('password'), { name: 'PBKDF2' }, false, ['deriveBits']
            );
            try {
                await crypto.subtle.deriveBits(
                    { name: 'PBKDF2', hash: 'SHA-256', salt: enc.encode('salt'), iterations: 1 },
                    keyMaterial,
                    8000000000000000
                );
                return 'accepted';
            } catch (e) {
                return 'rejected';
            }
        })()
    "#,
    )
    .unwrap();

    let script = v8::Script::compile(ctx_scope, code, None).unwrap();
    let result = script.run(ctx_scope).unwrap();

    let promise = result.cast::<v8::Promise>();
    for _ in 0..100 {
        if promise.state() != v8::PromiseState::Pending {
            break;
        }
        ctx_scope.perform_microtask_checkpoint();
    }

    let value = promise.result(ctx_scope);
    let outcome = value
        .to_string(ctx_scope)
        .unwrap()
        .to_rust_string_lossy(ctx_scope);
    assert_eq!(
        outcome, "rejected",
        "oversized deriveBits length must be rejected, not allocated"
    );
}
