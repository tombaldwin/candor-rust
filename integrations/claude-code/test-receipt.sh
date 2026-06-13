#!/usr/bin/env bash
# Tests for candor-run.sh's receipt logic — coverage detection, freshness, and the
# sidecar-vs-report glob (the bug that once made the calibrated/encountered sidecars get
# parsed as reports). Runs WITHOUT a `cargo dylint` build: we pre-populate `.candor` with
# fixture reports and freeze the source hash so the script treats them as current.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="$HERE/candor-run.sh"
# The receipt is powered by candor-query now (was inline Python). Build/locate it and point
# candor-run.sh at it via CANDOR_QUERY. (stop-hook.sh still parses Claude's hook JSON with python3 —
# the stop-hook subtests below skip if python3 is absent.)
QUERY=""   # newest-by-mtime among release/debug (not `ls|head`, which sorts alphabetically → debug)
for q in "$HERE/../../target/release/candor-query" "$HERE/../../target/debug/candor-query"; do
  [ -x "$q" ] || continue
  if [ -z "$QUERY" ] || [ "$q" -nt "$QUERY" ]; then QUERY="$q"; fi
done
[ -n "$QUERY" ] || { echo "SKIP: candor-query not built (run: cargo build -p candor-query)"; exit 0; }
export CANDOR_QUERY="$QUERY"

pass=0; fail=0
chk()  { if printf '%s' "$2" | grep -qE -- "$3"; then echo "  ok   $1"; pass=$((pass+1)); else echo "  FAIL $1 — want /$3/"; echo "       in: $2"; fail=$((fail+1)); fi; }
nchk() { if printf '%s' "$2" | grep -qE -- "$3"; then echo "  FAIL $1 — unwanted /$3/"; fail=$((fail+1)); else echo "  ok   $1"; pass=$((pass+1)); fi; }

T=$(mktemp -d)/proj; mkdir -p "$T/src" "$T/.candor"
printf '[package]\nname="proj"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$T/Cargo.toml"
printf 'fn a(){}\nfn b(){}\n' > "$T/src/lib.rs"

# Three real report entries…
cat > "$T/.candor/report.proj.Rlib.json" <<'JSON'
[{"fn":"a","loc":"src/lib.rs:1","inferred":["Fs"],"direct":["Fs"],"unresolved":false},
 {"fn":"b","loc":"src/lib.rs:2","inferred":["Net","Unknown"],"direct":["Net"],"unresolved":true},
 {"fn":"c","loc":"src/lib.rs:3","inferred":["Db"],"direct":["Db"],"unresolved":false}]
JSON
# …and the two sidecars that must NOT be counted as functions (the glob-collision regression).
echo '{"crates":["reqwest"],"prefixes":["aws_sdk_"]}' > "$T/.candor/report.calibrated.json"
echo '["scylla","serde","tokio"]' > "$T/.candor/report.encountered-proj-Rlib.json"

# Freeze the source hash (same canonical `candor-query state` the hook uses) so candor-run treats the
# report as CURRENT (no dylint re-run).
"$QUERY" state "$T" > "$T/.candor/state"

out=$("$RUN" "$T" 2>/dev/null)
echo "receipt: $out"
chk  "counts 3 fns (sidecars excluded — the glob-collision regression)" "$out" '3 fns'
chk  "effect breakdown rendered"                                        "$out" '(Db|Net|Fs)'
chk  "1 unresolved (b carries Unknown)"                                 "$out" '1 unresolved'
chk  "freshness = current (source hash unchanged)"                      "$out" 'current @'
chk  "coverage flags the uncalibrated effectful dep (scylla)"           "$out" 'scylla'
nchk "calibrated/std deps NOT flagged (reqwest/serde/tokio)"            "$out" '(reqwest|serde|tokio)'

# A real source change must flip freshness off "current".
printf 'fn a(){}\nfn b(){}\nfn d(){}\n' > "$T/src/lib.rs"
out2=$("$RUN" "$T" 2>/dev/null || true)
nchk "after an edit, no longer reports 'current'"                       "$out2" 'current @'
rm -rf "$(dirname "$T")"

# The receipt must stamp the dylib's TRUE build version (read from the embedded tag), not a git
# guess — the honest-provenance fix. Needs a built dylib + strings; skip otherwise.
LIBV=$(find "$HERE/../../target/debug" -maxdepth 1 \( -name 'libcandor@*.dylib' -o -name 'libcandor@*.so' \) 2>/dev/null | head -1)
tag=""; [ -n "$LIBV" ] && command -v strings >/dev/null 2>&1 && \
  tag=$(strings -a "$LIBV" 2>/dev/null | grep -oE 'candor-build-version=[0-9a-fA-F]+' | head -1 | cut -d= -f2)
if [ -n "$tag" ]; then
  V=$(mktemp -d)/p; mkdir -p "$V/src" "$V/.candor"
  printf '[package]\nname="p"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$V/Cargo.toml"
  printf 'fn a(){}\n' > "$V/src/lib.rs"
  echo '[{"fn":"a","loc":"src/lib.rs:1","inferred":["Fs"],"direct":["Fs"],"unresolved":false}]' > "$V/.candor/report.p.Rlib.json"
  echo "CANDOR_LIB=$LIBV" > "$V/.candor/config"
  "$QUERY" state "$V" > "$V/.candor/state"
  outv=$("$RUN" "$V" 2>/dev/null)
  chk "receipt stamps the dylib's true build version (@$tag)"           "$outv" "@$tag"
  rm -rf "$(dirname "$V")"
else
  echo "  skip version-stamp check (no built dylib / no strings)"
fi

# The receipt must summarize the v0.2 self-describing envelope, not just the legacy bare array.
E=$(mktemp -d)/e; mkdir -p "$E/src" "$E/.candor"
printf '[package]\nname="e"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$E/Cargo.toml"
printf 'fn a(){}\n' > "$E/src/lib.rs"
cat > "$E/.candor/report.e.Rlib.json" <<'JSON'
{"candor":{"version":"deadbee","toolchain":"nightly-x"},
 "functions":[{"fn":"a","loc":"src/lib.rs:1","inferred":["Db"],"direct":["Db"],"unresolved":false},
              {"fn":"b","loc":"src/lib.rs:2","inferred":["Net","Unknown"],"direct":["Net"],"unresolved":true}]}
JSON
"$QUERY" state "$E" > "$E/.candor/state"
oute=$("$RUN" "$E" 2>/dev/null)
chk  "receipt summarizes the v0.2 envelope (2 fns from {candor,functions})"  "$oute" '2 fns'
chk  "envelope: effect breakdown + unresolved counted through .functions"    "$oute" '1 unresolved'
rm -rf "$(dirname "$E")"

# §2 edit-time self-review (CANDOR_REVIEW): a newly-introduced effect → prompt + exit 11, once.
R=$(mktemp -d)/r; mkdir -p "$R/src" "$R/.candor"
printf '[package]\nname="r"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$R/Cargo.toml"
printf 'fn worker(){}\n' > "$R/src/lib.rs"
echo "CANDOR_REVIEW=1" > "$R/.candor/config"
# baseline: worker performs Fs only.  current report: worker gained Net.  (no version file → gate open)
echo '{"candor":{"version":"v1"},"functions":[{"fn":"worker","inferred":["Fs"],"direct":["Fs"],"unresolved":false}]}' > "$R/.candor/baseline.r.Rlib.json"
echo '{"candor":{"version":"v1"},"functions":[{"fn":"worker","inferred":["Fs","Net"],"direct":["Fs","Net"],"unresolved":false}]}' > "$R/.candor/report.r.Rlib.json"
"$QUERY" state "$R" > "$R/.candor/state"
outr="$("$RUN" "$R" 2>/dev/null)"; codr=$?
chk  "review: candor-run exits 11 on a newly-introduced effect"  "$codr" '^11$'
chk  "review: the prompt names the gained Net"                   "$outr" 'gained \{ Net'
outr2="$("$RUN" "$R" 2>/dev/null)"; codr2=$?
nchk "review: the same effect is NOT re-surfaced (exit ≠ 11)"    "$codr2" '^11$'
# stop-hook feeds the agent only when not already looping. (stop-hook.sh parses the hook JSON with
# python3 — skip these two if it's unavailable.)
if command -v python3 >/dev/null 2>&1; then
  rm -f "$R/.candor/review-seen"
  hb=$(printf '{"cwd":"%s","stop_hook_active":false}' "$R" | bash "$HERE/stop-hook.sh" 2>/dev/null)
  chk  "stop-hook: blocks + feeds the agent on a new effect"       "$hb" '"decision": ?"block"'
  chk  "stop-hook: routes the prompt via additionalContext"        "$hb" 'additionalContext'
  rm -f "$R/.candor/review-seen"
  hl=$(printf '{"cwd":"%s","stop_hook_active":true}' "$R" | bash "$HERE/stop-hook.sh" 2>/dev/null)
  nchk "stop-hook: does NOT block when already looping (loop guard)" "$hl" '"decision"'
else
  echo "  skip stop-hook subtests (no python3)"
fi
rm -rf "$(dirname "$R")"

# Zero-install fallback: with NO nightly dylib but candor-scan present, the receipt must still produce a
# report (via the stable backend) and mark itself honestly. Needs the scan binary; skip otherwise.
SCANB=""
for s in "$HERE/../../target/release/candor-scan" "$HERE/../../target/debug/candor-scan"; do
  [ -x "$s" ] && { SCANB="$s"; break; }
done
if [ -n "$SCANB" ]; then
  S=$(mktemp -d)/s; mkdir -p "$S/src"
  printf '[package]\nname="s"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$S/Cargo.toml"
  printf 'use std::fs;\npub fn load(){ let _ = fs::read_to_string("/etc/hosts"); }\n' > "$S/src/lib.rs"
  # CANDOR_HOME points nowhere with a dylib; CANDOR_SCAN forces the scanner; no CANDOR_LIB.
  outs=$(CANDOR_BACKEND=scan CANDOR_HOME="$(mktemp -d)" CANDOR_SCAN="$SCANB" "$RUN" --force "$S" 2>/dev/null)
  chk  "no-dylib: receipt falls back to the stable scanner (produces a report)" "$outs" '[0-9]+ fns'
  chk  "no-dylib: receipt detects the Fs effect via scan"                       "$outs" 'Fs'
  chk  "no-dylib: receipt is honestly marked as the stable backend"             "$outs" 'stable backend'
  chk  "no-dylib: a scan report file was written"                               "$(ls "$S/.candor/" 2>/dev/null)" 'report\..*\.scan\.json'

  # Contract-refresh nudge: the stamp must hold candor-scan's real SEMVER on the scan backend (not
  # the literal "scan"), so a `cargo install` upgrade actually moves it and fires the nudge.
  chk  "agents-nudge: stamp holds the scan SEMVER (not the literal 'scan')" "$(cat "$S/.candor/engine-version" 2>/dev/null)" 'candor-scan [0-9]'
  outs=$(CANDOR_BACKEND=scan CANDOR_HOME="$(mktemp -d)" CANDOR_SCAN="$SCANB" "$RUN" --force "$S" 2>/dev/null)
  nchk "agents-nudge: same engine version -> no refresh nudge"  "$outs" 're-read the contract'
  printf 'candor-scan 0.0.1\n' > "$S/.candor/engine-version"   # simulate an OLD installed version
  outs=$(CANDOR_BACKEND=scan CANDOR_HOME="$(mktemp -d)" CANDOR_SCAN="$SCANB" "$RUN" --force "$S" 2>/dev/null)
  chk  "agents-nudge: a scan version change fires the re-read nudge" "$outs" 'candor-scan 0.0.1 → candor-scan.*re-read the contract'
  chk  "agents-nudge: the stamp updates after the nudge"        "$(cat "$S/.candor/engine-version" 2>/dev/null)" 'candor-scan [0-9]'
  rm -rf "$(dirname "$S")"

  # REVIEW + contract-refresh on the SAME turn: the nudge must ride the exit-11 review prompt, never
  # be swallowed (the stamp advances regardless, so a swallowed nudge is lost forever). The review
  # only fires on the LINT backend (by design — §2 self-review needs the lint's soundness), so this
  # uses a built dylib via CANDOR_LIB (deterministic; skips when none is built) + lint-named reports.
  DYLIB=""
  for d in "$HERE/../../target/debug"/libcandor@*.dylib "$HERE/../../target/debug"/libcandor@*.so; do
    [ -e "$d" ] && { DYLIB="$d"; break; }
  done
  if [ -n "$DYLIB" ]; then
    VERX="$("$QUERY" engine-version "$DYLIB" 2>/dev/null || echo lintver)"
    SR=$(mktemp -d)/sr; mkdir -p "$SR/src" "$SR/.candor"
    printf '[package]\nname="sr"\nversion="0.1.0"\nedition="2021"\n[dependencies]\n' > "$SR/Cargo.toml"
    printf 'fn worker(){}\n' > "$SR/src/lib.rs"
    echo "CANDOR_REVIEW=1" > "$SR/.candor/config"
    echo '{"candor":{"version":"v1"},"functions":[{"fn":"worker","inferred":["Fs"],"direct":["Fs"],"unresolved":false}]}' > "$SR/.candor/baseline.sr.Rlib.json"
    echo '{"candor":{"version":"v1"},"functions":[{"fn":"worker","inferred":["Fs","Net"],"direct":["Fs","Net"],"unresolved":false}]}' > "$SR/.candor/report.sr.Rlib.json"
    "$QUERY" state "$SR" > "$SR/.candor/state"
    printf 'stale-engine-xyz\n' > "$SR/.candor/engine-version"   # != VERX -> nudge must fire
    outsr=$(CANDOR_LIB="$DYLIB" CANDOR_HOME="$(mktemp -d)" "$RUN" "$SR" 2>/dev/null); codsr=$?
    chk  "review+nudge: exits 11 on the review"                            "$codsr" '^11$'
    chk  "review+nudge: the contract-refresh nudge rides the review prompt" "$outsr" 're-read the contract'
    rm -rf "$(dirname "$SR")"
  else
    echo "  skip review+nudge check (no dylib built)"
  fi
else
  echo "  skip scan-fallback check (candor-scan not built)"
fi

echo
echo "receipt tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
