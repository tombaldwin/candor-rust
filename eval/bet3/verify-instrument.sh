#!/usr/bin/env bash
# verify-instrument.sh — the teeth ON the teeth. Proves the Bet 3 verification scripts can tell
# "candor found nothing" apart from "candor never ran".
#
# Why this exists (R108, measured 2026-09-02). verify.sh and verify-layering.sh each pair a positive
# assertion with an absence control ("the allowed host is NOT flagged", "the pure function is NOT
# flagged"). Absence is also exactly what a dead instrument produces, so when their shared locator
# handed `cargo dylint` a library it could not load, the positive assertion failed and the absence
# control printed PASS — and both scripts read as "the harness is live, the detection is broken",
# which was the inverse of the truth. AGENT-CORPUS-BRIEF §E3: a control that asserts an absence is no
# evidence at all unless it has been proven able to fail.
#
# So this script drives the three ways the instrument can be dead and asserts, each time, that the
# scripts REFUSE to adjudicate — in particular that no line starting `PASS` is ever printed over a run
# candor did not perform. Case 4 is the green control: with a real library the same copy passes, so a
# guard that simply rejected everything could not survive this file either.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$ROOT/_lib.sh"

REAL="$(candor_lib "$ROOT/../../target/debug")"
if [ -z "$REAL" ]; then
  echo "FAIL: no host-ABI candor lint library to run the green control against — build the repo first"
  exit 1
fi
case "$(uname -s)" in Darwin) EXT="dylib" ;; *) EXT="so" ;; esac

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir -p "$SCRATCH/target/debug" "$SCRATCH/eval"
cp -R "$ROOT" "$SCRATCH/eval/bet3"
rm -rf "$SCRATCH"/eval/bet3/*/target "$SCRATCH"/eval/bet3/*/r.*.json

fail=0

# $1 label, $2 script, $3 substring the diagnostic must contain
expect_refusal() {
  local label="$1" script="$2" want="$3" out rc
  out="$(bash "$SCRATCH/eval/bet3/$script" 2>&1)"; rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "FAIL: $label — $script exited 0 with a dead instrument"; fail=1; return
  fi
  if ! printf '%s\n' "$out" | grep -qF "$want"; then
    echo "FAIL: $label — $script did not report \"$want\""; printf '%s\n' "$out" | head -5; fail=1; return
  fi
  # The one that matters: an absence control must not be credited over a run that did not happen.
  if printf '%s\n' "$out" | grep -q '^PASS'; then
    echo "FAIL: $label — $script printed a PASS over a dead instrument (the vacuous control is back)"
    printf '%s\n' "$out" | grep '^PASS'; fail=1; return
  fi
  echo "PASS: $label — $script refuses to adjudicate, and credits no control"
}

# 1) No lint library at all: the unbuilt-tree case. The old guard could not fire here, because
#    `basename ""` left $LIB pointing at the target/debug DIRECTORY, which `-e` accepts.
rm -f "$SCRATCH"/target/debug/libcandor@*
expect_refusal "no lint library present"     verify-layering.sh "no candor lint library"
expect_refusal "no lint library present"     verify.sh          "no candor lint library"

# 2) Only a FOREIGN-ABI library present: the container-leg case that triggered R108 — a Linux .so beside
#    (or instead of) the macOS dylib. Selecting it is not a degraded run, it is no run.
case "$EXT" in dylib) OTHER="so" ;; *) OTHER="dylib" ;; esac
rm -f "$SCRATCH"/target/debug/libcandor@*
cp "$REAL" "$SCRATCH/target/debug/libcandor@nightly-0000-00-00-foreign-triple.$OTHER"
expect_refusal "only a foreign-ABI library"  verify-layering.sh "no candor lint library"

# 3) A HOST-named library that dylint cannot load. This is the exact shape that produced the false
#    "detection is broken" reading: the locator succeeds, the run does not.
rm -f "$SCRATCH"/target/debug/libcandor@*
printf 'not a shared library\n' > "$SCRATCH/target/debug/libcandor@nightly-0000-00-00-host.$EXT"
expect_refusal "unloadable host library"     verify-layering.sh "candor did not run"
# verify.sh reaches its snapshot precondition first — its own liveness check, on the artefact rather
# than the stream, because that step discards its output.
expect_refusal "unloadable host library"     verify.sh          "snapshot wrote no httpkit report"

# 4) GREEN CONTROL. The same copy, with a real library, must still pass — otherwise the three refusals
#    above would be satisfied by a check that rejects everything.
rm -f "$SCRATCH"/target/debug/libcandor@*
cp "$REAL" "$SCRATCH/target/debug/$(basename "$REAL")"
gout="$(bash "$SCRATCH/eval/bet3/verify-layering.sh" 2>&1)"; grc=$?
if [ "$grc" -eq 0 ] && printf '%s\n' "$gout" | grep -q '^PASS: cross-crate dependency flagged'; then
  echo "PASS: green control — the same scripts still pass with a real lint library"
else
  echo "FAIL: green control — verify-layering.sh no longer passes with a real lint library (exit $grc)"
  printf '%s\n' "$gout" | head -10; fail=1
fi

exit $fail
