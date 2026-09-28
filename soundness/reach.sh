#!/usr/bin/env bash
# reach.sh — the drop-glue REACH probe, CALIBRATED THEN RUN.  SOUNDNESS R766.
#
#     bash soundness/reach.sh                 # calibrate, then measure the pinned rust census
#     bash soundness/reach.sh --calibrate     # calibrate only
#     bash soundness/reach.sh --corpus <dir>  # calibrate, then measure <dir> (one crate per subdir)
#
# THE CALIBRATION IS NOT OPTIONAL AND IT GATES THE MEASUREMENT. R766 is a row about a probe that
# produced an authoritative-looking number while measuring the wrong set, and this register is full
# of instruments that were green on arrival and wrong. So: five planted fixtures run first, and a
# single MISS exits non-zero before any census number is printed.
#
#   pos            every row's own re-verified cell, on a type with a local `impl Drop`  -> TIGHT 1 each
#   neg-nodrop     the same file with `impl Drop for H` deleted                          -> TIGHT 0 each
#   neg-otherdrop  the same file plus an UNRELATED `impl Drop for G`                     -> TIGHT 0 each
#   neg-shape      the ROWS' OWN CHARGED CONTROLS, all on a type WITH a destructor       -> TIGHT 0 each
#   pos-owned      the `owned_drops` half, with R718's borrowed controls beside it
#
# `neg-otherdrop` is the one that matters: it is R766's own defect as a fixture. The old probe's
# predicate — *crate declares a local `impl Drop`* AND *the shape appears* — scores 9 hits on it.
# The predicate a FIX needs scores 0.
#
# The census is the PINNED roster (`candor/bin/corpus-census-rust.tsv`, acquired by
# `corpus-census.sh rust` into `~/.candor/census/rust`), for R663's reason: a percentage with an
# unrepeatable denominator is not a measurement.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REACH="$HERE/reach"
CENSUS="${CANDOR_CENSUS_HOME:-$HOME/.candor/census}/rust"
TARGET="${CANDOR_REACH_TARGET:-$REACH/target}"
MODE="${1:-}"
CORPUS="$CENSUS"
[ "$MODE" = "--corpus" ] && CORPUS="${2:?--corpus needs a directory}"

BIN="$TARGET/release/candor-reach"
echo "== build =="
( cd "$REACH" && CARGO_TARGET_DIR="$TARGET" cargo build --release ) || exit 2
[ -x "$BIN" ] || { echo "reach.sh: no binary at $BIN"; exit 2; }

echo
echo "== the fixtures are real Rust =="
for d in pos neg-nodrop neg-otherdrop neg-shape pos-owned; do
  ( cd "$REACH/calibration/$d" && CARGO_TARGET_DIR="$TARGET/cal-$d" cargo check --quiet ) \
    || { echo "reach.sh: calibration fixture $d does not compile"; exit 2; }
  echo "  cargo check $d OK"
done

echo
echo "== CALIBRATION =="
fail=0
for d in pos neg-nodrop neg-otherdrop neg-shape pos-owned; do
  echo "-- $d"
  "$BIN" --crate "$REACH/calibration/$d" --expect "$REACH/calibration/expect-$d.txt" | sed -n '/CALIBRATION/,$p'
  # NOT `$?` after a pipe (feedback-measure-directly): read the exit status of the probe itself.
  st=${PIPESTATUS[0]}
  [ "$st" -ne 0 ] && fail=1
done
if [ "$fail" -ne 0 ]; then
  echo
  echo "CALIBRATION FAILED — no census number is printed. The probe is not evidence."
  exit 1
fi

[ "$MODE" = "--calibrate" ] && { echo; echo "calibration only — done."; exit 0; }

if [ ! -d "$CORPUS" ]; then
  echo
  echo "reach.sh: no corpus at $CORPUS"
  echo "  acquire it:  bash ../candor/bin/corpus-census.sh rust"
  exit 2
fi

echo
echo "== MEASUREMENT: $CORPUS =="
"$BIN" --crates "$CORPUS" --hits 6 --tsv "$TARGET/reach-hits.tsv"
echo
echo "READ soundness/REACH.md BEFORE QUOTING ANY OF THIS. It states what the probe cannot see."
