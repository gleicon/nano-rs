#!/usr/bin/env bash
# nano-rs integration test suite runner
# Usage: ./tests/suite/run.sh [--suite=foundation,wintertc,...] [--build]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="${NANO_BINARY:-$REPO_ROOT/target/release/nano-rs}"
REPORTS_DIR="${REPORTS_DIR:-$REPO_ROOT/reports/suite}"

# ── optional build ─────────────────────────────────────────────────────────────
if [[ "${1:-}" == "--build" ]]; then
  shift
  echo "Building nano-rs (release)..."
  (cd "$REPO_ROOT" && cargo build --release)
fi

# ── binary check ───────────────────────────────────────────────────────────────
if [[ ! -x "$BINARY" ]]; then
  echo "Error: binary not found at $BINARY"
  echo "Run with --build to compile, or set NANO_BINARY=/path/to/nano-rs"
  exit 1
fi

# ── node check ─────────────────────────────────────────────────────────────────
if ! command -v node &>/dev/null; then
  echo "Error: node is required to run the test suite"
  exit 1
fi

export NANO_BINARY="$BINARY"
export REPORTS_DIR="$REPORTS_DIR"
mkdir -p "$REPORTS_DIR"

exec node "$SCRIPT_DIR/runner/index.js" "$@"
