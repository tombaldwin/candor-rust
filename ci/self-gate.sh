#!/usr/bin/env bash
# candor-rust self-gate (candor-spec §7.12): candor analyzes ITSELF and holds its own declared policy.
# An effect-gate vendor whose own gate is red has no business gating anyone else. Uses the STABLE
# scanner (no nightly toolchain needed) — the same path an adopter's CI uses — and enforces the
# `deny Net Db Exec Ipc` boundary in .candor/policy by asserting no analyzed function reaches those.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Always rebuild when using the default binary (cargo is incremental, so this is ~free when up to date).
# `[ -x "$SCAN" ] ||` only rebuilt when the binary was MISSING — so a STALE target/release binary that
# predates a gate fix (e.g. the §6.2 ASCII-whitespace split) was used as-is, silently gate-evadable. Only
# trust a binary verbatim when explicitly provided via CANDOR_SCAN_BIN.
[ -n "${CANDOR_SCAN_BIN:-}" ] || cargo build --release -p candor-scan --manifest-path "$ROOT/Cargo.toml" || exit 2
SCAN="${CANDOR_SCAN_BIN:-$ROOT/target/release/candor-scan}"
# The denied effects, read from .candor/policy's `deny` line (the file is the source of truth). POSIX
# awk (not `\s` — BSD sed/grep don't support it): strip a trailing comment, drop the `deny` keyword.
DENIED=$(awk '/^[[:space:]]*deny[[:space:]]/{sub(/#.*/,""); $1=""; sub(/^[[:space:]]+/,""); print; exit}' "$ROOT/.candor/policy")
[ -n "$DENIED" ] || { echo "self-gate: no deny rule in .candor/policy"; exit 2; }
# EVERY WRITE GOES TO A TEMP DIR, and nothing under the working tree is touched.
#
# This loop used to `rm -rf "$d/.candor"` before AND after each scan, to be sure it read a fresh report
# rather than a stale one. Those directories hold EIGHT TRACKED FILES (`crates/*/.candor/report.*.json`),
# so a plain run of this script deleted them and never put them back — and it caught an agent inside a
# `git add -A`, committing the deletions. Restoring afterwards would still leave a window, and scoping
# the removal to untracked paths would make the script's correctness depend on what happens to be
# checked in. `--out <temp prefix>` is the version with no destructive step at all: freshness comes from
# the directory being NEW, which is also what the old `rm -rf` was actually buying.
WS="$(mktemp -d "${TMPDIR:-/tmp}/candor-self-gate.XXXXXX")"
trap 'rm -rf "$WS"' EXIT
rc=0
for c in candor-report candor-classify candor-scan candor-query; do
  d="$ROOT/crates/$c"
  # THE SCANNER'S EXIT CODE IS PART OF THE ANSWER, and this script ignored it entirely. A crate whose
  # only source fails to parse exits 2 under ⟨0.21⟩ (the fail-closed "could not evaluate" verdict) while
  # still writing a report — and the loop below, reading only the report's `functions`, found nothing
  # denied and printed `self-gate: OK`, exit 0. A FALSE ALL-CLEAR: strictly worse than the mislabelling
  # the other three engines' self-gates were corrected for in this same release, and missed when that
  # correction was described as covering "all three" — there are four.
  "$SCAN" "$d" --out "$WS/$c/report" >/dev/null 2>&1
  scan_rc=$?
  if [ "$scan_rc" -eq 2 ]; then
    echo "$c: COULD NOT EVALUATE — candor-scan exited 2 (unanalyzable source; the boundary was never"
    echo "  judged, so this is neither clean nor a violation). Fix the input, then re-run."
    rc=2; continue
  elif [ "$scan_rc" -ne 0 ]; then
    echo "$c: candor-scan exited $scan_rc — not a verdict this gate can read"; rc=2; continue
  fi
  # A glob loop, not `ls | grep`: an unmatched glob stays literal, so the `-e` guard is what makes
  # "no report" distinguishable from "a file named callgraph" — the same fix bin/corpus.sh took.
  rpt=""
  for f in "$WS/$c"/report.*.scan.json; do
    case "$f" in *callgraph*) continue;; esac
    [ -e "$f" ] && { rpt="$f"; break; }
  done
  [ -n "$rpt" ] || { echo "self-gate: candor-scan produced no report for $c"; rc=2; continue; }
  out="$(DENIED="$DENIED" python3 - "$rpt" <<'PY'
import json, os, sys
denied = set(os.environ["DENIED"].split())
d = json.load(open(sys.argv[1])); fns = d["functions"] if isinstance(d, dict) else d
# ⟨0.21⟩ COMPLETENESS FIRST, and this was the whole defect. Checking `functions` against the denylist
# asks "did anything we analyzed reach a denied effect" — over a crate whose sources did not parse, the
# answer is no, and this gate printed OK. The engine DID disclose it (`analyzed.count: 0`, a non-empty
# `unanalyzed`); nothing here read either. That is a false all-clear over an unjudged boundary, which is
# strictly worse than the mislabelling the other three self-gates were corrected for in this release —
# and it is the fourth engine, missed when that correction was described as covering "all three".
if isinstance(d, dict):
    un = d.get("unanalyzed") or []
    if un:
        for e in un[:5]:
            print(f"  INCOMPLETE  {e.get('path')} — {e.get('reason')}")
        print("              the boundary was never judged over these; a clean denylist check across the")
        print("              REST of the crate is not evidence about them. (⟨0.21⟩ analyzed/unanalyzed.)")
        sys.exit(2)
    if (d.get("analyzed") or {}).get("count") == 0 and not fns:
        print("  INCOMPLETE  analyzed.count is 0 — the scan judged nothing, so 'no denied effects' is vacuous")
        sys.exit(2)
bad = [(e["fn"], sorted(set(e["inferred"]) & denied)) for e in fns if set(e["inferred"]) & denied]
for fn, eff in bad: print(f"  AS-EFF-006  {fn}  performs {eff}, forbidden by `deny {' '.join(sorted(denied))}`")
sys.exit(1 if bad else 0)
PY
)"
  py_rc=$?
  if [ "$py_rc" -eq 2 ]; then echo "$c:"; echo "$out"; rc=2
  elif [ -n "$out" ]; then echo "$c:"; echo "$out"; rc=1; fi
done
[ "$rc" -eq 0 ] && echo "self-gate: OK (candor's own code performs no Net/Db/Exec/Ipc)" || echo "self-gate: FAILED"
exit "$rc"
