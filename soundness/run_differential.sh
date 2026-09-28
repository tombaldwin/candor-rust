#!/usr/bin/env bash
# run_differential.sh — THE TWO RUST ENGINES, DIFFERENCED. SOUNDNESS R758.
#
# Nothing in this repo compared the nightly rustc/MIR engine with `candor-scan`. `run.sh`, `run_cross.sh`,
# `run_drop.sh`, `oracle.sh` and `oracle_pf.sh` drive the DEEP engine only; `run_q.sh` and `run_macro.sh`
# drive `candor-scan` only. That is why R756 survived three weeks: on its own one-line pair `candor-scan`
# answered correctly and the deep engine — the one every other soundness script uses as the ORACLE — was
# silent, and no instrument put the two answers next to each other.
#
# The two engines are the strongest differential pair available here because they answer from DIFFERENT
# AUTHORITIES: one asks rustc (types, MIR, dispatch), one parses syn. Agreement between them is evidence
# rather than agreement between two copies of one mistake.
#
# WHAT IT CHECKS. Both engines emit the §3.3 structured verdict `{ spec, ok, violations }` — the deep one
# via CANDOR_GATE_JSON, `candor-scan` via --gate-json — under ONE policy file. `candor-scan` describes
# itself as a "syntactic floor — a clean run is necessary, not sufficient", so the deep engine is meant to
# be a SUPERSET: every function the floor flags, the sound engine must flag too. A FLOOR-ONLY violation
# fails the run, and it is A PAIR TO READ, NOT A VERDICT — see differential_check.py for the two
# directions it can mean. DEEP-ONLY is expected and prints as INFO.
#
#   bash soundness/run_differential.sh [N]         # N generated drop crates (default 12). THE GATE.
#   bash soundness/run_differential.sh --calibrate # prove this instrument can FAIL, then exit
#   DIRS="/path/a /path/b" bash soundness/run_differential.sh   # ad-hoc crates; same verdict
#
# CALIBRATION, AND WHY IT IS NOT OPTIONAL (§1b / §6): a gate that has never been seen to fail is not
# evidence. `--calibrate` builds a crate whose only effectful function sits behind an INACTIVE `#[cfg]` —
# rustc never compiles it, syn always parses it — so `candor-scan` flags a function the deep engine has
# never heard of, and this script must report FLOOR-ONLY. That disagreement is permanent and correct
# (it is candor-scan's documented over-charge direction), which is what makes it a usable seed.
#
# SCOPE, STATED: the generated-crate arm has no `#[cfg]`, no test targets and no dependencies, so its
# FLOOR-ONLY bucket must be EMPTY and any row in it is a finding. On an arbitrary `DIRS=` crate the same
# bucket legitimately fills with cfg-gated items and target-selection differences; read it, do not assume
# it. The exit code is the same in both modes on purpose — an advisory that cannot fail is not a check.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

POLICY_RULES="${POLICY_RULES:-deny Fs Net Exec Env}"

echo "soundness (differential): building both rust engines…"
cargo build -q --workspace 2>/dev/null || { echo "FAIL: workspace did not build"; exit 1; }
# NEWEST by mtime, never the first glob match: the filename carries the toolchain, so a stale dylib from
# an older pinned nightly sorts ahead alphabetically and would silently be the engine under test.
LIB=$(ls -t "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so 2>/dev/null | head -1)
[ -n "$LIB" ] || { echo "FAIL: no candor dylib under target/debug"; exit 1; }
SCAN="$ROOT/target/debug/candor-scan"
[ -x "$SCAN" ] || { echo "FAIL: no candor-scan under target/debug"; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

# Run both engines over one crate dir and difference the verdicts. Echoes the checker's output; returns
# its exit status (0 clean / 1 FLOOR-ONLY / 2 a verdict could not be read).
compare_one() {
  local d="$1" label="$2"
  printf '%s\n' "$POLICY_RULES" > "$d/candor.policy"
  rm -f "$d/deep.gate" "$d/deep.gate.parts" "$d/scan.gate"
  # The deep engine emits diagnostics only on a RECOMPILE, so clear dylint's cache each time.
  ( cd "$d" && rm -rf target/dylint \
      && CANDOR_POLICY="$d/candor.policy" CANDOR_GATE_JSON="$d/deep.gate" \
         cargo dylint --lib-path "$LIB" >"$d/deep.out" 2>&1 )
  env -u CANDOR_CONFIG -u CANDOR_POLICY -u CANDOR_BASELINE -u CANDOR_JSON \
      "$SCAN" "$d" --policy "$d/candor.policy" --gate-json "$d/scan.gate" >"$d/scan.out" 2>&1
  python3 "$ROOT/soundness/differential_check.py" "$d/deep.gate" "$d/scan.gate" "$label"
}

if [ "${1:-}" = "--calibrate" ]; then
  echo "soundness (differential): CALIBRATION — an effectful fn behind an inactive #[cfg]"
  c="$WORK/cal"; mkdir -p "$c/src"
  printf '[package]\nname="candor_diff_cal"\nversion="0.1.0"\nedition="2021"\n\n[features]\noff = []\n' > "$c/Cargo.toml"
  cat > "$c/src/main.rs" <<'RS'
// The feature is declared and never enabled, so rustc does not compile this item and the deep engine
// cannot see it. candor-scan parses the source with syn and does, so it flags it — a FLOOR-ONLY row.
#[cfg(feature = "off")]
fn behind_inactive_cfg() {
    let _ = std::fs::read("/tmp/candor_diff_cal");
}
fn main() {}
RS
  ( cd "$c" && cargo build -q >/dev/null 2>&1 ) || { echo "FAIL: calibration crate did not compile"; exit 1; }
  compare_one "$c" "calibration"; rc=$?
  if [ "$rc" -eq 1 ]; then
    echo "soundness (differential): CALIBRATED — the seeded disagreement FIRED (exit 1 on FLOOR-ONLY)"
    exit 0
  fi
  echo "soundness (differential): CALIBRATION FAILED — the seeded disagreement did NOT fire (rc=$rc)."
  echo "  This instrument cannot be trusted to report a real disagreement until it does."
  exit 1
fi

N="${1:-12}"
dirs="${DIRS:-}"
if [ -z "$dirs" ]; then
  echo "soundness (differential): generating $N drop crates (all FORMS x all SITES)…"
  for s in $(seq 1 "$N"); do
    d="$WORK/s$s"
    python3 "$ROOT/soundness/gen_drop.py" "$s" "$d" || { echo "FAIL: gen_drop.py seed $s"; exit 1; }
    ( cd "$d" && cargo build -q >/dev/null 2>&1 ) || { echo "FAIL: generated crate $s does not compile"; exit 1; }
    dirs="$dirs $d"
  done
fi

pass=0; fail=0; broken=0
for d in $dirs; do
  [ -d "$d" ] || { echo "  $d: NOT A DIRECTORY"; broken=$((broken+1)); continue; }
  compare_one "$d" "$(basename "$d")"
  case $? in
    0) pass=$((pass+1)) ;;
    1) fail=$((fail+1)) ;;
    *) broken=$((broken+1)) ;;
  esac
done

echo
echo "soundness (differential): $pass agreed, $fail with FLOOR-ONLY rows, $broken unreadable"
[ "$fail" -eq 0 ] && [ "$broken" -eq 0 ]
