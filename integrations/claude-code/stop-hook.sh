#!/usr/bin/env bash
# stop-hook.sh — Claude Code Stop hook for candor.
#
# Fires when Claude finishes a turn. Runs candor-run.sh; if (and only if) the run
# re-analyzed because Rust sources changed — or produced a STALE warning — it
# surfaces the receipt to the HUMAN via the `systemMessage` JSON field. On turns
# with no Rust change it stays silent. It never blocks and never forces a continue,
# so there is no risk of a Stop-hook loop.
set -uo pipefail

IN="$(cat)"   # Stop-hook input JSON on stdin

# Extract cwd from the input JSON (python3 if available, else a flat-field sed).
if command -v python3 >/dev/null 2>&1; then
  DIR="$(printf '%s' "$IN" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("cwd",""))' 2>/dev/null)"
else
  DIR="$(printf '%s' "$IN" | sed -n 's/.*"cwd"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
fi
[ -z "$DIR" ] && DIR="${CLAUDE_PROJECT_DIR:-$PWD}"

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$("$SELF_DIR/candor-run.sh" "$DIR" 2>/dev/null)"; CODE=$?

# Surface only when candor-run asked us to (exit 10). Quiet otherwise.
if [ "$CODE" = 10 ] && [ -n "$OUT" ]; then
  MSG="$(printf '%s' "$OUT" | tail -1 | sed 's/\\/\\\\/g; s/"/\\"/g')"
  printf '{"systemMessage": "%s", "suppressOutput": true}\n' "$MSG"
fi
exit 0
