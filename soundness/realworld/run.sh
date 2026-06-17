#!/usr/bin/env bash
# Real-world DYNAMIC oracle (Bet 1, phase 3) — kernel ground truth on REAL-crate classification.
#
# For each driver crate (which exercises a REAL effectful crate with a distinctive marker): build it, RUN
# it under strace, and confirm the effect actually executed (its marker appears in the trace). If it did,
# assert candor-scan's STATIC prediction for the program contains that effect — OR discloses uncertainty
# (Unknown / blind / invisible / unresolved), which is honest. An effect that demonstrably RAN but which
# candor predicts NOWHERE and discloses NOWHERE (silent-pure) is a real under-report — the dangerous lie.
#
# Unlike the generated oracle (soundness/oracle.sh, std-only synthetic seeds) this tests candor's REAL
# κ-table against the kernel, incl. an UNCALIBRATED crate (net_minreq) — the true honesty probe. Uses
# candor-scan (stable, the deployed engine; no nightly). Linux + strace only.
#
#   bash soundness/realworld/run.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

case "$(uname -s)" in Linux) : ;; *) echo "realworld oracle: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;; esac
command -v strace >/dev/null 2>&1 || { echo "realworld oracle: strace not installed — skipping."; exit 0; }

# The repo's .cargo/config forces `-C linker=dylint-link` (for the nightly lint). This oracle uses only
# candor-scan (stable, no dylint), so override RUSTFLAGS to the normal linker — avoids needing dylint-link
# installed. (Setting RUSTFLAGS supersedes the config's rustflags entirely.)
export RUSTFLAGS="-C linker=cc"

# Retry a (cargo) command — crates.io fetches flake transiently in CI (SSL eof), which is NOT an oracle
# finding; a retry keeps a network hiccup from masquerading as a failure.
retry() { local n=0; until "$@"; do n=$((n+1)); [ "$n" -ge 3 ] && return 1; echo "  (retry $n after transient failure: $*)"; sleep 5; done; }

echo "realworld oracle: building candor-scan (stable)…"
retry cargo +stable build -q --manifest-path "$ROOT/Cargo.toml" -p candor-scan || { echo "FAIL: candor-scan build"; exit 1; }
SCAN="$ROOT/target/debug/candor-scan"

# KNOWN, TRIAGED under-reports — tracked so the oracle is a clean gate (green on known gaps, red only on
# NEW findings). Each needs a real fix; listed here with the root cause, not silently ignored. Empty now:
# the duct cmd!() macro-receiver under-report this oracle FOUND is FIXED (scan_builder_entry_effect in
# candor-scan; the entry is over-approximated Exec for the syntactic engine, the deep engine stays precise).
KNOWN_UNDER=()

# member | effect ("" = pure control) | marker (must appear in the strace iff the effect ran)
CASES=(
  "net_std|Net|192.0.2.1"
  "net_minreq|Net|192.0.2.2"
  "exec_duct|Exec|candor-oracle-exec"
  "fs_fserr|Fs|/tmp/candor-oracle-fs-marker"
  "pure_ctrl||__no_marker__"
)

pass=0; under=0; known=0; skip=0; fab=0; failed=""
for row in "${CASES[@]}"; do
  IFS='|' read -r m eff marker <<<"$row"
  d="$HERE/$m"
  retry cargo +stable build -q --manifest-path "$HERE/Cargo.toml" -p "$m" 2>/dev/null \
    || { echo "  $m: build failed — SKIP"; skip=$((skip+1)); continue; }
  bin="$HERE/target/debug/$m"
  [ -x "$bin" ] || { echo "  $m: no binary — SKIP"; skip=$((skip+1)); continue; }

  strace -f -e trace=connect,socket,openat,open,execve -o "$d/trace.log" "$bin" >/dev/null 2>&1 || true
  ran=0; grep -qF "$marker" "$d/trace.log" 2>/dev/null && ran=1

  rm -rf "$d/.candor" 2>/dev/null
  "$SCAN" "$d" >/dev/null 2>&1
  rep=$(ls "$d"/.candor/report.*.scan.json 2>/dev/null | grep -v callgraph | head -1)
  read -r pred uncertain <<<"$(python3 - "${rep:-/dev/null}" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    funcs = d.get("functions", [])
except Exception:
    funcs = []
main = None; union = set(); unc = False
for f in funcs:
    s = set(f.get("inferred", [])); union |= s
    if "Unknown" in s or f.get("unresolved") or f.get("invisible") or f.get("blind") or f.get("incomplete"):
        unc = True
    if f.get("fn", "").split("::")[-1] == "main":
        main = s
pred = main if main is not None else union
print((",".join(sorted(pred)) or "-"), ("uncertain" if unc else "certain"))
PY
)"

  echo "  $m: ran=$ran  effect=${eff:-none}  candor=[$pred] $uncertain"
  if [ -z "$eff" ]; then  # pure control: nothing should run, nothing should be predicted
    { [ "$ran" = "0" ] && [ "$pred" = "-" ]; } && pass=$((pass+1)) || { echo "    ⚠ control: ran=$ran pred=$pred (expected none/none)"; fab=$((fab+1)); }
    continue
  fi
  if [ "$ran" = "0" ]; then echo "    SKIP ($eff did not execute under strace this run)"; skip=$((skip+1)); continue; fi
  if echo ",$pred," | grep -q ",$eff," || echo ",$pred," | grep -q ",Unknown," || [ "$uncertain" = "uncertain" ]; then
    pass=$((pass+1))
  elif printf '%s\n' "${KNOWN_UNDER[@]}" | grep -qx "$m"; then
    echo "    ⚠ KNOWN under-report (tracked, awaiting fix): ran $eff but candor predicts [$pred] — see KNOWN_UNDER"
    known=$((known+1))
  else
    echo "    ✗ NEW UNDER-REPORT: ran $eff (marker '$marker' in trace) but candor predicts [$pred] with no uncertainty"
    under=$((under+1)); failed="$failed $m"
  fi
done

echo
echo "realworld oracle: $pass honest, $known KNOWN under-report(s), $under NEW under-report(s), $fab fabrication(s), $skip skipped"
[ -n "$failed" ] && echo "realworld oracle: NEW under-reporting drivers:$failed"
# Green on known gaps; red only on a NEW under-report or a fabrication (a regression).
{ [ "$under" -eq 0 ] && [ "$fab" -eq 0 ]; }
