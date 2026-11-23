#!/usr/bin/env bash
# Wrapper smoke lane: drive EVERY cargo-candor subcommand end to end on a tiny fixture, pinning the
# exit-code contract — especially the FAIL-CLOSED negatives (a gate that could not run must exit 2,
# never 0). integration.sh covers the deep scenarios; this lane is the cheap "no subcommand is broken
# or silently fail-open" sweep the wrapper (plain bash, no unit tests) has shipped real bugs without.
# Run from the repo root: bash ci/wrapper-smoke.sh   (expects the workspace + lint already built)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CC="$ROOT/cargo-candor"
# cargo-candor `stash()` mirrors whichever candor-scan/candor-query it ran (a debug build from target/) into
# $CANDOR_CACHE/bin. Unset, that is ~/.candor/bin, which `candor` and lanes treat as the PUBLISHED engine.
CANDOR_CACHE="$(mktemp -d)/cache"; export CANDOR_CACHE
# ORACLE LANE. The wrapper's default route is candor-scan, on which `snapshot`/`guard`/`policy`/`strict`/`risk`/
# `explain` refuse. This lane drives the lint route on purpose, so it opts in; the section at the end runs with
# the opt-in REMOVED and pins the default route (and the refusals) instead.
export CANDOR_BACKEND=lint

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

echo "== gate-sink integrity (SPEC §3.3.1 (3) input exemption, §3.3 a document on every exit-2) =="
# Every row asserts the BYTES at the artifact, never just the exit code: each defect below was
# measured 2026-08-12 exiting 2 while doing the wrong thing, so an exit-code row stays green through
# all of them. The `rm -f`-up-front design deleted whatever the sink named before anything read it.
base_member=$(ls .candor/base.*.json | head -1)
before=$(cat "$base_member")
rc=0; out=$("$CC" guard .candor/base --gate-json "$base_member" 2>&1) || rc=$?
assert_rc "guard --gate-json <a baseline member> is refused" 2 "$rc"
[ -e "$base_member" ] && [ "$(cat "$base_member")" = "$before" ] \
  && ok "…and the baseline member is byte-identical (it was DELETED before the fix, and the run then reported 'no baseline found')" \
  || bad "…the baseline member was destroyed or altered: $(ls -la "$base_member" 2>&1)"
assert_has "…and the refusal names the input relationship" "$out" "names the same file as"
before=$(cat .candor/base.candor-version)
rc=0; out=$("$CC" guard .candor/base --gate-json .candor/base.candor-version 2>&1) || rc=$?
assert_rc "guard --gate-json <the .candor-version provenance sidecar> is refused" 2 "$rc"
[ -e .candor/base.candor-version ] && [ "$(cat .candor/base.candor-version)" = "$before" ] \
  && ok "…and the provenance sidecar is byte-identical (deleted before the fix — the run then called the baseline unverifiable)" \
  || bad "…the provenance sidecar was destroyed: $(ls -la .candor/base.candor-version 2>&1)"
# a usage error must not leave a PREVIOUS run's green verdict at a sink named elsewhere in the argv
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; "$CC" guard .candor/base --gate-json stale.json --frobnicate >/dev/null 2>&1 || rc=$?
assert_rc "guard with an unknown flag beside a named sink" 2 "$rc"
if grep -q '"refused": true' stale.json && grep -q '"ok": false' stale.json && ! grep -q '"violations"' stale.json; then
  ok "…and the sink holds the fail-closed refusal document, not the previous run's green (which is what it held before the fix)"
else
  bad "…sink content wrong: $(cat stale.json 2>/dev/null)"
fi

# THE SPEC KEY ON THAT DOCUMENT, and both of its branches. `refusal_doc` derives the version from the
# installed candor-query rather than hardcoding it — a literal here meant the wrapper stamped the OLD
# contract onto every refusal after a floor bump. Nothing asserted the key at all, and the same change
# that introduced the derivation stripped `spec` out of these fixtures, so a fresh clone with no engine
# built would have written every refusal without it and this suite would have stayed green.
DECLARED_SPEC="$("$(dirname "$CC")/target/debug/candor-query" --version 2>/dev/null | sed -n 's/.*(spec \([0-9][0-9.]*\)).*/\1/p' | head -1)"
[ -n "$DECLARED_SPEC" ] || DECLARED_SPEC="$(candor-query --version 2>/dev/null | sed -n 's/.*(spec \([0-9][0-9.]*\)).*/\1/p' | head -1)"
if [ -n "$DECLARED_SPEC" ]; then
  grep -q "\"spec\": \"$DECLARED_SPEC\"" stale.json \
    && ok "…and it declares the spec the ENGINE declares ($DECLARED_SPEC), not a literal frozen at some past floor" \
    || bad "…refusal document's spec is not the engine's $DECLARED_SPEC: $(cat stale.json 2>/dev/null)"
else
  # The OTHER branch, and it must be asserted rather than assumed: with no engine resolvable the key is
  # OMITTED, never guessed. §2.1 reads an absent `spec` as predating the field — a smaller, truer claim
  # than a stale literal. Reaching here at all means no engine was found, which is itself worth saying.
  ! grep -q '"spec"' stale.json \
    && ok "no engine resolvable: the refusal OMITS spec rather than guessing one" \
    || bad "no engine resolvable, yet the refusal carries a spec: $(cat stale.json 2>/dev/null)"
fi
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; "$CC" guard --frobnicate --gate-json stale.json >/dev/null 2>&1 || rc=$?
assert_rc "…the other argv order too (sink parsed after the broken flag)" 2 "$rc"
grep -q '"refused": true' stale.json && ok "…and that sink holds the refusal document too" \
  || bad "…sink content wrong: $(cat stale.json 2>/dev/null)"
# a plain could-not-evaluate exit-2 leaves the refusal document, and the stream form carries it on stdout
rm -f g2.json
rc=0; "$CC" guard nosuch-prefix --gate-json g2.json >/dev/null 2>&1 || rc=$?
assert_rc "guard (no baseline) with a sink" 2 "$rc"
grep -q '"refused": true' g2.json 2>/dev/null && ok "…and the sink holds the refusal document (before the fix: deleted, nothing written)" \
  || bad "…no document at the sink: $(cat g2.json 2>/dev/null)"
rc=0; sout=$("$CC" guard nosuch-prefix --gate-json - 2>/dev/null) || rc=$?
assert_rc "guard (no baseline) stream form" 2 "$rc"
printf '%s' "$sout" | grep -q '"refused": true' && ok "…and stdout carries the refusal document (before the fix: 0 bytes)" \
  || bad "…stream empty or wrong: '$sout'"
# the policy verb: its own file is an input, and its missing-file exit leaves the document
before=$(cat deny.policy)
rc=0; out=$("$CC" policy deny.policy --gate-json deny.policy 2>&1) || rc=$?
assert_rc "policy --gate-json <the policy itself> is refused" 2 "$rc"
[ "$(cat deny.policy)" = "$before" ] && ok "…and the policy is byte-identical" || bad "…the policy was destroyed"
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; "$CC" policy nosuch.policy --gate-json stale.json >/dev/null 2>&1 || rc=$?
assert_rc "policy (missing file) with a sink" 2 "$rc"
grep -q '"refused": true' stale.json && ok "…and the sink holds the refusal document, not the previous green" \
  || bad "…sink content wrong: $(cat stale.json 2>/dev/null)"
# a CONFIG-LOAD refusal exits before the verb runs — the ⟨0.27⟩ "armed after config load" window;
# the sink must still end holding the refusal document, and a sink naming an INPUT stays untouched
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; CANDOR_CONFIG=/no/such "$CC" policy deny.policy --gate-json stale.json >/dev/null 2>&1 || rc=$?
assert_rc "config-load refusal with a sink" 2 "$rc"
grep -q '"refused": true' stale.json && ok "…and the sink holds the refusal document through the config window" \
  || bad "…sink content wrong through the config window: $(cat stale.json 2>/dev/null)"
before=$(cat deny.policy)
rc=0; CANDOR_CONFIG=/no/such "$CC" policy deny.policy --gate-json deny.policy >/dev/null 2>&1 || rc=$?
assert_rc "config-load refusal, sink = the positional policy" 2 "$rc"
[ "$(cat deny.policy)" = "$before" ] && ok "…and the policy is byte-identical (the exemption outranks the window document)" \
  || bad "…the policy was destroyed through the config window"

cd /; rm -rf "$(dirname "$D")"

# ══ THE DEFAULT ROUTE: candor-scan is the ONLY user route (Tom's ruling 2026-11-23, BACKLOG §5.1 option A) ══
# Everything above ran with CANDOR_BACKEND=lint (the oracle lanes' opt-in). Here the opt-in is REMOVED.
# Each assertion is on bytes/exit, and each is paired with the control that would make it vacuous: a
# refusal row is only evidence if the SAME verb succeeds under the opt-in (the sections above), and a
# "scanner produced the report" row is only evidence if the lint did not (no non-.scan report file).
echo "== default route: no opt-in => candor-scan, and the lint-only verbs refuse =="
unset CANDOR_BACKEND
R=$(mktemp -d)/route; mkdir -p "$R/src"
printf '[package]\nname="route"\nversion="0.1.0"\nedition="2021"\n' > "$R/Cargo.toml"
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > "$R/src/main.rs"
echo "deny Fs  domain" > "$R/deny.policy"
cd "$R"
# PLANT THE LINT ROUTE'S PRECONDITION: a findable dylib must NOT change the default. Put one in the cache the
# wrapper searches (a copy of the real one when this lane built it; otherwise a placeholder file — find_lib
# only has to FIND it for the route decision to be exercised, and the opt-in is absent so it is never loaded).
CLONE="$(cd "$(dirname "$CC")" && pwd)"
REALLIB="$(ls -t "$CLONE"/target/debug/libcandor@*.dylib "$CLONE"/target/debug/libcandor@*.so 2>/dev/null | head -1)"
mkdir -p "$CANDOR_CACHE/lib"
if [ -n "$REALLIB" ]; then cp -f "$REALLIB" "$CANDOR_CACHE/lib/"; else : > "$CANDOR_CACHE/lib/libcandor@nightly-0000-00-00-placeholder.dylib"; fi
out=$("$CC" show domain_logic 2>&1); rc=$?
assert_rc "show (default route)" 0 "$rc"
ls .candor/ | grep -q '^report\.route\.scan\.json$' && ok "the report is the SCANNER's (report.route.scan.json) although a dylib is findable" \
  || bad "default route did not produce a scan report: $(ls .candor/ 2>&1)"
ls .candor/ | grep -E '^report\.route\.(Rlib|Executable|Cdylib|Staticlib|Dylib|ProcMacro)\.json$' >/dev/null 2>&1 && bad "a LINT-named report appeared on the default route" || ok "…and no lint-named report exists (the control)"
assert_has "…and the answer reaches the Fs effect" "$out" "Fs"
rc=0; "$CC" audit >/dev/null 2>&1 || rc=$?
assert_rc "audit (default route)" 0 "$rc"
rc=0; "$CC" whatif domain_logic Net >/dev/null 2>&1 || rc=$?
assert_rc "whatif (default route, report-only verb)" 0 "$rc"
for v in "strict" "no-ambient" "risk" "snapshot .candor/b" "explain domain_logic" "audit --all"; do
  rc=0; out=$("$CC" $v 2>&1) || rc=$?
  assert_rc "$v refuses without the opt-in" 2 "$rc"
  assert_has "…naming the oracle opt-in" "$out" "CANDOR_BACKEND=lint"
done
rc=0; out=$("$CC" snapshot .candor/b 2>&1) || rc=$?
assert_has "snapshot's refusal names the candor-scan equivalent" "$out" "candor-scan . --out"
ls .candor/b.* >/dev/null 2>&1 && bad "snapshot refused but wrote a lint baseline" || ok "…and wrote no baseline"
# policy / guard: refused AFTER the sink is armed — the bytes at the sink are the §3.3 refusal document
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; out=$("$CC" policy deny.policy --gate-json stale.json 2>&1) || rc=$?
assert_rc "policy refuses without the opt-in" 2 "$rc"
assert_has "…and names candor-scan --policy" "$out" "candor-scan . --policy deny.policy"
if grep -q '"refused": true' stale.json && grep -q '"ok": false' stale.json && ! grep -q '"violations"' stale.json; then
  ok "…and the sink holds the refusal document, never a previous green"
else bad "…sink content wrong: $(cat stale.json 2>/dev/null)"; fi
printf '{"ok":true,"violations":[]}\n' > stale.json
rc=0; out=$("$CC" guard .candor/b --gate-json stale.json 2>&1) || rc=$?
assert_rc "guard refuses without the opt-in" 2 "$rc"
assert_has "…and names CANDOR_BASELINE with candor-scan" "$out" "CANDOR_BASELINE=.candor/b candor-scan"
grep -q '"refused": true' stale.json && ok "…and its sink holds the refusal document" || bad "…guard sink content wrong: $(cat stale.json 2>/dev/null)"
rc=0; sout=$("$CC" policy deny.policy --gate-json - 2>/dev/null) || rc=$?
printf '%s' "$sout" | grep -q '"refused": true' && ok "policy's stream form carries the refusal document on stdout" || bad "policy stream form empty: '$sout'"
# the same verbs on the USER route do the job: candor-scan --policy / CANDOR_BASELINE, as the refusals say
SCANBIN="$(ls -t "$CLONE"/target/debug/candor-scan "$CLONE"/target/release/candor-scan 2>/dev/null | head -1)"
if [ -n "$SCANBIN" ]; then
  rc=0; "$SCANBIN" . --policy deny.policy >/dev/null 2>&1 || rc=$?
  assert_rc "the remedy works: candor-scan . --policy (violation)" 1 "$rc"
  rc=0; "$SCANBIN" . --out .candor/sbase >/dev/null 2>&1 || rc=$?
  assert_rc "the remedy works: candor-scan . --out <baseline>" 0 "$rc"
  rc=0; out=$("$CC" diff .candor/sbase 2>&1) || rc=$?
  assert_rc "diff against a candor-scan baseline (default route, no change)" 0 "$rc"
  printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > src/main.rs
  out=$("$CC" diff .candor/sbase --json 2>&1); rc=$?
  assert_has "diff names the GAINED effect (the control: not a vacuous 'no change')" "$out" "Net"
  rm -f .candor/report.*.json .candor/state
  # a baseline recorded by the OTHER backend is refused, never compared
  printf '[{"fn":"leaf","loc":"src/main.rs:1","inferred":["Fs"],"direct":["Fs"],"unresolved":false}]\n' > .candor/lbase.route.Executable.json
  rc=0; out=$("$CC" diff .candor/lbase 2>&1) || rc=$?
  assert_rc "diff against an oracle-lint baseline is refused on the scanner route" 2 "$rc"
  assert_has "…and says how to re-record" "$out" "candor-scan . --out .candor/lbase"
fi
# the opt-in still reaches the lint (the control for every refusal above)
rc=0; out=$(CANDOR_BACKEND=lint "$CC" risk 2>&1) || rc=$?
assert_rc "CANDOR_BACKEND=lint risk runs (the opt-in is the way in)" 0 "$rc"
cd /; rm -rf "$(dirname "$R")"
echo
echo "wrapper smoke: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
