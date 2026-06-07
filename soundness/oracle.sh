#!/usr/bin/env bash
# oracle.sh — the soundness fuzzer's DYNAMIC oracle (Bet 1, phase 2). Linux + strace only.
#
# For each seed: generate a crate that performs a syscall-observable effect (Fs/Net/Exec), RUN it under
# strace, and confirm the effect actually executed (its marker appears in the trace). If it did, assert
# candor's static prediction for the program (`main`'s transitive `inferred`) contains that effect — or
# `Unknown`. A program that demonstrably performs an effect candor predicts NOWHERE is a silent
# under-report. This is ground truth from the kernel — it trusts nothing about the generator.
#
#   bash soundness/oracle.sh [N]      # oracle the first N seeds (default 30)
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

case "$(uname -s)" in
  Linux) : ;;
  *) echo "soundness oracle: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;;
esac
command -v strace >/dev/null 2>&1 || { echo "soundness oracle: strace not installed — skipping."; exit 0; }

echo "soundness oracle: building candor…"
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
  # Restrict to syscall-observable effects (Env reads no syscall).
  CANDOR_FUZZ_EFFECTS="Fs Net Exec" python3 "$ROOT/soundness/gen.py" "$s" "$d" || { fail=$((fail+1)); continue; }
  ( cd "$d" && cargo build -q >/dev/null 2>&1 ) || { echo "  seed $s: build failed"; fail=$((fail+1)); continue; }
  # Run under strace, tracing the file/network/process syscalls; -f to follow the Exec child.
  strace -f -e trace=openat,open,connect,socket,execve -o "$d/trace.log" \
    "$d/target/debug/candor_fuzz" >/dev/null 2>&1 || true
  # candor's static prediction.
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  ls "$d"/r.candor_fuzz.*.json >/dev/null 2>&1 || { echo "  seed $s: no candor report"; fail=$((fail+1)); continue; }

  res="$(python3 "$ROOT/soundness/oracle_check.py" "$d" "$d/trace.log")"
  case "$res" in
    OK)       pass=$((pass+1)) ;;
    SKIP*)    skip=$((skip+1)) ;;
    *)        fail=$((fail+1)); failed="$failed $s"; echo "  seed $s: $res" ;;
  esac
done

echo
echo "soundness oracle: $pass observed-and-predicted, $skip skipped, $fail failed"
[ -n "$failed" ] && echo "soundness oracle: failing seeds:$failed"
[ "$fail" -eq 0 ]
