.PHONY: build test clean check fmt lint doc run help

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
