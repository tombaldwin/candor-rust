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

pass=0; fail=0; skip=0; failed=""
for d in "$HERE"/pf_*/; do
  name="$(basename "$d")"
  [ -f "$d/src/main.rs" ] || continue
  eff=$(python3 -c "import json;print(json.load(open('$d/truth.json'))['effect'])")
  marker=$(python3 -c "import json;print(json.load(open('$d/truth.json'))['marker'])")
  # Build + run the driver under strace (its own target dir so the host tree stays clean).
  ( cd "$d" && CARGO_TARGET_DIR="$d/target" cargo build -q ) || { echo "  $name: build failed — SKIP"; skip=$((skip+1)); continue; }
  bin="$d/target/debug/$(python3 -c "import re,glob,os;print(os.path.basename('$d'.rstrip('/')))")"
  [ -x "$bin" ] || bin="$(ls "$d"/target/debug/pf_* 2>/dev/null | grep -v '\.d$' | grep -vF '.so' | head -1)"
  strace -f -e trace=write,connect,openat,open,execve -o "$d/trace.log" "$bin" >/dev/null 2>&1 || true
  # candor-scan the driver source (syntactic — no build).
  rm -rf "$d/.candor"; "$SCAN" "$d" >/dev/null 2>&1
  rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  [ -n "$rep" ] || { echo "  $name: no candor report — SKIP"; skip=$((skip+1)); continue; }
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
    OK*)   echo "  $name: $res"; pass=$((pass+1)) ;;
    SKIP*) echo "  $name: $res"; skip=$((skip+1)) ;;
    *)     echo "  $name: $res"; fail=$((fail+1)); failed="$failed $name" ;;
  esac
done
echo
echo "pf-realcrate: $pass passed, $skip skipped, $fail failed"
[ -n "$failed" ] && { echo "pf-realcrate: per-function under-report on:$failed"; exit 1; }
exit 0
