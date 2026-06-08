#!/usr/bin/env bash
# verify-laundered.sh — teeth for AS-EFF-009 layering through a THIRD crate (the hardest case).
#
# Three crates: app -> util -> infra. The `domain` module (in app) calls `util::store`; `util::store`
# (a crate away, invisible at the call site) reaches `infra`. Policy: `forbid domain -> infra`. The
# dependency is *laundered* through `util`, so following it requires util's layering-reachability
# sidecar — which the workspace gate (`cargo candor policy`) produces. Expected: the gate blocks
# (exit 1) and flags domain::checkout; a pure domain function (subtotal) is not flagged.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WS="$ROOT/layering-laundered"
cd "$WS"

# find_lib in cargo-candor needs the build; ensure the workspace compiles.
cargo build -q 2>/dev/null || { echo "FAIL: workspace does not compile"; exit 1; }
rm -rf target/dylint

out="$(bash "$ROOT/../../cargo-candor" policy .candor/policy 2>&1)"; rc=$?
echo "$out" | grep -E '\[AS-EFF-009\]' || true
echo "---"
fail=0
if [ "$rc" -eq 1 ] && echo "$out" | grep -q 'AS-EFF-009.*checkout'; then
  echo "PASS: laundered cross-crate dependency caught (domain::checkout -> util -> infra), gate blocked"
else
  echo "FAIL: expected the workspace gate to block (exit 1) and flag domain::checkout (got exit $rc)"; fail=1
fi
if echo "$out" | grep -q 'AS-EFF-009.*subtotal'; then
  echo "FAIL: pure domain::subtotal flagged (false positive)"; fail=1
else
  echo "PASS: pure domain::subtotal not flagged"
fi

rm -rf target/dylint
exit $fail
