#!/usr/bin/env bash
# soundness/calltarget/run.sh — the call-target matrix (calltarget round 1): every call form × placement × implementor
# set × arm × SPELLING, each program BUILT AND EXECUTED for ground truth, measured at the caller `zz_caller` with
# `deny <E> zz_caller` / `deny Unknown zz_caller`, and compared with the committed expected verdicts (baseline.tsv).
#
#   bash soundness/calltarget/run.sh [WORKDIR] [JOBS]          # build this tree's engine, run, check the baseline
#   PRE=/path/to/bindir bash soundness/calltarget/run.sh ...    # also scan with another build and list transitions
#   UPDATE=1 bash soundness/calltarget/run.sh ...               # rewrite baseline.tsv from this run (commit it WITH the change)
#
# Not wired into per-commit CI: a full run builds and executes ~6,600 programs (≈20 min per spelling at 2 jobs on an
# M-series Mac) and holds a TCP listener on 127.0.0.1:$CT_PORT (default 47123). Needs `rustc` on PATH.
# Exit: 0 = matches the baseline; 1 = a verdict moved (named); 3 = a program failed to build or its executed effect
# did not match the generator (not a measurement).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
W="${1:-${TMPDIR:-/tmp}/candor-calltarget}"; J="${2:-2}"
cargo build --release -j "$J" -p candor-scan -p candor-query --manifest-path "$ROOT/Cargo.toml" >&2 || exit 3
TD="${CARGO_TARGET_DIR:-$ROOT/target}/release"
BINS="head=$TD"; [ -n "${PRE:-}" ] && BINS="pre=$PRE,$BINS"
for sp in crate bare use; do
  mkdir -p "$W/$sp"; rm -f "$W/$sp/results.jsonl"
  CT_SPELL=$sp python3 "$HERE/gen.py" "$W/$sp" >&2 || exit 3
  python3 "$HERE/run.py" "$W/$sp" "$J" "$BINS" >&2 || exit 3
done
ARGS=(--outdir "$W"); [ -n "${PRE:-}" ] && ARGS+=(--pre pre)
if [ -n "${UPDATE:-}" ]; then ARGS+=(--write-baseline "$HERE/baseline.tsv"); else ARGS+=(--check "$HERE/baseline.tsv"); fi
python3 "$HERE/report.py" "$W" head "${ARGS[@]}"
