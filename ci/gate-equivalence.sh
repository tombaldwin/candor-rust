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

# THE FAIL-CLOSED ARM, which no in-tree crate can reach: a scan whose own analysis was INCOMPLETE writes
# the ⟨0.21⟩ `ok:false` + `incomplete:true` + `unanalyzed` verdict. The manifest travels ON the report, so
# the report route must reach the same verdict from the same fact, and that document has a different
# SHAPE from every row above.
#
# TWO ROWS, because SPEC §3.3 makes two claims about this state and they had different fates. "MUST fail
# closed (exit ≠ 0)" was held by both routes; "a real violation (exit 1) still dominates" was held by
# NEITHER — both dropped the violations they had just computed and wrote `violations: []` (measured
# 2026-07-28). One arm with a NON-violating policy could never see that, and this file had only that arm:
# the two routes were byte-equal because they were making the same mistake.
inc="$WS/incomplete"; mkdir -p "$inc/src"
printf '[package]\nname = "incomp"\n' > "$inc/Cargo.toml"
printf 'pub fn ok_fn() { let _ = std::fs::read_to_string("/x"); }\n' > "$inc/src/lib.rs"
printf 'fn broken( { { {\n' > "$inc/src/bad.rs"          # the file the parser cannot read
mkdir -p "$inc/out"
# Row A — a policy nothing violates: fail closed at exit 2, `violations: []` for the RIGHT reason.
# Row B — `deny Fs`, which `ok_fn` really does violate: the finding DOMINATES (exit 1) and must be IN
#         the document, alongside `incomplete`/`unanalyzed` rather than instead of them.
for arm in "A 2 deny Db" "B 1 deny Fs"; do
  set -- $arm; tag=$1; want=$2; shift 2; rule="$*"
  printf '%s\n' "$rule" > "$inc/policy"
  rm -f "$WS/inc.$tag.scan.json" "$WS/inc.$tag.gate.json" "$inc/out/r".*
  "$SCAN" "$inc" --out "$inc/out/r" --policy "$inc/policy" --gate-json "$WS/inc.$tag.scan.json" >/dev/null 2>&1
  rc_scan=$?
  "$QUERY" gate --report "$inc/out/r" --policy "$inc/policy" --gate-json "$WS/inc.$tag.gate.json" >/dev/null 2>&1
  rc_gate=$?
  rows=$((rows+1))
  if [ "$rc_scan" -ne "$want" ] || [ "$rc_gate" -ne "$want" ]; then
    echo "  FAIL incomplete-report/$tag ('$rule'): exit $rc_scan (scan) vs $rc_gate (gate), both must be $want"
    echo "       (§3.3: a gate over unanalyzed code fails closed, and a real violation still dominates)"
    bad=$((bad+1)); continue
  fi
  if ! cmp -s "$WS/inc.$tag.scan.json" "$WS/inc.$tag.gate.json"; then
    echo "  FAIL incomplete-report/$tag: the ⟨0.21⟩ incomplete verdict is NOT byte-equal"
    diff "$WS/inc.$tag.scan.json" "$WS/inc.$tag.gate.json" | head -20
    bad=$((bad+1)); continue
  fi
  # THE CONTENT CHECK, without which byte-equality is satisfied by two identically-empty documents.
  if ! grep -q '"incomplete": true' "$WS/inc.$tag.scan.json"; then
    echo "  FAIL incomplete-report/$tag: the verdict does not disclose `incomplete`"
    bad=$((bad+1)); continue
  fi
  if [ "$tag" = "B" ] && ! grep -q '"fn": "ok_fn"' "$WS/inc.$tag.scan.json"; then
    echo "  FAIL incomplete-report/B: the real violation was DELETED from the verdict — a CI consumer"
    echo "       reads ok:false with nothing in it and the finding never reaches the PR (SPEC §3.3)"
    cat "$WS/inc.B.scan.json"
    bad=$((bad+1))
  fi
done

# THE JUDGED-NOTHING ARM, which no in-tree crate can reach either: a crate with no functions at all, so
# this engine's OWN scan writes `analyzed.count: 0` and exits 0 with a clean verdict. ⟨0.24⟩ makes that a
# DISCLOSURE and not an exit code — "the exit code and the verdict document are UNCHANGED" — precisely so
# that the two routes still agree here. This row is the one that caught the contradiction: the gate route
# refused with exit 2 and wrote no document at all, against a scan-produced report, on a measured 7-10%
# of real dependency reports.
jn="$WS/judgednothing"; mkdir -p "$jn/src" "$jn/out"
printf '[package]\nname = "facade"\n' > "$jn/Cargo.toml"
printf 'pub type Alias = u32;\npub struct S;\n' > "$jn/src/lib.rs"   # types only: nothing to judge
printf 'deny Net\n' > "$jn/policy"
rm -f "$WS/jn.scan.json" "$WS/jn.gate.json" "$jn/out/r".*
"$SCAN" "$jn" --out "$jn/out/r" --policy "$jn/policy" --gate-json "$WS/jn.scan.json" >/dev/null 2>&1
rc_scan=$?
"$QUERY" gate --report "$jn/out/r" --policy "$jn/policy" --gate-json "$WS/jn.gate.json" 2>"$WS/jn.err" >/dev/null
rc_gate=$?
rows=$((rows+1))
if [ "$rc_scan" -ne "$rc_gate" ]; then
  echo "  FAIL judged-nothing: exit $rc_scan (scan) vs $rc_gate (gate) on a report the SCAN produced."
  echo "       §3.1 requires the two routes to agree, and SPEC 0744d29 rules this a stderr disclosure"
  echo "       with the exit code and the verdict document UNMOVED."
  bad=$((bad+1))
elif [ ! -f "$WS/jn.gate.json" ] || ! cmp -s "$WS/jn.scan.json" "$WS/jn.gate.json"; then
  echo "  FAIL judged-nothing: the count-0 verdict is NOT byte-equal (or the gate wrote none at all)"
  diff "$WS/jn.scan.json" "$WS/jn.gate.json" 2>&1 | head -20
  bad=$((bad+1))
# …and the disclosure the exit code no longer carries MUST be on stderr, naming the package. Without
# this the row above is satisfied by simply deleting the refusal, which is the defect in fix's clothing.
elif ! grep -q 'JUDGED NOTHING' "$WS/jn.err" || ! grep -q 'facade' "$WS/jn.err"; then
  echo "  FAIL judged-nothing: the verb must SAY the report judged nothing and NAME the package —"
  echo "       ⟨0.24⟩ moves the obligation to the disclosure, it does not remove it. stderr was:"
  cat "$WS/jn.err"
  bad=$((bad+1))
fi

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
