#!/usr/bin/env bash
# run_cross.sh — the CROSS-CRATE construction fuzzer (extends run.sh to the crate boundary).
#
# Each seed generates one package compiled as lib+bin (gen_cross.py): the lib performs a known effect,
# the bin chains into it across the crate boundary using random call forms. One `cargo dylint`
# invocation lints BOTH crates; the bin must inherit the lib's effect cross-crate (precisely, or as a
# sound `Unknown`). check.py asserts every reachable function (in either crate) is effect-or-Unknown;
# a reachable function reported PURE/omitted is a silent cross-crate under-report.
#
#   bash soundness/run_cross.sh [N]     # default 30 seeds
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "soundness (cross): building candor…"
cargo build -q 2>/dev/null || { echo "FAIL: candor did not build"; exit 1; }
LIB=""
for c in "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so; do
  [ -e "$c" ] || continue
  { [ -z "$LIB" ] || [ "$c" -nt "$LIB" ]; } && LIB="$c"
done
[ -n "$LIB" ] || { echo "FAIL: no candor dylib"; exit 1; }

N="${1:-30}"
SEEDS="${SEEDS:-$(seq 1 "$N")}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; failed=""
for s in $SEEDS; do
  d="$WORK/s$s"
  python3 "$ROOT/soundness/gen_cross.py" "$s" "$d" || { echo "  seed $s: GEN ERROR"; fail=$((fail+1)); continue; }
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  if ! ls "$d"/r.xc.*.json >/dev/null 2>&1; then
    echo "  seed $s: NO REPORT (build failed under dylint?)"; fail=$((fail+1)); continue
  fi
  result="$(python3 "$ROOT/soundness/check.py" "$d")"
  if [ "${result%% *}" = "OK" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed="$failed $s"; echo "  seed $s: $result"
  fi
done

echo
echo "soundness (cross): $pass passed, $fail failed"
[ -n "$failed" ] && echo "soundness (cross): failing seeds:$failed"
[ "$fail" -eq 0 ]
