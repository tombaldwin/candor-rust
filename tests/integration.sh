#!/usr/bin/env bash
# End-to-end integration tests for candor — the paths the in-process unit/ui tests can't reach
# because they need a real `cargo dylint` build: the AS-EFF mode diagnostics (env-driven) and
# cross-crate effect propagation across a lib+bin boundary. Run from the repo root:
#   bash tests/integration.sh
# Requires python3 (report inspection). Wired into CI.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "building candor lint…"
cargo build -q || { echo "FAIL: candor build"; exit 1; }
# Absolute path — the scenarios `cd` into fixture dirs, so a relative lib path would break.
LIB=$(ls "$ROOT"/target/debug/libcandor@*.dylib "$ROOT"/target/debug/libcandor@*.so 2>/dev/null | head -1)
[ -n "$LIB" ] || { echo "FAIL: no dylib under target/debug"; exit 1; }
command -v python3 >/dev/null || { echo "FAIL: python3 required"; exit 1; }

pass=0; fail=0
want()   { if printf '%s' "$2" | grep -qF -- "$3"; then echo "  ok   $1"; pass=$((pass+1)); else echo "  FAIL $1 — missing: $3"; fail=$((fail+1)); fi; }
absent() { if printf '%s' "$2" | grep -qF -- "$3"; then echo "  FAIL $1 — unexpected: $3"; fail=$((fail+1)); else echo "  ok   $1"; pass=$((pass+1)); fi; }
# Run dylint on a dir with a fresh lint pass (dylint emits only on recompile).
dl() { ( cd "$1"; rm -rf target/dylint; shift; "$@" cargo dylint --lib-path "$LIB" 2>&1 ); }

# ── 1. Conformance: AS-EFF-001/002/003 (CANDOR_STRICT) on the capability-discipline sample ──
echo "== conformance / AS-EFF-001/002/003 (CANDOR_STRICT, sample/) =="
out=$(dl sample env CANDOR_STRICT=1)
want   "AS-EFF-001 performs-but-undeclared (sneaky_read)" "$out" '[AS-EFF-001] `sneaky_read`'
want   "AS-EFF-002 declared-but-unused (greet)"           "$out" '[AS-EFF-002] `greet`'
want   "AS-EFF-003 unresolvable (run_callback)"           "$out" '[AS-EFF-003] `run_callback`'
absent "conformant read_config is NOT flagged"            "$out" '[AS-EFF-001] `read_config`'

# ── 2. Ambient authority: AS-EFF-004 (CANDOR_NO_AMBIENT) ──
echo "== ambient authority / AS-EFF-004 (CANDOR_NO_AMBIENT, sample/) =="
out=$(dl sample env CANDOR_NO_AMBIENT=1)
want "AS-EFF-004 flags a direct ambient reach" "$out" "[AS-EFF-004]"

# ── 3. Regression guard: AS-EFF-005 (CANDOR_BASELINE) ──
echo "== regression guard / AS-EFF-005 (CANDOR_BASELINE) =="
G=$(mktemp -d)/g; mkdir -p "$G/src" "$G/.candor"
printf '[package]\nname="g"\nversion="0.1.0"\nedition="2021"\n' > "$G/Cargo.toml"
printf 'fn touches_net() { let _ = std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main() { touches_net(); }\n' > "$G/src/main.rs"
dl "$G" env CANDOR_JSON="$G/.candor/base" >/dev/null
# Simulate an older baseline that predates touches_net's Net, then guard: it must flag the gain.
python3 - "$G/.candor" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + '/base.*.json'):
    if '.calibrated' in f or '.encountered' in f: continue
    try: d = json.load(open(f))
    except Exception: continue
    # v0.2 envelope {candor, functions} or legacy bare array; edit in place, preserve structure.
    funcs = d['functions'] if isinstance(d, dict) and isinstance(d.get('functions'), list) else d if isinstance(d, list) else None
    if funcs is None: continue
    for e in funcs:
        if e['fn'] == 'touches_net': e['inferred'] = []; e['direct'] = []
    json.dump(d, open(f, 'w'))
PY
out=$(dl "$G" env CANDOR_BASELINE="$G/.candor/base")
want "AS-EFF-005 flags touches_net gaining Net" "$out" '[AS-EFF-005] `touches_net`'

# ── 4. Cross-crate effect propagation: a bin inherits its lib's effect (CRITIQUE §8) ──
echo "== cross-crate effect inheritance (lib+bin) =="
X=$(mktemp -d)/xc; mkdir -p "$X/src" "$X/.candor"
printf '[package]\nname="xc"\nversion="0.1.0"\nedition="2021"\n' > "$X/Cargo.toml"
printf 'pub fn writes() { let _ = std::fs::write("/tmp/xc_probe", b"x"); }\n' > "$X/src/lib.rs"
printf 'fn caller() { xc::writes(); }\nfn main() { caller(); }\n' > "$X/src/main.rs"
dl "$X" env CANDOR_JSON="$X/.candor/r" >/dev/null
caller_eff=$(python3 - "$X/.candor" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + '/r.xc.Executable.json'):
    d = json.load(open(f))
    funcs = d['functions'] if isinstance(d, dict) else d   # v0.2 envelope or v0.1 array
    for e in funcs:
        if e['fn'] == 'caller':
            print(','.join(e['inferred']))
PY
)
want "bin 'caller' inherits lib Fs across the crate boundary" "$caller_eff" "Fs"

# ── 5. Version provenance: the dylib self-reports its build version, and reports are self-describing ──
echo "== version stamping (build tag + self-describing sidecar) =="
tag=$(strings -a "$LIB" 2>/dev/null | grep -oE 'candor-build-version=[0-9a-fA-F]+' | head -1)
want "dylib embeds a build-version tag readable without running it" "$tag" "candor-build-version="
sidecar_ver=$(python3 - "$X/.candor" <<'PY'
import json, glob, sys
for f in glob.glob(sys.argv[1] + '/r.calibrated.json'):
    d = json.load(open(f)); print(d.get('candor_version',''), d.get('toolchain',''))
PY
)
want "calibrated sidecar carries candor_version (self-describing report)" "$sidecar_ver" "${tag#candor-build-version=}"
absent "sidecar version is not the 'unknown' fallback"                    "$sidecar_ver" "unknown unknown"

# ── 6. At-a-glance audit: `cargo candor audit` aggregates the project into a profile ──
echo "== at-a-glance audit (cargo candor audit) =="
A=$(mktemp -d)/aud; mkdir -p "$A/src"
printf '[package]\nname="aud"\nversion="0.1.0"\nedition="2021"\n' > "$A/Cargo.toml"
printf 'fn reads(){ let _=std::fs::read("/tmp/x"); }\nfn runs(){ let _=std::process::Command::new("x").status(); }\nfn main(){ reads(); runs(); }\n' > "$A/src/main.rs"
aud=$( cd "$A"; "$ROOT/cargo-candor" audit 2>/dev/null )
want "audit: header carries the engine version"      "$aud" "candor @"
want "audit: effectful-function count line"          "$aud" "effectful functions"
want "audit: effect tally rendered"                  "$aud" "effects"
want "audit: broadest-surface section"               "$aud" "broadest effect surface"
want "audit: main shows its transitive { Exec Fs }"  "$aud" "Exec Fs"
rm -rf "$(dirname "$A")"

# ── 7. Agent-facing effect diff: `cargo candor diff` describes the per-function delta (P0 §1) ──
echo "== effect diff (cargo candor diff) =="
D=$(mktemp -d)/d; mkdir -p "$D/src"
printf '[package]\nname="d"\nversion="0.1.0"\nedition="2021"\n' > "$D/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn main(){ worker(); }\n' > "$D/src/main.rs"
( cd "$D"; "$ROOT/cargo-candor" snapshot .candor/baseline >/dev/null 2>&1 )
# an agent adds a network call deep in `worker` — a LOCAL edit with a NON-LOCAL consequence.
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main(){ worker(); }\n' > "$D/src/main.rs"
dout=$( cd "$D"; "$ROOT/cargo-candor" diff 2>/dev/null )
djson=$( cd "$D"; "$ROOT/cargo-candor" diff --json 2>/dev/null )
want "diff: the edited fn (worker) is flagged"             "$dout" "worker"
want "diff: +Net rendered"                                 "$dout" "+Net"
want "diff: the caller (main) inherits the gain — blast radius" "$dout" "main"
want "diff --json: machine-readable for the agent"         "$djson" '"gained"'
want "diff --json: carries the gained Net"                 "$djson" 'Net'
rm -rf "$(dirname "$D")"

rm -rf "$(dirname "$G")" "$(dirname "$X")" 2>/dev/null

echo
echo "integration: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
