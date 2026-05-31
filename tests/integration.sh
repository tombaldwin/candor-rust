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
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn mid(){ worker(); }\nfn main(){ mid(); }\n' > "$D/src/main.rs"
( cd "$D"; "$ROOT/cargo-candor" snapshot .candor/baseline >/dev/null 2>&1 )
# an agent adds a network call deep in `worker` — a LOCAL edit with a NON-LOCAL consequence; it
# propagates worker → mid → main, so `main` is the top-level surface and `mid` is plumbing.
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn mid(){ worker(); }\nfn main(){ mid(); }\n' > "$D/src/main.rs"
dout=$( cd "$D"; "$ROOT/cargo-candor" diff 2>/dev/null )
djson=$( cd "$D"; "$ROOT/cargo-candor" diff --json 2>/dev/null )
want "diff: the edited fn (worker) is flagged"             "$dout" "worker"
want "diff: marks the source with * (worker introduced Net)" "$dout" "+Net*"
want "diff: per-effect headline names the source"          "$dout" "introduced in"
want "diff (§9): headline names where it surfaces (reaches main)" "$dout" "reaches main"
want "diff (§9): the top-level endpoint is tagged"         "$dout" "top-level"
want "diff (§9): the intermediate plumbing (mid) is collapsed" "$dout" "intermediate caller"
want "diff --json: machine-readable for the agent"         "$djson" '"gained"'
want "diff --json: classifies introduced vs inherited"     "$djson" '"introduced"'
rm -rf "$(dirname "$D")"

# ── 8. explain: trace the call path to each effect's source (P0 §3) ──
echo "== explain (cargo candor explain) =="
P=$(mktemp -d)/p; mkdir -p "$P/src"
printf '[package]\nname="p"\nversion="0.1.0"\nedition="2021"\n' > "$P/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn middle(){ leaf(); }\nfn main(){ middle(); }\n' > "$P/src/main.rs"
xout=$( cd "$P"; "$ROOT/cargo-candor" explain main 2>/dev/null )
want "explain: header for the queried function"   "$xout" "candor explain — main"
want "explain: traces the multi-hop call path"    "$xout" "main → middle → leaf"
want "explain: names the leaf effectful call"     "$xout" "TcpStream::connect"
rm -rf "$(dirname "$P")"

# ── 9. Effect policy: enforce architectural boundaries (AS-EFF-006, P0′ §6) ──
echo "== effect policy / AS-EFF-006 (CANDOR_POLICY) =="
PL=$(mktemp -d)/pl; mkdir -p "$PL/src"
printf '[package]\nname="pl"\nversion="0.1.0"\nedition="2021"\n' > "$PL/Cargo.toml"
# domain_logic is pure-LOOKING but reaches the filesystem transitively via leaf(); domain_pure doesn't.
printf 'fn leaf(){ let _=std::fs::read("/tmp/x"); }\nfn domain_logic(){ leaf(); }\nfn domain_pure(){ let _=1+1; }\nfn main(){ domain_logic(); domain_pure(); }\n' > "$PL/src/main.rs"
echo "deny Fs Net  domain" > "$PL/policy"
out=$(dl "$PL" env CANDOR_POLICY="$PL/policy")
want   "AS-EFF-006 flags the TRANSITIVE boundary violation (domain_logic reaches Fs via a helper)" "$out" '[AS-EFF-006] `domain_logic`'
absent "the genuinely-pure domain fn is NOT flagged"                                               "$out" '[AS-EFF-006] `domain_pure`'
rm -rf "$(dirname "$PL")"

# ── 10. Taint heuristic: an effect on caller-derived input (AS-EFF-007, P0′ §7) ──
echo "== taint heuristic / AS-EFF-007 (CANDOR_TAINT) =="
TZ=$(mktemp -d)/tz; mkdir -p "$TZ/src"
printf '[package]\nname="tz"\nversion="0.1.0"\nedition="2021"\n' > "$TZ/Cargo.toml"
# read_user builds the path from a parameter (injection class); read_fixed uses a literal (safe).
printf 'fn read_user(key:&str)->Option<String>{ std::fs::read_to_string(format!("/var/cache/{key}")).ok() }\nfn read_fixed()->Option<String>{ std::fs::read_to_string("/etc/app.conf").ok() }\nfn main(){ let _=read_user("x"); let _=read_fixed(); }\n' > "$TZ/src/main.rs"
out=$(dl "$TZ" env CANDOR_TAINT=1)
want   "AS-EFF-007 flags Fs on a parameter-derived path (read_user)" "$out" '[AS-EFF-007] `read_user`'
absent "a literal-path Fs is NOT flagged (read_fixed)"               "$out" '[AS-EFF-007] `read_fixed`'
rm -rf "$(dirname "$TZ")"

# ── 11. diff FAST PATH: read the kept-fresh report instead of recompiling (P0′ §8 / speed) ──
echo "== diff fast path (no recompile when .candor/report is fresh) =="
FP=$(mktemp -d)/fp; mkdir -p "$FP/src" "$FP/.candor"
printf '[package]\nname="fp"\nversion="0.1.0"\nedition="2021"\n' > "$FP/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn main(){ worker(); }\n' > "$FP/src/main.rs"
( cd "$FP"; "$ROOT/cargo-candor" snapshot .candor/baseline >/dev/null 2>&1 )
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main(){ worker(); }\n' > "$FP/src/main.rs"
# simulate the Stop hook: a fresh report + matching state hash for the edited source
( cd "$FP"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
( cd "$FP"; find "$PWD" -name '*.rs' -not -path '*/target/*' -print0 | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1 > .candor/state )
fpout=$( cd "$FP"; "$ROOT/cargo-candor" diff 2>&1 )
want   "fast-path diff still computes the delta (worker +Net)"  "$fpout" "+Net"
absent "fast path did NOT recompile (no 'analyzing…')"          "$fpout" "analyzing"
rm -rf "$(dirname "$FP")"

# ── 12. watch: keep the report fresh in the background, off the critical path (P0′ §8 / speed) ──
echo "== watch (background report freshness) =="
WV=$(mktemp -d)/wv; mkdir -p "$WV/src"
printf '[package]\nname="wv"\nversion="0.1.0"\nedition="2021"\n' > "$WV/Cargo.toml"
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); }\nfn main(){ worker(); }\n' > "$WV/src/main.rs"
( cd "$WV"; CANDOR_WATCH_INTERVAL=1 "$ROOT/cargo-candor" watch >/dev/null 2>&1 ) & WPID=$!
for _ in $(seq 1 90); do ls "$WV"/.candor/report.*.*.json >/dev/null 2>&1 && break; sleep 1; done   # initial report
printf 'fn worker(){ let _=std::fs::read("/tmp/x"); let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn main(){ worker(); }\n' > "$WV/src/main.rs"
seen=no
for _ in $(seq 1 90); do grep -q Net "$WV"/.candor/report.*.*.json 2>/dev/null && { seen=yes; break; }; sleep 1; done
kill "$WPID" 2>/dev/null; wait "$WPID" 2>/dev/null
want "watch auto-refreshed the report after an edit (worker gained Net)" "$seen" "yes"
rm -rf "$(dirname "$WV")"

# ── 13. Instant read-only queries served from the report: show / where (P0′ §8 / speed) ──
echo "== instant queries (show / where) =="
Q=$(mktemp -d)/q; mkdir -p "$Q/src" "$Q/.candor"
printf '[package]\nname="q"\nversion="0.1.0"\nedition="2021"\n' > "$Q/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn handler(){ leaf(); }\nfn main(){ handler(); }\n' > "$Q/src/main.rs"
# a fresh report (as watch/the hook would maintain) → the queries serve instantly, no recompile
( cd "$Q"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
( cd "$Q"; find "$PWD" -name '*.rs' -not -path '*/target/*' -print0 | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1 > .candor/state )
shout=$( cd "$Q"; "$ROOT/cargo-candor" show handler 2>&1 )
whout=$( cd "$Q"; "$ROOT/cargo-candor" where Net 2>&1 )
whj=$(   cd "$Q"; "$ROOT/cargo-candor" where Net --json 2>&1 )
clout=$(  cd "$Q"; "$ROOT/cargo-candor" callers leaf 2>&1 )
want   "show: handler's transitive Net is reported"   "$shout" "Net"
absent "show served from the report (did NOT recompile)" "$shout" "generating one"
want   "where: splits the direct source out"          "$whout" "directly"
want   "where: names the source (leaf)"               "$whout" "leaf"
want   "where --json: machine-readable"               "$whj" '"directly"'
want   "callers: leaf's caller (handler) found from the report" "$clout" "handler"
rm -rf "$(dirname "$Q")"

# ── 14. MCP server: candor's queries as native agent tools (P0′ §10) ──
echo "== MCP server (candor-mcp.py) =="
MCP="$ROOT/integrations/mcp/candor-mcp.py"
mout=$(printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | python3 "$MCP" 2>/dev/null)
want "MCP: initialize returns serverInfo"        "$mout" '"serverInfo"'
want "MCP: tools/list exposes candor_where"      "$mout" 'candor_where'
want "MCP: tools/list exposes candor_callers"    "$mout" 'candor_callers'
M=$(mktemp -d)/m; mkdir -p "$M/src" "$M/.candor"
printf '[package]\nname="m"\nversion="0.1.0"\nedition="2021"\n' > "$M/Cargo.toml"
printf 'fn leaf(){ let _=std::net::TcpStream::connect("127.0.0.1:1"); }\nfn handler(){ leaf(); }\nfn main(){ handler(); }\n' > "$M/src/main.rs"
( cd "$M"; CANDOR_JSON="$PWD/.candor/report" cargo dylint --lib-path "$LIB" >/dev/null 2>&1 )
( cd "$M"; find "$PWD" -name '*.rs' -not -path '*/target/*' -print0 | sort -z | xargs -0 shasum 2>/dev/null | shasum | cut -d' ' -f1 > .candor/state )
cout=$( cd "$M"; printf '%s\n' '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"candor_where","arguments":{"effect":"Net"}}}' | python3 "$MCP" 2>/dev/null )
want "MCP: tools/call candor_where returns the live result" "$cout" 'leaf'
rm -rf "$(dirname "$M")"

rm -rf "$(dirname "$G")" "$(dirname "$X")" 2>/dev/null

echo
echo "integration: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
