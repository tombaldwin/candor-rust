#!/usr/bin/env bash
# verify.sh — teeth test for AS-EFF-008 host-allowlist enforcement ACROSS a crate boundary.
#
# The workspace under host-allowlist/ has a `billing` module (in crate `app`) whose only network
# access goes through a shared `httpkit` crate. The forbidden host (metrics.growthtracker.io) is a
# literal in httpkit — nowhere near billing. Policy: `allow Net in billing api.stripe.com …`.
#
# Expected: candor flags `billing::record_activity` (reaches the non-allowlisted host transitively,
# cross-crate) and NOT `billing::charge_customer` (reaches only api.stripe.com). Reaching the host
# detail requires cross-crate host propagation + CANDOR_REPORTS read-only resolution — so this is the
# end-to-end Bet 3 "enforce at workspace scale" path.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WS="$ROOT/host-allowlist"
cd "$WS"

LIB="$(cd "$ROOT/../../target/debug" && pwd)/$(basename "$(find "$ROOT/../../target/debug" -maxdepth 1 -name 'libcandor@*.dylib' -o -name 'libcandor@*.so' | head -1)")"
[ -e "$LIB" ] || { echo "FAIL: no candor dylib (run cargo build first)"; exit 1; }

rm -rf target/dylint r.*.json
cargo build -q 2>/dev/null || { echo "FAIL: workspace does not compile"; exit 1; }

# 1) Snapshot every workspace crate's report (writes r.httpkit.Rlib.json, r.app.Executable.json — with
#    per-fn `hosts`). CANDOR_JSON writes reports and suppresses enforcement.
CANDOR_JSON="$WS/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1

# 2) Enforce the policy WITH the siblings loaded read-only (CANDOR_REPORTS). No CANDOR_JSON here, so
#    enforcement runs; CANDOR_REPORTS resolves app's calls into httpkit, inheriting its hosts.
rm -rf target/dylint
out="$(CANDOR_POLICY="$WS/.candor/policy" CANDOR_REPORTS="$WS/r" cargo dylint --lib-path "$LIB" 2>&1)"

echo "$out" | grep -E '\[AS-EFF-008\]' || true
echo "---"

fail=0
if echo "$out" | grep -q 'AS-EFF-008.*record_activity'; then
  echo "PASS: forbidden cross-crate host (metrics.growthtracker.io) flagged on billing::record_activity"
else
  echo "FAIL: expected AS-EFF-008 on billing::record_activity (forbidden host reached cross-crate)"; fail=1
fi
if echo "$out" | grep -q 'AS-EFF-008.*charge_customer'; then
  echo "FAIL: billing::charge_customer flagged, but api.stripe.com IS on the allowlist (false positive)"; fail=1
else
  echo "PASS: allowed host (api.stripe.com) not flagged on billing::charge_customer"
fi

# 3) The SINGLE-COMMAND workspace gate: `cargo candor policy` snapshots the workspace then enforces
#    with siblings loaded, in one invocation. Must block (exit 1) on the cross-crate host violation.
echo "---"
rm -rf target/dylint r.*.json
wout="$(bash "$ROOT/../../cargo-candor" policy .candor/policy 2>&1)"; wrc=$?
if [ "$wrc" -eq 1 ] && echo "$wout" | grep -q 'AS-EFF-008.*record_activity'; then
  echo "PASS: single-command \`cargo candor policy\` blocks (exit 1) on the cross-crate violation"
else
  echo "FAIL: \`cargo candor policy\` should exit 1 and flag record_activity (got exit $wrc)"; fail=1
fi

rm -rf target/dylint r.*.json
exit $fail
