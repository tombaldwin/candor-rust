#!/usr/bin/env bash
# Real-world DYNAMIC oracle — the DEEP (nightly dylint) engine against kernel ground truth.
#
# The SIBLING of run.sh: identical driver crates, identical strace ground truth, identical CASES table
# and triage logic — but it drives the NIGHTLY dylib (`src/lib.rs`, the SOUND engine) via
# `cargo dylint --lib-path <libcandor@*.so>` + CANDOR_JSON, instead of candor-scan. The driver crates are
# engine-AGNOSTIC (they're just real effectful programs), so the only differences are the build/predict
# path here. This closes the CI evidence gap: the continuous real-world syscall oracle covered only the
# STABLE scanner; the deep engine's strace oracle (soundness/oracle.sh) ran only on the SYNTHETIC fuzzer.
# Now the engine candor calls "the sound gate / continuously oracle-verified" is verified the same way,
# against the same real crates, on every push.
#
# An effect that demonstrably RAN (its marker is in the trace) but which the deep engine predicts NOWHERE
# and discloses NOWHERE (no Unknown / invisible / incomplete) is a silent under-report — the dangerous lie.
#
# Linux + strace only, AND needs the pinned nightly + `dylint-link` linker (rust-toolchain + the repo's
# .cargo/config). On macOS / without strace it SKIPs (exit 0), like run.sh.
#
#   bash soundness/realworld/run_deep.sh
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

case "$(uname -s)" in Linux) : ;; *) echo "realworld DEEP oracle: needs Linux + strace (got $(uname -s)) — skipping."; exit 0 ;; esac
command -v strace >/dev/null 2>&1 || { echo "realworld DEEP oracle: strace not installed — skipping."; exit 0; }

# Retry a (cargo) command — crates.io fetches flake transiently in CI (SSL eof), which is NOT an oracle
# finding; a retry keeps a network hiccup from masquerading as a failure. (Mirrors run.sh.)
retry() { local n=0; until "$@"; do n=$((n+1)); [ "$n" -ge 3 ] && return 1; echo "  (retry $n after transient failure: $*)"; sleep 5; done; }

# Build the nightly dylib (the engine under test). cargo picks up the pinned nightly from rust-toolchain;
# the repo's .cargo/config supplies `-C linker=dylint-link`. Unlike run.sh we do NOT override RUSTFLAGS —
# the deep engine's drivers are linked the dylint way, exactly as in soundness/oracle.sh.
echo "realworld DEEP oracle: building the nightly candor dylib…"
retry cargo build -q --manifest-path "$ROOT/Cargo.toml" || { echo "FAIL: candor (nightly dylib) build"; exit 1; }
LIB=""
for c in "$ROOT"/target/debug/libcandor@*.so; do
  [ -e "$c" ] || continue
  # NEWEST by mtime — the filename carries the toolchain, so a stale build from an old pinned nightly
  # must not shadow the fresh one (same reasoning as cargo-candor's `newest_of` / soundness/oracle.sh).
  { [ -z "$LIB" ] || [ "$c" -nt "$LIB" ]; } && LIB="$c"
done
[ -n "$LIB" ] || { echo "FAIL: no candor dylib (.so) under target/debug"; exit 1; }

# KNOWN, TRIAGED under-reports for the DEEP engine — tracked so the oracle is a clean gate (green on known
# gaps, red only on NEW findings). Empty now; a real gap gets a row here + a tracking note, never silence.
KNOWN_UNDER=()

# member | effect ("" = pure control) | marker (must appear in the strace iff the effect ran).
# SAME table as run.sh — the driver crates are shared (engine-agnostic), so the ground truth is identical.
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
)

pass=0; under=0; known=0; skip=0; fab=0; failed=""
for row in "${CASES[@]}"; do
  IFS='|' read -r m eff marker <<<"$row"
  d="$HERE/$m"
  # Build + RUN the driver to get kernel ground truth (the binary is engine-agnostic; building it under
  # the deep toolchain is fine — it's an ordinary program). The deep engine then analyses the SAME crate.
  retry cargo build -q --manifest-path "$HERE/Cargo.toml" -p "$m" 2>/dev/null \
    || { echo "  $m: build failed — SKIP"; skip=$((skip+1)); continue; }
  bin="$HERE/target/debug/$m"
  [ -x "$bin" ] || { echo "  $m: no binary — SKIP"; skip=$((skip+1)); continue; }

  strace -f -e trace=connect,socket,openat,open,execve -o "$d/trace.log" "$bin" >/dev/null 2>&1 || true
  ran=0; grep -qF "$marker" "$d/trace.log" 2>/dev/null && ran=1

  # The deep engine's static prediction: cargo dylint over the crate, writing a CANDOR_JSON report.
  # Clear target/dylint so dylint actually re-runs the lint (CANDOR_* env is not in cargo's fingerprint).
  rm -rf "$d/r."*.json "$d/target/dylint" 2>/dev/null
  ( cd "$d" && CANDOR_JSON="$d/r" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
  rep=$(ls "$d"/r.*.json 2>/dev/null | grep -v -e callgraph -e calibrated -e encountered -e layerreach | head -1)
  read -r pred uncertain <<<"$(python3 - "${rep:-/dev/null}" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    funcs = d.get("functions", []) if isinstance(d, dict) else (d if isinstance(d, list) else [])
except Exception:
    funcs = []
main = None; union = set(); unc = False
for f in funcs:
    s = set(f.get("inferred", [])); union |= s
    # Deep-engine uncertainty disclosure: an Unknown effect, or a disclosed blind/masked surface.
    if "Unknown" in s or f.get("invisible") or f.get("incomplete"):
        unc = True
    if f.get("fn", "").split("::")[-1] == "main":
        main = s
pred = main if main is not None else union
print((",".join(sorted(pred)) or "-"), ("uncertain" if unc else "certain"))
PY
)"

  echo "  $m: ran=$ran  effect=${eff:-none}  candor(deep)=[$pred] $uncertain"
  if [ -z "$eff" ]; then  # pure control: nothing should run, nothing should be predicted
    { [ "$ran" = "0" ] && [ "$pred" = "-" ]; } && pass=$((pass+1)) || { echo "    ⚠ control: ran=$ran pred=$pred (expected none/none)"; fab=$((fab+1)); }
    continue
  fi
  if [ "$ran" = "0" ]; then echo "    SKIP ($eff did not execute under strace this run)"; skip=$((skip+1)); continue; fi
  if echo ",$pred," | grep -q ",$eff," || echo ",$pred," | grep -q ",Unknown," || [ "$uncertain" = "uncertain" ]; then
    pass=$((pass+1))
  elif printf '%s\n' "${KNOWN_UNDER[@]}" | grep -qx "$m"; then
    echo "    ⚠ KNOWN under-report (tracked, awaiting fix): ran $eff but candor(deep) predicts [$pred] — see KNOWN_UNDER"
    known=$((known+1))
  else
    echo "    ✗ NEW UNDER-REPORT: ran $eff (marker '$marker' in trace) but candor(deep) predicts [$pred] with no uncertainty"
    under=$((under+1)); failed="$failed $m"
  fi
done

echo
echo "realworld DEEP oracle: $pass honest, $known KNOWN under-report(s), $under NEW under-report(s), $fab fabrication(s), $skip skipped"
[ -n "$failed" ] && echo "realworld DEEP oracle: NEW under-reporting drivers:$failed"
# Green on known gaps; red only on a NEW under-report or a fabrication (a regression).
{ [ "$under" -eq 0 ] && [ "$fab" -eq 0 ]; }
