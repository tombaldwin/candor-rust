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

# ---- locate the candor-query binary (used for the receipt AND the source-state hash) ----
# Newest of the candidates by mtime — the ONE locator primitive (a fresh build beats a stale one; NOT
# first-existing, which would pick a stale-toolchain dylib). Non-matching globs stay literal; `[ -e ]`
# skips them.
newest_of() {
  local newest="" c
  for c in "$@"; do
    [ -e "$c" ] || continue
    if [ -z "$newest" ] || [ "$c" -nt "$newest" ]; then newest="$c"; fi
  done
  [ -n "$newest" ] && printf '%s\n' "$newest"
}
find_query() {
  [ -n "${CANDOR_QUERY:-}" ] && [ -x "${CANDOR_QUERY}" ] && { printf '%s\n' "$CANDOR_QUERY"; return 0; }
  newest_of "${CANDOR_HOME:-}"/target/release/candor-query "${CANDOR_HOME:-}"/target/debug/candor-query \
            "${CANDOR_CACHE:-$HOME/.candor}"/bin/candor-query \
            "$DIR"/../candor/target/release/candor-query "$DIR"/../candor/target/debug/candor-query \
            /tmp/candor/target/release/candor-query /tmp/candor/target/debug/candor-query
}
QUERY="$(find_query 2>/dev/null || true)"

# ---- content hash of all Rust sources (path + content, target/.git excluded) ----
# ONE canonical implementation (`candor-query state`), shared with cargo-candor, so the hook and the
# wrapper can never disagree on whether a report is fresh. Empty if the binary isn't found yet, which
# the freshness gate below reads as "changed" (re-run) — the safe direction.
src_hash() { [ -n "$QUERY" ] && "$QUERY" state 2>/dev/null; }
CUR="$(src_hash)"; SHORT="${CUR:0:8}"
PREV=""; [ -f "$STATE" ] && PREV="$(cat "$STATE" 2>/dev/null)"

# ---- locate the candor dylib ----
# The nightly dylib (newest-mtime; an explicit CANDOR_LIB override wins). After a pinned-toolchain bump
# the previous `libcandor@<old>.dylib` lingers beside the new one and sorts first alphabetically; loading
# it runs a stale engine — newest_of avoids that.
find_lib() {
  [ -n "${CANDOR_LIB:-}" ] && [ -e "${CANDOR_LIB:-}" ] && { echo "$CANDOR_LIB"; return 0; }
  newest_of "${CANDOR_HOME:-}"/target/debug/libcandor@*.dylib "${CANDOR_HOME:-}"/target/debug/libcandor@*.so \
            "$DIR"/../candor/target/debug/libcandor@*.dylib "$DIR"/../candor/target/debug/libcandor@*.so \
            /tmp/candor/target/debug/libcandor@*.dylib /tmp/candor/target/debug/libcandor@*.so
}

# The STABLE backend (candor-scan) — the zero-install fallback. When no nightly dylib is present (fresh
# machine, stable-only CI, locked-down box), the receipt still works: candor-scan produces the same
# report JSON on stock `cargo`. An explicit CANDOR_SCAN override wins.
find_scan() {
  [ -n "${CANDOR_SCAN:-}" ] && [ -x "${CANDOR_SCAN}" ] && { printf '%s\n' "$CANDOR_SCAN"; return 0; }
  newest_of "${CANDOR_HOME:-}"/target/release/candor-scan "${CANDOR_HOME:-}"/target/debug/candor-scan \
            "${CANDOR_CACHE:-$HOME/.candor}"/bin/candor-scan \
            "$DIR"/../candor/target/release/candor-scan "$DIR"/../candor/target/debug/candor-scan \
            /tmp/candor/target/release/candor-scan /tmp/candor/target/debug/candor-scan
}

# (candor-query located above, as QUERY — used both for the source-state hash and the receipt below.)

# ---- version stamp + update/rebuild nudges (the clone is the single source of truth) ----
LIBP="$(find_lib 2>/dev/null || true)"
SCANP="$(find_scan 2>/dev/null || true)"
VER=""; NUDGE=""; BACKEND=""
# The TRUE version of the running dylib — the tag build.rs embedded — NOT CANDOR_HOME's git HEAD,
# which races ahead of an un-rebuilt dylib. The receipt must not claim a version the loaded binary
# isn't; this is the engine that actually produced the report below.
[ -n "$LIBP" ] && [ -n "$QUERY" ] && VER="$("$QUERY" engine-version "$LIBP" 2>/dev/null)"
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
# Report is current iff it exists AND the source hash hasn't moved since we wrote it. Use candor-query's
# `reports --exists` (the EXACT `report_files` two-segment rule) so a 3-segment sidecar
# (`.callgraph.json` / `.layerreach.json`) is never miscounted as a report — the coarse `ls *.*.json`
# glob matches those (`*` matches dots) and the receipt would then claim a non-existent map is present.
# Fall back to the glob only when the query binary isn't located yet.
report_exists() {
  if [ -n "$QUERY" ]; then "$QUERY" reports "$REPORT_PREFIX" --exists; else ls "$REPORT_PREFIX".*.*.json >/dev/null 2>&1; fi
}
# Which backend produced the report — `candor-query reports --backend` (the single owner, shared with
# the wrapper). Falls back to the filename glob only when candor-query isn't located yet.
is_scan_report() {
  if [ -n "$QUERY" ]; then [ "$("$QUERY" reports "$REPORT_PREFIX" --backend 2>/dev/null)" = scan ]
  else ls "$REPORT_PREFIX".*.scan.json >/dev/null 2>&1; fi
}
need_run=$FORCE
[ "$CUR" != "$PREV" ] && need_run=1
report_exists || need_run=1
# Backend transition: if a cached report was produced by the stable scanner but a nightly dylib is NOW
# available, regenerate with the lint — otherwise the receipt would attribute the syntactic scan report
# to the nightly engine (and never self-correct until the source changes).
[ -n "$LIBP" ] && is_scan_report && need_run=1

# Touch every workspace member's crate root (mtime only) to force `cargo dylint` to
# recompile — dylint emits the report ONLY on recompilation, so an already-built
# project would otherwise produce nothing.
touch_roots() {
  # Touch every workspace member's crate root so dylint recompiles. Prefer `cargo metadata` — it lists
  # ALL members, including OUT-OF-TREE ones (`members = ["../sibling"]`) a `$DIR` tree scan would miss;
  # parse manifest_path with grep/sed (no python3). Fall back to a tree scan if cargo metadata fails.
  local mf d f roots
  roots="$(cargo metadata --no-deps --format-version 1 2>/dev/null \
           | grep -o '"manifest_path":"[^"]*"' | sed 's/.*:"//; s/"$//')"
  [ -n "$roots" ] || roots="$(find "$DIR" -name Cargo.toml -not -path '*/target/*' 2>/dev/null)"
  printf '%s\n' "$roots" | while IFS= read -r mf; do
    [ -n "$mf" ] || continue
    d="$(dirname "$mf")"
    for f in "$d/src/lib.rs" "$d/src/main.rs"; do [ -f "$f" ] && touch "$f"; done
  done
  for r in src/lib.rs src/main.rs; do [ -f "$DIR/$r" ] && touch "$DIR/$r"; done
}
# Remove the OTHER backend's report before (re)generating, so lint and scan reports never coexist under
# one prefix (which would double-count fns and mix backends). `candor-query reports --clear-other <keep>`
# clears only the non-kept backend's files (exact report_files semantics, incl. sidecars) — never the
# one we're about to write — so a build failure still keeps the same-backend last-good report.
clear_scan_reports() { [ -n "$QUERY" ] && "$QUERY" reports "$REPORT_PREFIX" --clear-other lint >/dev/null 2>&1 || true; }
clear_lint_reports() { [ -n "$QUERY" ] && "$QUERY" reports "$REPORT_PREFIX" --clear-other scan >/dev/null 2>&1 || true; }
run_lint() { clear_scan_reports; CANDOR_JSON="$REPORT_PREFIX" cargo dylint --lib-path "$LIB" >/dev/null 2>"$STATE_DIR/last-error.log"; }
# The stable backend: produce the report with no nightly toolchain. candor-scan parses sources (it does
# NOT build the crate), so it can't fail on a compile error — but it under-reports vs the lint (no
# Unknown; misses some method/macro/cross-crate effects). Marked clearly so the receipt stays honest.
run_scan() { clear_lint_reports; "$SCAN" "$DIR" --out "$REPORT_PREFIX" >/dev/null 2>"$STATE_DIR/last-error.log"; }

ran_ok=1; surfaced=0
if [ "$need_run" = 1 ]; then
  surfaced=10
  MARK="$STATE_DIR/.mark"; : > "$MARK"
  emitted() { [ -n "$(find "$STATE_DIR" -name 'report.*.*.json' -newer "$MARK" 2>/dev/null)" ]; }
  if [ -n "$LIBP" ]; then
    # Preferred: the nightly lint (soundness contract, Unknown, conformance).
    LIB="$LIBP"
    if run_lint && emitted; then
      echo "$CUR" > "$STATE"; ran_ok=1
    else
      # dylint produced no fresh report — either the crate didn't recompile (already built) or it failed
      # to compile. Force a recompile and retry once.
      touch_roots
      : > "$MARK"
      if run_lint && emitted; then
        echo "$CUR" > "$STATE"; ran_ok=1
      else
        ran_ok=0   # genuine build error: keep last good report (if any), flag STALE
      fi
    fi
  elif [ -n "$SCANP" ]; then
    # Zero-install fallback: the stable scanner. No nightly, no dylint — works anywhere `cargo` does.
    SCAN="$SCANP"
    BACKEND=" · stable backend (syntactic — install the nightly lint for Unknown/soundness)"
    # Stamp the report as `@scan` UNCONDITIONALLY (override any clone-HEAD VER set above): the report was
    # produced by the syntactic scanner, not the nightly lint at that commit, so the receipt must say so.
    VER="scan"; VERSTAMP=" @scan"
    if run_scan && emitted; then
      echo "$CUR" > "$STATE"; ran_ok=1
    else
      ran_ok=0
    fi
  else
    echo "candor$VERSTAMP ⚠ not installed — no candor backend found. Build candor (cargo build in the clone, or 'cargo candor setup') or set CANDOR_HOME in .candor/config.$NUDGE"
    exit 10
  fi
fi
# If we're SERVING a cached scan report (no rerun this turn, no dylib), keep the receipt honest: mark it
# the stable backend and stamp `@scan`, regardless of any clone-HEAD VER computed above.
if [ -z "$LIBP" ] && is_scan_report; then
  [ -z "$BACKEND" ] && BACKEND=" · stable backend (syntactic — install the nightly lint for Unknown/soundness)"
  VER="scan"; VERSTAMP=" @scan"
fi

# ---- aggregate the report (candor-query preferred; degrade gracefully without it) ----
# One `candor-query receipt` yields the fn count, effect breakdown, unresolved count, the calibrated
# set (the engine's <prefix>.calibrated.json sidecar — the single source of truth) and the encountered
# crates — replacing several inline Python heredocs with the typed query binary. The hardcoded
# The calibrated set is the engine's, read from the report's `.calibrated.json` sidecar (via the
# receipt's `calibrated` line). We deliberately do NOT hardcode a copy here — it drifted out of sync
# with the engine's real set (a stale list mislabels now-calibrated crates as coverage gaps). Until the
# sidecar is available (before the first report), the coverage gap check is simply deferred.
CALIBRATED=""
CALIB_PREFIXES=""
FNS=""; EFFS=""; UNRES=""; ENCOUNTERED=""; calib=""
RECEIPT=""
[ -n "$QUERY" ] && RECEIPT="$("$QUERY" receipt "$REPORT_PREFIX" 2>/dev/null)"
if [ -n "$RECEIPT" ]; then
  # Parse the five key<TAB>value lines in ONE pass. A real fn-count receipt always prints all five
  # keys, so an EMPTY $RECEIPT means candor-query is missing OR failed (e.g. a stale binary lacking the
  # `receipt` subcommand) — handled by the else branch, NOT a silent "0 fns".
  while IFS=$'\t' read -r k v; do
    case "$k" in
      fns)         FNS="$v" ;;
      effects)     EFFS="$v" ;;
      unresolved)  UNRES="$v" ;;
      encountered) ENCOUNTERED="$v" ;;
      calibrated)  calib="$v" ;;
    esac
  done <<< "$RECEIPT"
  # The calibrated set comes from the engine's sidecar; flag when we actually got it (the coverage gap
  # check below only runs then, so it can never disagree with the engine's real calibration).
  [ -n "$calib" ] && [ "$calib" != "|" ] && { CALIBRATED="${calib%%|*}"; CALIB_PREFIXES="${calib##*|}"; CALIB_SIDECAR=1; }
else
  # candor-query missing OR failed (stale binary, etc.): honest crude fn-count rather than a false
  # "0 fns". Build/refresh it via install.sh or `cargo candor setup` for the full breakdown.
  FNS=$(grep -ho '"fn"' "$REPORT_PREFIX".*.*.json 2>/dev/null | wc -l | tr -d ' ')
  EFFS="(effect breakdown unavailable — build/refresh candor-query)"
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
if [ -n "$SUSPECT" ] && [ "${CALIB_SIDECAR:-0}" = 1 ]; then
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
if [ "${CALIB_SIDECAR:-0}" != 1 ] && [ -z "$LIBP" ] && is_scan_report; then
  # The stable backend writes no calibrated sidecar, so the gap check can't run — say so plainly rather
  # than "deferred (arrives with the first report)", which never comes true on a scan-only machine.
  COV="coverage: n/a on the stable backend (install the nightly lint for blind-spot detection)"
elif [ "${CALIB_SIDECAR:-0}" != 1 ]; then
  COV="coverage: deferred (calibrated set arrives with the first report)"
elif [ -n "$gaps" ]; then
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
# Skip the review when the CURRENT report is a SCAN report (`! is_scan_report`): the baseline is
# lint-produced and carries `Unknown`, so diffing a syntactic scan report against it would false-fire
# (scan-only artifacts read as "gained") or silently defeat the review (a real effect scan can't see
# makes the report show FEWER effects). The self-review needs the lint's soundness on both sides.
if [ "${CANDOR_REVIEW:-0}" != 0 ] && [ "$ran_ok" = 1 ] && ! is_scan_report \
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
  printf 'candor — functions have effects NOT in the committed .candor baseline (surfaced once):\n%s\n\nA local edit can change the effect surface non-locally — a new effect propagates to every caller. For each: was it intended? If a function gained Net/Db/Exec/Fs/Env/Ipc it lacked, confirm it is necessary, and prefer threading a capability over reaching for ambient authority. A gained `Unknown` means candor can no longer prove that function complete — read it. If all are intended, just finish; this will not re-prompt for these.\n' "$REVIEW"
  exit 11
fi
echo "candor$VERSTAMP · ${FNS} fns · ${EFFS} · ${UNRES} unresolved · ${FRESH} · ${COV} · report: .candor/report.*.json$BACKEND$NUDGE"
exit "${surfaced:-0}"
