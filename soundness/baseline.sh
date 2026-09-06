#!/usr/bin/env bash
# baseline.sh — (re)measure soundness/known_open.tsv, the known-open register the two property
# gates (run_q.sh, run_macro.sh) subtract before failing.
#
# It builds the EXHAUSTIVE crate for each generator — every point of the shape space, not a sample —
# compiles it, runs its ground truth, scans it, and writes every VIOLATION/DISCLOSED/DRIFT shape
# it finds.
# Exhaustive on purpose: a baseline built from random seeds would mark a shape "new" the first time
# a later seed happened to reach it, and the gate would go red on nothing.
#
# EVERY LINE IT WRITES IS A SPELLING PAIR THAT STILL ANSWERS DIFFERENTLY. `VIOLATION` is the silent
# under-report — the cardinal sin. `DISCLOSED` is the same equivalence failure with an `Unknown` in
# the macro spelling: SOUND under SPEC §4, still debt, never a pass. This file is a debt register.
# Re-running it after a fix is how the debt is shown to have shrunk; re-running it to make a red
# gate green is how a cardinal sin gets accepted as a low residual, which this family does not do.
#
#   bash soundness/baseline.sh            # rewrite known_open.tsv from the current build
#   bash soundness/baseline.sh --check    # measure but do NOT write; print the diff and exit 1
#
# Exit 0 = written (or unchanged under --check), 1 = --check found a difference, 2 = error,
# 3 = SELFSKIP.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/soundness/known_open.tsv"
MODE="${1:-write}"

command -v cargo >/dev/null 2>&1 || { echo "baseline: SELFSKIP — no cargo"; echo "RESULT: SELFSKIP"; exit 3; }
GEN_TOOLCHAIN="${CANDOR_GEN_TOOLCHAIN:-stable}"
gcargo() {  # cargo for the GENERATED crates, on GEN_TOOLCHAIN if there is one
  if [ -n "$GEN_TOOLCHAIN" ]; then RUSTUP_TOOLCHAIN="$GEN_TOOLCHAIN" RUSTFLAGS="" cargo "$@"
  else RUSTFLAGS="" cargo "$@"; fi
}
gcargo --version >/dev/null 2>&1 || GEN_TOOLCHAIN=""

if [ -n "${CANDOR_SCAN_BIN:-}" ]; then
  SCAN="$CANDOR_SCAN_BIN"
else
  # See run_q.sh: cargo honours CARGO_TARGET_DIR, so the read path must honour it too (R112).
  TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
  CARGO_TARGET_DIR="$TDIR" cargo build --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" >/dev/null 2>&1 \
    || { echo "baseline: candor-scan did not build"; echo "RESULT: ERROR"; exit 2; }
  SCAN="$TDIR/release/candor-scan"
  [ -x "$SCAN" ] || { echo "baseline: build reported success but no binary at $SCAN"; echo "RESULT: ERROR"; exit 2; }
fi
SHA="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
DIRTY=""; git -C "$ROOT" diff --quiet 2>/dev/null || DIRTY=" (WORKING TREE DIRTY — not a commit's answer)"
echo "baseline: scanner = $SCAN, at $SHA$DIRTY"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/candor-baseline.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
TMP="$WORK/known.tsv"
{
  echo "# soundness/known_open.tsv — the KNOWN-OPEN register for the two property gates."
  echo "#"
  echo "# Each line is a shape at which candor-scan reports two spellings of ONE program"
  echo "# differently, i.e. A SILENT UNDER-REPORT THAT IS STILL OPEN. The gates subtract these so a"
  echo "# NEW instance fails, and print them every run so they are not forgotten. This is a debt"
  echo "# register, not a list of acceptable behaviours."
  echo "#"
  echo "# Columns: <generator kind>  <verdict>  <shape key>"
  echo "# MEASURED exhaustively (every point of both shape spaces) by soundness/baseline.sh"
  echo "# at candor-rust $SHA. Regenerate with: bash soundness/baseline.sh"
} > "$TMP"

rc=0
for pair in "q:gen_q.py" "macro:gen_macro.py"; do
  kind="${pair%%:*}"; gen="${pair##*:}"
  d="$WORK/$kind"
  python3 "$ROOT/soundness/$gen" --all "$d" || { echo "baseline: $gen --all failed"; rc=2; continue; }
  ( cd "$d" && gcargo build -q --examples >/dev/null 2>&1 ) \
    || { echo "baseline: $kind exhaustive crate does not compile"; rc=2; continue; }
  ( cd "$d" && gcargo run -q --example gt > "$d/gt.txt" 2>/dev/null ) \
    || { echo "baseline: $kind ground-truth run failed"; rc=2; continue; }
  rm -rf "$d/target"
  "$SCAN" "$d" --json > "$d/report.json" 2>/dev/null
  [ "$?" -eq 2 ] && { echo "baseline: $kind scan INCOMPLETE (exit 2)"; rc=2; continue; }
  python3 "$ROOT/soundness/check_pair.py" "$d" "$d/report.json" > "$d/chk.txt"
  n=$(grep -cE '^  (VIOLATION|DISCLOSED|DRIFT) ' "$d/chk.txt")
  echo "baseline: $kind — $(head -1 "$d/chk.txt")"
  grep -E '^  (VIOLATION|DISCLOSED|DRIFT) ' "$d/chk.txt" \
    | sed -E "s/^  (VIOLATION|DISCLOSED|DRIFT) .*\[(.*)\]\$/$kind"$'\t'"\\1"$'\t'"\\2/" \
    | sort -u >> "$TMP"
  echo "baseline: $kind — $n shape instances, $(grep -c "^$kind"$'\t' "$TMP") distinct keys so far"
done
[ "$rc" -ne 0 ] && { echo "RESULT: ERROR"; exit 2; }

if [ "$MODE" = "--check" ]; then
  if diff -u <(grep -v '^#' "$OUT" 2>/dev/null | sort) <(grep -v '^#' "$TMP" | sort); then
    echo "RESULT: CLEAN — known_open.tsv matches the current build"; exit 0
  fi
  echo "RESULT: FINDING — known_open.tsv is out of date (diff above)"; exit 1
fi
cp "$TMP" "$OUT"
echo "baseline: wrote $OUT ($(grep -vc '^#' "$OUT") entries)"
echo "RESULT: CLEAN — baseline written"
