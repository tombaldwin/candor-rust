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
#
# ⟨0.24⟩ …AND "SOMETHING FIRED" IS READ OFF THE DOCUMENT, NOT THE EXIT CODE. AUDITED 2026-07-28 by
# building a mutant serializer that keeps every exit code and writes `violations: []` regardless: of 51
# rows, exactly ONE noticed — the incomplete/B arm, which is the only one that ever looked INSIDE a
# document. All 48 matrix rows passed, because both routes deleted the same violations and stayed
# byte-equal while still exiting 1, and both empty-verdict arms passed because they have no violations
# to lose. Byte-equality between two routes cannot see a defect the two routes share, and the exit code
# is one bit. So every row now asserts the §3.3 agreement directly: **exit 1 ⟺ the document carries at
# least one violation record**, on BOTH documents.
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

# Does a verdict document carry at least one violation RECORD? The shared serializer pretty-prints an
# empty list as exactly `"violations": []`, and anything else is multi-line — so this is a content read,
# not a guess about the exit code.
has_violation() { ! grep -q '"violations": \[\]' "$1"; }

rows=0; fired=0; fired_doc=0; bad=0
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
    # ⟨0.24⟩ THE EXIT CODE AND THE DOCUMENT MUST AGREE, on each route independently. This is the arm the
    # 2026-07-28 mutant audit added: without it a route that computed the violations, exited 1 on them
    # and then wrote `violations: []` was indistinguishable from a correct one — and that is not a
    # hypothetical shape, it is what BOTH routes did over an incomplete analysis until `ff34070`.
    for side in scan gate; do
      [ "$side" = scan ] && { doc="$a"; rc="$rc_scan"; } || { doc="$b"; rc="$rc_gate"; }
      if has_violation "$doc"; then
        fired_doc=$((fired_doc+1))
        if [ "$rc" -ne 1 ]; then
          echo "  FAIL $c / '$p' ($side): exit $rc but the document carries violations — §3.3 forbids the"
          echo "       exit code and the verdict disagreeing in EITHER direction"
          bad=$((bad+1))
        fi
      elif [ "$rc" -eq 1 ]; then
        echo "  FAIL $c / '$p' ($side): exit 1 with an EMPTY \`violations\` list — the finding was printed"
        echo "       and then DELETED from the channel a CI consumer reads. The exit code is one bit; the"
        echo "       document is the evidence."
        bad=$((bad+1))
      fi
    done
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

# ⟨0.24⟩ THE CONFIG-ANCHOR ARM, which nothing above can reach because every row above files the policy
# INSIDE the scan target. SPEC §3.1: policy VOCABULARY (`unknown-alias`) resolves relative to the
# `--policy` file's directory on BOTH routes. Until 2026-07-28 the scan route anchored at the TARGET and
# the gate verb at the POLICY, so with the policy stored elsewhere the same rule expanded differently —
# **byte-equality breakable by a file that is neither the report nor the policy.** Measured then: scan
# exit 1 / gate exit 0 on one report and one policy.
#
# TWO ROWS, because one cannot tell "the alias resolved" from "the alias was ignored and the rule widened
# to a bare `deny Unknown`": the FIRING definition and the TOLERATING one must both agree across routes.
va="$WS/vocab"; mkdir -p "$va/tgt/src" "$va/home/.candor" "$va/tgt/out"
printf '[package]\nname = "vocab"\n' > "$va/tgt/Cargo.toml"
printf 'pub fn go(f: &dyn Fn() -> i32) -> i32 { f() }\n' > "$va/tgt/src/lib.rs"  # the hole is `indirect`
printf 'deny Unknown[corp]\n' > "$va/home/org.policy"
for arm in "fires indirect 1" "tolerates reflect 0"; do
  set -- $arm; tag=$1; cls=$2; want=$3
  printf 'unknown-alias corp = %s\n' "$cls" > "$va/home/.candor/config"
  rm -f "$WS/vocab.$tag.scan.json" "$WS/vocab.$tag.gate.json" "$va/tgt/out/r".*
  "$SCAN" "$va/tgt" --out "$va/tgt/out/r" --policy "$va/home/org.policy" \
      --gate-json "$WS/vocab.$tag.scan.json" >/dev/null 2>&1; rc_scan=$?
  "$QUERY" gate --report "$va/tgt/out/r" --policy "$va/home/org.policy" \
      --gate-json "$WS/vocab.$tag.gate.json" >/dev/null 2>&1; rc_gate=$?
  rows=$((rows+1))
  if [ "$rc_scan" -ne "$want" ] || [ "$rc_gate" -ne "$want" ]; then
    echo "  FAIL config-anchor/$tag (corp = $cls): exit $rc_scan (scan) vs $rc_gate (gate), both must be $want"
    echo "       §3.1: policy VOCABULARY anchors at the --policy file's directory on BOTH routes."
    bad=$((bad+1)); continue
  fi
  if [ ! -f "$WS/vocab.$tag.gate.json" ] || ! cmp -s "$WS/vocab.$tag.scan.json" "$WS/vocab.$tag.gate.json"; then
    echo "  FAIL config-anchor/$tag: the two routes wrote different documents from one report + one policy"
    diff "$WS/vocab.$tag.scan.json" "$WS/vocab.$tag.gate.json" 2>&1 | head -20
    bad=$((bad+1)); continue
  fi
  # …and the file that MOVED the verdict is NAMED on it. A verdict changed by a file the operator cannot
  # see named is the ambient-input failure the format exists to refuse; without this row the arm above is
  # satisfied by two routes that both ignore the config.
  if ! grep -q '"vocabulary"' "$WS/vocab.$tag.scan.json" || ! grep -q '"corp"' "$WS/vocab.$tag.scan.json"; then
    echo "  FAIL config-anchor/$tag: the verdict does not NAME the config that supplied the vocabulary"
    cat "$WS/vocab.$tag.scan.json"
    bad=$((bad+1))
  fi
done

# ⟨0.24⟩ THE POLICY-ERROR ARM (SPEC §6.2). An unrecognised reason-class token cannot be honoured AS
# WRITTEN, and dropping it REWRITES the rule — narrowing it when the typo sits beside valid tokens, which
# is the common case and is fail-open. Both routes take the unreadable-policy posture: exit 2.
#
# ⟨0.24⟩ …AND BOTH NOW WRITE A REFUSAL DOCUMENT (candor-spec `1503368` (b) removes the "no document on a
# config-shaped exit 2" carve-out — a CI wrapper reading the path unconditionally re-reads yesterday's
# green otherwise). This arm asserted "NEITHER route writes", which was a byte-equality claim about an
# absence; it now asserts the far stronger thing, that both write and both write the SAME SHAPE:
# `ok:false`, `refused:true`, NO `violations` key, and the offending token named IN THE DOCUMENT (stderr
# is not the channel CI reads). The `reason` PROSE is deliberately not compared — each route names its
# own remedy, and forcing one string would make the check about the copy rather than the contract.
pe="$WS/policyerr"; mkdir -p "$pe/src" "$pe/out"
printf '[package]\nname = "polerr"\n' > "$pe/Cargo.toml"
printf 'pub fn go(f: &dyn Fn() -> i32) -> i32 { f() }\n' > "$pe/src/lib.rs"
printf 'deny Unknown[dispatch,indirct]\n' > "$pe/policy"        # typo BESIDE a valid token
printf 'deny Unknown[dispatch,indirect]\n' > "$pe/policy.ok"    # the control: correctly spelled
rm -f "$WS/pe.scan.json" "$WS/pe.gate.json" "$pe/out/r".*
"$SCAN" "$pe" --out "$pe/out/r" --policy "$pe/policy.ok" --gate-json "$WS/pe.ctl.json" >/dev/null 2>&1
rc_ctl=$?
"$SCAN" "$pe" --out "$pe/out/r" --policy "$pe/policy" --gate-json "$WS/pe.scan.json" >/dev/null 2>&1
rc_scan=$?
"$QUERY" gate --report "$pe/out/r" --policy "$pe/policy" --gate-json "$WS/pe.gate.json" >/dev/null 2>&1
rc_gate=$?
rows=$((rows+1))
if [ "$rc_ctl" -ne 1 ]; then
  echo "  FAIL policy-error: the CONTROL (correctly-spelled rule) did not fire — the row below is vacuous"
  bad=$((bad+1))
elif [ "$rc_scan" -ne 2 ] || [ "$rc_gate" -ne 2 ]; then
  echo "  FAIL policy-error: exit $rc_scan (scan) vs $rc_gate (gate), both must be 2. A policy that cannot"
  echo "       be honoured as written must not be silently rewritten into a different policy (§6.2)."
  bad=$((bad+1))
elif [ ! -f "$WS/pe.scan.json" ] || [ ! -f "$WS/pe.gate.json" ]; then
  echo "  FAIL policy-error: a route wrote NO document on exit 2 — a consumer reading that path sees the"
  echo "       PREVIOUS run's verdict, which is stale (SPEC §3.1 ⟨0.24⟩, candor-spec 1503368)"
  bad=$((bad+1))
else
  for r in scan gate; do
    d="$WS/pe.$r.json"
    if ! grep -q '"refused": true' "$d" || ! grep -q '"ok": false' "$d"; then
      echo "  FAIL policy-error/$r: the refusal document's naive read is not the fail-closed one"; cat "$d"
      bad=$((bad+1))
    elif grep -q '"violations"' "$d"; then
      echo "  FAIL policy-error/$r: a refusal must make NO claim about violations"; cat "$d"
      bad=$((bad+1))
    elif ! grep -q 'indirct' "$d"; then
      echo "  FAIL policy-error/$r: the document does not NAME the token — stderr is not the CI channel"
      cat "$d"
      bad=$((bad+1))
    fi
  done
fi

if [ "$fired" -eq 0 ] || [ "$fired_doc" -eq 0 ]; then
  echo "gate-equivalence: VACUOUS — no policy in the matrix produced a violation IN A DOCUMENT"
  echo "                  (exit-1 rows: $fired, rows whose verdict carries a violation record: $fired_doc);"
  echo "                  byte-equal empty verdicts prove nothing. Fix the matrix, do not relax the check."
  exit 1
fi
if [ "$bad" -ne 0 ]; then
  echo "gate-equivalence: FAILED — $bad of $rows rows diverged"
  echo '  A divergence here is a finding ABOUT THE GATE: `scan --policy` and `gate --report` are two'
  echo '  routes into one gate, and the byte-equality is what makes that a property of the code.'
  exit 1
fi
echo "gate-equivalence: OK — $rows rows byte-equal; $fired exited 1 and $fired_doc verdict documents"
echo "                  carry a violation RECORD (SPEC §3.1 ⟨0.24⟩)"
exit 0
