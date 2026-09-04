#!/usr/bin/env bash
# run_q.sh — the `?`-POSITION property gate for the stable scanner (candor-scan).
#
# THE PROPERTY. Each generated pair is TWO SPELLINGS OF ONE PROGRAM, emitted from one description:
#
#   hoist    `let t = EXPR; t?;`   vs   `EXPR?;`      — identical evaluation, so identical charges
#   looprot  `loop { CTOR ..?; }`  vs   `loop { ..?; CTOR }`
#                                        — a `?` in a loop body is live for everything that body
#                                          builds, so the `?`-first spelling can only charge MORE
#
# A spelling that loses an effect its equivalent was charged is a SILENT UNDER-REPORT. Both sides
# of every pair are EXECUTED by `examples/gt.rs`, which reports the in-frame drop counts, so the
# comparison is never between two absences.
#
# WHY IT EXISTS. Six cardinal-sin regressions were introduced and caught in the ⟨0.35⟩ round —
# R187, R194, R199, R203, R204, R210. The 1,504-crate wide-key corpus A/B caught ZERO of them
# (every one measured 0 corpus incidence); the corpus is the family's FABRICATION gate. Every
# silence was caught by somebody hand-writing the right fixture. This gate checks a property over
# GENERATED shapes instead, so the shape does not have to be guessed first. Its retro-rediscovery
# calibration against the six pre-fix binaries is recorded in soundness/README.md — read that
# before trusting a clean result, because a generator that cannot fail is worse than none.
#
#   bash soundness/run_q.sh [N]            # fuzz the first N seeds (default 40)
#   SEEDS="1 2 99" bash soundness/run_q.sh
#   CANDOR_SCAN_BIN=/path/to/candor-scan bash soundness/run_q.sh 40    # measure a chosen arm
#
# Exit 0 = clean, 1 = a real finding, 2 = harness/build error, 3 = SELFSKIP (with a stated reason).
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GEN="${GEN_SCRIPT:-$ROOT/soundness/gen_q.py}"
LABEL="${GATE_LABEL:-soundness (?-position)}"

command -v cargo >/dev/null 2>&1 || { echo "$LABEL: SELFSKIP — no cargo on PATH"; echo "RESULT: SELFSKIP"; exit 3; }
python3 -c 'import json' >/dev/null 2>&1 || { echo "$LABEL: SELFSKIP — no python3"; echo "RESULT: SELFSKIP"; exit 3; }

# The generated crates are plain stable Rust; prefer `stable` so this gate does not depend on the
# repo's pinned nightly. Where stable is not installed (CI installs only the pinned toolchain), fall
# back to whatever cargo resolves rather than skipping — a gate that SELFSKIPs on the one machine
# that runs it is a gate that never runs.
GEN_TOOLCHAIN="${CANDOR_GEN_TOOLCHAIN:-stable}"
gcargo() {  # cargo for the GENERATED crates, on GEN_TOOLCHAIN if there is one
  if [ -n "$GEN_TOOLCHAIN" ]; then RUSTUP_TOOLCHAIN="$GEN_TOOLCHAIN" RUSTFLAGS="" cargo "$@"
  else RUSTFLAGS="" cargo "$@"; fi
}
if ! gcargo --version >/dev/null 2>&1; then
  echo "$LABEL: toolchain '$GEN_TOOLCHAIN' unavailable — falling back to the active toolchain"
  GEN_TOOLCHAIN=""
fi

if [ -n "${CANDOR_SCAN_BIN:-}" ]; then
  SCAN="$CANDOR_SCAN_BIN"
  [ -x "$SCAN" ] || { echo "$LABEL: CANDOR_SCAN_BIN='$SCAN' is not executable"; echo "RESULT: ERROR"; exit 2; }
else
  echo "$LABEL: building candor-scan…"
  # SOUNDNESS R112's shape: cargo reads CARGO_TARGET_DIR itself, so a build redirected by it lands
  # nowhere near a hardcoded "$ROOT/target/..." path — pin the same value for both halves.
  TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
  CARGO_TARGET_DIR="$TDIR" cargo build --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" >/dev/null 2>&1 \
    || { echo "$LABEL: candor-scan did not build"; echo "RESULT: ERROR"; exit 2; }
  SCAN="$TDIR/release/candor-scan"
  # A build that reports success and leaves no binary where this script is about to look must FAIL,
  # never degrade into "every seed's report is silently absent" — that reads as findings, not a bug.
  [ -x "$SCAN" ] || { echo "$LABEL: build reported success but no binary at $SCAN (CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<unset>})"; echo "RESULT: ERROR"; exit 2; }
fi
echo "$LABEL: scanner = $SCAN"

N="${1:-40}"
SEEDS="${SEEDS:-$(seq 1 "$N")}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/candor-runq.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
BASE="$(basename "$GEN" .py)"
# The known-open register. Every line in it is a silent under-report that is STILL OPEN at the
# commit named in its header — subtracted so a NEW instance fails, printed so it is not forgotten.
# Regenerate with `bash soundness/baseline.sh`; never edit it by hand to make this gate green.
KNOWN="${CANDOR_KNOWN_OPEN:-$ROOT/soundness/known_open.tsv}"
[ -f "$KNOWN" ] || { echo "$LABEL: known-open register missing at $KNOWN — run soundness/baseline.sh"; echo "RESULT: ERROR"; exit 2; }
LOG="$ROOT/soundness/.last-run.$BASE.log"
: > "$LOG"

pass=0; fail=0; err=0; failed_seeds=""; v_total=0; d_total=0; bp_total=0
for s in $SEEDS; do
  d="$WORK/s$s"
  python3 "$GEN" "$s" "$d" >/dev/null 2>&1 || { echo "  seed $s: GEN ERROR"; err=$((err+1)); continue; }

  # COMPILE AND RUN FIRST. A generated program that does not compile is a harness bug, and a control
  # asserting an absence over a program that never runs is asserting something about nothing (§E3).
  # Each seed gets its OWN target dir on purpose: a target dir SHARED between crates at different
  # paths hands `cargo run` a STALE `examples/gt` hardlink from the previous seed, and the ground
  # truth then describes a program that is not the one being scanned. Measured while building this.
  if ! ( cd "$d" && gcargo build -q --examples >/dev/null 2>&1 ); then
    echo "  seed $s: GENERATOR BUG — crate does not compile"; err=$((err+1))
    { echo "=== seed $s: compile failure ==="; ( cd "$d" && gcargo build --examples 2>&1 ); } >> "$LOG"
    continue
  fi
  if ! ( cd "$d" && gcargo run -q --example gt > "$d/gt.txt" 2>>"$LOG" ); then
    echo "  seed $s: GENERATOR BUG — ground-truth run failed"; err=$((err+1)); continue
  fi
  # The scan must not see the build output. `--json` writes to stdout; nothing is written into $d.
  rm -rf "$d/target"

  "$SCAN" "$d" --json > "$d/report.json" 2>>"$LOG"
  rc=$?
  if [ "$rc" -eq 2 ]; then
    # ⟨0.21⟩ fail-closed: the scanner could not evaluate the crate. Reading `functions` anyway and
    # finding nothing denied is a FALSE ALL-CLEAR — the shape ci/self-gate.sh was corrected for.
    echo "  seed $s: SCAN INCOMPLETE (exit 2) — verdict not usable"; err=$((err+1)); continue
  fi
  [ -s "$d/report.json" ] || { echo "  seed $s: NO REPORT"; err=$((err+1)); continue; }

  result="$(python3 "$ROOT/soundness/check_pair.py" "$d" "$d/report.json" --known "$KNOWN")"
  { echo "=== seed $s ==="; echo "$result"; } >> "$LOG"
  v_total=$((v_total + $(printf '%s\n' "$result" | grep -cE '^  (VIOLATION|DRIFT) ')))
  d_total=$((d_total + $(printf '%s\n' "$result" | grep -c '^  KNOWN-OPEN ')))
  bp_total=$((bp_total + $(printf '%s\n' "$result" | grep -c '^  BOTH-PURE ')))
  if [ "${result%% *}" = "OK" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1)); failed_seeds="$failed_seeds $s"
    echo "  seed $s:"; printf '%s\n' "$result" | sed 's/^/    /'
    # Keep the offending crate where a human can read it — a finding needs a reproduction.
    keep="$ROOT/soundness/.last-run.$BASE.seed$s"
    rm -rf "$keep"; cp -R "$d" "$keep" 2>/dev/null && echo "    (crate kept at $keep)"
  fi
done

echo
echo "$LABEL: $pass seeds clean, $fail seeds with property violations, $err harness/generator errors"
echo "$LABEL: $v_total NEW property violation(s); $d_total hit(s) on the known-open register"
echo "$LABEL: $bp_total pair(s) BOTH-PURE — neither spelling charged, though the run dropped. Not this"
echo "$LABEL:   gate's differential; it is a separate silent under-report. See $LOG."
[ -n "$failed_seeds" ] && echo "$LABEL: failing seeds:$failed_seeds"
echo "$LABEL: per-seed detail kept at $LOG"

if [ "$fail" -gt 0 ]; then
  echo "RESULT: FINDING — $v_total NEW property violation(s) over $fail seed(s)"
  exit 1
fi
if [ "$err" -gt 0 ]; then
  echo "RESULT: ERROR — $err seed(s) could not be measured"
  exit 2
fi
echo "RESULT: CLEAN — $pass seeds, 0 NEW property violations ($d_total known-open hits)"
exit 0
