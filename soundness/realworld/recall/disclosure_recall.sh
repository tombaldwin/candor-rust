#!/usr/bin/env bash
# DISCLOSURE-RECALL battery for the syscall oracle.
#
# The syscall arm reports "0 violations". On its own that is not evidence the honesty invariant held -- it
# is equally consistent with an instrument that cannot see a violation. This battery settles which, by
# seeding the exact defect the oracle exists to catch and requiring it to turn red.
#
# Three passes of the REAL oracle (soundness/realworld/run.sh), identical except that the mutants falsify
# candor's report between the analyzer and the verdict:
#   control  unmutated               -> must be green (calibrate against a working oracle, not a broken one)
#   silent   all effects + all disclosure stripped -> the canonical false all-clear; must be caught
#   wrong    a decoy effect replaces the real one  -> must be caught, and must fabricate on the pure control
#
# Nothing else is stubbed: each pass builds the drivers, runs them under strace, and reads real traces, so
# what is calibrated is the deployed instrument end to end. Per-driver recall + the uncalibrated remainder
# are both reported -- see disclosure_recall_check.py for why they must travel together.
#
#   bash soundness/realworld/recall/disclosure_recall.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ORACLE="$HERE/../run.sh"
OUT="$HERE/.recall-out"

case "$(uname -s)" in Linux) : ;; *) echo "disclosure-recall battery: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;; esac
command -v strace >/dev/null 2>&1 || { echo "disclosure-recall battery: strace not installed — skipping."; exit 0; }

mkdir -p "$OUT"
rc=0

# Both syscall oracles are calibrated: the program-level verdict (run.sh) and the per-function one
# (pf/run_pf.sh, which attributes each syscall to every frame on the reconstructed stack). Calibrating only
# the first would leave the stronger of the two claims resting on an untested instrument.
calibrate() {  # <label> <oracle path> <checker format> <mutant modes...>
  local label="$1" oracle="$2" fmt="$3"; shift 3
  local modes=("$@") mode ec args=()
  echo "### oracle: $label"
  for mode in control "${modes[@]}"; do
    echo "=== pass: $mode ==="
    if [ "$mode" = control ]; then
      unset CANDOR_ORACLE_MUTATE
      bash "$oracle" >"$OUT/$label.$mode.log" 2>&1
    else
      CANDOR_ORACLE_MUTATE="$mode" bash "$oracle" >"$OUT/$label.$mode.log" 2>&1
    fi
    ec=$?
    echo "  exit=$ec  ($(grep -c . "$OUT/$label.$mode.log") lines -> $OUT/$label.$mode.log)"
    [ "$mode" = control ] || args+=("$mode=$OUT/$label.$mode.log")
  done
  echo
  python3 "$HERE/disclosure_recall_check.py" "$fmt" "$OUT/$label.control.log" "${args[@]}" || rc=1
  echo
}

# The program-level verdict unions the whole report, so only a lie spanning it is visible there. The
# per-function verdict adjudicates every frame on the reconstructed stack, so it additionally gets the
# `transitive` mutant — a lie confined to one caller with the effect's leaf left honest, which is what a
# dropped call-graph edge looks like and what neither of the other two mutants can distinguish.
calibrate program "$ORACLE" program silent wrong
[ -f "$HERE/../pf/run_pf.sh" ] && calibrate perfn "$HERE/../pf/run_pf.sh" perfn silent wrong transitive

exit $rc
