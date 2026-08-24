# NANO-RS Technical Debt Analysis

**Date:** 2026-05-06  
**Version:** 2.7.0 (v8-crate: 150.4.0)  
**Status:** Post-V8 v150 Migration

---

## Executive Summary

This document catalogs all technical debt, unused code, unimplemented features, and areas requiring future work identified during the V8 v150 migration.

---

## 1. Unused Code (Compiler Warnings)

### 1.1 CLI Output Module (`src/cli/output.rs`)
**Status:** Fully implemented but never used

**Unused Items:**
- `enum Style` - Associated items `ansi_code`, `reset_code` never used
- `static COLOR_ENABLED` - Never accessed
- `fn use_color()`, `set_color_enabled()` - Never called
- All styled output functions:
  - `styled()`, `success()`, `error()`, `warning()`, `info()`, `header()`, `dim()`, `bold()`
  - `print_success()`, `print_error()`, `print_warning()`, `print_info()`
  - `print_table()`, `print_list()`, `print_checklist()`, `print_section()`, `print_kv()`
  - `format_size()`, `format_duration()`, `format_timestamp()`
  - `indented()`, `wrap_text()`, `confirm()`, `print_boxed()`

**Recommendation:** Remove entire module or integrate with CLI

---

### 1.2 CLI Progress Module (`src/cli/progress.rs`)
**Status:** Fully implemented but never used

**Unused Items:**
- `struct ProgressBar` - Never constructed
  - Associated items: `new`, `set_message`, `inc`, `finish`, `finish_error`
- `struct Spinner` - Never constructed
  - Associated items: `new`, `tick`, `finish`, `finish_error`, `check_threshold`, `render`
- `const PROGRESS_THRESHOLD_MS` - Never used
- `const CHECK` - Never used
- `fn with_progress()` - Never used
- `fn with_spinner()` - Never used
- `fn is_tty()` - Never used

**Recommendation:** Remove or integrate with sliver creation/management

---

### 1.3 WebCrypto Subtle Module (`src/runtime/crypto/subtle.rs`)
**Status:** Partially implemented - many stub functions

**Unused Variables (stub parameters):**
- `extract_key()` - `extractable`, `usages`, `key_data`, `algorithm`
- `wrap_key()` - `key`, `wrapping_key`, `wrap_algorithm`
- `unwrap_key()` - `wrapped_key`, `unwrapping_key`, `unwrap_algorithm`, `unwrapped_key_algorithm`, `extractable`, `usages`
- `derive_key()` - `base_key`, `derived_key_algorithm`, `extractable`, `usages`
- `derive_bits()` - `base_key`, `length`
- `import_key()` - `format`, `key_data`, `algorithm`, `extractable`, `usages` (JWK variants)

**Recommendation:** Complete implementation or remove stubs

---

### 1.4 ECDSA Module (`src/runtime/crypto/ecdsa.rs`)
**Status:** Partial implementation with placeholder

**Not Implemented:**
- `verify_ecdsa()` - `salt_len`, `params` unused (PSS verification)
- Key import for ECDH - "simplified placeholder"

---

### 1.5 RSA Module (`src/runtime/crypto/rsa.rs`)
**Status:** Partial implementation

**Not Implemented:**
- PKCS#1 v1.5 signature - "requires different crate setup"
- PKCS#1 v1.5 verification - not implemented

---

### 1.6 HTTP Client (`src/http/client.rs`)
**Status:** Has TODO comments

**Issues:**
- TODO: `timeout` configured but uses reqwest default (line 31)
- Several tests use "mock response" (lines 419-451)

---

### 1.7 VFS Loader (`src/vfs/loader.rs`)
**Status:** Has unreachable pattern

**Issue:**
- Unreachable pattern for S3 backend (line 483)

---

### 1.8 Metrics/Tenant (`src/metrics/tenant.rs`)
**Status:** Has placeholder code

**Unused:**
- `regenerate_isolate_id()` method - never used
- Histogram buckets "reserved for future percentile calculations" (lines 568, 573)
- Prometheus metric family placeholder (line 674)

---

### 1.9 Worker Pool (`src/worker/pool.rs`)
**Status:** Has dead code

**Unused:**
- `init_code_cache()` function - never called

---

## 2. Placeholder/Future Features

### 2.1 WinterTC Handler Execution (`src/http/router.rs`)
**Status:** Placeholder only

**Lines 206-214:**
```rust
HandlerType::WinterTCHandler(_path) => {
    // Phase 3: Execute JavaScript handler
    // Router integration for handler execution is working
    // Full execution will be enabled after platform initialization fixes
    tracing::debug!("WinterTC handler for path: {} (Phase 3)", _path);
    NanoResponse::ok()
        .with_header("Content-Type", "text/plain")
        .with_body(format!("JS handler (Phase 3): {}", _path))
}
```

**Impact:** Router doesn't actually execute JS handlers - only returns placeholder

---

### 2.2 Module Import Resolution (`src/v8/module.rs`)
**Status:** Placeholder VFS

**Lines 514-520:**
```rust
// For now, we use a placeholder approach - in production, this should be
// the actual isolate's VFS
let vfs_placeholder = IsolateVfs::new(
    VfsNamespace::from_hostname("placeholder"),
    VfsBackendEnum::memory(MemoryBackend::default()),
);
```

**Line 699:**
```rust
// We need to determine the base path - for now, use a placeholder
```

---

### 2.3 Worker Pool Comments
**Status:** Future enhancement comments

**Line 1193:** "request affinity in later phases"

---

### 2.4 HTTP Config (`src/http/config.rs`)
**Status:** Future feature comments

**Line 44:** "Full configuration file support comes in Phase 5"
**Line 243:** "Register example routes (will be configurable in Phase 5)"

---

### 2.5 V8 Isolate Stub
**Status:** Future method stub

**Line 430 in `src/v8/isolate.rs`:**
```rust
/// This creates an isolate that can later be serialized to a snapshot blob.
pub fn into_snapshot_builder(self) {
    // This is a stub for future implementation
}
```

---

### 2.6 Sliver VFS Capture
**Status:** Not implemented

**Line 160 in `src/sliver/vfs_capture.rs`:**
```rust
tracing::debug!("walk_and_capture: recursive directory walking not yet implemented");
```

---

### 2.7 Sliver Validation
**Status:** Placeholder

**Line 296 in `src/sliver/validation.rs`:**
```rust
// For now, return a placeholder
```

---

### 2.8 Sliver Packager
**Status:** Placeholder heap

**Lines 126-171:** Create placeholder heap for directory-based slivers
- Uses `NANO_SNAPSHOT_PLACEHOLDER_V1` string instead of actual V8 snapshot

---

### 2.9 Unix Socket Admin
**Status:** Placeholder

**Line 274 in `src/admin/unix_socket.rs`:**
```rust
// a Unix socket placeholder.
```

---

## 3. Partial WebCrypto Implementation

### 3.1 Missing Algorithms

| Algorithm | Status | Location |
|-----------|--------|----------|
| RSA-PSS | Partial | `src/runtime/crypto/rsa.rs` |
| RSA-OAEP | Partial | `src/runtime/crypto/rsa.rs` |
| ECDSA | Partial | `src/runtime/crypto/ecdsa.rs` |
| ECDH | Placeholder | `src/runtime/crypto/ecdsa.rs:275` |
| deriveKey | Stub | `src/runtime/crypto/subtle.rs` |
| deriveBits | Stub | `src/runtime/crypto/subtle.rs` |
| wrapKey | Stub | `src/runtime/crypto/subtle.rs` |
| unwrapKey | Stub | `src/runtime/crypto/subtle.rs` |
| importKey (JWK) | Partial | `src/runtime/crypto/` |

---

## 4. CLI Integration Gaps

### 4.1 CLI Error Module (`src/cli/error.rs`)
**Line 132:**
```rust
// TODO: Re-enable helper constructors when CLI integration is complete
```

---

### 4.2 CLI Validation (`src/cli/validation.rs`)
**Line 125:**
```rust
/// available for future CLI enhancements.
```

---

## 5. Planned but Not Implemented (from Roadmap)

### 5.1 v2.0 Advanced Features (Phases 23-34)

| Phase | Feature | Status |
|-------|---------|--------|
| 23 | WebSocket Server | 📋 Planned |
| 24 | Advanced Crypto (RSA, ECDSA, deriveKey) | 📋 Planned |
| 25 | Compression Streams | 📋 Planned |
| 26 | Inter-Isolate Messaging | 📋 Planned |
| 27 | Production Multi-Tenancy | 📋 Planned |
| 28 | WASM Async Event Loop | ✅ Fixed (v150) |
| 29 | Missing Test Creation | 📋 Planned |
| 30 | Test Reporting Accuracy | 📋 Planned |
| 31 | WebCrypto Completion | 📋 Planned |
| 32 | CPU Limit Fixes | 📋 Planned |
| 33 | Adversarial & CF Fixes | 📋 Planned |
| 34 | Documentation Corrections | 📋 Planned |

---

## 6. Technical Debt Priorities

### P0 - Critical (Block Production)

1. **Router WinterTC Handler Placeholder** - Currently returns placeholder instead of executing JS
2. **Module Import VFS Placeholder** - Uses placeholder VFS instead of actual isolate VFS

### P1 - High (Important for Completeness)

1. **WebCrypto RSA/ECDSA Completion** - Missing key operations
2. **Remove Dead Code** - CLI output/progress modules
3. **Sliver Recursive Walking** - vfs_capture.rs not fully implemented

### P2 - Medium (Nice to Have)

1. **CLI Integration** - Connect output/progress modules
2. **HTTP Client TODO** - Timeout configuration
3. **Worker Pool Request Affinity** - Future enhancement

### P3 - Low (Future Versions)

1. **WebSocket Server** (Phase 23)
2. **Compression Streams** (Phase 25)
3. **Inter-Isolate Messaging** (Phase 26)

---

## 7. Recommendations

### Immediate Actions (v1.5.x)

1. **Remove or integrate CLI output/progress modules**
   - If not used in v1.5, remove to reduce binary size (45.9 MB)
   - If planned for v1.6, add to roadmap

2. **Fix Router WinterTC Handler**
   - Actually execute JS handlers or remove feature
   - Currently misleading (returns "JS handler (Phase 3)" but doesn't execute)

3. **Fix Module Import VFS**
   - Connect to actual isolate VFS
   - Placeholder prevents real ESM import resolution

### Short-term (v1.6)

1. Complete WebCrypto RSA/ECDSA
2. Implement deriveKey/deriveBits
3. Remove or implement Sliver placeholder heap
4. Add CRUD tests (currently missing)
5. Add Performance tests (currently missing)

### Long-term (v2.0)

1. WebSocket Server
2. Compression Streams  
3. Inter-Isolate Messaging
4. Production Multi-Tenancy

---

## 8. Binary Size Optimization

**Current:** 45.9 MB

**Potential Savings:**
- Remove CLI output/progress modules: ~500KB-1MB estimated
- Remove dead crypto code: ~200KB estimated
- **Total potential:** ~1-1.5 MB reduction

---

## 9. Test Coverage Gaps

### Missing Test Files (Claimed but Don't Exist)

1. CRUD operations test suite (6 tests claimed)
2. Performance benchmark tests (4 tests claimed)
3. Edge case tests (10 tests claimed)

### Partial Test Coverage

1. WebCrypto RSA - only basic operations tested
2. WebCrypto ECDSA - mostly stubbed
3. CPU limits - timeout behavior not fully tested
4. VFS S3 backend - no tests (marked unreachable)

---

## 10. Code Quality Issues

### Compiler Warnings (51 total)

- **Unused imports:** 2
- **Unreachable patterns:** 1 (VFS S3)
- **Unused mut:** 3
- **Dead code:** Multiple modules (output, progress, crypto stubs)

### Recommendations

1. Run `cargo fix --lib -p nano-rs` to auto-fix 48 warnings
2. Manually address remaining 3 warnings
3. Add `#[allow(dead_code)]` with justification for intentional stubs
4. Remove or implement placeholder code

---

## Summary

**Critical Issues:** 2 (Router placeholder, Module VFS placeholder)
**High Priority:** 5 (WebCrypto completion, dead code removal)
**Medium Priority:** 8 (CLI integration, various TODOs)
**Low Priority:** 10+ (Future v2.0 features)

**Estimated Cleanup Effort:** 2-3 days for P0/P1 items

