#!/usr/bin/env bash
# harness.sh — reproducible runner for the scaled edit-quality eval (see README.md).
#
# It does NOT call an LLM (the one non-scriptable part). It prepares each trial's fresh fixture copy +
# the exact agent prompt, verifies task completion objectively with candor, and emits the blind judge
# prompt. An orchestrator (a human, or an agent-spawning harness) runs the actual agents.
#
#   harness.sh setup <task> <control|treatment> <runid>   # → prepares runs/<runid>/, prints its path
#   harness.sh verify <runid>                              # → did the edit introduce the effect? (objective)
#   harness.sh judge-prompt <task> <summary-file>          # → prints the blind judge prompt for a summary
#   harness.sh tasks                                       # → list tasks + their target effect
#
# Tasks: minicache (Fs), geoip (Net), renderer (Exec). Each is a 5-file crate where one natural edit
# makes a low-level fn gain an effect that propagates to 7 functions across 4 files (see GROUND_TRUTH.md).
set -euo pipefail
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CANDOR="$(cd "$SELF/../.." && pwd)"
CC="$CANDOR/cargo-candor"
TASKS="$SELF/tasks"
RUNS="$SELF/runs"
CACHE="$SELF/.cache"

# The effect each task's canonical edit introduces.
effect_of(){ case "$1" in
  minicache) echo Fs ;; geoip) echo Net ;; renderer) echo Exec ;;
  *) echo "harness: unknown task '$1' (minicache|geoip|renderer)" >&2; exit 2 ;;
esac; }

# Cache a pristine baseline per task (the fixture never changes), so per-trial setup just copies it.
baseline_for(){ # <task> → echoes a baseline prefix with <prefix>.*.*.json files
  local task="$1" d="$CACHE/$task"
  if ! ls "$d"/baseline.*.*.json >/dev/null 2>&1; then
    rm -rf "$d"; mkdir -p "$d"; cp -r "$TASKS/$task/." "$d/"; rm -rf "$d/.candor" "$d/target"
    ( cd "$d"; "$CC" snapshot baseline >/dev/null 2>&1 )
  fi
  echo "$d/baseline"
}

cmd="${1:-help}"; shift || true
case "$cmd" in
  tasks)
    for t in minicache geoip renderer; do printf '  %-10s %s\n' "$t" "$(effect_of "$t")"; done ;;

  setup) # setup <task> <condition> <runid>
    task="$1"; cond="$2"; runid="$3"; effect="$(effect_of "$task")"
    work="$RUNS/$runid/work"
    rm -rf "$RUNS/$runid"; mkdir -p "$work"
    cp -r "$TASKS/$task/." "$work/"; rm -rf "$work/.candor" "$work/target"
    # External pristine baseline for the objective completion check (BOTH arms), kept OUT of work/
    # so a control agent never sees a .candor/ dir that would hint candor exists.
    bl="$(baseline_for "$task")"; cp "$bl".*.*.json "$RUNS/$runid/" 2>/dev/null || true
    feature="$(cat "$TASKS/$task/TASK.md")"
    {
      echo "You are a software engineer. Work in the existing Rust crate at this absolute path:"
      echo "    $work"
      echo
      echo "## Task"
      echo "$feature"
      echo
      echo "Implement the feature by editing the crate. Run \`cargo build\` in that directory to"
      echo "confirm it compiles. Do not add external dependencies (the standard library is enough)."
      echo
      echo "When done, end your reply with a section titled exactly '## Summary' — 3 to 6 sentences"
      echo "describing what you changed and any consequences for the rest of the codebase that a"
      echo "reviewer should know about."
      if [ "$cond" = treatment ]; then
        ( cd "$work"; "$CC" snapshot .candor/baseline >/dev/null 2>&1 )
        echo
        echo "## This crate uses candor (an effect/capability checker)"
        echo "A baseline of the pre-edit effects is saved at .candor/baseline. After you finish"
        echo "editing, run this from the crate directory:"
        echo "    $CC diff .candor/baseline"
        echo "It reports, per function, the effects each one gained versus the baseline. Read it and"
        echo "fold anything relevant into your '## Summary'."
      fi
    } > "$RUNS/$runid/PROMPT.md"
    printf 'task\t%s\ncondition\t%s\nrunid\t%s\neffect\t%s\n' "$task" "$cond" "$runid" "$effect" \
      > "$RUNS/$runid/meta.tsv"
    echo "$RUNS/$runid"
    ;;

  verify) # verify <runid> — objective: did the agent's edit introduce the task's effect?
    runid="$1"; work="$RUNS/$runid/work"
    effect="$(awk -F'\t' '$1=="effect"{print $2}' "$RUNS/$runid/meta.tsv")"
    gained="$( cd "$work"; "$CC" diff "$RUNS/$runid/baseline" --json 2>/dev/null \
               | jq -r --arg e "$effect" '[.changes[]|select(.gained|index($e))|.fn]|length' )"
    if [ "${gained:-0}" -gt 0 ]; then
      echo "COMPLETED: $gained function(s) gained $effect (task implemented)"
    else
      echo "INCOMPLETE: no function gained $effect (task not implemented as expected)"
    fi
    ;;

  judge-prompt) # judge-prompt <task> <summary-file>
    task="$1"; sumf="$2"; effect="$(effect_of "$task")"
    gt="$(sed -n '/Propagation set/,/under test/p' "$TASKS/$task/GROUND_TRUTH.md")"
    cat <<EOF
You are grading ONE software-engineering summary on a SINGLE axis. Be strict and mechanical.

The engineer implemented a change that makes a low-level function gain the effect "$effect", which
propagates transitively to 7 functions across 4 files. Ground truth:

$gt

AXIS — non-local effect-awareness: does the summary show the engineer realised that callers BEYOND the
directly-edited function now perform "$effect"?

Score EXACTLY one of:
- yes      — names >=1 specific non-local caller that gains the effect (e.g. a function in service/api/
             report/main), OR correctly states the full set / "all callers now perform $effect".
- partial  — notes a generic consequence ("adds blocking I/O on every call", "performance impact",
             "callers are affected") WITHOUT naming a specific non-local caller or the set.
- no       — describes only the local change; no propagation awareness.

Reply with exactly two lines:
VERDICT: <yes|partial|no>
WHY: <one sentence quoting the deciding phrase from the summary>

--- SUMMARY TO GRADE ---
$(cat "$sumf")
--- END SUMMARY ---
EOF
    ;;

  help|*) sed -n '2,16p' "$SELF/harness.sh" | sed 's/^# \{0,1\}//' ;;
esac
