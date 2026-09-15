#!/usr/bin/env bash
# verify-binary.sh — prove a PACKAGED candor-scan / candor-query before it is attached to a release.
#
#   bash ci/verify-binary.sh <candor-scan> <candor-query> [expected-version]
#
# WHY THIS EXISTS. candor-rust published no release assets at all, so `candor update rust` could only
# ever `cargo install` — and on a machine with no toolchain the front door dead-ended: "fetching the
# rust engine…", then "skipped (no Rust toolchain)", then "could not be fetched — run `candor update
# rust`", a remedy whose second lap prints the same skip. candor-java and candor-swift both ship native
# binaries (no JVM, no Swift toolchain required); the RUST engine was the only one demanding a compiler,
# which is the reverse of what anyone would guess.
#
# WHAT IT CHECKS, AND WHY NOT "PARITY". candor-java's `native.yml` compares the native image's report
# against the jar's, because those are two genuinely different resolution paths that have diverged in
# the field — on v0.32.0 it withheld two binaries that reported an EMPTY scan at exit 0, 0 functions
# where the jar found 210. There is no second implementation here: a release binary is the same code
# `cargo install` builds. So the question this file can answer is the other half of that defect —
# **does the packaged binary actually analyse anything** — and it answers it against a fixture whose
# ground truth is known rather than against a threshold someone guessed.
#
# `sample/` is that fixture: a crate written in the capability discipline, already in this repo for
# trying conformance mode. Measured on candor-scan 0.38.0 (2026-09-14): 12 analyzed, 10 functions,
# effects {Clock, Env, Exec, Fs, Unknown}. The assertions below are floors and a required-effect SET,
# not exact equality — an engine improvement that finds MORE must not redden a release, while the
# failure this exists for (a binary that runs, exits 0 and finds nothing) cannot pass any of them.
#
# BOTH BINARIES, because shipping one unverified is the asymmetry that started this. `candor-query` is
# what `candor tour` / `candor where` run, and a `candor update rust` that installed a working scanner
# beside a broken query would look entirely healthy until the first question.
set -euo pipefail

SCAN="${1:?usage: verify-binary.sh <candor-scan> <candor-query> [expected-version]}"
QUERY="${2:?usage: verify-binary.sh <candor-scan> <candor-query> [expected-version]}"
WANT_VER="${3:-}"
# ACCEPT A TAG AS WELL AS A VERSION. Callers naturally pass `github.ref_name`, which is `v0.38.1`, while
# a binary reports `0.38.1` — so the substring test below failed on the FIRST real release run and the
# gate refused two perfectly good binaries. It was right to refuse (the check is "does this binary say
# what I expect"), and the argument was wrong. Strip one leading `v` here rather than at each call site,
# so a caller cannot get it wrong again.
WANT_VER="${WANT_VER#v}"

HERE="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
fail() { echo "verify-binary: ✘ $*" >&2; exit 1; }

[ -x "$SCAN" ]  || fail "not executable: $SCAN"
[ -x "$QUERY" ] || fail "not executable: $QUERY"

# [1] It runs at all, and it is the build we think it is. A binary built from the wrong ref passes every
#     behavioural check below — the version string is the only thing that can catch it.
sv="$("$SCAN"  --version 2>/dev/null | head -1)" || fail "candor-scan --version failed"
qv="$("$QUERY" --version 2>/dev/null | head -1)" || fail "candor-query --version failed"
echo "verify-binary: scan  = $sv"
echo "verify-binary: query = $qv"
if [ -n "$WANT_VER" ]; then
  case "$sv" in *"$WANT_VER"*) ;; *) fail "candor-scan reports '$sv', expected version $WANT_VER";; esac
  case "$qv" in *"$WANT_VER"*) ;; *) fail "candor-query reports '$qv', expected version $WANT_VER";; esac
fi

# [2] It analyses something. THIS is the check that would have caught candor-java's v0.32.0 binaries.
[ -d "$HERE/sample" ] || fail "fixture missing: $HERE/sample (this check cannot run, which is not a pass)"
"$SCAN" "$HERE/sample" --out "$WORK/r.json" >/dev/null 2>&1 || fail "scan of sample/ failed"

REPORT="$(find "$WORK" -name '*.json' ! -name '*callgraph*' | head -1)"
[ -n "$REPORT" ] || fail "scan produced no report file"

python3 - "$REPORT" <<'PY' || exit 1
import json, sys
d = json.load(open(sys.argv[1]))
fns = d.get("functions", [])
analyzed = (d.get("analyzed") or {}).get("count", 0)
effects = {e for f in fns for e in f.get("inferred", [])}
# Floors, measured on 0.38.0 (12 / 10). A packaged binary that finds LESS than the fixture demonstrably
# contains is broken; one that finds more is an improvement and must not redden a release.
need = {"Clock", "Env", "Exec", "Fs"}
bad = []
if analyzed < 12:      bad.append(f"analyzed.count {analyzed} < 12")
if len(fns) < 10:      bad.append(f"functions {len(fns)} < 10")
if not need <= effects: bad.append(f"missing effect(s) {sorted(need - effects)}; got {sorted(effects)}")
if bad:
    print("verify-binary: ✘ the packaged binary under-reports the known fixture:", file=sys.stderr)
    for b in bad: print("   ·", b, file=sys.stderr)
    print("   this is the shape candor-java shipped at v0.32.0 — runs, exits 0, finds nothing.", file=sys.stderr)
    raise SystemExit(1)
print(f"verify-binary: ✔ sample/ → {analyzed} analyzed, {len(fns)} functions, effects {sorted(effects)}")
PY

# [3] The QUERY binary answers off that report — the half that would otherwise ship unverified.
out="$("$QUERY" where Fs --report "$REPORT" 2>&1)" || fail "candor-query where Fs failed: $out"
case "$out" in
  *Fs*|*fs*) echo "verify-binary: ✔ candor-query answers off the report" ;;
  *) fail "candor-query returned nothing recognisable for 'where Fs': $out" ;;
esac

echo "verify-binary: OK — both binaries run, analyse the known fixture, and answer."
