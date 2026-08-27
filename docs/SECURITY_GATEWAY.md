# NANO Security Gateway

**Updated:** 2026-05-02  
**Status:** Production-Ready

## Overview

NANO implements defense-in-depth security to protect against malicious tenant code and common attack patterns. This document describes NANO's security architecture, guarantees, and adversarial test coverage.

## Security Guarantees

### 1. CPU Time Protection
- **Mechanism:** Per-request CPU time tracking with timer-based termination
- **Default Limit:** 50ms (matches Cloudflare Workers)
- **Mitigation:** Infinite loops, ReDoS attacks, and computationally expensive operations are terminated
- **Test Coverage:** 8 adversarial tests (infinite loops, ReDoS, algorithmic attacks, recursive bombs)

### 2. Memory Protection
- **Mechanism:** Memory monitoring with soft eviction and LRU replacement
- **Default Limit:** 128MB per isolate
- **Thresholds:**
  - Warning at 85% of limit
  - Soft eviction at 95% (draining mode, completes current requests)
  - Hard eviction at emergency levels
- **Mitigation:** Memory exhaustion attacks, allocation bombs, growth attacks
- **Test Coverage:** 7 adversarial tests (large arrays, string concatenation, buffer growth, closure leaks)

### 3. Filesystem Isolation
- **Mechanism:** VFS path validation with namespace isolation
- **Path Validation:** Rejects all `..` components regardless of encoding
- **Namespace Isolation:** Per-tenant VFS namespaces prevent cross-tenant file access
- **Mitigation:** Path traversal, symlink escapes, directory traversal attacks
- **Test Coverage:** 12 adversarial tests (basic traversal, URL encoding, null bytes, Unicode, symlinks)

### 4. Network Security
- **Mechanism:** Hostname-based routing, rate limiting, connection timeouts
- **Features:**
  - DNS rebinding protection via hostname validation
  - Request timeout enforcement (30s default)
  - Connection limits
- **Mitigation:** DNS rebinding, slowloris attacks, request flooding, SSRF
- **Test Coverage:** 6 adversarial tests (DNS rebinding, flooding, slowloris, header injection, SSRF)

### 5. JavaScript Context Isolation
- **Mechanism:** Fresh V8 context per request, restricted API surface
- **Features:**
  - No `eval()` exposed
  - No `Function` constructor
  - No `importScripts()`
  - Context reset between requests clears pollution attempts
- **Mitigation:** Prototype pollution, code injection, XSS
- **Test Coverage:** 8 adversarial tests (prototype pollution, constructor pollution, JSON.parse attacks)

### 6. WebAssembly Security
- **Mechanism:** WASM magic number and version validation before compilation
- **Features:**
  - Magic number verification (`\0asm`)
  - Version validation (v1, v2 supported)
  - V8's built-in sandbox for WASM execution
- **Mitigation:** Malicious WASM modules, validation bypasses
- **Test Coverage:** 12 adversarial tests (magic validation, version checks, malformed modules)

### 7. Multi-Tenant Isolation
- **Mechanism:** Per-tenant worker pools, isolated VFS namespaces, thread-local isolates
- **Features:**
  - Thread-local isolate ownership (!Send + !Sync)
  - Separate memory and CPU limits per tenant
  - No shared heap between isolates
  - VFS namespace isolation
- **Mitigation:** Cross-tenant data access, information disclosure, side-channel attacks
- **Test Coverage:** 7 adversarial tests (cross-tenant file access, memory isolation, hostname spoofing)

### 8. Cryptographic Security
- **Mechanism:** ring crate + subtle crate constant-time operations
- **Features:**
  - API key comparison: SHA-256 digest of both keys, then `subtle::ConstantTimeEq` — comparison time is independent of key length
  - Constant-time HMAC/verification for WebCrypto
  - Cryptographically secure random generation (getrandom)
  - Non-extractable key support
  - Weak key rejection (RSA < 2048, weak curves)
- **Mitigation:** Timing attacks (including length-based), weak keys, predictable randomness
- **Test Coverage:** 9 adversarial tests (weak RSA/EC/AES, constant-time, random validation)

## Adversarial Test Coverage

| Attack Vector | Tests | File | Coverage |
|---------------|-------|------|----------|
| CPU Exhaustion | 8 | `adversarial_cpu.rs` | Infinite loops, ReDoS, recursion, generators, crypto bombs |
| Memory Exhaustion | 7 | `adversarial_memory.rs` | Large arrays, string concat, buffer growth, closure leaks, circular refs |
| VFS Escape | 12 | `adversarial_vfs.rs` | Traversal, encoding, null bytes, Unicode, symlinks, absolute paths |
| Network Attacks | 6 | `adversarial_network.rs` | DNS rebinding, flooding, slowloris, header injection, SSRF |
| JS Injection | 8 | `adversarial_js_injection.rs` | Prototype pollution, eval, Function constructor, importScripts |
| WASM Attacks | 12 | `adversarial_wasm.rs` | Magic validation, version checks, malformed modules, host function abuse |
| Multi-Tenant Isolation | 7 | `adversarial_isolation.rs` | Cross-tenant access, memory isolation, hostname spoofing, side-channels |
| Cryptographic Attacks | 9 | `adversarial_crypto.rs` | Weak keys, timing attacks, random validation, key extraction |
| **Total** | **69** | - | Comprehensive coverage of 8 attack vectors |

## Running Security Tests

### Run All Security Tests
```bash
make test-security
```

### Run Specific Attack Vector Tests
```bash
# CPU exhaustion tests
cargo test --test security_adversarial -- adversarial_cpu

# VFS escape tests
cargo test --test security_adversarial -- adversarial_vfs

# Network attack tests
cargo test --test security_adversarial -- adversarial_network
```

### CVE Checking
```bash
# Basic CVE check
make test-cve-check

# Strict mode (deny warnings)
make test-cve-check-strict

# Update CVE database
make security-update-db

# Full security scan
make security-scan
```

### Security Gate (CI)
```bash
make security-gate
```

This runs both security tests and CVE checks, failing if either has issues.

## Trust Boundaries

```
┌─────────────────────────────────────────────────────────┐
│                    External HTTP                        │
│                   (Untrusted Input)                       │
└──────────────────┬────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│                   NANO Server                             │
│              (Request parsing, routing)                    │
└──────────────────┬────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│              Worker Pool (per hostname)                 │
│         (CPU/Memory limits, isolation)                  │
└──────────────────┬────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│              V8 Isolate (Thread-Local)                  │
│        (Fresh context per request, no eval)            │
└──────────────────┬────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│              VFS (Namespace Isolation)                  │
│         (Path validation, per-tenant data)              │
└──────────────────┬────────────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────────┐
│           Backend (Memory/Disk/S3)                      │
│         (Encrypted at rest, access logs)                │
└─────────────────────────────────────────────────────────┘
```

## CVE Response Process

### Severity SLA

| Severity | Response Time | Action |
|----------|---------------|--------|
| Critical | 24 hours | Emergency patch release |
| High | 7 days | Patch in next release |
| Medium | 30 days | Evaluate and patch if applicable |
| Low | 90 days | Track and patch in scheduled release |

### Process
1. **Detection:** `cargo audit` in CI or security monitoring
2. **Assessment:** Evaluate exploitability in NANO context
3. **Mitigation:** Update dependency or implement workaround
4. **Release:** Patch release with security advisory
5. **Communication:** Notify users via security advisory mailing list

### Exceptions
If a CVE is not applicable to NANO (e.g., affects features we don't use), document the justification in `.cargo/audit.toml`:

```toml
ignore = [
    { id = "RUSTSEC-YYYY-NNNN", reason = "NANO doesn't use the affected feature: <justification>" },
]
```

## Known Limitations

### 1. V8 Sandbox Escapes
- **Risk:** V8 vulnerabilities allowing escape from the JS/WASM sandbox
- **Mitigation:**
  - Keep V8 updated to latest stable release
  - Monitor Chromium Security Advisory
  - Use minimal V8 feature set
- **Status:** Depends on upstream V8 security

### 2. Spectre/Meltdown
- **Risk:** Side-channel attacks exploiting CPU speculative execution
- **Mitigation:**
  - Standard OS-level mitigations apply
  - Constant-time crypto operations for sensitive data
  - Thread-local isolates prevent cross-thread data access
- **Status:** Best-effort mitigations in place

### 3. Timing Side-Channels
- **Risk:** Operation timing differences revealing sensitive information
- **Mitigation:**
  - Constant-time comparison for crypto operations (ring crate)
  - Isolated execution contexts
  - No shared state between requests
- **Status:** Mitigated for crypto, best-effort for other operations

### 4. Resource Exhaustion (Host Level)
- **Risk:** Tenant exhausting host resources (disk, network bandwidth)
- **Mitigation:**
  - Per-tenant memory limits
  - CPU time limits
  - Request timeouts
- **Status:** Monitoring and alerting recommended for production

## Security Checklist for Production

Before deploying NANO in production:

- [ ] Enable CPU limits (50ms default recommended)
- [ ] Configure memory limits (128MB per tenant)
- [ ] Set request timeouts (30s default)
- [ ] Enable security tests in CI (`make security-gate`)
- [ ] Configure CVE scanning (daily via `.github/workflows/security.yml`)
- [ ] Review and customize `deny.toml` for your environment
- [ ] Set up security monitoring and alerting
- [ ] Document incident response procedures
- [ ] Review and approve any CVE exceptions in `.cargo/audit.toml`

## Reporting Security Issues

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please report them via:
- Email: security@nano-rs.dev (encrypted preferred)
- GPG Key: [Available in SECURITY.md]

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested mitigation (if any)

Response within 48 hours guaranteed.

## References

- [OWASP Top 10](https://owasp.org/www-project-top-ten/)
- [Chromium Security](https://www.chromium.org/Home/chromium-security/)
- [Rust Security Working Group](https://github.com/rust-secure/)
- [cargo-audit](https://github.com/RustSec/cargo-audit)
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)

---

**Test Coverage:** 69 adversarial tests passing  
**CVE Status:** Monitored daily via CI
