.PHONY: build test clean check fmt lint audit static-profile coverage coverage-gate mutants mutants-changed doc run help

BINARY = nano-rs
CONFIG = config.json

help:
	@echo "NANO build commands:"
	@echo "  make build    - Build release binary"
	@echo "  make test     - Run all tests"
	@echo "  make check    - Fast check (no build)"
	@echo "  make fmt      - Format code"
	@echo "  make lint     - Run clippy"
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
