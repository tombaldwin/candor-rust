#!/usr/bin/env bash
# verify-layering.sh — teeth for AS-EFF-009 CROSS-CRATE layering (forbid <A> -> <B> where B is a
# sibling crate). The `domain` module (in crate `app`) calls into a separate `infra` crate; the policy
# `forbid domain -> infra` must flag the domain functions that reach infra — directly (persist) and
# transitively (checkout) — and NOT a pure domain function (subtotal).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WS="$ROOT/layering-xcrate"
cd "$WS"

# shellcheck source=./_lib.sh
. "$ROOT/_lib.sh"
LIB="$(require_candor_lib "$ROOT/../../target/debug")" || exit 1

rm -rf target/dylint
cargo build -q 2>/dev/null || { echo "FAIL: workspace does not compile"; exit 1; }
out="$(CANDOR_POLICY="$WS/.candor/policy" cargo dylint --lib-path "$LIB" 2>&1)"
# Before either verdict below: prove candor RAN. `subtotal` is asserted ABSENT from this stream, and a
# stream candor never wrote satisfies that assertion perfectly — which is how a dead instrument reads as
# "detection broken, controls fine" instead of "nothing ran".
assert_live "AS-EFF-009 enforcement pass" "$out" || exit 1

echo "$out" | grep -E '\[AS-EFF-009\]' || true
echo "---"
fail=0
if echo "$out" | grep -q 'AS-EFF-009.*checkout'; then
  echo "PASS: cross-crate dependency flagged transitively (domain::checkout -> infra crate)"
else
  echo "FAIL: expected AS-EFF-009 on domain::checkout (transitive dep on the infra crate)"; fail=1
fi
if echo "$out" | grep -q 'AS-EFF-009.*subtotal'; then
  echo "FAIL: pure domain::subtotal flagged (false positive — it depends on nothing)"; fail=1
else
  echo "PASS: pure domain::subtotal not flagged"
fi

rm -rf target/dylint
exit $fail
