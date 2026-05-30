#!/usr/bin/env bash
# candor-run.sh — deterministic core of the candor Claude Code integration.
#
# Prints ONE human-readable "receipt" line to stdout describing the current effect
# map, its freshness, and any coverage gaps. Re-runs candor only when Rust sources
# changed since the last run (content hash), so it's cheap to call every turn.
#
# Usage:  candor-run.sh [--force] [PROJECT_DIR]
#   PROJECT_DIR defaults to $CLAUDE_PROJECT_DIR, else the git toplevel, else cwd.
#
# Exit code is a SURFACE HINT for the Stop hook (slash command ignores it):
#   0  quiet  — nothing changed; report already current (don't nag the user)
#   10 surface — re-ran this call (fresh map) OR stale warning OR first run
# Always exits 0/10 and always prints the receipt; status is in the text.
set -uo pipefail

FORCE=0
[ "${1:-}" = "--force" ] && { FORCE=1; shift; }
DIR="${1:-${CLAUDE_PROJECT_DIR:-}}"
[ -z "$DIR" ] && DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$DIR" 2>/dev/null || { echo "candor: cannot enter project dir"; exit 0; }
[ -f Cargo.toml ] || exit 0   # not a Rust project; say nothing

STATE_DIR="$DIR/.candor"
STATE="$STATE_DIR/state"
REPORT_PREFIX="$STATE_DIR/report"
CONFIG="$STATE_DIR/config"
mkdir -p "$STATE_DIR" 2>/dev/null || true
# Per-project config (written by install.sh) pins CANDOR_HOME (the candor clone) and
# CANDOR_LIB (the dylib). The clone is the single source of truth for the engine, these
# scripts, and AGENTS.md — `git pull` / `cargo candor update` updates all three together.
[ -f "$CONFIG" ] && . "$CONFIG"

# ---- content hash of all Rust sources (path + content, target/.git excluded) ----
src_hash() {
  find "$DIR" -name '*.rs' -not -path '*/target/*' -not -path '*/.git/*' -print0 2>/dev/null \
    | sort -z | xargs -0 shasum 2>/dev/null | shasum 2>/dev/null | cut -d' ' -f1
}
CUR="$(src_hash)"; SHORT="${CUR:0:8}"
PREV=""; [ -f "$STATE" ] && PREV="$(cat "$STATE" 2>/dev/null)"

# ---- locate the candor dylib ----
find_lib() {
  local c
  for c in "${CANDOR_LIB:-}" \
           "${CANDOR_HOME:-}"/target/debug/libcandor@*.dylib "${CANDOR_HOME:-}"/target/debug/libcandor@*.so \
           "$DIR"/../candor/target/debug/libcandor@*.dylib "$DIR"/../candor/target/debug/libcandor@*.so \
           /tmp/candor/target/debug/libcandor@*.dylib /tmp/candor/target/debug/libcandor@*.so; do
    [ -n "$c" ] && [ -e "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}

# ---- version stamp + update/rebuild nudges (the clone is the single source of truth) ----
LIBP="$(find_lib 2>/dev/null || true)"
VER=""; NUDGE=""
if [ -n "${CANDOR_HOME:-}" ] && git -C "$CANDOR_HOME" rev-parse --git-dir >/dev/null 2>&1; then
  VER="$(git -C "$CANDOR_HOME" rev-parse --short HEAD 2>/dev/null)"
  # Dylib older than engine source → the running binary lags the clone (e.g. you pulled
  # but didn't rebuild). Cheap mtime check, every run.
  if [ -n "$LIBP" ] && [ -f "$CANDOR_HOME/src/lib.rs" ] && [ "$LIBP" -ot "$CANDOR_HOME/src/lib.rs" ]; then
    NUDGE="$NUDGE · ⚠ dylib older than source — rebuild: cargo candor update"
  fi
  # Upstream check hits the network, so only the explicit /candor (--force) does it;
  # the per-turn Stop hook stays offline and fast.
  if [ "$FORCE" = 1 ]; then
    git -C "$CANDOR_HOME" fetch -q origin 2>/dev/null || true
    up="$(git -C "$CANDOR_HOME" rev-parse '@{u}' 2>/dev/null || git -C "$CANDOR_HOME" rev-parse origin/main 2>/dev/null || true)"
    head="$(git -C "$CANDOR_HOME" rev-parse HEAD 2>/dev/null || true)"
    [ -n "$up" ] && [ -n "$head" ] && [ "$up" != "$head" ] && NUDGE="$NUDGE · ⚠ candor update available — run: cargo candor update"
  fi
fi
VERSTAMP=""; [ -n "$VER" ] && VERSTAMP=" @$VER"

# ---- decide whether to (re)run candor ----
# Report is current iff it exists AND the source hash hasn't moved since we wrote it.
# Reports are `<prefix>.<crate>.<type>.json` (two middle segments); the `.*.*.json` glob
# matches those but NOT the single-segment `<prefix>.calibrated.json` coverage sidecar.
report_exists() { ls "$REPORT_PREFIX".*.*.json >/dev/null 2>&1; }
need_run=$FORCE
[ "$CUR" != "$PREV" ] && need_run=1
report_exists || need_run=1

# Touch every workspace member's crate root (mtime only) to force `cargo dylint` to
# recompile — dylint emits the report ONLY on recompilation, so an already-built
# project would otherwise produce nothing.
touch_roots() {
  if command -v cargo >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
    cargo metadata --no-deps --format-version 1 2>/dev/null | python3 -c '
import sys, json, os
try: m = json.load(sys.stdin)
except Exception: sys.exit(0)
for p in m.get("packages", []):
    d = os.path.dirname(p["manifest_path"])
    for f in ("src/lib.rs", "src/main.rs"):
        fp = os.path.join(d, f)
        if os.path.exists(fp): print(fp)
' | while IFS= read -r f; do touch "$f"; done
  else
    for r in src/lib.rs src/main.rs; do [ -f "$DIR/$r" ] && touch "$DIR/$r"; done
  fi
}
run_lint() { CANDOR_JSON="$REPORT_PREFIX" cargo dylint --lib-path "$LIB" >/dev/null 2>"$STATE_DIR/last-error.log"; }

ran_ok=1; surfaced=0
if [ "$need_run" = 1 ]; then
  surfaced=10
  if [ -z "$LIBP" ]; then
    echo "candor$VERSTAMP ⚠ not installed — no dylib found. Build candor (cargo build in the clone) or set CANDOR_HOME/CANDOR_LIB in .candor/config.$NUDGE"
    exit 10
  fi
  LIB="$LIBP"
  MARK="$STATE_DIR/.mark"; : > "$MARK"
  emitted() { [ -n "$(find "$STATE_DIR" -name 'report.*.*.json' -newer "$MARK" 2>/dev/null)" ]; }
  if run_lint && emitted; then
    echo "$CUR" > "$STATE"; ran_ok=1
  else
    # dylint produced no fresh report — either the crate didn't recompile (already
    # built) or it failed to compile. Force a recompile and retry once.
    touch_roots
    : > "$MARK"
    if run_lint && emitted; then
      echo "$CUR" > "$STATE"; ran_ok=1
    else
      ran_ok=0   # genuine build error: keep last good report (if any), flag STALE
    fi
  fi
fi

# ---- aggregate the report (python3 preferred; degrade gracefully) ----
read_summary() {
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$REPORT_PREFIX" <<'PY' 2>/dev/null
import sys, glob, json
pre = sys.argv[1]
fns = 0; eff = {}; unres = 0
for f in glob.glob(pre + '.*.*.json'):
    try:
        data = json.load(open(f))
    except Exception:
        continue
    if not isinstance(data, list):   # skip the calibrated.json sidecar, etc.
        continue
    for e in data:
        fns += 1
        inf = e.get('inferred', []) or []
        for x in inf:
            eff[x] = eff.get(x, 0) + 1
        if e.get('unresolved') or 'Unknown' in inf:
            unres += 1
order = ['Db', 'Net', 'Fs', 'Exec', 'Env', 'Clock', 'Ipc', 'Rand', 'Clipboard', 'Log']
parts = [f"{eff[k]} {k}" for k in order if eff.get(k)]
print(f"{fns}\t{', '.join(parts)}\t{unres}")
PY
  else
    # crude fallback: count "fn" keys across report files
    local n
    n=$(grep -ho '"fn"' "$REPORT_PREFIX".*.*.json 2>/dev/null | wc -l | tr -d ' ')
    printf '%s\t(install python3 for the effect breakdown)\t?\n' "$n"
  fi
}
SUM="$(read_summary)"
FNS="$(printf '%s' "$SUM" | cut -f1)"
EFFS="$(printf '%s' "$SUM" | cut -f2)"
UNRES="$(printf '%s' "$SUM" | cut -f3)"
[ -z "$FNS" ] && FNS=0
[ -z "$EFFS" ] && EFFS="no effects detected"

# ---- coverage: dependencies that LOOK effectful but candor has no rule for ----
# The calibrated set is read from what the ENGINE emitted beside the report
# (<prefix>.calibrated.json) — the single source of truth. The hardcoded list is only
# a fallback for when that sidecar isn't present (e.g. report not yet generated). The
# SUSPECT pattern is a deliberately-curated heuristic — it nudges, it does not certify.
CALIBRATED="reqwest isahc ureq sqlx rusqlite postgres tokio_postgres diesel redis mongodb mysql mysql_async sea_orm deadpool_postgres memmap2 rand getrandom fastrand chrono tracing arboard portable_pty"
CALIB_PREFIXES="aws_sdk_ aws_smithy cap_"
if [ -f "$REPORT_PREFIX.calibrated.json" ] && command -v python3 >/dev/null 2>&1; then
  line="$(python3 - "$REPORT_PREFIX.calibrated.json" <<'PY' 2>/dev/null
import sys, json
d = json.load(open(sys.argv[1]))
print(" ".join(d.get("crates", [])) + "|" + " ".join(d.get("prefixes", [])))
PY
)"
  if [ -n "$line" ]; then
    CALIBRATED="${line%%|*}"
    CALIB_PREFIXES="${line##*|}"
  fi
fi
SUSPECT='sql|sqlite|postgres|mysql|mariadb|mongo|redis|cassandra|scylla|cockroach|dynamo|surreal|diesel|sea_?orm|tiberius|oracle|clickhouse|influx|neo4j|hyper|surf|curl|reqwest|isahc|ureq|http|grpc|tonic|websocket|tungstenite|smtp|lettre|imap|ftp|ssh|nats|kafka|rdkafka|pulsar|amqp|lapin|rabbit|mqtt|rumqtt|zmq|etcd|consul|elastic|meili|minio|^s3|aws|azure|gcp|google_?cloud'
is_calibrated() {
  local c
  for c in $CALIB_PREFIXES; do
    case "$1" in "$c"*) return 0;; esac
  done
  for c in $CALIBRATED; do
    # exact, or an adapter/extension of a known crate (tokio_postgres_rustls, sqlx_*…)
    [ "$1" = "$c" ] && return 0
    case "$1" in "${c}_"*) return 0;; esac
  done
  return 1
}
# Prefer GROUND TRUTH: the external crates candor actually saw resolved calls into
# (emitted per crate as `<prefix>.encountered-<crate>.json`). This reflects real usage
# and catches deps declared in workspace MEMBERS — which a root-Cargo.toml scan misses
# (e.g. git2 in gitui's asyncgit member). Fall back to the manifest when absent.
deps=""
if ls "$REPORT_PREFIX".encountered-*.json >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
  deps=$(python3 - "$REPORT_PREFIX" <<'PY' 2>/dev/null
import sys, glob, json
seen = set()
for f in glob.glob(sys.argv[1] + '.encountered-*.json'):
    try: seen |= set(json.load(open(f)))
    except Exception: pass
print(" ".join(sorted(seen)))
PY
)
fi
if [ -z "$deps" ]; then
  deps=$(awk '
    /^\[(dependencies|build-dependencies)\]/{f=1;next}
    /^\[/{f=0}
    f && /^[A-Za-z0-9_-]+[[:space:]]*[=.]/{ gsub(/[=.[:space:]].*/,"",$0); print }
  ' Cargo.toml 2>/dev/null | sort -u)
fi
gaps=""
for d in $deps; do
  nd="${d//-/_}"
  is_calibrated "$nd" && continue
  if printf '%s' "$nd" | grep -Eiq "$SUSPECT"; then
    gaps="$gaps $d"
  fi
done
gaps="$(echo "$gaps" | xargs 2>/dev/null)"

# ---- freshness label ----
if [ "$need_run" = 1 ] && [ "$ran_ok" = 0 ]; then
  FRESH="⚠ STALE — sources changed but the crate did not compile; map is from the last good build"
elif [ "$need_run" = 1 ]; then
  FRESH="fresh @$SHORT"
elif [ "$CUR" = "$PREV" ]; then
  FRESH="current @$SHORT (no Rust change since last run)"
else
  FRESH="⚠ stale @$SHORT — run /candor to refresh"
fi

# ---- coverage label ----
if [ -n "$gaps" ]; then
  COV="⚠ coverage: $(echo "$gaps" | tr ' ' ',') uncalibrated — Db/Net may be incomplete for code using them"
else
  COV="coverage ✓ (all effectful-looking deps recognized)"
fi

# ---- emit the single-line receipt ----
if [ "$need_run" = 1 ] && [ "$ran_ok" = 0 ]; then
  echo "candor$VERSTAMP $FRESH. Fix the build, then /candor.$NUDGE"
else
  echo "candor$VERSTAMP · ${FNS} fns · ${EFFS} · ${UNRES} unresolved · ${FRESH} · ${COV} · report: .candor/report.*.json$NUDGE"
fi
exit "${surfaced:-0}"
