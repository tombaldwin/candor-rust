#!/usr/bin/env bash
# ⟨0.24⟩ SPEC §3.1's ACCEPTANCE TEST for `gate --report`, and it is BYTE-LEVEL.
#
#   For any report a scan produced, `candor-query gate --report <it> --policy P --gate-json B`
#   MUST produce a document BYTE-EQUAL to `candor-scan . --policy P --gate-json A`, with the same exit
#   code — `analyzed.count`, `reasonClass`, `netClass` and the coverage advisory included.
#
# Anything less lets the two routes drift into two gates, which is the failure the verb exists to make
# visible: until it existed the gate was reachable only THROUGH the classifier, so a defect in the gate
# and a defect in the classifier were indistinguishable from any test that could be written.
#
# It lives in CI rather than in `cargo test` because it needs BOTH binaries, and `CARGO_BIN_EXE_*` is
# only set for bins of the test's own package.
#
# THE ROW IS VACUOUS UNLESS SOMETHING FIRES. Byte-equal empty verdicts prove little, so the run FAILS
# when no policy in the matrix produced a violation.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo build -p candor-scan -p candor-query --manifest-path "$ROOT/Cargo.toml" || exit 2
SCAN="${CANDOR_SCAN_BIN:-$ROOT/target/debug/candor-scan}"
QUERY="${CANDOR_QUERY_BIN:-$ROOT/target/debug/candor-query}"

WS="$(mktemp -d "${TMPDIR:-/tmp}/candor-gate-equiv.XXXXXX")"
trap 'rm -rf "$WS"' EXIT

# `allow`/`forbid` are deliberately absent: §3.1 makes them REFUSALS (exit 2) on this verb, so they are
# not equivalence rows — they are pinned by the `gate_report_refuses_*` tests instead.
POLICIES=(
  "deny Fs"
  "deny Env"
  "deny Unknown"
  "deny Fs Env Net"
  "deny Net Unknown"
  "pure"
  "pure policy"
  "deny Fs scan"
  "deny Unknown[dispatch]"
  "deny Unknown[unresolved]"
  "deny Unknown[dynamic]"
  "deny Net[unknown-host]"
)

rows=0; fired=0; bad=0
for c in candor-report candor-classify candor-scan candor-query; do
  d="$ROOT/crates/$c"
  i=0
  for p in "${POLICIES[@]}"; do
    i=$((i+1))
    pol="$WS/$c.$i.policy"; printf '%s\n' "$p" > "$pol"
    a="$WS/$c.$i.scan.json"; b="$WS/$c.$i.gate.json"; pfx="$WS/$c.$i.rep"
    # Delete the outputs BEFORE measuring — a stale artefact is a flattering datapoint, and this suite
    # has been fooled by one before.
    rm -f "$a" "$b" "$pfx".*
    "$SCAN" "$d" --out "$pfx" --policy "$pol" --gate-json "$a" >/dev/null 2>&1; rc_scan=$?
    "$QUERY" gate --report "$pfx" --policy "$pol" --gate-json "$b" >/dev/null 2>&1; rc_gate=$?
    rows=$((rows+1))
    [ "$rc_scan" -eq 1 ] && fired=$((fired+1))
    if [ ! -f "$a" ] || [ ! -f "$b" ]; then
      echo "  FAIL $c / '$p': a --gate-json document was not written (scan rc $rc_scan, gate rc $rc_gate)"
      bad=$((bad+1)); continue
    fi
    if ! cmp -s "$a" "$b"; then
      echo "  FAIL $c / '$p': --gate-json NOT byte-equal (scan rc $rc_scan, gate rc $rc_gate)"
      diff "$a" "$b" | head -20
      bad=$((bad+1))
    fi
    if [ "$rc_scan" -ne "$rc_gate" ]; then
      echo "  FAIL $c / '$p': exit $rc_scan (scan) vs $rc_gate (gate)"
      bad=$((bad+1))
    fi
  done
done

if [ "$fired" -eq 0 ]; then
  echo "gate-equivalence: VACUOUS — no policy in the matrix produced a violation; byte-equal empty"
  echo "                  verdicts prove nothing. Fix the matrix, do not relax the check."
  exit 1
fi
if [ "$bad" -ne 0 ]; then
  echo "gate-equivalence: FAILED — $bad of $rows rows diverged"
  echo '  A divergence here is a finding ABOUT THE GATE: `scan --policy` and `gate --report` are two'
  echo '  routes into one gate, and the byte-equality is what makes that a property of the code.'
  exit 1
fi
echo "gate-equivalence: OK — $rows rows byte-equal, $fired of them with violations (SPEC §3.1 ⟨0.24⟩)"
exit 0
