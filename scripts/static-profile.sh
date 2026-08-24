#!/usr/bin/env bash
# Static serving profile: emulate a flamegraph from borescope's call graph (no
# runtime data). Width = distinct static call paths through each function.
# Requires: borescope index (.borescope/index.db) + inferno-flamegraph.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .borescope/index.db ] || { echo "run: borescope index --no-git"; exit 1; }
python3 scripts/static-profile.py
inferno-flamegraph \
  --title "nano-rs — Static Serving Profile (call-path frequency, no runtime data)" \
  --subtitle "router front + app loading + isolate serving | width = distinct static paths" \
  --countname paths \
  reports/static-profile/serving.folded > reports/static-profile/serving-flamegraph.svg
echo "wrote reports/static-profile/serving-flamegraph.svg"
