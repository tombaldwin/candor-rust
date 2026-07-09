#!/usr/bin/env bash
# Wrapper smoke lane: drive EVERY cargo-candor subcommand end to end on a tiny fixture, pinning the
# exit-code contract — especially the FAIL-CLOSED negatives (a gate that could not run must exit 2,
# never 0). integration.sh covers the deep scenarios; this lane is the cheap "no subcommand is broken
# or silently fail-open" sweep the wrapper (plain bash, no unit tests) has shipped real bugs without.
# Run from the repo root: bash ci/wrapper-smoke.sh   (expects the workspace + lint already built)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CC="$ROOT/cargo-candor"

pass=0; fail=0
ok()   { echo "  ok   $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL $1"; fail=$((fail+1)); }
# assert_rc <desc> <want-rc> <got-rc>
assert_rc() { if [ "$3" -eq "$2" ]; then ok "$1 (exit $2)"; else bad "$1 — exit $3, want $2"; fi; }
# assert_has <desc> <haystack> <needle>
assert_has() { if printf '%s' "$2" | grep -qF -- "$3"; then ok "$1"; else bad "$1 — missing: $3"; fi; }

# ── fixture: one tiny crate with a transitive Fs effect behind a `domain` boundary ────────────────
D=$(mktemp -d)/smoke; mkdir -p "$D/src"
printf '[package]\nname="smoke"\nversion="0.1.0"\nedition="2021"\n' > "$D/Cargo.toml"
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > "$D/src/main.rs"
echo "deny Fs  domain" > "$D/deny.policy"
echo "deny Net domain" > "$D/clean.policy"
cd "$D"

echo "== help / unknown command =="
rc=0; out=$("$CC" help 2>&1) || rc=$?
assert_rc "help" 0 "$rc"; assert_has "help lists impact" "$out" "impact"
rc=0; out=$("$CC" bogus 2>&1) || rc=$?
assert_rc "unknown command" 2 "$rc"; assert_has "unknown-command usage lists containment" "$out" "containment"

echo "== scan (stable backend) =="
rc=0; "$CC" scan >/dev/null 2>&1 || rc=$?
assert_rc "scan" 0 "$rc"
[ -e .candor/report.smoke.scan.json ] && ok "scan wrote the report" || bad "scan report missing"

echo "== snapshot / guard / diff (nightly lint) =="
rc=0; "$CC" snapshot .candor/base >/dev/null 2>&1 || rc=$?
assert_rc "snapshot" 0 "$rc"
rc=0; "$CC" guard .candor/base >/dev/null 2>&1 || rc=$?
assert_rc "guard (clean)" 0 "$rc"
rc=0; out=$("$CC" guard .candor/nosuch 2>&1) || rc=$?
assert_rc "guard with NO baseline fails closed" 2 "$rc"
assert_has "…and names the snapshot incantation" "$out" "cargo candor snapshot"
rc=0; out=$("$CC" diff .candor/base 2>&1) || rc=$?
assert_rc "diff (no changes)" 0 "$rc"

echo "== read-only queries (served from the report) =="
for q in "show domain_logic" "where Fs" "callers leaf" "map" "containment" "reachable" "path domain_logic Fs" "impact leaf"; do
  rc=0; "$CC" $q >/dev/null 2>&1 || rc=$?
  assert_rc "$q" 0 "$rc"
done
rc=0; "$CC" whatif domain_logic Net >/dev/null 2>&1 || rc=$?
assert_rc "whatif (no policy violation)" 0 "$rc"
rc=0; "$CC" explain domain_logic >/dev/null 2>&1 || rc=$?
assert_rc "explain" 0 "$rc"
rc=0; "$CC" risk >/dev/null 2>&1 || rc=$?
assert_rc "risk (advisory, exit 0)" 0 "$rc"

echo "== policy: violation / clean / --gate-json / fail-closed =="
rc=0; "$CC" policy deny.policy >/dev/null 2>&1 || rc=$?
assert_rc "policy (violation)" 1 "$rc"
rc=0; "$CC" policy clean.policy >/dev/null 2>&1 || rc=$?
assert_rc "policy (clean)" 0 "$rc"
rc=0; "$CC" policy deny.policy --gate-json verdict.json >/dev/null 2>&1 || rc=$?
assert_rc "policy --gate-json (violation)" 1 "$rc"
if [ -s verdict.json ] && grep -q '"ok": false' verdict.json && grep -q '"AS-EFF-006"' verdict.json; then
  ok "verdict.json carries { ok:false, AS-EFF-006 }"
else
  bad "verdict.json wrong/missing: $(cat verdict.json 2>/dev/null | head -5)"
fi
rc=0; out=$("$CC" policy nosuch.policy 2>&1) || rc=$?
assert_rc "policy with a missing file" 2 "$rc"
# build-broken crate → the gate cannot evaluate → exit 2, never "policy OK"
cp src/main.rs src/main.rs.good
printf 'fn main() { broken\n' > src/main.rs
rc=0; out=$("$CC" policy deny.policy 2>&1) || rc=$?
assert_rc "policy on a build-broken crate fails closed" 2 "$rc"
assert_has "…and says NOT evaluated" "$out" "policy NOT evaluated"
mv src/main.rs.good src/main.rs

echo "== .candor/config discovery =="
mkdir -p .candor
printf 'policy deny.policy\nunknownkey x\n' > .candor/config
rc=0; out=$("$CC" policy 2>&1) || rc=$?
assert_rc "config-supplied policy drives the gate" 1 "$rc"
assert_has "unknown config key warns" "$out" "unknown config key"
printf 'policy\n' > .candor/config
rc=0; out=$("$CC" policy 2>&1) || rc=$?
assert_rc "bare 'policy' config line fails closed" 2 "$rc"
rm -f .candor/config
rc=0; out=$(CANDOR_CONFIG=/no/such "$CC" audit 2>&1) || rc=$?
assert_rc "unusable CANDOR_CONFIG fails closed" 2 "$rc"

echo "== guard --gate-json (AS-EFF-005) =="
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > src/main.rs
rc=0; "$CC" guard .candor/base --gate-json gv.json >/dev/null 2>&1 || rc=$?
assert_rc "guard --gate-json (gain)" 1 "$rc"
grep -q '"AS-EFF-005"' gv.json && ok "guard verdict carries AS-EFF-005" || bad "guard verdict wrong: $(cat gv.json 2>/dev/null | head -5)"

cd /; rm -rf "$(dirname "$D")"
echo
echo "wrapper smoke: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
