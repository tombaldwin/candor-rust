#!/usr/bin/env bash
# candor-rust self-gate (candor-spec §7.12): candor analyzes ITSELF and holds its own declared policy.
# An effect-gate vendor whose own gate is red has no business gating anyone else. Uses the STABLE
# scanner (no nightly toolchain needed) — the same path an adopter's CI uses — and enforces the
# `deny Net Db Exec Ipc` boundary in .candor/policy by asserting no analyzed function reaches those.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCAN="${CANDOR_SCAN_BIN:-$ROOT/target/release/candor-scan}"
[ -x "$SCAN" ] || cargo build --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" || exit 2
SCAN="${CANDOR_SCAN_BIN:-$ROOT/target/release/candor-scan}"
# The denied effects, read from .candor/policy's `deny` line (the file is the source of truth). POSIX
# awk (not `\s` — BSD sed/grep don't support it): strip a trailing comment, drop the `deny` keyword.
DENIED=$(awk '/^[[:space:]]*deny[[:space:]]/{sub(/#.*/,""); $1=""; sub(/^[[:space:]]+/,""); print; exit}' "$ROOT/.candor/policy")
[ -n "$DENIED" ] || { echo "self-gate: no deny rule in .candor/policy"; exit 2; }
rc=0
for c in candor-report candor-classify candor-scan candor-query; do
  d="$ROOT/crates/$c"
  rm -rf "$d/.candor"; "$SCAN" "$d" >/dev/null 2>&1
  rpt="$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)"
  [ -n "$rpt" ] || { echo "self-gate: candor-scan produced no report for $c"; rc=2; continue; }
  out="$(DENIED="$DENIED" python3 - "$rpt" <<'PY'
import json, os, sys
denied = set(os.environ["DENIED"].split())
d = json.load(open(sys.argv[1])); fns = d["functions"] if isinstance(d, dict) else d
bad = [(e["fn"], sorted(set(e["inferred"]) & denied)) for e in fns if set(e["inferred"]) & denied]
for fn, eff in bad: print(f"  AS-EFF-006  {fn}  performs {eff}, forbidden by `deny {' '.join(sorted(denied))}`")
sys.exit(1 if bad else 0)
PY
)"
  if [ -n "$out" ]; then echo "$c:"; echo "$out"; rc=1; fi
  rm -rf "$d/.candor"
done
[ "$rc" -eq 0 ] && echo "self-gate: OK (candor's own code performs no Net/Db/Exec/Ipc)" || echo "self-gate: FAILED"
exit "$rc"
