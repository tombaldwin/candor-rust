# bash completion for candor (the `candor` command — a symlink to cargo-candor; also `cargo-candor`).
# Static: subcommands + flags. Dynamic: the effect vocabulary, and the FUNCTION NAMES candor found in the
# scanned report (.candor/report*.scan.json, or $CANDOR_REPORT) — so `candor path <TAB>` tabs through the
# functions candor knows about instead of you retyping them. Source this file, or let install.sh wire it in.
#   source /path/to/candor/completions/candor.bash

# List the function names from the discovered report(s). Fast, best-effort, silent on any miss —
# completion must never error or hang the shell.
_candor_fns() {
  local files f
  files=$( { ls .candor/report*.scan.json 2>/dev/null; ls "${CANDOR_REPORT}"*.scan.json 2>/dev/null; } | grep -v callgraph )
  [ -z "$files" ] && return 0
  for f in $files; do
    if command -v python3 >/dev/null 2>&1; then
      python3 -c 'import json,sys
try:
    for e in json.load(open(sys.argv[1])).get("functions",[]):
        n=e.get("fn")
        if n: print(n)
except Exception: pass' "$f" 2>/dev/null
    elif command -v jq >/dev/null 2>&1; then
      jq -r '.functions[].fn // empty' "$f" 2>/dev/null
    else
      grep -oE '"fn":"[^"]+"' "$f" 2>/dev/null | sed 's/^"fn":"//; s/"$//'
    fi
  done | sort -u
}

_candor() {
  local cur prev cword
  cur="${COMP_WORDS[COMP_CWORD]}"
  prev="${COMP_WORDS[COMP_CWORD-1]}"
  cword=$COMP_CWORD

  local subs="setup scan audit snapshot guard diff watch show where callers whatif fix fix-gate unverified rewire map containment reachable path impact explain policy risk strict no-ambient update help"
  local effects="Net Fs Db Exec Env Clock Rand Ipc Clipboard Log Unknown"
  local flags="--json --policy --gate-json --strict --out --link -h --help"

  # position 1 → the subcommand
  if [ "$cword" -eq 1 ]; then
    COMPREPLY=( $(compgen -W "$subs" -- "$cur") )
    return 0
  fi

  # a flag anywhere
  if [[ "$cur" == -* ]]; then
    COMPREPLY=( $(compgen -W "$flags" -- "$cur") )
    return 0
  fi
  # a value expected after certain flags
  case "$prev" in
    --policy|--gate-json|--out) COMPREPLY=( $(compgen -f -- "$cur") ); return 0 ;;
  esac

  local sub="${COMP_WORDS[1]}"
  case "$sub" in
    where)
      COMPREPLY=( $(compgen -W "$effects" -- "$cur") ) ;;
    path|whatif|fix|fix-gate)
      # <sub> <fn> <Effect>: function on the first arg, effect on the second
      if [ "$cword" -ge 3 ]; then
        COMPREPLY=( $(compgen -W "$effects" -- "$cur") )
      else
        COMPREPLY=( $(compgen -W "$(_candor_fns)" -- "$cur") )
      fi ;;
    show|callers|impact|reachable|containment|unverified)
      COMPREPLY=( $(compgen -W "$(_candor_fns)" -- "$cur") ) ;;
    scan|audit|guard|diff|snapshot|watch)
      COMPREPLY=( $(compgen -d -- "$cur") ) ;;   # a directory to scan
  esac

  # function names carry `::` — bash treats `:` as a word break, so trim the already-typed prefix
  # (the standard bash-completion trick) or completions insert a duplicated `mod::mod::fn`.
  if type __ltrim_colon_completions &>/dev/null; then
    __ltrim_colon_completions "$cur"
  fi
}

complete -F _candor candor cargo-candor
