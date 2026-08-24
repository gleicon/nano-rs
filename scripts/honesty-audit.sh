#!/usr/bin/env bash
#
# honesty-audit.sh — mechanical guard against the failure classes we keep finding:
#   1. Fabricated / mocked data served as real from production code paths.
#   2. False-green tests (assert-nothing, silent env-gates, unjustified ignores).
#
# Each finding is a lead, not always a defect. Add a trailing `// audit-ok: <why>`
# comment to a line to exempt a genuinely-legitimate use.
#
# Exit non-zero if any un-exempted violation is found. Wire into CI via `make audit`.

set -uo pipefail
cd "$(dirname "$0")/.."

RED=$'\033[31m'; YEL=$'\033[33m'; GRN=$'\033[32m'; RST=$'\033[0m'
violations=0

# Filter out allow-listed lines and blank results.
allow() { grep -v 'audit-ok' || true; }

report() {
  local title="$1"; shift
  local hits="$1"
  if [[ -n "$hits" ]]; then
    echo "${RED}✗ ${title}${RST}"
    echo "$hits" | sed 's/^/    /'
    violations=$((violations + $(echo "$hits" | grep -c .)))
  else
    echo "${GRN}✓ ${title}${RST}"
  fi
}

# --- 1. Theater assertions: assert!(true ...) can never fail. -----------------
report "no assert!(true) theater" \
  "$(grep -rniE 'assert!\(\s*true' src/ tests/ --include='*.rs' | allow)"

# --- 2. Silent env-gated tests: pass vacuously when a var is unset. ------------
# The `if !<x>_enabled() { return; }` shape reports green without running.
report "no silent env-gated tests" \
  "$(grep -rEn 'if +! *[a-z_]+_enabled\(\)' tests/ --include='*.rs' | allow)"

# --- 3. Bare #[ignore] with no justification string. --------------------------
# `#[ignore = "reason"]` is fine; a bare ignore hides a test with no paper trail.
report "no unjustified #[ignore]" \
  "$(grep -rEn '^\s*#\[ignore\]\s*$' src/ tests/ --include='*.rs' | allow)"

# --- 4. Fabricated-data tells in production code. -----------------------------
# High-signal phrases that historically marked endpoints returning invented data.
FABRICATION='in real implementation|simulate with test data|for test/demo|as basic implementation|full implementation would|always returns ready|would track worker pools|for now, return'
report "no fabricated-data markers in src/" \
  "$(grep -rniE "$FABRICATION" src/ --include='*.rs' | allow)"

# --- 5. Tests that assert a hardcoded fabricated constant (drift canary). ------
# Catches re-introduction of the '42 + worker_id' style fabricated telemetry.
report "no fabricated telemetry constants" \
  "$(grep -rniE '42 \+ .*worker_id|worker_id % 2|memory_mb.*/ 4' src/ --include='*.rs' | allow)"

# --- 6. Unjustified #[allow(dead_code/unused*)] — where dead code hides. --------
# These attributes suppress the compiler's own dead-code detection. Require a
# same-line `//` justification (like the EPT sentinel) so each is a deliberate,
# reviewable choice — not a silent mask over code that should be deleted.
report "no unjustified #[allow(dead_code/unused)]" \
  "$(grep -rEn '#\[allow\((dead_code|unused[a-z_]*)\)\]' src/ --include='*.rs' | grep -vE '\]\s*//' | allow)"

# --- 7. Version-pinned capability claims in comments — verify, don't trust. -----
# We repeatedly concluded a feature was impossible from a stale "vNNN limitation"
# comment, when the current dependency actually supported it. Any comment that
# pins a capability to a specific version ("v147 limitation", "returns None in
# this build", "not supported in v150") must be re-verified against the current
# crate — annotate with `// audit-ok: verified <date>` once confirmed.
report "no unverified version-pinned capability claims" \
  "$(grep -rniE 'v[0-9]{2,3} (limitation|bindings dont|bindings do not)|returns None in this (v8 )?build|not (supported|exposed|available) (in|with) (the )?v[0-9]' src/ --include='*.rs' | allow)"

echo
if [[ "$violations" -gt 0 ]]; then
  echo "${RED}honesty-audit: ${violations} violation(s). Fix them or annotate with '// audit-ok: <why>'.${RST}"
  exit 1
fi
echo "${GRN}honesty-audit: clean.${RST}"
