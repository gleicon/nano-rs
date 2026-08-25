.PHONY: build test clean check fmt lint audit tla tla-counterexample loom static-profile coverage coverage-gate mutants mutants-changed doc run help

BINARY = nano-rs
CONFIG = config.json

help:
	@echo "NANO build commands:"
	@echo "  make build    - Build release binary"
	@echo "  make test     - Run all tests"
	@echo "  make check    - Fast check (no build)"
	@echo "  make fmt      - Format code"
	@echo "  make lint     - Run clippy"
	@echo "  make audit    - Run the honesty audit"
	@echo "  make tla      - Model-check the hot-swap protocol (TLA+)"
	@echo "  make loom     - Model-check SliverPoolSlot's real Rust (loom)"
	@echo "  make clean    - Clean build artifacts"
	@echo "  make doc      - Build documentation"
	@echo "  make run      - Build and run with config.json"

build:
	cargo build --release
	@echo "Binary: target/release/$(BINARY)"

test:
	cargo test --all

check:
	cargo check

fmt:
	cargo fmt

lint:
	cargo clippy --lib --bins --tests --all-features -- -D warnings

audit:
	./scripts/honesty-audit.sh

# Formal verification of the sliver hot-swap protocol. See formal/README.md.
# TLA+ model-checks the design; loom model-checks the real RwLock+Arc code.
#
# The checker jar is cached inside the repo (not a world-writable path like /tmp,
# where another user could plant a malicious jar that `java -cp` would execute),
# pinned by SHA256, and re-verified before every run so a cache-poisoned jar is
# never handed to the JVM.
TLA_JAR ?= $(CURDIR)/formal/.cache/tla2tools.jar
TLA_JAR_URL = https://github.com/tlaplus/tlaplus/releases/download/v1.7.1/tla2tools.jar
TLA_JAR_SHA256 = d532ba31aafe17afba1130f92410d9257454ff7393d1eb2fe032f0c07f352da5

# Download to a temp file, verify the checksum, then atomically move into place.
# -f fails on non-2xx (no silently-saved error pages); -S surfaces errors.
$(TLA_JAR):
	mkdir -p $(dir $(TLA_JAR))
	curl -fSL -o $(TLA_JAR).tmp $(TLA_JAR_URL)
	echo "$(TLA_JAR_SHA256)  $(TLA_JAR).tmp" | shasum -a 256 -c -
	mv $(TLA_JAR).tmp $(TLA_JAR)

# Re-check the pinned hash right before invoking the JVM, so a jar tampered with
# after caching can never be executed.
tla: $(TLA_JAR)
	echo "$(TLA_JAR_SHA256)  $(TLA_JAR)" | shasum -a 256 -c -
	cd formal && java -cp $(TLA_JAR) tlc2.TLC -config HotSwap.cfg HotSwap.tla

# Reproduce the counterexample for the buggy hard-kill variant. TLC exits
# non-zero when it finds the (expected) violation, so swallow that — printing the
# counterexample trace is the point of this target.
tla-counterexample: $(TLA_JAR)
	echo "$(TLA_JAR_SHA256)  $(TLA_JAR)" | shasum -a 256 -c -
	cd formal && java -cp $(TLA_JAR) tlc2.TLC -config HotSwap_HardKill.cfg HotSwap.tla || true

loom:
	cd formal/loom-slot && RUSTFLAGS="--cfg loom" cargo test --release

# Static serving "profile": a flamegraph synthesized from borescope's call graph
# (no runtime data). Confirms which subsystems are on the most code paths.
# Requires: `borescope index --no-git` + `cargo install inferno`.
static-profile:
	./scripts/static-profile.sh

# Line-coverage report across the whole codebase (the denominator for "what's
# untested"). Open target/llvm-cov/html/index.html after running with `--html`.
coverage:
	cargo llvm-cov --summary-only --ignore-filename-regex 'tests/'

# Regression gate: fail if line coverage drops below the established baseline.
# Raise COVERAGE_MIN as coverage improves so it ratchets up, never down.
COVERAGE_MIN ?= 68
coverage-gate:
	cargo llvm-cov --summary-only --ignore-filename-regex 'tests/' --fail-under-lines $(COVERAGE_MIN)

# Mutation testing — finds tests that assert nothing meaningful (false-green).
# Scope to a module: `make mutants FILE=src/admin/diagnostics.rs`.
# Runs lib tests only (fast); a surviving mutant means missing/weak assertions.
FILE ?= src/admin/diagnostics.rs
mutants:
	cargo mutants --file $(FILE) -- --lib

# Per-release mutation testing scoped to files changed since a git ref (default:
# the latest tag). Fast, targeted — mutation-tests only what a release touched.
# Override the ref: `make mutants-changed REF=origin/main`.
REF ?= $(shell git describe --tags --abbrev=0 2>/dev/null || echo origin/main)
mutants-changed:
	@files=$$(git diff --name-only $(REF)...HEAD -- 'src/**/*.rs' | grep -E '\.rs$$' || true); \
	if [ -z "$$files" ]; then echo "No changed src/*.rs files since $(REF) — nothing to mutate."; exit 0; fi; \
	echo "Mutating files changed since $(REF):"; echo "$$files" | sed 's/^/  /'; \
	args=$$(echo "$$files" | sed 's/^/--file /' | tr '\n' ' '); \
	cargo mutants $$args -- --lib

clean:
	cargo clean
	rm -rf target/

doc:
	cargo doc --no-deps --open

run: build
	./target/release/$(BINARY) --config $(CONFIG)

# Development build (faster)
dev:
	cargo build

# Run with logging
debug: dev
	RUST_LOG=debug ./target/debug/$(BINARY) --config $(CONFIG)

# Security targets
.PHONY: test-security test-cve-check test-cve-check-strict security-gate security-scan security-update-db test-all

test-security:
	@echo "Running adversarial security tests..."
	cargo test --test security_adversarial -- --test-threads=1

test-cve-check:
	@echo "Checking dependencies for CVEs..."
	cargo audit

test-cve-check-strict:
	@echo "Checking dependencies for CVEs (strict mode)..."
	cargo audit --deny warnings

security-gate: test-security test-cve-check
	@echo "✅ Security gate passed"

security-scan:
	@echo "Running full security scan..."
	cargo run --bin cve-scanner -- --severity high

security-update-db:
	@echo "Updating CVE database..."
	cargo audit --update

# Full test suite including security
test-all: test test-security test-cve-check
	@echo "✅ All tests passed including security"

# ── Integration test suite (HTTP black-box, generates HTML report) ─────────────
.PHONY: test-suite test-suite-build test-suite-report

test-suite:
	@./tests/suite/run.sh

test-suite-build:
	@./tests/suite/run.sh --build

test-suite-report:
	@echo "Latest report: reports/suite/latest.html"
	@ls -lh reports/suite/latest.html 2>/dev/null || echo "(no report yet — run make test-suite)"
