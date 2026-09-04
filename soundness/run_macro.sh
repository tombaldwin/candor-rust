#!/usr/bin/env bash
# run_macro.sh — the MACRO-EQUIVALENCE property gate for the stable scanner (candor-scan).
#
# A single-arm `macro_rules!` invocation and the spelling it expands to are the same program, so
# they must be charged the same. This is candor-rust's most productive historical bug shape:
# R142, R143, R199, R203, R204, R206, R207, R210 are all instances, and in every one the DIRECT
# twin was a hand-written control somebody had to think of first. Here both spellings come from
# one generated description, so they cannot drift.
#
# It shares run_q.sh's whole runner — generate, COMPILE, RUN the ground truth, scan, check — so
# the two gates cannot drift apart either. See gen_macro.py for the property and soundness/README.md
# for the retro-rediscovery calibration.
#
#   bash soundness/run_macro.sh [N]
#   CANDOR_SCAN_BIN=/path/to/candor-scan bash soundness/run_macro.sh 40
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec env GEN_SCRIPT="$ROOT/soundness/gen_macro.py" GATE_LABEL="soundness (macro-equivalence)" \
  bash "$ROOT/soundness/run_q.sh" "$@"
