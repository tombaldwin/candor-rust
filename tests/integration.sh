#!/usr/bin/env bash
# End-to-end integration tests for candor — the paths the in-process unit/ui tests can't reach
# because they need a real `cargo dylint` build: the AS-EFF mode diagnostics (env-driven) and
# cross-crate effect propagation across a lib+bin boundary. Run from the repo root:
#   bash tests/integration.sh
# Requires python3 (report inspection). Wired into CI.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "building candor lint + tooling…"
# --workspace builds candor-query too (a member, not a lint dependency), so the query commands
# below exercise the CURRENT binary rather than a possibly-stale pre-built one.
cargo build -q --workspace || { echo "FAIL: candor build"; exit 1; }
# Absolute path — the scenarios `cd` into fixture dirs, so a relative lib path would break.
# NEWEST by mtime (`ls -t`), not the first glob match: the filename carries the toolchain
# (`libcandor@nightly-2026-06-14-…`), so a stale build from an OLD pinned nightly would otherwise sort
# ahead alphabetically and shadow the fresh `cargo build` — running the tests against the wrong engine
# (this is exactly why `cargo-candor`'s own `newest_of` uses `ls -t`).
LIB=$(ls -t "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so 2>/dev/null | head -1)
[ -n "$LIB" ] || { echo "FAIL: no dylib under target/debug"; exit 1; }
command -v python3 >/dev/null || { echo "FAIL: python3 required"; exit 1; }

pass=0; fail=0
want()   { if printf '%s' "$2" | grep -qF -- "$3"; then echo "  ok   $1"; pass=$((pass+1)); else echo "  FAIL $1 — missing: $3"; fail=$((fail+1)); fi; }
absent() { if printf '%s' "$2" | grep -qF -- "$3"; then echo "  FAIL $1 — unexpected: $3"; fail=$((fail+1)); else echo "  ok   $1"; pass=$((pass+1)); fi; }
# Run dylint on a dir with a fresh lint pass (dylint emits only on recompile).
dl() { ( cd "$1"; rm -rf target/dylint; shift; "$@" cargo dylint --lib-path "$LIB" 2>&1 ); }

# ── 1. Conformance: AS-EFF-001/002/003 (CANDOR_STRICT) on the capability-discipline sample ──
echo "== conformance / AS-EFF-001/002/003 (CANDOR_STRICT, sample/) =="
out=$(dl sample env CANDOR_STRICT=1)
want   "AS-EFF-001 performs-but-undeclared (sneaky_read)" "$out" '[AS-EFF-001] `sneaky_read`'
want   "AS-EFF-002 declared-but-unused (greet)"           "$out" '[AS-EFF-002] `greet`'
want   "AS-EFF-003 unresolvable (run_callback)"           "$out" '[AS-EFF-003] `run_callback`'
absent "conformant read_config is NOT flagged"            "$out" '[AS-EFF-001] `read_config`'

# ── 2. Ambient authority: AS-EFF-004 (CANDOR_NO_AMBIENT) ──
echo "== ambient authority / AS-EFF-004 (CANDOR_NO_AMBIENT, sample/) =="
out=$(dl sample env CANDOR_NO_AMBIENT=1)
want "AS-EFF-004 flags a direct ambient reach" "$out" "[AS-EFF-004]"

# ── 3. Regression guard: AS-EFF-005 (CANDOR_BASELINE) ──
echo "== regression guard / AS-EFF-005 (CANDOR_BASELINE) =="
G=$(mktemp -d)/g; mkdir -p "$G/src" "$G/.candor"
printf '[package]\nname="g"\nversion="0.1.0"\nedition="2021"\n' > "$G/Cargo.toml"
printf 'fn touches_net() { let _ = std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main() { touches_net(); }\n' > "$G/src/main.rs"
dl "$G" env CANDOR_JSON="$G/.candor/base" >/dev/null
# Simulate an older baseline that predates touches_net's Net, then guard: it must flag the gain.
python3 - "$G/.candor" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + '/base.*.json'):
    if '.calibrated' in f or '.encountered' in f: continue
    try: d = json.load(open(f))
    except Exception: continue
    # v0.2 envelope {candor, functions} or legacy bare array; edit in place, preserve structure.
    funcs = d['functions'] if isinstance(d, dict) and isinstance(d.get('functions'), list) else d if isinstance(d, list) else None
    if funcs is None: continue
    for e in funcs:
        if e['fn'] == 'touches_net': e['inferred'] = []; e['direct'] = []
    json.dump(d, open(f, 'w'))
PY
out=$(dl "$G" env CANDOR_BASELINE="$G/.candor/base")
want "AS-EFF-005 flags touches_net gaining Net" "$out" '[AS-EFF-005] `touches_net`'

# ── 4. Cross-crate effect propagation: a bin inherits its lib's effect (CRITIQUE §8) ──
echo "== cross-crate effect inheritance (lib+bin) =="
X=$(mktemp -d)/xc; mkdir -p "$X/src" "$X/.candor"
printf '[package]\nname="xc"\nversion="0.1.0"\nedition="2021"\n' > "$X/Cargo.toml"
printf 'pub fn writes() { let _ = std::fs::write("/tmp/xc_probe", b"x"); }\n' > "$X/src/lib.rs"
{ printf 'fn caller() { xc::writes(); }\n'                                          # cross-crate Fs only
  printf 'fn local_read() { let _ = std::fs::read("/tmp/xc_r"); }\n'                # local Fs read
  printf 'fn mixed() { let _ = std::fs::read("/tmp/xc_r"); xc::writes(); }\n'       # local read + cross write
  printf 'fn main() { caller(); local_read(); mixed(); }\n'; } > "$X/src/main.rs"
dl "$X" env CANDOR_JSON="$X/.candor/r" >/dev/null
xcfs=$(python3 - "$X/.candor" <<'PY'
import json, glob, sys
d = {}
for f in glob.glob(sys.argv[1] + '/r.xc.Executable.json'):
    data = json.load(open(f))
    funcs = data['functions'] if isinstance(data, dict) else data   # v0.2 envelope or v0.1 array
    for e in funcs:
        d[e['fn']] = (','.join(sorted(e.get('inferred', []))), ','.join(e.get('fs', [])))
for fn in ('caller', 'local_read', 'mixed'):
    inf, fs = d.get(fn, ('', ''))
    print(f"{fn} inferred=[{inf}] fs=[{fs}]")
PY
)
want "bin 'caller' inherits lib Fs across the crate boundary"      "$xcfs" "caller inferred=[Fs]"
want "a local Fs read carries its read/write detail"               "$xcfs" "local_read inferred=[Fs] fs=[read]"
want "cross-crate Fs carries NO read/write detail (kind unknown)"  "$xcfs" "caller inferred=[Fs] fs=[]"
# The regression: a fn that reads locally AND writes cross-crate must NOT report `fs=[read]` (a partial
# claim that hides the cross-crate write) — the detail is suppressed to nothing.
want "mixed local+cross-crate Fs suppresses the partial detail"    "$xcfs" "mixed inferred=[Fs] fs=[]"

# ── 5. Version provenance: the dylib self-reports its build version, and reports are self-describing ──
echo "== version stamping (build tag + self-describing sidecar) =="
tag=$(strings -a "$LIB" 2>/dev/null | grep -oE 'candor-build-version=[0-9a-fA-F]+' | head -1)
want "dylib embeds a build-version tag readable without running it" "$tag" "candor-build-version="
sidecar_ver=$(python3 - "$X/.candor" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + '/r.calibrated.json'):
    d = json.load(open(f)); print(d.get('candor_version',''), d.get('toolchain',''))
PY
)
want "calibrated sidecar carries candor_version (self-describing report)" "$sidecar_ver" "${tag#candor-build-version=}"
absent "sidecar version is not the 'unknown' fallback"                    "$sidecar_ver" "unknown unknown"

# ── 6. At-a-glance audit: `cargo candor audit` aggregates the project into a profile ──
echo "== at-a-glance audit (cargo candor audit) =="
A=$(mktemp -d)/aud; mkdir -p "$A/src"
printf '[package]\nname="aud"\nversion="0.1.0"\nedition="2021"\n' > "$A/Cargo.toml"
printf 'fn reads(){ let _=std::fs::read("/tmp/x"); }\nfn runs(){ let _=std::process::Command::new("x").status(); }\nfn main(){ reads(); runs(); }\n' > "$A/src/main.rs"
aud=$( cd "$A"; "$ROOT/cargo-candor" audit 2>/dev/null )
want "audit: header carries the engine version"      "$aud" "candor @"
want "audit: effectful-function count line"          "$aud" "effectful functions"
want "audit: effect tally rendered"                  "$aud" "effects"
want "audit: broadest-surface section"               "$aud" "broadest effect surface"
want "audit: main shows its transitive { Exec Fs }"  "$aud" "Exec Fs"
rm -rf "$(dirname "$A")"

# Per-crate label comes from the FILENAME (candor-query report_files), so an unreadable report still
# shows its <crate>.<type> label instead of blanking to "0 " (regression). A directory in place of the
# file makes read_to_string fail (EISDIR) for every user, including root.
QBIN="$ROOT/target/debug/candor-query"
QD=$(mktemp -d)
printf '[{"fn":"f","inferred":["Net"]}]' > "$QD/r.good.lib.json"   # readable, one effectful fn
mkdir "$QD/r.bad.lib.json"                                          # unreadable (a directory)
qaud=$("$QBIN" audit "$QD/r" testver /no/such/suspect 2>/dev/null || true)
want "audit: labels a readable crate report"          "$qaud" "good.lib"
want "audit: still labels an UNREADABLE crate report" "$qaud" "bad.lib"
rm -rf "$QD"

# coverage visibility: `audit --coverage` lists every external crate candor has NO rules for (the
# blind-spot surface), while path-matched runtimes (tokio/async_std/mio) and name/prefix-calibrated
# crates are correctly treated as COVERED (not false-flagged).
CV=$(mktemp -d)
printf '[{"fn":"app::main","inferred":["Net"],"direct":["Net"]}]' > "$CV/r.app.Bin.json"
printf '{"crates":["reqwest"],"prefixes":["aws_sdk_"],"path_crates":["tokio"]}' > "$CV/r.calibrated.json"
printf '["tokio","reqwest","aws_sdk_s3","serde","mystery_io"]' > "$CV/r.encountered-app-Bin.json"
cov=$("$QBIN" audit "$CV/r" testver /no/such/suspect --coverage 2>/dev/null || true)
want   "coverage: lists an uncalibrated crate as a blind spot"  "$cov" "mystery_io"
absent "coverage: a path-matched runtime is NOT a blind spot"   "$cov" "tokio"
absent "coverage: a name-calibrated crate is NOT a blind spot"  "$cov" "reqwest"
absent "coverage: a prefix-calibrated crate is NOT a blind spot" "$cov" "aws_sdk_s3"
covd=$("$QBIN" audit "$CV/r" testver /no/such/suspect 2>/dev/null || true)
want   "audit default: hints at the uncovered remainder"        "$covd" "audit --coverage"
rm -rf "$CV"

# ── 7. Agent-facing effect diff: `cargo candor diff` describes the per-function delta (P0 §1) ──
echo "== effect diff (cargo candor diff) =="
D=$(mktemp -d)/d; mkdir -p "$D/src"
printf '[package]\nname="d"\nversion="0.1.0"\nedition="2021"\n' > "$D/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn mid(){ worker(); }\nfn main(){ mid(); }\n' > "$D/src/main.rs"
( cd "$D"; "$ROOT/cargo-candor" snapshot .candor/baseline >/dev/null 2>&1 )
# an agent adds a network call deep in `worker` — a LOCAL edit with a NON-LOCAL consequence; it
# propagates worker → mid → main, so `main` is the top-level surface and `mid` is plumbing.
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn mid(){ worker(); }\nfn main(){ mid(); }\n' > "$D/src/main.rs"
dout=$( cd "$D"; "$ROOT/cargo-candor" diff 2>/dev/null )
djson=$( cd "$D"; "$ROOT/cargo-candor" diff --json 2>/dev/null )
want "diff: the edited fn (worker) is flagged"             "$dout" "worker"
want "diff: marks the source with * (worker introduced Net)" "$dout" "+Net*"
want "diff: per-effect headline names the source"          "$dout" "introduced in"
want "diff (§9): headline names where it surfaces (reaches main)" "$dout" "reaches main"
want "diff (§9): the top-level endpoint is tagged"         "$dout" "top-level"
want "diff (§9): the intermediate plumbing (mid) is collapsed" "$dout" "intermediate caller"
want "diff --json: machine-readable for the agent"         "$djson" '"gained"'
want "diff --json: classifies introduced vs inherited"     "$djson" '"introduced"'
rm -rf "$(dirname "$D")"

# ── 8. explain: trace the call path to each effect's source (P0 §3) ──
echo "== explain (cargo candor explain) =="
P=$(mktemp -d)/p; mkdir -p "$P/src"
printf '[package]\nname="p"\nversion="0.1.0"\nedition="2021"\n' > "$P/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn middle(){ leaf(); }\nfn main(){ middle(); }\n' > "$P/src/main.rs"
xout=$( cd "$P"; "$ROOT/cargo-candor" explain main 2>/dev/null )
want "explain: header for the queried function"   "$xout" "candor explain — main"
want "explain: traces the multi-hop call path"    "$xout" "main → middle → leaf"
want "explain: names the leaf effectful call"     "$xout" "TcpStream::connect"
rm -rf "$(dirname "$P")"

# ── 9. Effect policy: enforce architectural boundaries (AS-EFF-006, P0′ §6) ──
echo "== effect policy / AS-EFF-006 (CANDOR_POLICY) =="
PL=$(mktemp -d)/pl; mkdir -p "$PL/src"
printf '[package]\nname="pl"\nversion="0.1.0"\nedition="2021"\n' > "$PL/Cargo.toml"
# domain_logic is pure-LOOKING but reaches the filesystem transitively via leaf(); domain_pure doesn't.
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn domain_pure(){ let _=1+1; }\nfn main(){ domain_logic(); domain_pure(); }\n' > "$PL/src/main.rs"
echo "deny Fs Net  domain" > "$PL/policy"
# MACHINE-SIGNAL verdict (CANDOR_VIOLATIONS): the wrapper's gate now rides on this sentinel, not on
# grepping the diagnostic prose. A `deny` violation must (a) emit the human text AND (b) append a line
# to the sentinel; a clean run must produce neither. Absolute path — `dl` runs `cd`'d into the fixture.
VIO="$PL/violations"; : > "$VIO"
out=$(dl "$PL" env CANDOR_POLICY="$PL/policy" CANDOR_VIOLATIONS="$VIO")
want   "AS-EFF-006 flags the TRANSITIVE boundary violation (domain_logic reaches Fs via a helper)" "$out" '[AS-EFF-006] `domain_logic`'
absent "the genuinely-pure domain fn is NOT flagged"                                               "$out" '[AS-EFF-006] `domain_pure`'
# The sentinel is the signal the wrapper checks (`[ -s ]` → exit 1) — it must be non-empty and name the fn.
want   "AS-EFF-006 violation writes the CANDOR_VIOLATIONS sentinel"                                "$(cat "$VIO")" 'AS-EFF-006 domain_logic'
# A CLEAN run (a policy nothing violates) leaves the sentinel empty — the wrapper would exit 0.
: > "$VIO"; echo "deny Net  domain" > "$PL/policy-clean"   # the crate has no Net, so nothing fires
dl "$PL" env CANDOR_POLICY="$PL/policy-clean" CANDOR_VIOLATIONS="$VIO" >/dev/null
if [ -s "$VIO" ]; then echo "  FAIL clean run must leave the sentinel empty — got: $(cat "$VIO")"; fail=$((fail+1)); else echo "  ok   clean run leaves the CANDOR_VIOLATIONS sentinel empty"; pass=$((pass+1)); fi
rm -rf "$(dirname "$PL")"

# ── 9-u. `deny <Effect>` vs `Unknown` (SEMANTICS §6, family ruling): AS-EFF-006 fires iff the rule
# NAMES an effect provably in I(f). An Unknown-only fn does NOT trip `deny Net` (the reference engine,
# candor-scan and candor-ts all read the predicate this way); the strictness knob is the explicit
# `deny Unknown <scope>`, which keeps firing with effects = [Unknown].
echo "== deny-vs-Unknown semantics (AS-EFF-006, SEMANTICS §6) =="
PU=$(mktemp -d)/pu; mkdir -p "$PU/src"
printf '[package]\nname="pu"\nversion="0.1.0"\nedition="2021"\n' > "$PU/Cargo.toml"
# domain_unknown invokes an opaque boxed callback → honest `Unknown`, and provably NO Net.
printf 'fn domain_unknown(f: Box<dyn Fn()>) { f(); }\nfn main() { domain_unknown(Box::new(|| ())); }\n' > "$PU/src/main.rs"
echo "deny Net  domain" > "$PU/policy"
UVIO="$PU/violations"; : > "$UVIO"
out=$(dl "$PU" env CANDOR_POLICY="$PU/policy" CANDOR_VIOLATIONS="$UVIO")
absent "deny Net does NOT fire on an Unknown-only fn (no false positive)" "$out" '[AS-EFF-006]'
if [ -s "$UVIO" ]; then echo "  FAIL Unknown-only fn under deny Net must leave the sentinel empty — got: $(cat "$UVIO")"; fail=$((fail+1)); else echo "  ok   Unknown-only fn under deny Net leaves the sentinel empty"; pass=$((pass+1)); fi
# The knob: `deny Unknown` names the unprovable case and fires.
: > "$UVIO"; echo "deny Unknown  domain" > "$PU/policy-unknown"
out=$(dl "$PU" env CANDOR_POLICY="$PU/policy-unknown" CANDOR_VIOLATIONS="$UVIO")
want "deny Unknown fires on the Unknown-carrying fn"       "$out" '[AS-EFF-006] `domain_unknown`'
want "deny Unknown verdict carries Unknown as the effect"  "$out" 'performs { Unknown }'
want "deny Unknown violation writes the sentinel"          "$(cat "$UVIO")" 'AS-EFF-006 domain_unknown'
# `pure` forbids every REAL effect; `Unknown` is the §4 visibility marker (AS-EFF-003's concern) —
# matching the reference engine, a pure rule alone does not fire on an Unknown-only fn.
: > "$UVIO"; echo "pure  domain" > "$PU/policy-pure"
out=$(dl "$PU" env CANDOR_POLICY="$PU/policy-pure" CANDOR_VIOLATIONS="$UVIO")
absent "pure does NOT fire on an Unknown-only fn (AS-EFF-003 owns that)" "$out" '[AS-EFF-006]'
# candor-scan shares the ruling — the syntactic gate wrongly counted Unknown under `pure` until
# 2026-07-09 (a cross-engine verdict split on the same policy file). fn-typed-param callback →
# scan's own Unknown; an unscoped `pure` must pass it, `deny Unknown` must fire.
PSU=$(mktemp -d)/psu; mkdir -p "$PSU/src"
printf '[package]\nname="psu"\nversion="0.1.0"\nedition="2021"\n' > "$PSU/Cargo.toml"
printf 'pub fn entry(f: fn()) { f(); }\n' > "$PSU/src/lib.rs"
printf 'pure\n' > "$PSU/policy-pure"; printf 'deny Unknown\n' > "$PSU/policy-unknown"
env -u CANDOR_CONFIG CANDOR_POLICY="$PSU/policy-pure" "$ROOT/target/debug/candor-scan" "$PSU" >/dev/null 2>&1
rc_sp=$?
env -u CANDOR_CONFIG CANDOR_POLICY="$PSU/policy-unknown" "$ROOT/target/debug/candor-scan" "$PSU" >/dev/null 2>&1
rc_su=$?
if [ "$rc_sp" = 0 ]; then echo "  ok   candor-scan: pure passes an Unknown-only fn (exit 0)"; pass=$((pass+1)); else echo "  FAIL candor-scan: pure fired on an Unknown-only fn (exit $rc_sp)"; fail=$((fail+1)); fi
if [ "$rc_su" = 1 ]; then echo "  ok   candor-scan: deny Unknown fires (exit 1)"; pass=$((pass+1)); else echo "  FAIL candor-scan: deny Unknown expected exit 1, got $rc_su"; fail=$((fail+1)); fi
rm -rf "$(dirname "$PSU")"
rm -rf "$(dirname "$PU")"

# ── 9-bl. candor-scan AS-EFF-005 baseline guard (spec §7 item 5; candor-java checkBaseline is the model) ──
echo "== candor-scan baseline guard (CANDOR_BASELINE / config baseline key) =="
SB=$(mktemp -d)/sb; mkdir -p "$SB/src"
SCAN="$ROOT/target/debug/candor-scan"
printf '[package]\nname="sb"\nversion="0.1.0"\nedition="2021"\n' > "$SB/Cargo.toml"
printf 'pub fn go() { let _ = std::fs::read("/x"); }\n' > "$SB/src/lib.rs"
env -u CANDOR_CONFIG -u CANDOR_BASELINE -u CANDOR_POLICY "$SCAN" "$SB" --out "$SB/base" >/dev/null 2>&1
# clean compare (unchanged code) → exit 0 with the guard receipt
rc=0; out=$(env -u CANDOR_CONFIG -u CANDOR_POLICY CANDOR_BASELINE="$SB/base" "$SCAN" "$SB" 2>&1) || rc=$?
if [ "$rc" = 0 ]; then echo "  ok   scan guard: clean compare exits 0"; pass=$((pass+1)); else echo "  FAIL scan guard: clean compare exited $rc (want 0)"; fail=$((fail+1)); fi
want "scan guard: clean receipt printed" "$out" "baseline guard ✓"
# the ratchet: go gains Exec (a new fn is also added and must be EXEMPT — only go flags)
printf 'pub fn go() { let _ = std::fs::read("/x"); std::process::Command::new("sh").status().unwrap(); }\npub fn newbie() { let _ = std::net::TcpStream::connect("h:1"); }\n' > "$SB/src/lib.rs"
rc=0; out=$(env -u CANDOR_CONFIG -u CANDOR_POLICY CANDOR_BASELINE="$SB/base" "$SCAN" "$SB" 2>&1) || rc=$?
if [ "$rc" = 1 ]; then echo "  ok   scan guard: a gained effect exits 1"; pass=$((pass+1)); else echo "  FAIL scan guard: gain exited $rc (want 1)"; fail=$((fail+1)); fi
want   "scan guard: AS-EFF-005 names the fn + gained effect" "$out" '[AS-EFF-005] `go` gained effect { Exec }'
absent "scan guard: a NEW fn is exempt (reviewed as new code)" "$out" '[AS-EFF-005] `newbie`'
# absent baseline → note + exit unchanged (guard inactive)
rc=0; out=$(env -u CANDOR_CONFIG -u CANDOR_POLICY CANDOR_BASELINE="$SB/nosuch" "$SCAN" "$SB" 2>&1) || rc=$?
if [ "$rc" = 0 ]; then echo "  ok   scan guard: absent baseline leaves exit 0"; pass=$((pass+1)); else echo "  FAIL scan guard: absent baseline exited $rc (want 0)"; fail=$((fail+1)); fi
want "scan guard: absent baseline says how to record one" "$out" "regression guard is not active"
# a doctored producing version → exit 2 WITHOUT evaluating (§2.1 — never a stale compare)
python3 - "$SB/base.sb.scan.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); d["candor"]["version"] = "scan-0.0.0-doctored"
json.dump(d, open(sys.argv[1], "w"))
PY
rc=0; out=$(env -u CANDOR_CONFIG -u CANDOR_POLICY CANDOR_BASELINE="$SB/base" "$SCAN" "$SB" 2>&1) || rc=$?
if [ "$rc" = 2 ]; then echo "  ok   scan guard: version-mismatched baseline exits 2 (fail closed)"; pass=$((pass+1)); else echo "  FAIL scan guard: stale baseline exited $rc (want 2)"; fail=$((fail+1)); fi
absent "scan guard: NO AS-EFF-005 from a stale baseline" "$out" '[AS-EFF-005]'
# unparseable baseline → exit 2
printf '{ not json' > "$SB/base.sb.scan.json"
rc=0; out=$(env -u CANDOR_CONFIG -u CANDOR_POLICY CANDOR_BASELINE="$SB/base" "$SCAN" "$SB" 2>&1) || rc=$?
if [ "$rc" = 2 ]; then echo "  ok   scan guard: unparseable baseline exits 2"; pass=$((pass+1)); else echo "  FAIL scan guard: unparseable baseline exited $rc (want 2)"; fail=$((fail+1)); fi
# the config `baseline` key drives the guard (home-relative value, run from an unrelated CWD)
mkdir -p "$SB/.candor"; printf 'baseline .candor/cfgbase\n' > "$SB/.candor/config"
env -u CANDOR_CONFIG -u CANDOR_BASELINE -u CANDOR_POLICY "$SCAN" "$SB" --out "$SB/.candor/cfgbase" >/dev/null 2>&1
printf 'pub fn go() { let _ = std::fs::read("/x"); std::process::Command::new("sh").status().unwrap(); let _ = std::env::var("H"); }\npub fn newbie() { let _ = std::net::TcpStream::connect("h:1"); }\n' > "$SB/src/lib.rs"
rc=0; out=$( cd /tmp; env -u CANDOR_CONFIG -u CANDOR_BASELINE -u CANDOR_POLICY "$SCAN" "$SB" 2>&1 ) || rc=$?
if [ "$rc" = 1 ]; then echo "  ok   scan guard: config baseline key gates (home-anchored, exit 1)"; pass=$((pass+1)); else echo "  FAIL scan guard: config baseline key exited $rc (want 1)"; fail=$((fail+1)); fi
want "scan guard: config-driven gain names the new effect" "$out" '[AS-EFF-005] `go` gained effect { Env }'
rm -rf "$(dirname "$SB")"

# ── 9-fc. Fail-closed wrapper gates: `policy` and `guard` must never pass when they could not run ──
echo "== fail-closed gates (cargo candor policy / guard) =="
FC=$(mktemp -d)/fc; mkdir -p "$FC/src"
printf '[package]\nname="fc"\nversion="0.1.0"\nedition="2021"\n' > "$FC/Cargo.toml"
# (a) `policy` on a crate that DOESN'T BUILD: the gate cannot evaluate → exit 2, never "policy OK".
printf 'fn main() { this does not compile\n' > "$FC/src/main.rs"
echo "deny Net" > "$FC/policy"
pfc_rc=0; pfc=$( cd "$FC"; "$ROOT/cargo-candor" policy policy 2>&1 ) || pfc_rc=$?
want "policy on a build-broken crate says NOT evaluated"  "$pfc" "policy NOT evaluated"
absent "policy on a build-broken crate never says OK"     "$pfc" "policy OK"
if [ "$pfc_rc" -eq 2 ]; then echo "  ok   policy on a build-broken crate exits 2"; pass=$((pass+1)); else echo "  FAIL policy on a build-broken crate exited $pfc_rc (want 2)"; fail=$((fail+1)); fi
# (b) `guard` with NO baseline at all (never snapshotted / typo'd prefix): exit 2 + the incantation.
printf 'fn main() {}\n' > "$FC/src/main.rs"
gfc_rc=0; gfc=$( cd "$FC"; "$ROOT/cargo-candor" guard .candor/nosuch 2>&1 ) || gfc_rc=$?
want "guard with no baseline names the snapshot incantation" "$gfc" "cargo candor snapshot"
if [ "$gfc_rc" -eq 2 ]; then echo "  ok   guard with no baseline exits 2"; pass=$((pass+1)); else echo "  FAIL guard with no baseline exited $gfc_rc (want 2)"; fail=$((fail+1)); fi
# (c) `guard` with a PER-CRATE baseline gap (the prefix has files, but not for THIS crate — a new
# workspace member / renamed crate): the engine's GUARD-UNAVAILABLE sentinel → exit 2, not 0, not 1.
( cd "$FC"; "$ROOT/cargo-candor" snapshot .candor/base >/dev/null 2>&1 )
for f in "$FC"/.candor/base.fc.*.json; do
  [ -e "$f" ] && mv "$f" "$(printf '%s' "$f" | sed 's/base\.fc\./base.othercrate./')"
done
gap_rc=0; gap=$( cd "$FC"; "$ROOT/cargo-candor" guard .candor/base 2>&1 ) || gap_rc=$?
want "guard discloses the unloadable per-crate baseline" "$gap" "could not be loaded"
if [ "$gap_rc" -eq 2 ]; then echo "  ok   guard with a per-crate baseline gap exits 2 (fail closed)"; pass=$((pass+1)); else echo "  FAIL guard with a per-crate baseline gap exited $gap_rc (want 2)"; fail=$((fail+1)); fi
rm -rf "$(dirname "$FC")"

# ── 9-cfg. `.candor/config` (spec §3.4) drives the wrapper: policy/baseline keys, fail-closed, warnings ──
echo "== .candor/config discovery (cargo candor) =="
CG=$(mktemp -d)/cg; mkdir -p "$CG/src" "$CG/.candor"
printf '[package]\nname="cg"\nversion="0.1.0"\nedition="2021"\n' > "$CG/Cargo.toml"
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > "$CG/src/main.rs"
echo "deny Fs  domain" > "$CG/.candor/deny-fs.policy"
# RELATIVE values (SPEC §3.4: anchored to the config's HOME dir — the one containing .candor/) +
# a recognized-unwired key + a typo.
printf 'policy .candor/deny-fs.policy\nbaseline .candor/cfgbase   # inline comment\nstrict 1\npolcy typo\n' > "$CG/.candor/config"
cfgp_rc=0; cfgp=$( cd "$CG"; "$ROOT/cargo-candor" policy 2>&1 ) || cfgp_rc=$?
want "config 'policy' key (home-relative .candor/… value) drives the gate"  "$cfgp" '[AS-EFF-006] `domain_logic`'
if [ "$cfgp_rc" -eq 1 ]; then echo "  ok   config-supplied policy violation exits 1"; pass=$((pass+1)); else echo "  FAIL config-supplied policy exited $cfgp_rc (want 1)"; fail=$((fail+1)); fi
want "a recognized-but-unwired config key is disclosed"            "$cfgp" "NOT active"
want "an unknown config key warns (typo protection)"               "$cfgp" "unknown config key 'polcy'"
# config `baseline` key = the guard's default prefix: snapshot there, then a bare `guard` finds it.
( cd "$CG"; "$ROOT/cargo-candor" snapshot >/dev/null 2>&1 )
cfgg_rc=0; ( cd "$CG"; "$ROOT/cargo-candor" guard >/dev/null 2>&1 ) || cfgg_rc=$?
if [ -e "$CG/.candor/cfgbase.candor-version" ] && [ "$cfgg_rc" -eq 0 ]; then
  echo "  ok   config 'baseline' key anchors snapshot+guard (bare guard passes)"; pass=$((pass+1))
else
  echo "  FAIL config baseline key: sidecar-exists=$([ -e "$CG/.candor/cfgbase.candor-version" ] && echo yes || echo no) guard_rc=$cfgg_rc (want 0)"; fail=$((fail+1))
fi
# a set-but-unusable CANDOR_CONFIG fails closed (exit 2), never a silent no-config run.
cfgu_rc=0; cfgu=$( cd "$CG"; CANDOR_CONFIG=/no/such/config "$ROOT/cargo-candor" guard 2>&1 ) || cfgu_rc=$?
want "a set-but-unusable CANDOR_CONFIG is loud" "$cfgu" "not a readable file"
if [ "$cfgu_rc" -eq 2 ]; then echo "  ok   unusable CANDOR_CONFIG exits 2 (fail closed)"; pass=$((pass+1)); else echo "  FAIL unusable CANDOR_CONFIG exited $cfgu_rc (want 2)"; fail=$((fail+1)); fi
# a configured-but-EMPTY policy (a bare `policy` line) fails loud — never a silent gate skip.
printf 'policy\n' > "$CG/.candor/config"
cfgb_rc=0; cfgb=$( cd "$CG"; "$ROOT/cargo-candor" policy 2>&1 ) || cfgb_rc=$?
want "a bare 'policy' config line is loud" "$cfgb" "EMPTY value"
if [ "$cfgb_rc" -eq 2 ]; then echo "  ok   bare 'policy' config line exits 2 (fail closed)"; pass=$((pass+1)); else echo "  FAIL bare 'policy' config line exited $cfgb_rc (want 2)"; fail=$((fail+1)); fi
rm -rf "$(dirname "$CG")"

# ── 9-gj. --gate-json (spec §3.3): the deep path emits the SAME structured verdict as candor-scan ──
echo "== --gate-json structured verdict (cargo candor policy/guard vs candor-scan) =="
GJ=$(mktemp -d)/gj; mkdir -p "$GJ/src"
printf '[package]\nname="gj"\nversion="0.1.0"\nedition="2021"\n' > "$GJ/Cargo.toml"
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > "$GJ/src/main.rs"
echo "deny Fs  domain" > "$GJ/policy"
gjp_rc=0; ( cd "$GJ"; "$ROOT/cargo-candor" policy policy --gate-json verdict.json >/dev/null 2>&1 ) || gjp_rc=$?
if [ "$gjp_rc" -eq 1 ]; then echo "  ok   policy --gate-json still exits 1 on a violation"; pass=$((pass+1)); else echo "  FAIL policy --gate-json exited $gjp_rc (want 1)"; fail=$((fail+1)); fi
# The same fixture through the STABLE scanner's --gate-json — the verdicts must agree on the pinned
# projection (spec §3.3: ok + {rule, fn, effects}; `detail` is engine-natural prose, not pinned).
"$ROOT/target/debug/candor-scan" "$GJ" --policy "$GJ/policy" --gate-json "$GJ/scan-verdict.json" >/dev/null 2>&1 || true
gjcmp=$(python3 - "$GJ/verdict.json" "$GJ/scan-verdict.json" <<'PY'
import json, sys
deep, scan = (json.load(open(p)) for p in sys.argv[1:3])
proj = lambda d: (d["spec"], d["ok"], [(v["rule"], v["fn"], v["effects"]) for v in d["violations"]])
print("spec:", deep["spec"], "ok:", deep["ok"])
for v in deep["violations"]: print("viol:", v["rule"], v["fn"], ",".join(v["effects"]), "detail" if v.get("detail") else "")
print("PARITY" if proj(deep) == proj(scan) else f"MISMATCH {proj(deep)} vs {proj(scan)}")
PY
)
want "deep verdict declares spec 0.26 and fails"         "$gjcmp" "spec: 0.26 ok: False"
want "deep verdict pins the violation (rule/fn/effects)" "$gjcmp" "viol: AS-EFF-006 domain_logic Fs detail"
want "deep and scan verdicts agree on the §3.3 projection" "$gjcmp" "PARITY"
# A CLEAN gate writes the clean verdict { ok: true, violations: [] } and exits 0.
echo "deny Net  domain" > "$GJ/policy-clean"
gjc_rc=0; ( cd "$GJ"; "$ROOT/cargo-candor" policy policy-clean --gate-json verdict-clean.json >/dev/null 2>&1 ) || gjc_rc=$?
gjclean=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d["ok"], len(d["violations"]))' "$GJ/verdict-clean.json" 2>/dev/null)
want "a clean policy --gate-json writes { ok: true, [] }" "$gjclean" "True 0"
if [ "$gjc_rc" -eq 0 ]; then echo "  ok   clean policy --gate-json exits 0"; pass=$((pass+1)); else echo "  FAIL clean policy --gate-json exited $gjc_rc (want 0)"; fail=$((fail+1)); fi
# guard --gate-json: a gained effect (AS-EFF-005 — the deep-only rule) rides the same verdict shape.
( cd "$GJ"; "$ROOT/cargo-candor" snapshot .candor/base >/dev/null 2>&1 )
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn domain_logic(){ leaf(); }\nfn main(){ domain_logic(); }\n' > "$GJ/src/main.rs"
gjg_rc=0; ( cd "$GJ"; "$ROOT/cargo-candor" guard .candor/base --gate-json guard-verdict.json >/dev/null 2>&1 ) || gjg_rc=$?
gjguard=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); v=d["violations"]; print(d["ok"], v[0]["rule"] if v else "-", "Net" in (v[0]["effects"] if v else []))' "$GJ/guard-verdict.json" 2>/dev/null)
want "guard --gate-json pins the AS-EFF-005 gain (Net)" "$gjguard" "False AS-EFF-005 True"
if [ "$gjg_rc" -eq 1 ]; then echo "  ok   guard --gate-json still exits 1 on a gain"; pass=$((pass+1)); else echo "  FAIL guard --gate-json exited $gjg_rc (want 1)"; fail=$((fail+1)); fi
rm -rf "$(dirname "$GJ")"

# ── 9a. Host allowlist: enforce per-scope Net endpoints (AS-EFF-008) ──
echo "== host allowlist / AS-EFF-008 (CANDOR_POLICY allow Net) =="
HA=$(mktemp -d)/ha; mkdir -p "$HA/src"
printf '[package]\nname="ha"\nversion="0.1.0"\nedition="2021"\n' > "$HA/Cargo.toml"
# billing::charge reaches the allowed host (Stripe) via a helper; billing::track reaches a DIFFERENT
# host via a helper — the endpoint is in the helper, not in the named fn (transitive, like real code).
printf 'mod billing {\n  fn pay(){ let _=std::net::TcpStream::connect("api.stripe.com:443"); }\n  fn beacon(){ let _=std::net::TcpStream::connect("metrics.evil.example:443"); }\n  pub fn charge(){ pay(); }\n  pub fn track(){ beacon(); }\n}\nfn main(){ billing::charge(); billing::track(); }\n' > "$HA/src/main.rs"
echo "allow Net in billing  api.stripe.com" > "$HA/policy"
out=$(dl "$HA" env CANDOR_POLICY="$HA/policy")
want   "AS-EFF-008 flags the off-allowlist host reached transitively (billing::track)" "$out" '[AS-EFF-008] `billing::track`'
absent "the allowed-host path is NOT flagged (billing::charge → api.stripe.com)"       "$out" '[AS-EFF-008] `billing::charge`'
rm -rf "$(dirname "$HA")"

# ── 9a-Exec. Command allowlist: allow Exec <cmd> (AS-EFF-008) ──
echo "== command allowlist / AS-EFF-008 (allow Exec) =="
EX=$(mktemp -d)/ex; mkdir -p "$EX/src"
printf '[package]\nname="ex"\nversion="0.1.0"\nedition="2021"\n' > "$EX/Cargo.toml"
# build::sync runs `git` (allowed) via a helper; build::deploy runs `ssh` (not allowed) via a helper.
printf 'mod build {\n  fn run_git(){ let _=std::process::Command::new("git").status(); }\n  fn run_ssh(){ let _=std::process::Command::new("ssh").status(); }\n  pub fn sync(){ run_git(); }\n  pub fn deploy(){ run_ssh(); }\n}\nfn main(){ build::sync(); build::deploy(); }\n' > "$EX/src/main.rs"
echo "allow Exec in build  git" > "$EX/policy"
out=$(dl "$EX" env CANDOR_POLICY="$EX/policy")
want   "AS-EFF-008 flags the off-allowlist command reached transitively (build::deploy → ssh)" "$out" '[AS-EFF-008] `build::deploy`'
absent "the allowed command (git) is NOT flagged (build::sync)"                                "$out" '[AS-EFF-008] `build::sync`'
rm -rf "$(dirname "$EX")"

# ── 9a-Fs. Path allowlist: allow Fs <prefix> (AS-EFF-008) ──
echo "== path allowlist / AS-EFF-008 (allow Fs) =="
FA=$(mktemp -d)/fa; mkdir -p "$FA/src"
printf '[package]\nname="fa"\nversion="0.1.0"\nedition="2021"\n' > "$FA/Cargo.toml"
# config::load reads under /etc/app (allowed); config::leak reads /etc/shadow (outside the prefix).
printf 'mod config {\n  fn read_conf(){ let _=std::fs::read_to_string("/etc/app/conf.toml"); }\n  fn read_secret(){ let _=std::fs::read_to_string("/etc/shadow"); }\n  pub fn load(){ read_conf(); }\n  pub fn leak(){ read_secret(); }\n}\nfn main(){ config::load(); config::leak(); }\n' > "$FA/src/main.rs"
echo "allow Fs in config  /etc/app" > "$FA/policy"
out=$(dl "$FA" env CANDOR_POLICY="$FA/policy")
want   "AS-EFF-008 flags the path outside the allowed prefix (config::leak → /etc/shadow)" "$out" '[AS-EFF-008] `config::leak`'
absent "a path under the allowed prefix is NOT flagged (config::load → /etc/app/…)"       "$out" '[AS-EFF-008] `config::load`'
rm -rf "$(dirname "$FA")"

# ── 9a-mask. Masking fail-closed: a RUNTIME Fs path / Db table alongside a BENIGN allowed literal
# must FAIL the allowlist (the surface is incomplete) — a benign sibling must not certify the masked
# endpoint. This is the AS-EFF-008 masking guard generalized from Net/Exec to Fs/Db (gate-evasion fix).
echo "== masking fail-closed / AS-EFF-008 (allow Fs/Db, runtime locator) =="
MK=$(mktemp -d)/mk; mkdir -p "$MK/src"
printf '[package]\nname="mk"\nversion="0.1.0"\nedition="2021"\n' > "$MK/Cargo.toml"
# fs_mask: a benign /var/app write + a MASKED runtime-path write (format! → /etc/passwd).
# fs_ok: a single allowed literal write (no masking) — must certify.
printf 'pub fn fs_mask(){ let _=std::fs::write("/var/app/x", b"x"); let p=format!("/etc/{}","passwd"); let _=std::fs::write(p, b"x"); }\npub fn fs_ok(){ let _=std::fs::write("/var/app/x", b"x"); }\nfn main(){ fs_mask(); fs_ok(); }\n' > "$MK/src/main.rs"
echo "allow Fs  /var/app" > "$MK/policy"
out=$(dl "$MK" env CANDOR_POLICY="$MK/policy")
want   "AS-EFF-008 fails closed on a MASKED Fs path despite a benign sibling (fs_mask)" "$out" '[AS-EFF-008] `fs_mask`'
absent "a single allowed literal Fs path still certifies (fs_ok)"                       "$out" '[AS-EFF-008] `fs_ok`'
rm -rf "$(dirname "$MK")"

# ── 9b. Module layering: forbid a dependency direction (AS-EFF-009) ──
echo "== module layering / AS-EFF-009 (CANDOR_POLICY forbid) =="
LY=$(mktemp -d)/ly; mkdir -p "$LY/src"
printf '[package]\nname="ly"\nversion="0.1.0"\nedition="2021"\n' > "$LY/Cargo.toml"
# domain::checkout reaches infra::db::save TRANSITIVELY through a domain helper; domain::price doesn't.
printf 'mod infra { pub mod db { pub fn save(){} } }\nmod domain {\n  fn persist(){ crate::infra::db::save(); }\n  pub fn checkout(){ persist(); }\n  pub fn price()->u32{ 42 }\n}\nfn main(){ domain::checkout(); let _=domain::price(); }\n' > "$LY/src/main.rs"
echo "forbid domain -> infra" > "$LY/policy"
out=$(dl "$LY" env CANDOR_POLICY="$LY/policy")
want   "AS-EFF-009 flags the forbidden cross-layer dependency reached transitively (domain::checkout)" "$out" '[AS-EFF-009] `domain::checkout`'
absent "a domain fn that does NOT reach infra is not flagged (domain::price)"                          "$out" '[AS-EFF-009] `domain::price`'
rm -rf "$(dirname "$LY")"

# ── 9c. Implicit Drop: an effectful Drop guard propagates to the dropping fn (Bet 4 spike fix) ──
echo "== implicit drop edge (effectful Drop guard) =="
DR=$(mktemp -d)/dr; mkdir -p "$DR/src"
printf '[package]\nname="dr"\nversion="0.1.0"\nedition="2021"\n' > "$DR/Cargo.toml"
# Guard does network I/O on drop; via_drop holds one and lets it drop — HIR has no node for this, so
# the MIR-derived drop edge is what makes via_drop inherit Net. pure_fn must stay effect-free.
printf 'struct Guard;\nimpl Drop for Guard { fn drop(&mut self){ let _=std::net::TcpStream::connect("10.0.0.2:9"); } }\nfn via_drop(){ let _g = Guard; }\nfn pure_fn()->u32{ 1+2 }\nfn main(){ via_drop(); let _=pure_fn(); }\n' > "$DR/src/main.rs"
out=$(dl "$DR")
want   "the effectful Drop propagates to the dropping fn (via_drop gains Net)" "$out" '`via_drop` effects: { Net'
absent "a genuinely pure fn is NOT given a phantom effect (pure_fn)"           "$out" '`pure_fn` effects'
# LAUNDERED through a heap container: dropping a `Vec<Guard>` / `Box<Guard>` runs `Guard::drop` via the
# container's drop glue — hidden behind a raw pointer, so naive field-recursion would miss it. The
# curated owning-container type-arg walk (local_drop_impls) must still reach the element's effectful Drop.
printf 'struct Guard;\nimpl Drop for Guard { fn drop(&mut self){ let _=std::net::TcpStream::connect("10.0.0.2:9"); } }\nfn via_vec(){ let v = vec![Guard]; let _ = v; }\nfn via_box(){ let b = Box::new(Guard); let _ = b; }\nfn main(){ via_vec(); via_box(); }\n' > "$DR/src/main.rs"
lout=$(dl "$DR")
want   "Drop laundered through Vec<Guard> still propagates (via_vec gains Net)"  "$lout" '`via_vec` effects: { Net'
want   "Drop laundered through Box<Guard> still propagates (via_box gains Net)"  "$lout" '`via_box` effects: { Net'
# Through a TRAIT OBJECT: dropping `Box<dyn Job>` runs the concrete type's destructor via the vtable —
# statically unknown. candor CHAs local impls of the principal trait and follows their Drops, so a local
# effectful-Drop type behind `dyn Job` is caught (was a silent under-report). A `Box<dyn Error>` whose
# impls carry NO local Drop must stay pure — the over-approximation must not flood the common case.
printf 'trait Job {}\nstruct G;\nimpl Job for G {}\nimpl Drop for G { fn drop(&mut self){ let _=std::net::TcpStream::connect("10.0.0.2:9"); } }\nfn via_dyn(){ let b: Box<dyn Job> = Box::new(G); let _ = b; }\nfn drops_err(){ let _e: Box<dyn std::error::Error> = "x".into(); }\nfn main(){ via_dyn(); drops_err(); }\n' > "$DR/src/main.rs"
dout=$(dl "$DR")
want   "Drop through a trait object propagates (dyn Job with effectful Drop)"   "$dout" '`via_dyn` effects: { Net'
absent "trait-object drop with no local Drop impl does NOT flood (drops_err)"   "$dout" '`drops_err` effects'
# Through a hand-written container's `ptr::drop_in_place::<Guard>`: a raw-pointer container (smart
# pointer / arena) drops its element via the intrinsic, a non-local std call that carries no effect —
# so the element's Drop would be lost. The drop_in_place lang-item arm recovers `Guard` from the call's
# type arg and follows its local Drop, so the container's Drop (and its caller) gain Net.
printf 'use std::ptr::NonNull;\nstruct Guard;\nimpl Drop for Guard { fn drop(&mut self){ let _=std::net::TcpStream::connect("10.0.0.2:9"); } }\nstruct MyBox { p: NonNull<Guard> }\nimpl Drop for MyBox { fn drop(&mut self){ unsafe { std::ptr::drop_in_place(self.p.as_ptr()); } } }\nfn via_mybox(m: MyBox){ let _ = m; }\nfn main(){ let b = Box::new(Guard); via_mybox(MyBox{ p: NonNull::from(Box::leak(b)) }); }\n' > "$DR/src/main.rs"
pout=$(dl "$DR")
want   "Drop via ptr::drop_in_place propagates (MyBox::drop gains Net)"  "$pout" 'MyBox as std::ops::Drop>::drop` effects: { Net'
rm -rf "$(dirname "$DR")"

# ── 9d. Layering with a CRATE-NAME from-scope (the real-world fix: not a silent no-op) ──
echo "== layering crate-name from-scope (AS-EFF-009) =="
LC=$(mktemp -d)/lc; mkdir -p "$LC/src"
printf '[package]\nname="appcrate"\nversion="0.1.0"\nedition="2021"\n' > "$LC/Cargo.toml"
# `worker` reaches the `infra` module. The from-scope is the CRATE name `appcrate` — which a crate's own
# functions DON'T carry in def_path_str, so without the crate-prefix fix this rule would match nothing.
printf 'mod infra { pub fn save(){ let _=std::fs::write("/tmp/x",""); } }\nfn worker(){ infra::save(); }\nfn helper()->u32{ 1 }\nfn main(){ worker(); let _=helper(); }\n' > "$LC/src/main.rs"
echo "forbid appcrate -> infra" > "$LC/policy"
out=$(dl "$LC" env CANDOR_POLICY="$LC/policy")
want   "crate-name from-scope matches the crate's own fns (worker flagged)" "$out" '[AS-EFF-009] `worker`'
absent "a fn that doesn't reach infra is not flagged (helper)"              "$out" '[AS-EFF-009] `helper`'
rm -rf "$(dirname "$LC")"

# ── 10. Taint heuristic: an effect on caller-derived input (AS-EFF-007, P0′ §7) ──
echo "== taint heuristic / AS-EFF-007 (CANDOR_TAINT) =="
TZ=$(mktemp -d)/tz; mkdir -p "$TZ/src"
printf '[package]\nname="tz"\nversion="0.1.0"\nedition="2021"\n' > "$TZ/Cargo.toml"
# read_user builds the path from a parameter (injection class); read_fixed uses a literal (safe).
printf 'fn read_user(key:&str)->Option<String>{ std::fs::read_to_string(format!("/var/cache/{key}")).ok() }\nfn read_fixed()->Option<String>{ std::fs::read_to_string("/etc/app.conf").ok() }\nfn main(){ let _=read_user("x"); let _=read_fixed(); }\n' > "$TZ/src/main.rs"
out=$(dl "$TZ" env CANDOR_TAINT=1)
want   "AS-EFF-007 flags Fs on a parameter-derived path (read_user)" "$out" '[AS-EFF-007] `read_user`'
absent "a literal-path Fs is NOT flagged (read_fixed)"               "$out" '[AS-EFF-007] `read_fixed`'
rm -rf "$(dirname "$TZ")"

# ── 11. diff FAST PATH: read the kept-fresh report instead of recompiling (P0′ §8 / speed) ──
echo "== diff fast path (no recompile when .candor/report is fresh) =="
FP=$(mktemp -d)/fp; mkdir -p "$FP/src" "$FP/.candor"
printf '[package]\nname="fp"\nversion="0.1.0"\nedition="2021"\n' > "$FP/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn main(){ worker(); }\n' > "$FP/src/main.rs"
( cd "$FP"; "$ROOT/cargo-candor" snapshot .candor/baseline >/dev/null 2>&1 )
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main(){ worker(); }\n' > "$FP/src/main.rs"
# simulate the Stop hook: a fresh report + matching state hash for the edited source
( cd "$FP"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
"$QBIN" state "$FP" > "$FP/.candor/state"
fpout=$( cd "$FP"; "$ROOT/cargo-candor" diff 2>&1 )
want   "fast-path diff still computes the delta (worker +Net)"  "$fpout" "+Net"
absent "fast path did NOT recompile (no 'analyzing…')"          "$fpout" "analyzing"
rm -rf "$(dirname "$FP")"

# ── 12. watch: keep the report fresh in the background, off the critical path (P0′ §8 / speed) ──
echo "== watch (background report freshness) =="
WV=$(mktemp -d)/wv; mkdir -p "$WV/src"
printf '[package]\nname="wv"\nversion="0.1.0"\nedition="2021"\n' > "$WV/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn main(){ worker(); }\n' > "$WV/src/main.rs"
( cd "$WV"; CANDOR_WATCH_INTERVAL=1 "$ROOT/cargo-candor" watch >/dev/null 2>&1 ) & WPID=$!
for _ in $(seq 1 90); do ls "$WV"/.candor/report.*.*.json >/dev/null 2>&1 && break; sleep 1; done   # initial report
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main(){ worker(); }\n' > "$WV/src/main.rs"
seen=no
for _ in $(seq 1 90); do grep -q Net "$WV"/.candor/report.*.*.json 2>/dev/null && { seen=yes; break; }; sleep 1; done
kill "$WPID" 2>/dev/null; wait "$WPID" 2>/dev/null
want "watch auto-refreshed the report after an edit (worker gained Net)" "$seen" "yes"
rm -rf "$(dirname "$WV")"

# ── 13. Instant read-only queries served from the report: show / where (P0′ §8 / speed) ──
echo "== instant queries (show / where) =="
Q=$(mktemp -d)/q; mkdir -p "$Q/src" "$Q/.candor"
printf '[package]\nname="q"\nversion="0.1.0"\nedition="2021"\n' > "$Q/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn handler(){ leaf(); }\nfn reader(){ let _=std::fs::read_to_string("/tmp/cq_x"); }\nfn writer(){ let _=std::fs::write("/tmp/cq_x","y"); }\nfn both(){ reader(); writer(); }\nfn main(){ handler(); both(); }\n' > "$Q/src/main.rs"
# a fresh report (as watch/the hook would maintain) → the queries serve instantly, no recompile
( cd "$Q"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
"$QBIN" state "$Q" > "$Q/.candor/state"
shout=$( cd "$Q"; "$ROOT/cargo-candor" show handler 2>&1 )
whout=$( cd "$Q"; "$ROOT/cargo-candor" where Net 2>&1 )
whj=$(   cd "$Q"; "$ROOT/cargo-candor" where Net --json 2>&1 )
clout=$(  cd "$Q"; "$ROOT/cargo-candor" callers leaf 2>&1 )
want   "show: handler's transitive Net is reported"   "$shout" "Net"
absent "show served from the report (did NOT recompile)" "$shout" "generating one"
want   "where: splits the direct source out"          "$whout" "directly"
want   "where: names the source (leaf)"               "$whout" "leaf"
want   "where --json: machine-readable"               "$whj" '"directly"'
want   "callers: leaf's caller (handler) found from the report" "$clout" "handler"
# callers works on a PURE function too, transitively (the pre-edit blast-radius an agent asks for
# before adding an effect). The report omits pure fns, so this needs the call-graph sidecar.
cp_pure=$( cd "$Q"; "$ROOT/cargo-candor" callers reader 2>&1 )   # reader is Fs; both/main reach it
PC=$(mktemp -d)/pc; mkdir -p "$PC/src" "$PC/.candor"
printf '[package]\nname="pc"\nversion="0.1.0"\nedition="2021"\n' > "$PC/Cargo.toml"
# `helper` is PURE; reached by direct caller `mid` and transitively by `top`.
printf 'fn helper()->u32{ 41 }\nfn mid()->u32{ helper()+1 }\nfn top()->u32{ mid() }\nfn main(){ let _=top(); }\n' > "$PC/src/main.rs"
( cd "$PC"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
"$QBIN" state "$PC" > "$PC/.candor/state"
pcout=$( cd "$PC"; "$ROOT/cargo-candor" callers helper 2>&1 )
want   "callers on a PURE fn finds its direct caller (mid)"        "$pcout" "mid"
want   "callers on a PURE fn finds its TRANSITIVE caller (top)"    "$pcout" "top"
rm -rf "$(dirname "$PC")"
# Non-breaking Fs read/write refinement (P2): show annotates Fs with its access kind, transitively.
fsr=$( cd "$Q"; "$ROOT/cargo-candor" show reader 2>&1 )
fsb=$( cd "$Q"; "$ROOT/cargo-candor" show both 2>&1 )
fsj=$( cd "$Q"; "$ROOT/cargo-candor" show both --json 2>&1 )
want   "show: Fs read detail on a direct reader"      "$fsr" "Fs*(read)"
want   "show: Fs read+write propagates transitively"  "$fsb" "Fs(read,write)"
want   "show --json: fs detail is machine-readable"   "$fsj" '"fs"'
# Non-breaking Net host refinement (P2): a LITERAL connect address surfaces as Net(host), transitively.
hlf=$( cd "$Q"; "$ROOT/cargo-candor" show leaf 2>&1 )
hhd=$( cd "$Q"; "$ROOT/cargo-candor" show handler 2>&1 )
hjson=$( cd "$Q"; "$ROOT/cargo-candor" show leaf --json 2>&1 )
want   "show: literal Net endpoint on the source"    "$hlf" "Net*(127.0.0.1:1)"
want   "show: Net host propagates to the caller"     "$hhd" "Net(127.0.0.1:1)"
want   "show --json: hosts detail is machine-readable" "$hjson" '"hosts"'
# Closure-flow receiving side (P2): a HOF only ever passed NAMED fns resolves (no redundant Unknown);
# a HOF passed a closure keeps the honest Unknown.
CF=$(mktemp -d)/cf; mkdir -p "$CF/src"
printf '[package]\nname="cf"\nversion="0.1.0"\nedition="2021"\n' > "$CF/Cargo.toml"
printf 'fn named_hof(f: impl Fn()){ f(); }\nfn net_cb(){ let _=std::net::TcpStream::connect("h:1"); }\nfn user(){ named_hof(net_cb); }\nfn closure_hof(f: impl Fn()){ f(); }\nfn c2(){ closure_hof(|| { let _=std::net::TcpStream::connect("z:1"); }); }\nfn main(){ user(); c2(); }\n' > "$CF/src/main.rs"
( cd "$CF"; CANDOR_JSON="$PWD/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
cfa=$( cd "$CF"; "$ROOT/target/debug/candor-query" show "$PWD/r" named_hof 0 2>&1 )
cfu=$( cd "$CF"; "$ROOT/target/debug/candor-query" show "$PWD/r" user 0 2>&1 )
cfc=$( cd "$CF"; "$ROOT/target/debug/candor-query" show "$PWD/r" closure_hof 0 2>&1 )
# A HOF that invokes a generic param is honestly Unknown from its OWN standpoint — its effect depends on
# whatever callback the caller passes, so claiming a concrete effect was a leaky abstraction (and the
# crate-wide union of all callbacks FABRICATED the effect onto pure callers of a shared HOF). The
# per-call-site fix routes each callback's effect to the CALLER that passed it: `user` (passes net_cb)
# is precisely Net, with NO effect under-reported. (See ui/callbacks.rs for the domain_calc fabrication.)
want   "closure-flow: an invoked-param HOF is honestly Unknown (not a leaked caller effect)" "$cfa" "Unknown"
want   "closure-flow: the CALLER gets the callback's effect per-site (no under-report)" "$cfu" "Net"
want   "closure-flow: closure-passed HOF stays Unknown" "$cfc" "Unknown"
# candor-spec §2 entryPoint: the program `main` is a reachability root.
want   "report flags main as an entry point (entryPoint)" "$( cd "$CF"; cat r.*.json 2>/dev/null )" '"entryPoint": true'
# `reachable`: the program's runtime effect surface = union over entry points (main here reaches Net).
rch=$( cd "$CF"; "$ROOT/target/debug/candor-query" reachable "$PWD/r" 2>&1 )
want   "reachable: surfaces the program runtime effects from roots" "$rch" "Net"
want   "reachable: reports the entry-point union"                   "$rch" "union over"
# `path`: the provenance chain from a fn to an effect's direct source (handle -> mid -> leaf_net).
printf 'fn leaf_net(){ let _=std::net::TcpStream::connect("h:1"); }\nfn mid(){ leaf_net(); }\nfn handle(){ mid(); }\nfn main(){ handle(); }\n' > "$CF/src/main.rs"
( cd "$CF"; rm -f r.*.json; CANDOR_JSON="$PWD/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
pth=$( cd "$CF"; "$ROOT/target/debug/candor-query" path "$PWD/r" handle Net 2>&1 )
want   "path: traces through the middle fn"        "$pth" "mid"
want   "path: marks the direct source"             "$pth" "Net source"
want   "path: honest when fn lacks the effect"     "$( cd "$CF"; "$ROOT/target/debug/candor-query" path "$PWD/r" handle Db 2>&1 )" "does not perform Db"
# `impact`: blast radius — who transitively calls leaf_net, and which entry point (main) is downstream.
imp=$( cd "$CF"; "$ROOT/target/debug/candor-query" impact "$PWD/r" leaf_net 2>&1 )
want   "impact: counts transitive callers"         "$imp" "transitively call it"
want   "impact: surfaces the downstream entry point" "$imp" "main"
# SOUNDNESS regression guard: a HOF passed a NON-LOCAL named fn must keep the honest Unknown — it must
# NOT be silently dropped to pure (the non-local fn isn't edge-resolvable, so we can't certify purity).
printf 'fn nlhof(f: impl Fn(&str) -> String){ let _=f("x"); }\nfn nluser(){ nlhof(str::to_string); }\nfn main(){ nluser(); }\n' > "$CF/src/main.rs"
( cd "$CF"; rm -f r.*.json; CANDOR_JSON="$PWD/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
cfn=$( cd "$CF"; "$ROOT/target/debug/candor-query" show "$PWD/r" nlhof 0 2>&1 )
want   "closure-flow: NON-local callback keeps Unknown (soundness)" "$cfn" "Unknown"
# SOUNDNESS: a directly-invoked `Box<dyn Fn>` callback must be Unknown, not silently pure (it used to
# fall through resolve_callee's `_ => None` and record nothing at all).
printf 'fn boxed(cb: Box<dyn Fn()>){ cb(); }\nfn main(){ boxed(Box::new(|| {})); }\n' > "$CF/src/main.rs"
( cd "$CF"; rm -f r.*.json; CANDOR_JSON="$PWD/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
cfb=$( cd "$CF"; "$ROOT/target/debug/candor-query" show "$PWD/r" boxed 0 2>&1 )
want   "dyn-callable: boxed dyn Fn call is Unknown (soundness)" "$cfb" "Unknown"
# The report must say WHY it's Unknown (candor-spec §2 unknownWhy): a `callback:` origin tag for the
# unresolvable indirect call — distinguishing improvable opacity from irreducible.
cfraw=$( cd "$CF"; cat r.*.json 2>/dev/null )
want   "dyn-callable: report carries an unknownWhy origin tag" "$cfraw" '"unknownWhy"'
want   "dyn-callable: unknownWhy names the callback origin"    "$cfraw" 'callback:'
rm -rf "$(dirname "$CF")"
mapout=$( cd "$Q"; "$ROOT/cargo-candor" map 2>&1 )
want   "map: module/effects overview rendered"        "$mapout" "candor map"
want   "map: surfaces the Net effect"                 "$mapout" "Net"
rm -rf "$(dirname "$Q")"

# ── 13b. candor-scan --deps: registry-tree scan + chain (hermetic fake CARGO_HOME, no network) ──
echo "== candor-scan --deps (registry scan + cross-crate chain) =="
SCANBIN="$ROOT/target/debug/candor-scan"
DP=$(mktemp -d)
CHOME="$DP/cargo-home"; RIDX="$CHOME/registry/src/index.crates.io-0000000000000000"
mkdir -p "$RIDX/depi-0.3.0/src" "$DP/app/src"
printf '[package]\nname="depi"\n' > "$RIDX/depi-0.3.0/Cargo.toml"
printf 'pub fn eff() { let _ = std::fs::read("/etc/depi.conf"); }\n' > "$RIDX/depi-0.3.0/src/lib.rs"
printf '[package]\nname="app"\n\n[dependencies]\ndepi = "0.3.0"\n' > "$DP/app/Cargo.toml"
printf 'pub fn uses() { depi::eff(); }\n' > "$DP/app/src/lib.rs"
printf 'version = 3\n\n[[package]]\nname = "app"\nversion = "0.1.0"\n\n[[package]]\nname = "depi"\nversion = "0.3.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n' > "$DP/app/Cargo.lock"
dj=$(cd "$DP/app"; env -u CANDOR_DEPS -u CANDOR_POLICY -u CANDOR_CONFIG CARGO_HOME="$CHOME" "$SCANBIN" . --deps --json 2>"$DP/deps.err"); dj_rc=$?
want "deps: dep report written under .candor/deps/<name>@<version>/" "$(ls "$DP/app/.candor/deps/depi@0.3.0/" 2>/dev/null)" 'report.depi.scan.json'
want "deps: summary line counts the registry scan"                    "$(cat "$DP/deps.err")" 'scanned 1 of 1 registry dependencies'
want "deps: the dep effect crosses the crate boundary (uses gains Fs)" "$dj" '"Fs"'
want "deps: the dep literal surface rides the join"                    "$dj" '/etc/depi.conf'
if [ "$dj_rc" -eq 0 ]; then echo "  ok   deps: clean chained run exits 0"; pass=$((pass+1)); else echo "  FAIL deps: chained run exited $dj_rc (want 0)"; fail=$((fail+1)); fi
# missing lockfile → fail closed (exit 2), naming the incantation
rm "$DP/app/Cargo.lock"
nl_out=$(cd "$DP/app"; env -u CANDOR_DEPS -u CANDOR_POLICY -u CANDOR_CONFIG CARGO_HOME="$CHOME" "$SCANBIN" . --deps 2>&1); nl_rc=$?
want "deps: missing Cargo.lock names the fix"                          "$nl_out" 'generate-lockfile'
if [ "$nl_rc" -eq 2 ]; then echo "  ok   deps: missing Cargo.lock exits 2 (fail closed)"; pass=$((pass+1)); else echo "  FAIL deps: missing Cargo.lock exited $nl_rc (want 2)"; fail=$((fail+1)); fi
rm -rf "$DP"

# ── 14. MCP server: candor's queries as native agent tools (P0′ §10) ──
echo "== MCP server (candor-mcp.py) =="
MCP="$ROOT/integrations/mcp/candor-mcp.py"
mout=$(printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | python3 "$MCP" 2>/dev/null)
want "MCP: initialize returns serverInfo"        "$mout" '"serverInfo"'
want "MCP: tools/list exposes candor_where"      "$mout" 'candor_where'
want "MCP: tools/list exposes candor_callers"    "$mout" 'candor_callers'
M=$(mktemp -d)/m; mkdir -p "$M/src" "$M/.candor"
printf '[package]\nname="m"\nversion="0.1.0"\nedition="2021"\n' > "$M/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn handler(){ leaf(); }\nfn main(){ handler(); }\n' > "$M/src/main.rs"
( cd "$M"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
"$QBIN" state "$M" > "$M/.candor/state"
cout=$( cd "$M"; printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"candor_where","arguments":{"effect":"Net"}}}' | python3 "$MCP" 2>/dev/null )
want "MCP: tools/call candor_where returns the live result" "$cout" 'leaf'
rm -rf "$(dirname "$M")"

rm -rf "$(dirname "$G")" "$(dirname "$X")" 2>/dev/null

echo
echo "integration: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
