#!/usr/bin/env bash
# oracle_pf.sh — PER-FUNCTION dynamic oracle (Bet 1, phase 2, strengthened). Linux + strace only.
#
# Like oracle.sh, but instruments each chain function with eprintln entry/exit markers (visible to
# strace, invisible to candor) so it can reconstruct the CALL STACK at the moment the effect syscall
# fires. Every function on the stack at that moment demonstrably performs the effect transitively, so
# candor must report each with the effect or Unknown — attributed to the EXACT function, not just the
# whole program. Restricted to Fs/Net (single-process: a clean, fork-free stack to reconstruct).
#
#   bash soundness/oracle_pf.sh [N]     # default 30 seeds
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# SELF-SKIP exits 3, never 0 — see soundness/oracle.sh's identical comment for why.
case "$(uname -s)" in
  Linux) : ;;
  *) echo "per-function oracle: needs Linux + strace (got $(uname -s)) — skipping."; exit 3 ;;
esac
command -v strace >/dev/null 2>&1 || { echo "per-function oracle: strace not installed — skipping."; exit 3; }

echo "per-function oracle: building candor…"
cargo build -q 2>/dev/null || { echo "FAIL: candor did not build"; exit 1; }
LIB=""
for c in "$ROOT"/target/debug/libcandor@*.so; do
  [ -e "$c" ] || continue
  { [ -z "$LIB" ] || [ "$c" -nt "$LIB" ]; } && LIB="$c"
done
[ -n "$LIB" ] || { echo "FAIL: no candor dylib"; exit 1; }

N="${1:-30}"
SEEDS="${SEEDS:-$(seq 1 "$N")}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; skip=0; failed=""
for s in $SEEDS; do
  d="$WORK/s$s"
  CANDOR_FUZZ_EFFECTS="Fs Net" CANDOR_FUZZ_INSTRUMENT=1 python3 "$ROOT/soundness/gen.py" "$s" "$d" \
    || { fail=$((fail+1)); continue; }
  ( cd "$d" && cargo build -q >/dev/null 2>&1 ) || { echo "  seed $s: build failed"; fail=$((fail+1)); continue; }
  # No -f: Fs/Net stay in the main process, so the marker/effect stream is a single clean thread.
  strace -e trace=write,openat,open,connect -o "$d/trace.log" "$d/target/debug/candor_fuzz" >/dev/null 2>&1 || true
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  ls "$d"/r.candor_fuzz.*.json >/dev/null 2>&1 || { echo "  seed $s: no candor report"; fail=$((fail+1)); continue; }

  res="$(python3 "$ROOT/soundness/oracle_pf_check.py" "$d" "$d/trace.log")"
  case "$res" in
    OK)    pass=$((pass+1)) ;;
    SKIP*) skip=$((skip+1)) ;;
    *)     fail=$((fail+1)); failed="$failed $s"; echo "  seed $s: $res" ;;
  esac
done

echo
echo "per-function oracle: $pass passed, $skip skipped, $fail failed"
[ -n "$failed" ] && echo "per-function oracle: failing seeds:$failed"
[ "$fail" -eq 0 ]
