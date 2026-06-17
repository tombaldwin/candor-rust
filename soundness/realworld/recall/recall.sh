#!/usr/bin/env bash
# Non-syscall RECALL oracle — Db/Log/Rand/etc. (strace can't distinguish these; ground truth = known crate
# semantics in expected.json). candor-scan is SYNTACTIC, so this needs no strace, no Linux, no dep builds —
# runs ANYWHERE. Red only on a NEW under-report.  bash soundness/realworld/recall/recall.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
export RUSTFLAGS="-C linker=cc"   # bypass the repo's dylint-link linker; candor-scan needs no dylint
cargo +stable build -q --manifest-path "$ROOT/Cargo.toml" -p candor-scan || { echo "FAIL: candor-scan build"; exit 1; }
rm -rf "$HERE/.candor"
"$ROOT/target/debug/candor-scan" "$HERE" >/dev/null 2>&1
REP=$(ls "$HERE"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
[ -n "$REP" ] || { echo "FAIL: no candor-scan report"; exit 1; }
echo "non-syscall recall (Db/Log/...):"
python3 "$HERE/recall_check.py" "$REP" "$HERE/expected.json"
