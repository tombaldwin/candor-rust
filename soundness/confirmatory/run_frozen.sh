#!/usr/bin/env bash
# FROZEN confirmatory run — Rust syscall arm (see FROZEN.md). Linux + strace. Reuses the mechanism of the
# CI-green soundness/realworld oracle (candor-scan prediction vs kernel ground truth), on a held-out,
# version-pinned, pre-registered crate manifest.
#
#   EXPECT_SHA=<linux candor-scan sha256>  bash run_frozen.sh
#
# EXECUTION NOTE: authored + mechanism-proven; run this on a Linux CI runner / non-loaded Linux host. It was
# not executed on the author's macOS (Docker too CPU-starved to build candor-scan in reasonable time).
#
# ############################################################################################################
# # SOUNDNESS INVARIANT — READ BEFORE EDITING.                                                              #
# # The H-VIOLATION check runs on observed_raw (the FULL kernel-observed class set), NEVER on the           #
# # baseline-subtracted observed_crate. Baseline subtraction (harness-artifact removal) is INFORMATIONAL    #
# # ONLY: it sharpens the *reported coverage quality* so the story reflects the crate's own effects, not    #
# # the libtest runner's. Subtracting the baseline from the CHECKED set could delete a class that is BOTH a #
# # harness artifact AND a genuine crate effect — hiding a real under-report = the cardinal sin (a false    #
# # all-clear). Over-observation is the SAFE direction: it can only make a class easier to cover, never     #
# # hide a real miss. So the violation computation below reads observed_raw and observed_raw alone.         #
# ############################################################################################################
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
case "$(uname -s)" in Linux) : ;; *) echo "rust confirmatory: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;; esac
command -v strace >/dev/null 2>&1 || { echo "strace not installed — skipping."; exit 0; }
export RUSTFLAGS="${RUSTFLAGS:--C linker=cc}"   # the repo .cargo/config forces dylint-link; scan is stable

echo "building candor-scan (stable)…"
( cd "$ROOT" && cargo +stable build -q -p candor-scan ) || { echo "FAIL: candor-scan build"; exit 1; }
SCAN="$ROOT/target/debug/candor-scan"
[ -x "$SCAN" ] || { echo "FAIL: no candor-scan"; exit 1; }
GOT="$(sha256sum "$SCAN" | cut -d' ' -f1)"
if [ -n "${EXPECT_SHA:-}" ] && [ "$EXPECT_SHA" != "$GOT" ]; then
  echo "FROZEN ABORT: candor-scan hash mismatch (got $GOT want $EXPECT_SHA)"; exit 1
fi
echo "candor-scan sha256: $GOT   (set EXPECT_SHA to enforce)"

WORK="${CORPUS_WORK:-${TMPDIR:-/tmp}/candor-rust-corpus}"; mkdir -p "$WORK" "$HERE/results"
SUM="$HERE/results/FROZEN-SUMMARY.tsv"
# Columns (see FROZEN.md):
#   observed_raw  = every effect class the kernel emitted under the strace harness (THE CHECKED SET).
#   observed_crate= observed_raw MINUS the measured harness baseline — INFORMATIONAL only (see invariant).
#   named         = observed_raw classes some crate function's inferred set LITERALLY contains (strong).
#   unknown_only  = observed_raw classes covered ONLY by a disclosed Unknown (honest but weak/near-vacuous).
#   violations    = observed_raw classes NO function names AND NO function discloses Unknown (cardinal sin).
#   level         = per-function (strace -k stacks reconstructed) or program (fallback).
printf 'crate\ttag\tobserved_raw\tobserved_crate\tnamed\tunknown_only\tviolations\tlevel\tverdict\n' > "$SUM"

# ---------------------------------------------------------------------------------------------------------
# HARNESS BASELINE (informational). Compile a throwaway crate whose only test is `fn noop(){}` and strace
# its test binary through the SAME pipeline. Whatever effect classes appear are produced by libtest + the
# Rust runtime + the loader ITSELF (it opens shared objects -> Fs; may open a control socket -> Net; the
# parallel runner may spawn -> Exec), not by any crate under test. We subtract this set from observed_raw to
# report observed_crate. THIS NEVER GATES — see the soundness invariant banner above.
# ---------------------------------------------------------------------------------------------------------
BASELINE="-"
measure_baseline() {
  local bd="$WORK/__baseline__"
  rm -rf "$bd"; mkdir -p "$bd/src"
  printf '[package]\nname="candor_baseline"\nversion="0.0.0"\nedition="2021"\n[lib]\npath="src/lib.rs"\n' > "$bd/Cargo.toml"
  printf '#[test] fn noop() {}\n' > "$bd/src/lib.rs"
  ( cd "$bd" && cargo test -q --no-run ) >"$bd/build.log" 2>&1 || { echo "  baseline build failed — baseline empty"; return; }
  local btbs; mapfile -t btbs < <( cd "$bd" && cargo test --no-run --message-format=json 2>/dev/null \
    | python3 -c "import json,sys
for l in sys.stdin:
    try: m=json.loads(l)
    except: continue
    if isinstance(m,dict) and m.get('profile',{}).get('test') and m.get('executable'): print(m['executable'])" )
  : > "$bd/trace.log"
  local tb
  for tb in "${btbs[@]}"; do
    [ -x "$tb" ] || continue
    timeout -s KILL 120 strace -f -e trace=openat,openat2,open,connect,socket,execve,unlink,unlinkat -o "$bd/trace.one" "$tb" >/dev/null 2>"$bd/strace.err" || true
    cat "$bd/trace.one" >> "$bd/trace.log" 2>/dev/null; rm -f "$bd/trace.one"
  done
  BASELINE=$(python3 - "$bd/trace.log" <<'PY'
import sys,re
CLS={'openat':'Fs','openat2':'Fs','open':'Fs','unlink':'Fs','unlinkat':'Fs','connect':'Net','socket':'Net','execve':'Exec'}
RX=re.compile(r'(?:^|\s)(openat2|openat|open|connect|socket|execve|unlink|unlinkat)\(')
obs=set()
for line in open(sys.argv[1],errors='replace'):
    m=RX.search(line)
    if m and m.group(1) in CLS: obs.add(CLS[m.group(1)])
print(",".join(sorted(obs)) or "-")
PY
)
}
echo "measuring harness baseline (empty no-op test crate under the same strace pipeline)…"
measure_baseline
echo "harness baseline effect classes (informational, subtracted from observed_crate only): [$BASELINE]"

grep -vE '^\s*#|^\s*$' "$HERE/manifest.tsv" | while IFS=$'\t' read -r name url tag effects why; do
  echo; echo "################## $name ($tag) ##################"
  d="$WORK/$name"
  [ -d "$d/.git" ] || { rm -rf "$d"; git clone --quiet --depth 1 --branch "$tag" "$url" "$d" 2>/dev/null \
      || { echo "  clone-failed"; printf '%s\t%s\t-\t-\t-\t-\t-\t-\tclone-failed\n' "$name" "$tag" >>"$SUM"; continue; }; }

  # candor-scan the crate SOURCE (syntactic; no build). Collect the union of predicted effect classes and
  # whether ANY function discloses Unknown (program-level coverage).
  rm -rf "$d/.candor"; "$SCAN" "$d" >/dev/null 2>&1
  rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  [ -n "$rep" ] || { echo "  no report — scan-failed"; printf '%s\t%s\t-\t-\t-\t-\t-\t-\tscan-failed\n' "$name" "$tag" >>"$SUM"; continue; }

  # Build ALL the crate's test binaries (lib unit tests AND integration tests — the latter usually hold the
  # real I/O) and strace EACH (never cargo, whose own syscalls would pollute). cargo --message-format=json
  # reports every test executable's path.
  if ! ( cd "$d" && cargo test -q --no-run ) >"$d/build.log" 2>&1; then
    echo "  build-failed (see $d/build.log)"; printf '%s\t%s\t-\t-\t-\t-\t-\t-\tbuild-failed\n' "$name" "$tag" >>"$SUM"; continue
  fi
  mapfile -t TESTBINS < <( cd "$d" && cargo test --no-run --message-format=json 2>/dev/null \
    | python3 -c "import json,sys
for l in sys.stdin:
    try: m=json.loads(l)
    except: continue
    if isinstance(m,dict) and m.get('profile',{}).get('test') and m.get('executable'): print(m['executable'])" )
  [ "${#TESTBINS[@]}" -gt 0 ] || { echo "  no test bins"; printf '%s\t%s\t-\t-\t-\t-\t-\t-\tno-test-bin\n' "$name" "$tag" >>"$SUM"; continue; }
  echo "  ${#TESTBINS[@]} test binaries -> strace each"
  : > "$d/trace.log"; : > "$d/ktrace.log"
  kfrac_ok=0
  for tb in "${TESTBINS[@]}"; do
    [ -x "$tb" ] || continue
    # -f follow forks; +openat2 (modern glibc uses openat2 for opens); strace stderr -> a diag file, not /dev/null.
    timeout -s KILL 240 strace -f -e trace=openat,openat2,open,connect,socket,execve,unlink,unlinkat -o "$d/trace.one" "$tb" >/dev/null 2>"$d/strace.err" || true
    cat "$d/trace.one" >> "$d/trace.log" 2>/dev/null; rm -f "$d/trace.one"
    # -k kernel stack unwind at each effect syscall (best-effort; needs frame pointers/DWARF in the test bin).
    # Kept in a SEPARATE trace so the program-level observed_raw above is never affected by -k availability.
    timeout -s KILL 300 strace -f -k -e trace=openat,openat2,open,connect,socket,execve,unlink,unlinkat -o "$d/ktrace.one" "$tb" >/dev/null 2>>"$d/strace.err" || true
    cat "$d/ktrace.one" >> "$d/ktrace.log" 2>/dev/null; rm -f "$d/ktrace.one"
  done
  echo "  DIAG trace lines=$(wc -l < "$d/trace.log" 2>/dev/null) ktrace lines=$(wc -l < "$d/ktrace.log" 2>/dev/null) strace.err='$(tail -1 "$d/strace.err" 2>/dev/null)'"

  # ---- PROGRAM-LEVEL analysis on observed_raw (THE CHECKED SET) + informational columns. --------------
  observed=$(BASELINE="$BASELINE" python3 - "$d/trace.log" "$rep" <<'PY'
import json,sys,re,os
trace,rep=sys.argv[1],sys.argv[2]
CLS={'openat':'Fs','openat2':'Fs','open':'Fs','unlink':'Fs','unlinkat':'Fs','connect':'Net','socket':'Net','execve':'Exec'}
# observed_raw = EVERY effect class the kernel emitted. This is the set the H-violation check runs on.
RX=re.compile(r'(?:^|\s)(openat2|openat|open|connect|socket|execve|unlink|unlinkat)\(')
raw=set()
for line in open(trace,errors='replace'):
    m=RX.search(line)
    if m and m.group(1) in CLS: raw.add(CLS[m.group(1)])
# candor prediction: named classes (union of every fn's inferred minus Unknown) + program-level Unknown flag.
d=json.load(open(rep)); named=set(); unknown=False
for f in d.get('functions',[]):
    inf=set(f.get('inferred') or [])
    named |= (inf - {'Unknown'})
    if 'Unknown' in inf or f.get('invisible') or f.get('unresolved') or f.get('incomplete'): unknown=True
# harness baseline (informational only) -> observed_crate. NEVER used for the violation check below.
base=set(x for x in (os.environ.get('BASELINE','') or '').split(',') if x and x!='-')
crate = raw - base
# named-vs-Unknown breakdown + the VIOLATION check — all on observed_raw (raw), never on crate.
named_cov=set(); unknown_only=set(); viol=set()
for c in raw:                                  # <-- SOUNDNESS: iterate observed_raw, not observed_crate.
    if c in named: named_cov.add(c)            # strong: a function literally names this class.
    elif unknown:  unknown_only.add(c)         # weak: covered only by a disclosed Unknown (near-vacuous).
    else:          viol.add(c)                 # cardinal sin: undisclosed observed effect class.
j=lambda s: ",".join(sorted(s)) or "-"
print("%s|%s|%s|%s|%s"%(j(raw),j(crate),j(named_cov),j(unknown_only),j(viol)))
PY
)
  IFS='|' read -r obs_raw obs_crate named unk_only vio <<<"$observed"

  # ---- PER-FUNCTION upgrade via -k stacks (best-effort; honest fallback to program-level). -------------
  # Reconstruct the crate functions on the kernel stack at each effect syscall and check per-function H:
  # every ON-STACK crate function must NAME the effect class or disclose Unknown. Falls back to program-
  # level (level=program) whenever no -k stack yields a demangled crate frame we can attribute.
  pf=$(python3 - "$d/ktrace.log" "$rep" <<'PY'
import json,sys,re
ktrace,rep=sys.argv[1],sys.argv[2]
SYS2CLS={'openat':'Fs','openat2':'Fs','open':'Fs','unlink':'Fs','unlinkat':'Fs','connect':'Net','socket':'Net','execve':'Exec'}
# strace -k emits, after each traced syscall line, indented backtrace frames like:
#   > /path/libfoo.so(_ZN4crate3fooEv+0x12) [0x...]
# We collect the frames belonging to each effect syscall event.
SYSLINE=re.compile(r'(?:^|\s)(openat2|openat|open|connect|socket|execve|unlink|unlinkat)\(')
FRAME=re.compile(r'^\s*>\s')
# symbol inside parens: name(+0xNN) — strip the offset.
SYM=re.compile(r'\(([^)+]+)')

def demangle_leaf(sym):
    # Rust legacy mangling: _ZN<len><ident><len><ident>...E, with a trailing hash segment. Decode the
    # path components and drop the 17-hex hash. Fall back: if it isn't mangled, use the raw symbol. The
    # crate-function LEAF is the last path component (matches how the candor report keys fns: fn.split('::')[-1]).
    s=sym
    if s.startswith('_ZN') and s.endswith('E'):
        body=s[3:-1]; parts=[]; i=0
        while i < len(body):
            j=i
            while j < len(body) and body[j].isdigit(): j+=1
            if j==i: break
            n=int(body[i:j]); i=j; comp=body[i:i+n]; i+=n
            parts.append(comp)
        # drop trailing hash component like h1a2b3c4... (17 chars starting 'h')
        parts=[p for p in parts if not (len(p)>=17 and p[0]=='h' and all(c in '0123456789abcdef' for c in p[1:]))]
        if parts: return parts[-1]
    # v0 mangling (_R...) or already-demangled 'crate::mod::fn' — take the last :: segment, strip generics.
    s=re.sub(r'<[^<>]*>','',s)
    return s.split('::')[-1].strip()

# Parse: walk lines; when we hit an effect syscall line, start capturing its following frame block.
events=[]   # list of (cls, [leaf,...])
cur=None
for line in open(ktrace,errors='replace'):
    if FRAME.match(line):
        if cur is not None:
            m=SYM.search(line)
            if m: cur[1].append(demangle_leaf(m.group(1)))
        continue
    m=SYSLINE.search(line)
    if m:
        cls=SYS2CLS.get(m.group(1))
        if cur is not None: events.append(cur)
        cur=[cls,[]] if cls else None
    else:
        if cur is not None: events.append(cur); cur=None
if cur is not None: events.append(cur)

# candor per-function table, keyed by leaf name (report 'fn' is like 'Mod::method' or bare 'fn').
d=json.load(open(rep)); fns={}
for f in d.get('functions',[]):
    inf=set(f.get('inferred') or [])
    disclosed = ('Unknown' in inf) or any(f.get(k) for k in ('unresolved','invisible','blind','incomplete'))
    leaf=(f.get('fn') or '').split('::')[-1]
    if leaf: fns[leaf]=(inf,disclosed)

# For every effect event, intersect its on-stack leaves with functions candor actually reported for THIS
# crate (frames from std/libc/other crates aren't in the report and aren't ours to check). A per-function
# violation = a reported crate function demonstrably on the stack at the effect that neither names the
# class nor discloses Unknown.
checked=0; bad=set()
for cls,leaves in events:
    onstack=[l for l in leaves if l in fns]
    if not onstack: continue
    checked+=1
    for l in onstack:
        inf,disclosed=fns[l]
        if cls in inf or disclosed: continue
        bad.add("%s@%s{%s}"%(l,cls,",".join(sorted(inf)) or "-"))
if checked==0:
    print("program|-")           # honest fallback: no attributable crate frame -> program-level only.
else:
    print("perfn|%s"%(";".join(sorted(bad)) or "-"))
PY
)
  IFS='|' read -r level pf_bad <<<"$pf"
  [ "$level" = "perfn" ] || level="program"

  # VERDICT is derived from the observed_raw violation set (vio) — the program-level cardinal-sin gate.
  # The per-function pass is an ADDITIONAL, stricter datapoint reported alongside; a per-function bad set
  # is surfaced in the verdict string but the primary H gate remains observed_raw (never weaker).
  if [ "$vio" != "-" ]; then verdict="VIOLATION[$vio]"
  elif [ "$level" = "perfn" ] && [ "$pf_bad" != "-" ]; then verdict="PF-VIOLATION[$pf_bad]"
  else verdict="H-holds"; fi
  echo "  observed_raw=[$obs_raw] observed_crate=[$obs_crate] named=[$named] unknown_only=[$unk_only] level=$level -> $verdict"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$tag" "$obs_raw" "$obs_crate" "$named" "$unk_only" "$vio" "$level" "$verdict" >>"$SUM"
done

echo; echo "===================== RUST CONFIRMATORY (program-level H on observed_raw) ====================="
echo "harness baseline (informational, subtracted only in observed_crate): [$BASELINE]"
column -t -s "$(printf '\t')" "$SUM" 2>/dev/null || cat "$SUM"
nviol=$(awk -F'\t' 'NR>1 && $9 ~ /^VIOLATION/' "$SUM" | wc -l | tr -d ' ')
npf=$(awk -F'\t' 'NR>1 && $9 ~ /^PF-VIOLATION/' "$SUM" | wc -l | tr -d ' ')
echo; echo "crates with an undisclosed observed effect class (program-level false all-clear, on observed_raw): $nviol"
echo "crates with a per-function under-report (stricter -k check, program-level still H-holds): $npf"
