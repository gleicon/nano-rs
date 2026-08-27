# ADR-005: Rust Crypto Implementation Strategy

**Status:** Accepted  
**Date:** 2026-04-19  
**Deciders:** Core Team  
**Technical Story:** Implement WebCrypto via Rust crates vs using V8's built-in crypto

---

## Context and Problem Statement

WebCrypto API (`crypto.subtle`) needs implementation for:
- SHA-256/512 hashing
- AES-GCM encryption/decryption
- HMAC signing/verification
- Key generation and import/export

V8 has built-in `crypto.subtle` C++ implementation, but using it has drawbacks:
1. **Complex C++ bindings** — Need to bridge Rust ↔ C++ for every operation
2. **Less control** — V8 decides algorithm implementations
3. **Harder to audit** — C++ crypto code more error-prone than Rust
4. **Version coupling** — Tied to V8 version for updates

Alternative: Use Rust crypto crates (`ring`, `aes_gcm`) and expose via V8 bindings.

---

## Decision Drivers

* **Security** — Prefer auditable, memory-safe crypto code
* **Control** — Full control over algorithms and implementations
* **Safety** — Rust memory safety vs C++
* **Maintenance** — Easier to update dependencies independently
* **Performance** — Rust crypto highly optimized
* **Auditability** — Pure Rust easier to review than C++

---

## Considered Options

### Option 1: Rust Crypto (ring, aes_gcm)

Pure Rust, safe, auditable crypto.

### Option 2: V8 Built-in

Use V8's crypto.subtle implementation.

### Option 3: OpenSSL via FFI

Industry standard, but unsafe FFI bindings.

### Option 4: Rustls

TLS-focused, not general crypto.

---

## Decision Outcome

**Chosen option: "Rust Crypto"**

Implementation uses:
- **`ring` crate** — SHA-256, SHA-512, HMAC (audited, widely used)
- **`aes_gcm` crate** — AES-GCM encryption (pure Rust, safe)
- **`zeroize` crate** — Secure key material erasure on Drop

All operations happen in Rust, results passed to V8 as ArrayBuffers.

**Rationale:**
- Memory-safe crypto operations
- Easy to audit (pure Rust)
- Full algorithm control
- Simple dependency updates
- Consistent with Rust-first architecture

---

## Implementation Details

### Architecture

```
┌─────────────────────────────────────────────┐
│         JavaScript Layer                    │
│  crypto.subtle.encrypt({ name: "AES-GCM" })│
└──────────────┬──────────────────────────────┘
               │ V8 binding
               ▼
┌─────────────────────────────────────────────┐
│         Rust Runtime Layer                  │
│  - Parse algorithm parameters             │
│  - Validate key material                    │
│  - Call crypto crate                        │
└──────────────┬──────────────────────────────┘
               │ Rust call
               ▼
┌─────────────────────────────────────────────┐
│         Crypto Crates                       │
│  - ring::aead::Aes256Gcm                    │
│  - ring::hmac::HMAC                        │
│  - ring::digest::SHA256                    │
└─────────────────────────────────────────────┘
```

### Example: AES-GCM Encryption

```rust
// JavaScript: crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, data)

pub fn aes_gcm_encrypt(
    key: &[u8],
    iv: &[u8],
    plaintext: &[u8]
) -> Result<Vec<u8>, Error> {
    // Use ring crate (audited, safe)
    use ring::aead::{Aes256Gcm, Nonce, Aad};
    
    let key = UnboundKey::new(&AES_256_GCM, key)?;
    let nonce = Nonce::try_assume_unique_for_key(iv)?;
    
    // In-place encryption
    let mut ciphertext = plaintext.to_vec();
    let tag = seal_in_place_separate_tag(
        &key,
        nonce,
        Aad::empty(),
        &mut ciphertext,
    )?;
    
    // Append authentication tag
    ciphertext.extend_from_slice(tag.as_ref());
    Ok(ciphertext)
}
```

### Key Security

```rust
pub struct CryptoKey {
    algorithm: Algorithm,
    extractable: bool,
    usages: Vec<KeyUsage>,
    // Secure key material with automatic zeroization
    material: SecretVec<u8>,  // from zeroize crate
}

impl Drop for CryptoKey {
    fn drop(&mut self) {
        // zeroize crate automatically clears memory
        // Even if allocation not freed, key material is zeros
    }
}
```

---

## Algorithm Support

| Algorithm | Status | Crate | Notes |
|-----------|--------|-------|-------|
| SHA-256 | ✅ | ring | NIST-approved, audited |
| SHA-512 | ✅ | ring | NIST-approved, audited |
| AES-GCM | ✅ | aes_gcm | Pure Rust, safe |
| HMAC | ✅ | ring | Constant-time verify |
| RSA | ❌ Not implemented | rsa | — |
| ECDSA | ❌ Not implemented | p256 | — |
| deriveKey | ❌ Not implemented | hkdf | — |

---

## Positive Consequences

* **Memory-safe crypto operations** — Rust prevents buffer overflows
* **Easy to audit** — Pure Rust, no C++ complexity
* **Full algorithm control** — We decide implementations
* **Simple dependency updates** — `cargo update`, not V8 upgrade
* **Consistent with Rust-first architecture** — No C++ crypto code
* **Key material protection** — `zeroize` crate for secure erasure
* **Constant-time operations** — ring provides timing-safe implementations

---

## Negative Consequences

* **More initial implementation work** — Than using V8 built-in
* **Must re-implement WebCrypto API surface** — All methods need bindings
* **Async operations require Promise bridging** — Rust async → V8 Promise
* **Not "automatically" updated** — Manual dependency management
* **Potential API drift** — If WebCrypto spec changes

---

## Security Audit Trail

| Component | Audit Status | Source |
|-----------|--------------|--------|
| ring | ✅ Audited | Google, BoringSSL fork |
| aes_gcm | ✅ Formal verification | RustCrypto project |
| zeroize | ✅ Reviewed | Community, used by signal-protocol |

---

## Alternatives Rejected

### Option 2: V8 Built-in — Rejected

**Why:** C++ code harder to audit, less control, tied to V8 version. Complex bridging for every operation.

### Option 3: OpenSSL via FFI — Rejected

**Why:** FFI is unsafe (no borrow checker), OpenSSL has had vulnerabilities, adds external dependency.

### Option 4: Rustls — Rejected

**Why:** Rustls is TLS-focused, not general crypto. Doesn't provide AES-GCM, HMAC in public API.

---

## Async Handling

All `crypto.subtle.*` methods return Promises. Implementation:

```rust
pub fn subtle_encrypt(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
) {
    // Extract arguments in JS thread
    let algorithm = extract_algorithm(&args);
    let key = extract_key(&args);
    let data = extract_data(&args);
    
    // Create Promise
    let promise_resolver = v8::PromiseResolver::new(scope);
    let promise = promise_resolver.get_promise(scope);
    
    // Spawn async task
    tokio::spawn(async move {
        // Do crypto in async runtime (off JS thread)
        let result = crypto::encrypt(&algorithm, &key, &data).await;
        
        // Resolve Promise on JS thread
        isolate_ptr.post_task(move |scope| {
            match result {
                Ok(ciphertext) => {
                    let array_buffer = v8::ArrayBuffer::new(scope, ciphertext);
                    promise_resolver.resolve(scope, array_buffer.into());
                }
                Err(e) => {
                    let error = create_dom_exception(scope, &e);
                    promise_resolver.reject(scope, error);
                }
            }
        });
    });
}
```

---

## Related Decisions

* [ADR-007: ESM Strategy](007-esm-strategy.md) — Same Rust-first philosophy
* `docs/COMPATIBILITY.md` — WebCrypto coverage details
* `src/runtime/crypto/` — Implementation

---

## Code References

- `src/runtime/crypto/subtle.rs` — SubtleCrypto API
- `src/runtime/crypto/aes_gcm.rs` — AES-GCM implementation
- `src/runtime/crypto/hmac.rs` — HMAC implementation
- `src/runtime/crypto/crypto_key.rs` — Key management with zeroize

## Crates

- `ring` — Core crypto primitives
- `aes_gcm` — AES-GCM authenticated encryption
- `zeroize` — Secure memory erasure

---

*Last updated: 2026-04-19*
