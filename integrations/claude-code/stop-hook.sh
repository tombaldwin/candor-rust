#!/usr/bin/env bash
# stop-hook.sh — Claude Code Stop hook for candor.
#
# Fires when Claude finishes a turn. Runs candor-run.sh, then:
#   exit 11  (CANDOR_REVIEW, opt-in): the agent's edits introduced a NEW effect. Feed the
#            self-review prompt BACK TO THE AGENT (decision:block + additionalContext) so it
#            reviews — UNLESS we're already in a stop→continue loop (stop_hook_active true), in
#            which case we only tell the human. `.candor/review-seen` already makes each effect
#            prompt at most once; stop_hook_active and Claude's 8-block cap are the backstops.
#   exit 10  a normal re-analysis receipt → surface to the HUMAN (systemMessage); never blocks.
#   else     silent.
set -uo pipefail

IN="$(cat)"   # Stop-hook input JSON on stdin

# Pull a field from the input JSON (python3 preferred; sed fallback for `cwd`).
field() { command -v python3 >/dev/null 2>&1 && printf '%s' "$IN" \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print($1)" 2>/dev/null; }
DIR="$(field 'd.get("cwd","")')"
[ -z "$DIR" ] && DIR="$(printf '%s' "$IN" | sed -n 's/.*"cwd"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
[ -z "$DIR" ] && DIR="${CLAUDE_PROJECT_DIR:-$PWD}"
ACTIVE="$(field 'str(d.get("stop_hook_active", False)).lower()')"

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$("$SELF_DIR/candor-run.sh" "$DIR" 2>/dev/null)"; CODE=$?

emit_human() {   # surface candor's output to the human only (never reaches the model)
  # A plain receipt (exit 10) is one line; a §2 self-review (exit 11 in the loop-guard branch) is
  # MULTI-line — the gained-effect detail then the advice. Send the WHOLE message (JSON escapes the
  # newlines) so the loop-guard human still sees WHICH function/effect changed, not just the last line.
  if command -v python3 >/dev/null 2>&1; then
    printf '%s' "$OUT" | python3 -c 'import sys,json
m=sys.stdin.read().rstrip("\n")
if m: print(json.dumps({"systemMessage": m, "suppressOutput": True}))'
  else
    local msg; msg="$(printf '%s' "$OUT" | tail -1 | sed 's/\\/\\\\/g; s/"/\\"/g')"
    [ -n "$msg" ] && printf '{"systemMessage": "%s", "suppressOutput": true}\n' "$msg"
  fi
}

if [ "$CODE" = 11 ] && [ "$ACTIVE" != true ] && command -v python3 >/dev/null 2>&1 && [ -n "$OUT" ]; then
  # Feed the agent: `decision:block` continues the turn; `additionalContext` is what Claude acts on.
  # (`reason` is human-facing; we send the same text both ways so the signal lands regardless.)
  printf '%s' "$OUT" | python3 -c '
import sys, json
p = sys.stdin.read().strip()
print(json.dumps({
  "decision": "block",
  "reason": p,
  "hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": p},
}))'
elif [ "$CODE" = 11 ] || [ "$CODE" = 10 ]; then
  emit_human   # already looping (stop_hook_active), or a plain receipt → human only
fi
exit 0
