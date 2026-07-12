#!/usr/bin/env bash
# Automated test for the shell completions (candor.bash + the zsh _candor). Hermetic: a temp `.candor/`
# report fixture, no engine, no network. Asserts the static surface (subcommands, flags), the dynamic
# surface (function names + the effect vocabulary from the discovered report), and the SMART part — a
# `path <fn> <TAB>` effect slot offers only THAT function's effects. Run: bash completions/test.sh
set -u
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "$HERE/candor.bash"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
mkdir -p "$T/proj/.candor"
cat > "$T/proj/.candor/report.test.scan.json" <<'JSON'
{"functions":[{"fn":"settings::Settings::needs_update","inferred":["Clock","Db","Net","Unknown"]},{"fn":"api::get_history","inferred":["Env","Net","Unknown"]}]}
JSON
cd "$T/proj" || exit 2

fails=0
comp() { local base="$1"; shift; COMP_WORDS=("$base" "$@"); COMP_CWORD=$(( ${#COMP_WORDS[@]} - 1 )); COMPREPLY=(); _candor; REPLY="${COMPREPLY[*]:-}"; }
has()   { if [[ " $3 " == *" $2 "* ]]; then echo "  ok   $1"; else echo "  FAIL $1 — want '$2' in: [$3]"; fails=$((fails+1)); fi; }
hasnt() { if [[ " $3 " != *" $2 "* ]]; then echo "  ok   $1"; else echo "  FAIL $1 — '$2' should NOT be in: [$3]"; fails=$((fails+1)); fi; }

echo "bash completion:"
comp candor '';                                       has "candor → subcommands (where)"     "where"    "$REPLY"
comp candor '';                                       has "candor → subcommands (path)"      "path"     "$REPLY"
comp candor-java '';                                  has "candor-java → its subs (containment)" "containment" "$REPLY"
comp candor-swift '';                                 has "candor-swift → fix-gate only"     "fix-gate" "$REPLY"
comp candor where '';                                 has "where → effect vocab (Net)"       "Net"      "$REPLY"
comp candor path '';                                  has "path → discovered fn names"       "settings::Settings::needs_update" "$REPLY"
comp candor path settings::Settings::needs_update ''; has "path <fn> → that fn's effects (Db)" "Db"     "$REPLY"
comp candor path settings::Settings::needs_update ''; hasnt "context-aware: excludes Fs (not this fn's)" "Fs" "$REPLY"
comp candor path api::get_history '';                 has "path <fn> → the OTHER fn's effects (Env)" "Env" "$REPLY"
comp candor path api::get_history '';                 hasnt "context-aware: excludes Db (not this fn's)" "Db" "$REPLY"
comp candor where --json '';                          has "flag-skip: --json before effect"  "Net"      "$REPLY"
comp candor '--ver';                                  has "top-level flag: --version"        "--version" "$REPLY"
comp candor where '--re';                             has "mid-query flag: --report"         "--report" "$REPLY"

echo "zsh completion:"
if command -v zsh >/dev/null 2>&1; then
  if zsh -n "$HERE/_candor" 2>/dev/null; then echo "  ok   zsh _candor parses"; else echo "  FAIL zsh _candor syntax error"; fails=$((fails+1)); fi
  # the dynamic helpers produce the same report-derived data
  effs="$(cd "$T/proj" && zsh -c "source =(sed '1d;\$d' '$HERE/_candor'); _candor_fns settings::Settings::needs_update" 2>/dev/null | tr '\n' ' ')"
  has "zsh helper: fn effects (Net)" "Net" "$effs"
else
  echo "  ok   (zsh not installed — skipped)"
fi

echo
if [ "$fails" -eq 0 ]; then echo "completions: OK"; else echo "completions: $fails FAILED"; exit 1; fi
