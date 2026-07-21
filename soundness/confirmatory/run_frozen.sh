#!/usr/bin/env bash
# FROZEN confirmatory run — Rust syscall arm (see FROZEN.md). Linux + strace. Reuses the mechanism of the
# CI-green soundness/realworld oracle (candor-scan prediction vs kernel ground truth), on a held-out,
# version-pinned, pre-registered crate manifest.
#
#   EXPECT_SHA=<linux candor-scan sha256>  bash run_frozen.sh
#
# EXECUTION NOTE: authored + mechanism-proven; run this on a Linux CI runner / non-loaded Linux host. It was
# not executed on the author's macOS (Docker too CPU-starved to build candor-scan in reasonable time).
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
printf 'crate\ttag\tobserved\tcovered\tviolations\tverdict\n' > "$SUM"

# map a strace effect syscall to a candor effect class
class_of() { case "$1" in openat|open|unlink|unlinkat) echo Fs;; connect) echo Net;; execve) echo Exec;; *) echo "";; esac; }

grep -vE '^\s*#|^\s*$' "$HERE/manifest.tsv" | while IFS=$'\t' read -r name url tag effects why; do
  echo; echo "################## $name ($tag) ##################"
  d="$WORK/$name"
  [ -d "$d/.git" ] || { rm -rf "$d"; git clone --quiet --depth 1 --branch "$tag" "$url" "$d" 2>/dev/null \
      || { echo "  clone-failed"; printf '%s\t%s\t-\t-\t-\tclone-failed\n' "$name" "$tag" >>"$SUM"; continue; }; }

  # candor-scan the crate SOURCE (syntactic; no build). Collect the union of predicted effect classes and
  # whether ANY function discloses Unknown (program-level coverage).
  rm -rf "$d/.candor"; "$SCAN" "$d" >/dev/null 2>&1
  rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  [ -n "$rep" ] || { echo "  no report — scan-failed"; printf '%s\t%s\t-\t-\t-\tscan-failed\n' "$name" "$tag" >>"$SUM"; continue; }

  # Build ALL the crate's test binaries (lib unit tests AND integration tests — the latter usually hold the
  # real I/O) and strace EACH (never cargo, whose own syscalls would pollute). cargo --message-format=json
  # reports every test executable's path.
  if ! ( cd "$d" && cargo test -q --no-run ) >"$d/build.log" 2>&1; then
    echo "  build-failed (see $d/build.log)"; printf '%s\t%s\t-\t-\t-\tbuild-failed\n' "$name" "$tag" >>"$SUM"; continue
  fi
  mapfile -t TESTBINS < <( cd "$d" && cargo test --no-run --message-format=json 2>/dev/null \
    | python3 -c "import json,sys
for l in sys.stdin:
    try: m=json.loads(l)
    except: continue
    if isinstance(m,dict) and m.get('profile',{}).get('test') and m.get('executable'): print(m['executable'])" )
  [ "${#TESTBINS[@]}" -gt 0 ] || { echo "  no test bins"; printf '%s\t%s\t-\t-\t-\tno-test-bin\n' "$name" "$tag" >>"$SUM"; continue; }
  echo "  ${#TESTBINS[@]} test binaries -> strace each"
  : > "$d/trace.log"
  for tb in "${TESTBINS[@]}"; do
    [ -x "$tb" ] || continue
    # -f follow forks; +openat2 (modern glibc uses openat2 for opens); strace stderr -> a diag file, not /dev/null.
    strace -f -e trace=openat,openat2,open,connect,socket,execve,unlink,unlinkat -o "$d/trace.one" "$tb" >/dev/null 2>"$d/strace.err" || true
    cat "$d/trace.one" >> "$d/trace.log" 2>/dev/null; rm -f "$d/trace.one"
  done
  echo "  DIAG trace lines=$(wc -l < "$d/trace.log" 2>/dev/null) strace.err='$(tail -1 "$d/strace.err" 2>/dev/null)'"

  observed=$(python3 - "$d/trace.log" "$rep" <<'PY'
import json,sys,re
trace,rep=sys.argv[1],sys.argv[2]
CLS={'openat':'Fs','openat2':'Fs','open':'Fs','unlink':'Fs','unlinkat':'Fs','connect':'Net','socket':'Net','execve':'Exec'}
obs=set()
# strace -f prefixes lines as "PID  syscall(...)" (pid, spaces, syscall) — not "[pid N] syscall(". Match the
# effect syscall name wherever it sits, tolerant of the pid prefix, resumed/unfinished lines, and no-prefix.
RX=re.compile(r'(?:^|\s)(openat2|openat|open|connect|socket|execve|unlink|unlinkat)\(')
for line in open(trace,errors='replace'):
    m=RX.search(line)
    if m and m.group(1) in CLS: obs.add(CLS[m.group(1)])
d=json.load(open(rep)); named=set(); unknown=False
for f in d.get('functions',[]):
    inf=set(f.get('inferred') or [])
    named|= (inf - {'Unknown'})
    if 'Unknown' in inf or f.get('invisible') or f.get('unresolved') or f.get('incomplete'): unknown=True
# a Net/Db/Llm refinement: Db/Llm count as Net-covered
covered=set(); viol=set()
for c in obs:
    if c in named or unknown: covered.add(c)
    else: viol.add(c)
print("%s|%s|%s"%(",".join(sorted(obs)) or "-", ",".join(sorted(covered)) or "-", ",".join(sorted(viol)) or "-"))
PY
)
  IFS='|' read -r obs cov vio <<<"$observed"
  verdict=$([ "$vio" = "-" ] && echo "H-holds" || echo "VIOLATION[$vio]")
  echo "  observed=[$obs] covered=[$cov] -> $verdict"
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$tag" "$obs" "$cov" "$vio" "$verdict" >>"$SUM"
done

echo; echo "===================== RUST CONFIRMATORY (program-level H) ====================="
column -t -s "$(printf '\t')" "$SUM" 2>/dev/null || cat "$SUM"
nviol=$(awk -F'\t' 'NR>1 && $6 ~ /VIOLATION/' "$SUM" | wc -l | tr -d ' ')
echo; echo "crates with an undisclosed observed effect class (program-level false all-clear): $nviol"
