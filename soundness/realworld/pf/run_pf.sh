#!/usr/bin/env bash
# PER-FUNCTION dynamic oracle on REAL CRATES (the syscall-arm answer to "oracle_pf never run per-function
# on real third-party code — only synthetic seeds"). Linux + strace only; uses the DEPLOYED candor-scan
# (stable, no nightly).
#
# Each driver is a small program that reaches a REAL third-party crate's effect through a chain of its own
# functions, each bracketed with eprintln entry/exit markers (`CFE <fn>`/`CFX <fn>` — visible to strace as
# write(2,…), invisible to candor). We strace the run, reconstruct the CALL STACK at the moment the effect
# syscall fires, and assert candor-scan's PER-FUNCTION prediction for EVERY function on that stack contains
# the effect OR discloses Unknown. A function demonstrably on the stack at the effect but read pure is a
# per-function silent under-report — the cardinal case, on real-crate code, attributed to the exact fn.
#
#   bash run_pf.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
case "$(uname -s)" in Linux) : ;; *) echo "pf-realcrate: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;; esac
command -v strace >/dev/null 2>&1 || { echo "pf-realcrate: strace not installed — skipping."; exit 0; }
# The verdict is computed by an inline python3 reader. Without it every prediction reads EMPTY and the
# harness reports a violation on every effectful driver — a missing interpreter would masquerade as a wall
# of findings. Fail fast and say so, rather than report a violation on every driver at once.
command -v python3 >/dev/null 2>&1 || { echo "pf-realcrate: python3 not found — cannot read candor's report; skipping."; exit 0; }

# The repo .cargo/config forces -C linker=dylint-link (nightly lint); candor-scan + drivers are stable,
# so override to the normal linker (supersedes the config rustflags entirely), as run.sh does.
export RUSTFLAGS="-C linker=cc"

echo "pf-realcrate: building candor-scan (stable)…"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}" cargo +stable build -q -p candor-scan --manifest-path "$ROOT/Cargo.toml" \
  || { echo "FAIL: candor-scan build"; exit 1; }
SCAN="${CARGO_TARGET_DIR:-$ROOT/target}/debug/candor-scan"
[ -x "$SCAN" ] || { echo "FAIL: no candor-scan at $SCAN"; exit 1; }

# KNOWN, TRIAGED under-reports + the stale-entry ratchet. Shared with ../run.sh — ONE list mechanism
# for both oracles, not a second spelling of it (SOUNDNESS R102). See known_under.sh's header.
. "$HERE/../known_under.sh"

# Retry a (cargo) command — crates.io fetches flake transiently, which is NOT an oracle finding. Same
# shape as ../run.sh's. It exists here now because a driver that fails to build is RED below, so a
# network hiccup must not be able to masquerade as an instrument failure.
retry() { local n=0; until "$@"; do n=$((n+1)); [ "$n" -ge 3 ] && return 1; echo "  (retry $n after transient failure: $*)"; sleep 5; done; }

pass=0; fail=0; skip=0; known=0; broke=0; failed=""; status=""
for d in "$HERE"/pf_*/; do
  name="$(basename "$d")"
  [ -f "$d/src/main.rs" ] || continue
  # Every driver the oracle adjudicates records a status here — including the two early SKIP exits
  # below. The ratchet reads it, so a driver that silently never reached the verdict would show up
  # as a DEAD allowlist entry rather than as an entry quietly assumed still-open.
  note() { status="$status
$name $1"; }
  eff=$(python3 -c "import json;print(json.load(open('$d/truth.json'))['effect'])")
  marker=$(python3 -c "import json;print(json.load(open('$d/truth.json'))['marker'])")
  # Build + run the driver under strace (its own target dir so the host tree stays clean).
  # A driver that will not BUILD is an instrument failure, not a finding-free state: its coverage
  # silently vanishes and the run still says "0 failed". MEASURED 2026-09-02 — a canary driver added
  # to this directory without an empty `[workspace]` table was rejected by cargo, SKIPPED, and the
  # oracle exited 0 over it, which is precisely the "a build failure fakes a clean pass" shape the
  # canary exists to rule out. So: red, after `retry` has ruled out a transient fetch.
  ( cd "$d" && retry env CARGO_TARGET_DIR="$d/target" cargo build -q ) \
    || { echo "  $name: BUILD FAILED — the driver did not run at all (instrument failure, not a finding)"; broke=$((broke+1)); note broke; continue; }
  bin="$d/target/debug/$(python3 -c "import re,glob,os;print(os.path.basename('$d'.rstrip('/')))")"
  [ -x "$bin" ] || bin="$(ls "$d"/target/debug/pf_* 2>/dev/null | grep -v '\.d$' | grep -vF '.so' | head -1)"
  # Same class as BUILD FAILED, one step later: with no binary, strace runs nothing, the marker never
  # fires and the driver would read as an innocuous `SKIP marker-not-observed` — a lost driver wearing
  # the label of a legitimate one. ../run.sh already checks this; the per-function arm did not.
  [ -n "$bin" ] && [ -x "$bin" ] \
    || { echo "  $name: NO BINARY — the driver did not run at all (instrument failure, not a finding)"; broke=$((broke+1)); note broke; continue; }
  strace -f -e trace=write,connect,openat,open,execve -o "$d/trace.log" "$bin" >/dev/null 2>&1 || true
  # candor-scan the driver source (syntactic — no build).
  rm -rf "$d/.candor"; scanout="$("$SCAN" "$d" 2>&1)"; scanec=$?
  rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  if [ -z "$rep" ]; then
    # One loud retry before calling it. MEASURED 2026-09-02: one driver in one pass out of six reported
    # no report while 60 consecutive scans of that same driver in isolation produced 60 — i.e. a
    # directory-listing race under load (a bind-mounted tree), not candor-scan failing. The retry is
    # PRINTED, with candor-scan's own exit code and output, so a real instrument failure is still
    # distinguishable from a transient: a silent retry here would hide exactly what this branch is for.
    echo "  $name: no report on the first read (candor-scan exit=$scanec) — retrying once"
    [ -z "$scanout" ] || echo "      candor-scan said: $(printf '%s' "$scanout" | head -c 300)"
    "$SCAN" "$d" >/dev/null 2>&1
    rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  fi
  # Same class as a build failure: candor-scan produced no report for a driver that built. Nothing was
  # adjudicated, so this cannot count as a pass and must not be quietly absorbed as a skip.
  [ -n "$rep" ] || { echo "  $name: NO CANDOR REPORT — nothing was adjudicated (instrument failure, not a finding)"; broke=$((broke+1)); note broke; continue; }
  # DISCLOSURE-RECALL calibration hook — UNSET in every normal run. See ../recall/README.md.
  if [ -n "${CANDOR_ORACLE_MUTATE:-}" ]; then
    python3 "$HERE/../recall/mutate_report.py" "$CANDOR_ORACLE_MUTATE" "$rep" || true
  fi

  res=$(python3 - "$d/trace.log" "$rep" "$eff" "$marker" <<'PY'
import json,re,sys
trace,rep,eff,marker=sys.argv[1],sys.argv[2],sys.argv[3],sys.argv[4]
MARK=re.compile(r'write\(2, "CF([EX]) (\w+)')
stack=[]; on=set(); ran=False
for line in open(trace,errors="replace"):
    m=MARK.search(line)
    if m:
        k,n=m.group(1),m.group(2)
        if k=="E": stack.append(n)
        else:
            for i in range(len(stack)-1,-1,-1):
                if stack[i]==n: del stack[i]; break
        continue
    if marker in line:
        ran=True; on.update(stack)
if not ran:
    print("SKIP marker-not-observed"); sys.exit()
doc=json.load(open(rep)); fns={}
for f in (doc.get("functions",doc) if isinstance(doc,dict) else doc):
    if not isinstance(f,dict): continue
    inf=set(f.get("inferred") or [])
    # A function is HONEST if it names the effect OR discloses uncertainty in ANY disclosure channel —
    # exactly run.sh's rule: Unknown in inferred, or unresolved / invisible / blind / incomplete set.
    disclosed = ("Unknown" in inf) or any(f.get(k) for k in ("unresolved","invisible","blind","incomplete"))
    fns[f.get("fn","").split("::")[-1]]=(inf, disclosed)
bad=[]; held=0
for fn in sorted(on):
    rec=fns.get(fn)
    if rec is None: bad.append(fn+"(pure/omitted)"); continue
    inf, disclosed = rec
    if eff in inf: pass                       # precise
    elif disclosed: held+=1                   # honest — held by disclosure (e.g. invisible:<crate>)
    else: bad.append(fn+"{"+",".join(sorted(inf))+"}")
tag = " (%d held by disclosure)"%held if held else ""
print("FAIL on-stack-at-%s but candor missed: %s"%(eff," ".join(bad)) if bad
      else "OK (%d fns on stack at %s, all carry %s or disclose)%s"%(len(on),eff,eff,tag))
PY
)
  case "$res" in
    OK*)   echo "  $name: $res"; pass=$((pass+1)); note pass ;;
    SKIP*) echo "  $name: $res"; skip=$((skip+1)); note skip ;;
    *)
      # KNOWN, TRIAGED under-report -> green with a note, exactly as ../run.sh treats its own list.
      # The verdict text is kept verbatim so the suppression cannot hide a CHANGE in the failure.
      if meta="$(known_under_lookup "$name" ${KNOWN_UNDER_PERFN[@]+"${KNOWN_UNDER_PERFN[@]}"})"; then
        echo "  $name: KNOWN under-report (${meta%%|*}, tracked, awaiting fix): ${res#FAIL }"
        echo "      SOUNDNESS ${meta%%|*}: ${meta#*|}"
        known=$((known+1)); note known
      else
        echo "  $name: $res"; fail=$((fail+1)); failed="$failed $name"; note fail
      fi ;;
  esac
done
echo
# §H — zero failures is not zero gates. The driver set comes from a GLOB, so an empty or moved
# directory would otherwise aggregate to "0 failed" and exit 0 over nothing at all.
adjudicated=$((pass+skip+known+fail+broke))
[ "$adjudicated" -gt 0 ] || { echo "pf-realcrate: FAIL — no driver was adjudicated ($HERE/pf_*/ matched nothing runnable)."; exit 1; }
echo "pf-realcrate: $pass passed, $known KNOWN under-report(s) (allowlisted), $skip skipped, $fail NEW failure(s), $broke driver(s) that did not run"
ratchet=0
known_under_ratchet "$status" ${KNOWN_UNDER_PERFN[@]+"${KNOWN_UNDER_PERFN[@]}"} || ratchet=1
[ -n "$failed" ] && echo "pf-realcrate: per-function under-report on:$failed"
[ "$broke" -gt 0 ] && echo "pf-realcrate: $broke driver(s) never reached the verdict — their coverage is GONE this run, which is why this is red rather than skipped."
# Red on a NEW finding; equally red on an allowlist entry that has gone stale or dead (an allowlist
# consulted only in the failing branch is a gate that can never go red again); equally red on a driver
# that never ran (lost coverage is not a clean pass).
{ [ -z "$failed" ] && [ "$ratchet" -eq 0 ] && [ "$broke" -eq 0 ]; }
