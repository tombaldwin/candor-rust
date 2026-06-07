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

# ---- locate the candor-query binary (the receipt's report aggregation; was inline Python) ----
find_query() {
  local c
  for c in "${CANDOR_QUERY:-}" \
           "${CANDOR_HOME:-}"/target/release/candor-query "${CANDOR_HOME:-}"/target/debug/candor-query \
           "${CANDOR_CACHE:-$HOME/.candor}"/bin/candor-query \
           "$DIR"/../candor/target/release/candor-query "$DIR"/../candor/target/debug/candor-query \
           /tmp/candor/target/release/candor-query /tmp/candor/target/debug/candor-query; do
    [ -n "$c" ] && [ -x "$c" ] && { echo "$c"; return 0; }
  done
  return 1
}
QUERY="$(find_query 2>/dev/null || true)"

# ---- version stamp + update/rebuild nudges (the clone is the single source of truth) ----
LIBP="$(find_lib 2>/dev/null || true)"
VER=""; NUDGE=""
# The TRUE version of the running dylib — the tag build.rs embedded — NOT CANDOR_HOME's git HEAD,
# which races ahead of an un-rebuilt dylib. The receipt must not claim a version the loaded binary
# isn't; this is the engine that actually produced the report below.
[ -n "$LIBP" ] && VER="$(strings -a "$LIBP" 2>/dev/null | grep -oE 'candor-build-version=[0-9a-fA-F]+' | head -1 | cut -d= -f2)"
if [ -n "${CANDOR_HOME:-}" ] && git -C "$CANDOR_HOME" rev-parse --git-dir >/dev/null 2>&1; then
  head="$(git -C "$CANDOR_HOME" rev-parse --short HEAD 2>/dev/null)"
  [ -z "$VER" ] && VER="$head"   # fall back to clone HEAD only if the tag is unreadable
  # Running binary lags the clone (e.g. pulled but didn't rebuild) → exact version comparison,
  # which beats the old mtime heuristic; fall back to mtime for *uncommitted* source edits.
  if [ -n "$VER" ] && [ -n "$head" ] && [ "$VER" != "$head" ]; then
    NUDGE="$NUDGE · ⚠ engine @$VER but clone @$head — rebuild: cargo candor update"
  elif [ -n "$LIBP" ] && [ -f "$CANDOR_HOME/src/lib.rs" ] && [ "$LIBP" -ot "$CANDOR_HOME/src/lib.rs" ]; then
    NUDGE="$NUDGE · ⚠ dylib older than source — rebuild: cargo candor update"
  fi
  # Upstream check hits the network, so only the explicit /candor (--force) does it;
  # the per-turn Stop hook stays offline and fast.
  if [ "$FORCE" = 1 ]; then
    git -C "$CANDOR_HOME" fetch -q origin 2>/dev/null || true
    up="$(git -C "$CANDOR_HOME" rev-parse '@{u}' 2>/dev/null || git -C "$CANDOR_HOME" rev-parse origin/main 2>/dev/null || true)"
    fullhead="$(git -C "$CANDOR_HOME" rev-parse HEAD 2>/dev/null || true)"
    [ -n "$up" ] && [ -n "$fullhead" ] && [ "$up" != "$fullhead" ] && NUDGE="$NUDGE · ⚠ candor update available — run: cargo candor update"
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
  # Touch every workspace member's crate root: find each Cargo.toml (target/ excluded) and touch its
  # src/lib.rs / src/main.rs. Plain bash — no `cargo metadata` / python3.
  local mf d f
  find "$DIR" -name Cargo.toml -not -path '*/target/*' 2>/dev/null | while IFS= read -r mf; do
    d="$(dirname "$mf")"
    for f in "$d/src/lib.rs" "$d/src/main.rs"; do [ -f "$f" ] && touch "$f"; done
  done
  for r in src/lib.rs src/main.rs; do [ -f "$DIR/$r" ] && touch "$DIR/$r"; done
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

# ---- aggregate the report (candor-query preferred; degrade gracefully without it) ----
# One `candor-query receipt` yields the fn count, effect breakdown, unresolved count, the calibrated
# set (the engine's <prefix>.calibrated.json sidecar — the single source of truth) and the encountered
# crates — replacing several inline Python heredocs with the typed query binary. The hardcoded
# CALIBRATED list is only a fallback for before a sidecar exists (report not yet generated).
CALIBRATED="reqwest isahc ureq sqlx rusqlite postgres tokio_postgres diesel redis mongodb mysql mysql_async sea_orm deadpool_postgres memmap2 rand getrandom fastrand chrono tracing arboard portable_pty"
CALIB_PREFIXES="aws_sdk_ aws_smithy cap_"
FNS=0; EFFS="no effects detected"; UNRES="?"; ENCOUNTERED=""
if [ -n "$QUERY" ]; then
  RECEIPT="$("$QUERY" receipt "$REPORT_PREFIX" 2>/dev/null)"
  if [ -n "$RECEIPT" ]; then
    FNS="$(printf '%s\n' "$RECEIPT" | awk -F'\t' '$1=="fns"{print $2; exit}')"
    EFFS="$(printf '%s\n' "$RECEIPT" | awk -F'\t' '$1=="effects"{print $2; exit}')"
    UNRES="$(printf '%s\n' "$RECEIPT" | awk -F'\t' '$1=="unresolved"{print $2; exit}')"
    ENCOUNTERED="$(printf '%s\n' "$RECEIPT" | awk -F'\t' '$1=="encountered"{print $2; exit}')"
    calib="$(printf '%s\n' "$RECEIPT" | awk -F'\t' '$1=="calibrated"{print $2; exit}')"
    # Override the fallback only when the engine emitted a (non-empty) calibrated sidecar.
    if [ -n "$calib" ] && [ "$calib" != "|" ]; then
      CALIBRATED="${calib%%|*}"; CALIB_PREFIXES="${calib##*|}"
    fi
  fi
else
  # No candor-query (build it via install.sh): crude fn count, no breakdown.
  FNS=$(grep -ho '"fn"' "$REPORT_PREFIX".*.*.json 2>/dev/null | wc -l | tr -d ' ')
  EFFS="(build candor-query for the effect breakdown)"
fi
[ -z "$FNS" ] && FNS=0
[ -z "$EFFS" ] && EFFS="no effects detected"
[ -z "$UNRES" ] && UNRES="?"
# Coverage heuristic: crates that LOOK effectful but aren't calibrated. Single source of truth —
# `candor-suspect` in the clone (also read by cargo-candor). Empty if not found → the nudge is skipped.
_self="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd)"
SUSPECT="$(cat "${CANDOR_HOME:-/dev/null}/candor-suspect" "${_self:-/dev/null}/../../candor-suspect" 2>/dev/null | head -1)"
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
# Prefer GROUND TRUTH: crates candor actually saw resolved calls into (the `encountered` set from the
# receipt above). Catches deps declared in workspace MEMBERS that a root-Cargo.toml scan misses (e.g.
# git2 in gitui's asyncgit member). Fall back to the manifest when the encountered set is empty.
deps="$ENCOUNTERED"
if [ -z "$deps" ]; then
  deps=$(awk '
    /^\[(dependencies|build-dependencies)\]/{f=1;next}
    /^\[/{f=0}
    f && /^[A-Za-z0-9_-]+[[:space:]]*[=.]/{ gsub(/[=.[:space:]].*/,"",$0); print }
  ' Cargo.toml 2>/dev/null | sort -u)
fi
gaps=""
if [ -n "$SUSPECT" ]; then
  for d in $deps; do
    nd="${d//-/_}"
    is_calibrated "$nd" && continue
    if printf '%s' "$nd" | grep -Eiq "$SUSPECT"; then
      gaps="$gaps $d"
    fi
  done
fi
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

# ---- §2 edit-time self-review (opt-in: CANDOR_REVIEW=1 in .candor/config or env) ----
# When enabled, diff the fresh report against the committed baseline; if THIS turn introduced an
# effect not surfaced before, emit a self-review PROMPT (exit 11) that the Stop hook feeds back to
# the agent. Conservative: needs CANDOR_REVIEW, a good current report (ran_ok), a baseline, a
# matching engine version (else the delta is reclassification noise, not the agent's edit), and
# candor-query. `.candor/review-seen` makes each gained effect prompt at most once. Default OFF.
REVIEW=""
BASELINE_PREFIX="$STATE_DIR/baseline"
if [ "${CANDOR_REVIEW:-0}" != 0 ] && [ "$ran_ok" = 1 ] \
   && ls "$BASELINE_PREFIX".*.*.json >/dev/null 2>&1 && [ -n "$QUERY" ]; then
  bver=""; [ -f "$BASELINE_PREFIX.candor-version" ] && bver="$(cat "$BASELINE_PREFIX.candor-version" 2>/dev/null)"
  if [ -z "$bver" ] || [ -z "$VER" ] || [ "$bver" = "$VER" ]; then
    seen="$STATE_DIR/review-seen"; [ -f "$seen" ] || : > "$seen"
    # candor-query emits every `<fn>\t<effect>` gained vs the baseline; bash owns the seen-file so the
    # query stays read-only. fresh = gains not seen before; then record ALL gains (each prompts once).
    allg="$("$QUERY" gains "$REPORT_PREFIX" "$BASELINE_PREFIX" 2>/dev/null)"
    if [ -n "$allg" ]; then
      fresh="$(printf '%s\n' "$allg" | grep -vxF -f "$seen" 2>/dev/null)"
      printf '%s\n' "$allg" > "$seen"
      [ -n "$fresh" ] && REVIEW="$(printf '%s\n' "$fresh" | awk -F'\t' '
        $1 != p && p != "" { emit() }
        { e = e " " $2; if ($2 == "Unknown") u = 1; p = $1 }
        END { if (p != "") emit() }
        function emit() {
          printf "  + %s  gained { %s }%s\n", p, substr(e, 2),
                 (u ? "  (Unknown — effect set no longer provably complete)" : "")
          e = ""; u = 0
        }')"
    fi
  fi
fi

# ---- emit ----
if [ "$need_run" = 1 ] && [ "$ran_ok" = 0 ]; then
  echo "candor$VERSTAMP $FRESH. Fix the build, then /candor.$NUDGE"
  exit "${surfaced:-0}"
fi
if [ -n "$REVIEW" ]; then
  printf 'candor — your edits this turn gave functions NEW effects (vs the committed .candor baseline):\n%s\n\nA local edit can change the effect surface non-locally — a new effect propagates to every caller. For each: was it intended? If a function gained Net/Db/Exec/Fs/Env/Ipc it lacked, confirm it is necessary, and prefer threading a capability over reaching for ambient authority. A gained `Unknown` means candor can no longer prove that function complete — read it. If all are intended, just finish; this will not re-prompt for these.\n' "$REVIEW"
  exit 11
fi
echo "candor$VERSTAMP · ${FNS} fns · ${EFFS} · ${UNRES} unresolved · ${FRESH} · ${COV} · report: .candor/report.*.json$NUDGE"
exit "${surfaced:-0}"
