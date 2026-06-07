#!/usr/bin/env bash
# check.sh — the architecture gate (treatment arm only). Runs candor with the
# project's policy (.candor/policy) and reports any AS-EFF-006 violations: a
# module performing an effect its declared boundary forbids. Exits non-zero if
# the boundary is violated, like a CI gate would. Placed in the crate root of a
# treatment working copy; the agent is told to run it and resolve any violation.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="$CANDOR_LIB"   # absolute path, exported by the runner

out="$(cd "$ROOT" && rm -rf target/dylint \
        && CANDOR_POLICY="$ROOT/.candor/policy" cargo dylint --lib-path "$LIB" 2>&1)"
viol="$(printf '%s\n' "$out" | grep 'AS-EFF-006' || true)"
if [ -n "$viol" ]; then
  echo "candor: ARCHITECTURE VIOLATION (this would fail CI):"
  printf '%s\n' "$viol"
  exit 1
fi
echo "candor: OK — no architecture-boundary violations."
