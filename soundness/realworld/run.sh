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

# SELF-SKIP exits 3, never 0 — see soundness/oracle.sh's identical comment for why.
case "$(uname -s)" in Linux) : ;; *) echo "realworld oracle: needs Linux + strace (got $(uname -s)) — skipping."; exit 3 ;; esac
command -v strace >/dev/null 2>&1 || { echo "realworld oracle: strace not installed — skipping."; exit 3; }
# The verdict is computed by an inline python3 reader. Without it every prediction reads EMPTY and the
# harness reports a violation on every effectful driver — a missing interpreter would masquerade as a wall
# of findings. Fail fast and say so, rather than report a violation on every driver at once.
command -v python3 >/dev/null 2>&1 || { echo "realworld oracle: python3 not found — cannot read candor's report; skipping."; exit 3; }

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
# both builder-chain under-reports this oracle found (duct cmd!()→run, ureq get()→call) are FIXED by the
# GENERALIZED scan_builder_entry_effect table in candor-scan (over-approximate the entry for the syntactic
# engine; the deep engine types the terminal verb and stays precise). New verb-keyed crates that under-report
# get a table row + leave this empty.
KNOWN_UNDER=()

# member | effect ("" = pure control) | marker (must appear in the strace iff the effect ran)
CASES=(
  "net_std|Net|192.0.2.1"
  "net_minreq|Net|192.0.2.2"
  "net_ureq|Net|192.0.2.3"
  "exec_duct|Exec|candor-oracle-exec"
  "exec_xshell|Exec|candor-oracle-xshell"
  "exec_std|Exec|candor-oracle-exec-std"
  "exec_subprocess|Exec|candor-oracle-subprocess"
  "fs_fserr|Fs|/tmp/candor-oracle-fs-marker"
  "fs_std|Fs|/tmp/candor-oracle-fs-std"
  "fs_writefmt|Fs|/tmp/candor-oracle-writefmt"
  "fs_walkdir|Fs|candor-oracle-walk"
  "fs_tempfile|Fs|candor-oracle-temp"
  "fs_fsextra|Fs|/tmp/candor-oracle-fsextra"
  "pure_ctrl||__no_marker__"
  "net_socket2|Net|203.0.113.10"
  "fs_glob|Fs|candor-mk-glob"
  "fs_memmap2|Fs|candor-mk-mmap"
  "fs_filetime|Fs|candor-mk-ft"
  "fs_zip|Fs|candor-mk-zip"
  "ffi_libc|Fs|candor-mk-ffi"
)

pass=0; under=0; known=0; skip=0; fab=0; blame=0; failed=""
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
  # DISCLOSURE-RECALL calibration hook — UNSET in every normal run. When set, candor's signature is
  # falsified here (downstream of the analyzer, upstream of the verdict) so a known cardinal sin is
  # injected and this oracle MUST turn red. That is what makes a green run evidence rather than silence.
  # See recall/README.md; driven by recall/disclosure_recall.sh.
  if [ -n "${CANDOR_ORACLE_MUTATE:-}" ] && [ -n "$rep" ]; then
    python3 "$HERE/recall/mutate_report.py" "$CANDOR_ORACLE_MUTATE" "$rep" || true
  fi
  # Extract candor's PRECISE claim (the inferred effects EXCEPT Unknown — Unknown is disclosure, not a
  # precise effect), whether it disclosed any uncertainty, and the unknownWhy REASONS (the blame data:
  # the exact unresolved edge — dispatch:… / callback:… / reflect:… — to fix for a precise answer).
  # Prefer main()'s claim; fall back to the whole-program union when there is no main.
  read -r pred uncertain whys <<<"$(python3 - "${rep:-/dev/null}" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    funcs = d.get("functions", [])
except Exception:
    funcs = []
main = None; union = set(); unc = False; whys = set()
for f in funcs:
    s = set(f.get("inferred", [])); union |= s
    if "Unknown" in s or f.get("unresolved") or f.get("invisible") or f.get("blind") or f.get("incomplete"):
        unc = True
    for w in (f.get("unknownWhy") or []): whys.add(w)
    if f.get("fn", "").split("::")[-1] == "main":
        main = s
inferred = main if main is not None else union
precise = inferred - {"Unknown"}
print((",".join(sorted(precise)) or "-"), ("uncertain" if unc else "certain"), (";".join(sorted(whys)) or "-"))
PY
)"

  echo "  $m: ran=$ran  effect=${eff:-none}  candor=[$pred] $uncertain"
  if [ -z "$eff" ]; then  # pure control: nothing should run, nothing should be predicted
    { [ "$ran" = "0" ] && [ "$pred" = "-" ]; } && pass=$((pass+1)) || { echo "    ⚠ control: ran=$ran pred=$pred (expected none/none)"; fab=$((fab+1)); }
    continue
  fi
  if [ "$ran" = "0" ]; then echo "    SKIP ($eff did not execute under strace this run)"; skip=$((skip+1)); continue; fi
  # Three-way honesty verdict (mirrors candor-swift / candor-ts verify-core):
  #  (1) PRECISE   — the effect is in candor's precise (non-Unknown) claim: held tightly.
  #  (2) HELD BY DISCLOSURE — not precise, but Unknown was disclosed → honest, and BLAME-TRACKED: the
  #      unknownWhy reason names the exact unresolved edge to fix for a precise answer.
  #  (3) VIOLATION — neither: a silent-pure that demonstrably ran = the cardinal sin.
  # Pass/fail is UNCHANGED from the two-way form: precise ⇒ pass, disclosed-Unknown ⇒ pass (now blamed),
  # otherwise KNOWN or NEW under-report exactly as before.
  if echo ",$pred," | grep -q ",$eff,"; then
    pass=$((pass+1))
  elif [ "$uncertain" = "uncertain" ]; then
    echo "    ⓘ $eff held by DISCLOSURE (Unknown), not a precise claim — blame: [$whys]  (resolve this edge → precise $eff)"
    blame=$((blame+1)); pass=$((pass+1))
  elif printf '%s\n' "${KNOWN_UNDER[@]}" | grep -qx "$m"; then
    echo "    ⚠ KNOWN under-report (tracked, awaiting fix): ran $eff but candor predicts [$pred] — see KNOWN_UNDER"
    known=$((known+1))
  else
    echo "    ✗ NEW UNDER-REPORT: ran $eff (marker '$marker' in trace) but candor predicts [$pred] with no uncertainty"
    under=$((under+1)); failed="$failed $m"
  fi
done

echo
echo "realworld oracle: $pass honest ($blame held by disclosure+blamed), $known KNOWN under-report(s), $under NEW under-report(s), $fab fabrication(s), $skip skipped"
[ -n "$failed" ] && echo "realworld oracle: NEW under-reporting drivers:$failed"
# Green on known gaps; red only on a NEW under-report or a fabrication (a regression).
{ [ "$under" -eq 0 ] && [ "$fab" -eq 0 ]; }
