#!/usr/bin/env bash
# install.sh — install the candor Claude Code integration into a Rust project.
#
#   ./install.sh [TARGET_PROJECT_DIR]   (defaults to cwd)
#
# Installs:
#   <target>/.claude/candor/candor-run.sh   (+ stop-hook.sh)   the deterministic core
#   <target>/.claude/commands/candor.md      the /candor slash command
#   <target>/.claude/settings.json           merges in the Stop hook (auto-refresh)
#   <target>/.candor/config                  pins the resolved CANDOR_LIB path
#
# Idempotent: re-running updates the scripts and re-merges the hook.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${1:-$PWD}"
TARGET="$(cd "$TARGET" && pwd)"
[ -f "$TARGET/Cargo.toml" ] || { echo "error: $TARGET is not a Rust project (no Cargo.toml)"; exit 1; }

echo "Installing candor Claude Code integration into: $TARGET"
mkdir -p "$TARGET/.claude/candor" "$TARGET/.claude/commands" "$TARGET/.candor"
cp "$SRC/candor-run.sh" "$SRC/stop-hook.sh" "$TARGET/.claude/candor/"
cp "$SRC/commands/candor.md" "$TARGET/.claude/commands/"
chmod +x "$TARGET/.claude/candor/candor-run.sh" "$TARGET/.claude/candor/stop-hook.sh"

# Resolve and pin the dylib so the hook is reliable across machines/cwd.
LIB=""
for c in "${CANDOR_LIB:-}" \
         "$TARGET"/../candor/target/debug/libcandor@*.dylib "$TARGET"/../candor/target/debug/libcandor@*.so \
         /tmp/candor/target/debug/libcandor@*.dylib /tmp/candor/target/debug/libcandor@*.so; do
  [ -n "$c" ] && [ -e "$c" ] && { LIB="$c"; break; }
done
if [ -n "$LIB" ]; then
  printf 'CANDOR_LIB=%q\n' "$LIB" > "$TARGET/.candor/config"
  echo "  pinned CANDOR_LIB=$LIB"
else
  echo "  note: no candor dylib found yet. Build candor (see its README) then either set"
  echo "        CANDOR_LIB in $TARGET/.candor/config or place the clone at /tmp/candor."
fi

# Merge the Stop hook into .claude/settings.json (python3 preferred for safe JSON merge).
SETTINGS="$TARGET/.claude/settings.json"
HOOK_CMD='${CLAUDE_PROJECT_DIR}/.claude/candor/stop-hook.sh'
if command -v python3 >/dev/null 2>&1; then
  python3 - "$SETTINGS" "$HOOK_CMD" <<'PY'
import json, os, sys
path, cmd = sys.argv[1], sys.argv[2]
data = {}
if os.path.exists(path):
    try: data = json.load(open(path))
    except Exception: data = {}
hooks = data.setdefault("hooks", {})
stop = hooks.setdefault("Stop", [])
def has(cmd):
    for grp in stop:
        for h in grp.get("hooks", []):
            if h.get("command") == cmd: return True
    return False
if not has(cmd):
    stop.append({"matcher": "*", "hooks": [{"type": "command", "command": cmd}]})
json.dump(data, open(path, "w"), indent=2)
open(path, "a").write("\n")
print("  merged Stop hook into", path)
PY
else
  echo "  python3 not found — add this to $SETTINGS manually:"
  echo '    {"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"'"$HOOK_CMD"'"}]}]}}'
fi

# Ignore generated artifacts (keep the repo clean; baseline is opt-in to commit).
GI="$TARGET/.gitignore"
grep -qxF '.candor/report.*.json' "$GI" 2>/dev/null || {
  { echo '.candor/report.*.json'; echo '.candor/state'; echo '.candor/last-error.log'; } >> "$GI"
}

echo "Done. Try it: run '/candor' in Claude Code, or just edit a .rs file and finish a turn."
