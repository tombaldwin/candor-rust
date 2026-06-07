#!/usr/bin/env bash
# run.sh — the soundness fuzzer's runner + checker (Bet 1, phase 1).
#
# For each seed: generate a crate (gen.py), run candor over it, and assert EVERY function the generator
# knows reaches the effect is reported with that effect OR with `Unknown` (a sound over-approximation).
# A reachable function reported PURE — or omitted from the report (candor omits effect-free fns) — is a
# SILENT UNDER-REPORT, the bug class this harness exists to catch. `Unknown` is a PASS: the harness
# tests SOUNDNESS (never silent-pure), not precision.
#
#   bash soundness/run.sh [N]        # fuzz the first N seeds (default 40)
#   SEEDS="1 2 99" bash soundness/run.sh   # specific seeds
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "soundness: building candor…"
cargo build -q 2>/dev/null || { echo "FAIL: candor did not build"; exit 1; }
# newest libcandor@<toolchain>.{dylib,so} by mtime
LIB=""
for c in "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so; do
  [ -e "$c" ] || continue
  { [ -z "$LIB" ] || [ "$c" -nt "$LIB" ]; } && LIB="$c"
done
[ -n "$LIB" ] || { echo "FAIL: no candor dylib found"; exit 1; }

N="${1:-40}"
SEEDS="${SEEDS:-$(seq 1 "$N")}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; failed_seeds=""
for s in $SEEDS; do
  d="$WORK/s$s"
  python3 "$ROOT/soundness/gen.py" "$s" "$d" || { echo "  seed $s: GEN ERROR"; fail=$((fail+1)); continue; }
  # Compile first: a non-compiling crate is a generator bug (no report ⇒ false "all pure"), not candor's.
  if ! ( cd "$d" && cargo build -q >/dev/null 2>&1 ); then
    echo "  seed $s: GENERATOR BUG — crate does not compile"; fail=$((fail+1)); continue
  fi
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  if ! ls "$d"/r.candor_fuzz.*.json >/dev/null 2>&1; then
    echo "  seed $s: NO REPORT (build failed under dylint?)"; fail=$((fail+1)); continue
  fi
  # Check soundness: every expected fn must be present with effect-or-Unknown (see check.py).
  result="$(python3 "$ROOT/soundness/check.py" "$d")"
  if [ "${result%% *}" = "OK" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_seeds="$failed_seeds $s"
    echo "  seed $s: $result"
  fi
done

echo
echo "soundness: $pass passed, $fail failed"
[ -n "$failed_seeds" ] && echo "soundness: failing seeds:$failed_seeds"
[ "$fail" -eq 0 ]
