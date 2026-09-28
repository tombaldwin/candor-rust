#!/usr/bin/env bash
# run_drop.sh — the Drop-soundness fuzzer's runner (Bet 4 follow-up).
#
# For each seed: generate a crate whose `Guard::drop` performs a known effect, with the guard wrapped
# in random container forms (direct / field / tuple / array / Option / Box / Vec / Rc / Arc / HashMap /
# nested) AND placed in a random BODY — a top-level fn, a closure body, an uncalled closure, a nested
# closure, an `async` block, an `async` block across an `.await`, a combinator callback — then assert
# every dropping function inherits that effect (or `Unknown`). A dropping function reported PURE is a
# silent under-report — the implicit-Drop hole the Bet 4 fix closed.
#
# THE SITE AXIS IS WHY R757 EXISTS. Every form this generator knew emitted a top-level `fn`, so all 40
# default seeds landed in a `DefKind::Fn` body — and R756 was a filter that admitted exactly those. The
# gate was green over 40 cases none of which could reach the defect. Restricting the site pool to `fn`
# reproduces that blind spot exactly; the other six sites all FAIL on a pre-R756-fix engine.
#
#   bash soundness/run_drop.sh [N]        # default 40 seeds
#   CANDOR_FUZZ_DROP_SITES=closure bash soundness/run_drop.sh 10     # one body kind
#   CANDOR_FUZZ_DROP_FORMS="box vec" bash soundness/run_drop.sh 10   # one wrapper kind
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "soundness (drop): building candor…"
cargo build -q 2>/dev/null || { echo "FAIL: candor did not build"; exit 1; }
LIB=""
for c in "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so; do
  [ -e "$c" ] || continue
  { [ -z "$LIB" ] || [ "$c" -nt "$LIB" ]; } && LIB="$c"
done
[ -n "$LIB" ] || { echo "FAIL: no candor dylib"; exit 1; }

N="${1:-40}"
SEEDS="${SEEDS:-$(seq 1 "$N")}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; failed=""
for s in $SEEDS; do
  d="$WORK/s$s"
  python3 "$ROOT/soundness/gen_drop.py" "$s" "$d" || { echo "  seed $s: GEN ERROR"; fail=$((fail+1)); continue; }
  if ! ( cd "$d" && cargo build -q >/dev/null 2>&1 ); then
    echo "  seed $s: GENERATOR BUG — crate does not compile"; fail=$((fail+1)); continue
  fi
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  if ! ls "$d"/r.candor_drop.*.json >/dev/null 2>&1; then
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
echo "soundness (drop): $pass passed, $fail failed"
[ -n "$failed" ] && echo "soundness (drop): failing seeds:$failed"
[ "$fail" -eq 0 ]
