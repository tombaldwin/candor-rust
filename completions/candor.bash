# Unified bash completion for the whole candor family: `candor` (a symlink to cargo-candor), `candor-ts-query`,
# `candor-java`, `candor-swift`, `candor-agents`. Since candor-spec §3.3.1 every engine drives a query the SAME
# way — `<cmd> <verb> <args…> [--report <loc>] [--policy <file>] [--json] [--strict] [--include-unknown]`, with
# the report DISCOVERED from a `.candor/` ancestor — so this is ONE grammar, not five. Static subcommands +
# flags, plus DYNAMIC completion from the discovered report: the effect vocabulary, the FUNCTION NAMES candor
# found, and — the smart part — on a `path <fn> <TAB>` effect slot, only the effects THAT function has.
#   source /path/to/candor/completions/candor.bash

# --- report resolution + extraction (best-effort, silent, fast; completion must never error) ------------
_cc_extract() {  # $1 = "__fns__" (list functions) | a fn-name (its inferred effects) ; report files as $2…
  command -v python3 >/dev/null 2>&1 || { for f in "${@:2}"; do grep -oE '"fn":"[^"]+"' "$f" 2>/dev/null|sed 's/^"fn":"//;s/"$//'; done; return; }
  python3 - "$@" <<'PY' 2>/dev/null
import json,sys
mode=sys.argv[1]; out=[]
for p in sys.argv[2:]:
    try: fns=json.load(open(p)).get("functions",[])
    except Exception: continue
    if mode=="__fns__":
        out += [e["fn"] for e in fns if e.get("fn")]
    else:  # mode is a function name → its inferred effects
        for e in fns:
            if e.get("fn")==mode: out += e.get("inferred",[])
print("\n".join(sorted(set(out))))
PY
}

# The report file(s) the ENGINE itself would use for THIS command line (§3.3.1): an explicit `--report <loc>`
# already on the line (resolved dir → `.candor/report` | `.json` path | prefix), else $CANDOR_REPORT, else
# discovery — walk UP from $PWD for a `.candor/` directory (the same rule the engines share). Never per-engine.
_cc_reports() {
  local i loc="" pfx="" d
  for ((i=0; i<${#COMP_WORDS[@]}; i++)); do [ "${COMP_WORDS[i]}" = "--report" ] && loc="${COMP_WORDS[i+1]}"; done
  if [ -n "$loc" ]; then
    if   [ -d "$loc" ];      then pfx="$loc/.candor/report"
    elif [[ "$loc" == *.json ]]; then echo "$loc"; return
    else                          pfx="$loc"; fi
  elif [ -n "$CANDOR_REPORT" ]; then pfx="$CANDOR_REPORT"
  else
    d="$PWD"
    while :; do [ -d "$d/.candor" ] && { pfx="$d/.candor/report"; break; }; [ "$d" = "/" ] && break; d="$(dirname "$d")"; done
  fi
  [ -n "$pfx" ] && ls "$pfx"*.json 2>/dev/null | grep -v -e callgraph -e hierarchy | head -12
}

# --- per-command surface --------------------------------------------------------------------------------
_cc_subs() { case "$1" in
  candor|cargo-candor) echo "setup scan audit snapshot guard diff watch show where callers whatif fix fix-gate unverified rewire map containment reachable path impact explain policy risk no-ambient strict update help" ;;
  candor-ts-query)     echo "show where callers map containment diff reachable impact blindspots gains path whatif fix fix-gate unverified parsepolicy agents" ;;
  candor-java)         echo "show where callers map diff containment reachable path impact blindspots gains whatif fix fix-gate unverified rewire parsepolicy" ;;
  candor-swift)        echo "fix fix-gate unverified parsepolicy" ;;
  candor-agents)       echo "scan observe drift guard digest stats savings log-gate" ;;
esac; }
# the positional argument sequence AFTER a verb (the report is a `--report` FLAG now, never a leading
# positional): each slot is `fn` or `effect`. diff/gains take two report locators — completed as files below.
_cc_argtypes() { case "$1" in
  where)               echo "effect" ;;
  path|whatif|fix)     echo "fn effect" ;;
  show|callers|impact) echo "fn" ;;
  *)                   echo "" ;;
esac; }

_candor() {
  local cmd cur prev cword sub effects flags
  cmd="$(basename -- "${COMP_WORDS[0]}")"
  cur="${COMP_WORDS[COMP_CWORD]}"; prev="${COMP_WORDS[COMP_CWORD-1]}"; cword=$COMP_CWORD
  effects="Net Fs Db Exec Env Clock Rand Ipc Clipboard Log Unknown"
  flags="--report --policy --json --strict --include-unknown --gate-json --out --version -V --help -h"

  # position 1 → a subcommand, or a top-level flag (--version/--help/--agents)
  if [ "$cword" -eq 1 ]; then
    if [[ "$cur" == -* ]]; then COMPREPLY=( $(compgen -W "--version -V --help -h --agents" -- "$cur") )
    else COMPREPLY=( $(compgen -W "$(_cc_subs "$cmd")" -- "$cur") ); fi
    return
  fi
  # a flag's VALUE (report/policy/out/gate-json all take a path)
  case "$prev" in --report|--policy|--out|--gate-json) COMPREPLY=( $(compgen -f -- "$cur") ); return ;; esac
  # a flag being typed
  if [[ "$cur" == -* ]]; then COMPREPLY=( $(compgen -W "$flags" -- "$cur") ); return; fi

  sub="${COMP_WORDS[1]}"
  case "$sub" in parsepolicy|diff|gains) COMPREPLY=( $(compgen -f -- "$cur") ); return ;; esac  # policy/report files
  case "$cmd" in
    candor|cargo-candor) case "$sub" in scan|audit|guard|snapshot|watch) COMPREPLY=( $(compgen -d -- "$cur") ); return ;; esac ;;
    candor-agents)       COMPREPLY=( $(compgen -d -- "$cur") ); return ;;   # every agents verb takes a project dir
  esac

  # positional slot = count the non-flag tokens after the verb (word 1) up to the cursor; `pfn` is the last
  # one seen (the function that precedes an effect slot, for context-aware completion).
  local -a types=( $(_cc_argtypes "$sub") ); [ ${#types[@]} -eq 0 ] && return
  local i tok pos=0 pfn=""
  for ((i=2; i<cword; i++)); do
    tok="${COMP_WORDS[i]}"
    case "$tok" in
      --report|--policy|--out|--gate-json) ((i++)); continue ;;   # skip the flag AND its value
      -*) continue ;;                                             # a boolean flag
    esac
    pfn="$tok"; ((pos++))
  done
  local t="${types[$pos]}"
  case "$t" in
    effect)
      # CONTEXT-AWARE: on `path/whatif/fix <fn> <TAB>`, offer ONLY that function's effects (fall back to all).
      local fneffs=""
      if [ "$pos" -ge 1 ] && [ -n "$pfn" ]; then fneffs="$(_cc_extract "$pfn" $(_cc_reports))"; fi
      COMPREPLY=( $(compgen -W "${fneffs:-$effects}" -- "$cur") ) ;;
    fn)
      COMPREPLY=( $(compgen -W "$(_cc_extract __fns__ $(_cc_reports))" -- "$cur") ) ;;
  esac

  # function names carry `::` — trim the already-typed prefix (bash treats `:` as a word break)
  type __ltrim_colon_completions &>/dev/null && __ltrim_colon_completions "$cur"
}

complete -F _candor candor cargo-candor candor-ts-query candor-java candor-swift candor-agents
