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
# Overridable so a batch can point at a fixture set (tasks / tasks-v2) and its own run dir.
TASKS="${CANDOR_EVAL_TASKS:-$SELF/tasks}"
RUNS="${CANDOR_EVAL_RUNS:-$SELF/runs}"
CACHE="$SELF/.cache/$(basename "$TASKS")"

# The effect each task's canonical edit introduces.
effect_of(){ case "$1" in
  minicache) echo Fs ;; geoip) echo Net ;; renderer) echo Exec ;;
  orderflow) echo Net ;;   # batch-3 LARGE fixture (point CANDOR_EVAL_TASKS at tasks-v3)
  *) echo "harness: unknown task '$1' (minicache|geoip|renderer|orderflow)" >&2; exit 2 ;;
esac; }

# The NON-LOCAL functions each task's effect propagates to (the edited fn excluded) — the
# completeness denominator. Names are matched against the agent's summary by the judge.
nonlocal_of(){ case "$1" in
  minicache) echo "Service::lookup Service::batch api::get_one api::get_many report::build main" ;;
  geoip)     echo "GeoService::locate GeoService::batch api::lookup_one api::lookup_many report::summary main" ;;
  renderer)  echo "Page::render_token Page::render api::render_one api::render_many report::build_all main" ;;
  orderflow) echo "Pricing::quote_bulk Cart::line_total Cart::subtotal Cart::total Discount::for_cart Checkout::review Checkout::place OrderService::quote_one OrderService::quote_many OrderService::checkout api::get_quote api::list_quotes api::post_checkout report::daily_revenue admin::recompute_prices main" ;;
esac; }

# Cache a pristine baseline per task (the fixture never changes), so per-trial setup just copies it.
baseline_for(){ # <task> → echoes a baseline prefix with <prefix>.*.*.json files
  local task="$1" d="$CACHE/$task"
  if ! ls "$d"/baseline.*.*.json >/dev/null 2>&1; then
    rm -rf "$d"; mkdir -p "$d/src"; cp "$TASKS/$task/Cargo.toml" "$d/"; cp -r "$TASKS/$task/src/." "$d/src/"
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
    rm -rf "$RUNS/$runid"; mkdir -p "$work/src"
    # Copy ONLY the crate (Cargo.toml + src) into the agent's working dir — never GROUND_TRUTH.md /
    # TASK.md, which would hand the agent the answer. The task reaches the agent via the prompt only.
    cp "$TASKS/$task/Cargo.toml" "$work/"; cp -r "$TASKS/$task/src/." "$work/src/"
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
        # Give the working copy the same pre-edit baseline (copied from the per-task cache, not
        # re-snapshotted — identical content, no extra compile) so the agent can diff against it.
        mkdir -p "$work/.candor"; cp "$(dirname "$bl")"/baseline.* "$work/.candor/" 2>/dev/null || true
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
    # Capture the diff JSON separately (|| true so a tool failure doesn't abort under set -e). An empty
    # result, or jq failing, means the check itself couldn't run — report ERROR, distinct from a genuine
    # INCOMPLETE (diff ran, no function gained the effect), so a scaled batch never scores infra
    # breakage as a model false-negative.
    dj="$( cd "$work"; "$CC" diff "$RUNS/$runid/baseline" --json 2>/dev/null || true )"
    gained="$(printf '%s' "$dj" | jq -r --arg e "$effect" '[.changes[]|select(.gained|index($e))|.fn]|length' 2>/dev/null || true)"
    # `gained` must be a single integer; empty / non-numeric / multi-line all mean the check couldn't
    # run → ERROR (not a misrouted INCOMPLETE, and `[ -gt ]` never sees a non-number).
    if [ -z "$dj" ]; then
      echo "ERROR: could not evaluate (no baseline, build failure, or diff produced no output)"
    elif case "$gained" in ''|*[!0-9]*) true ;; *) false ;; esac; then
      echo "ERROR: could not evaluate (jq missing or unexpected diff output)"
    elif [ "$gained" -gt 0 ]; then
      echo "COMPLETED: $gained function(s) gained $effect (task implemented)"
    else
      echo "INCOMPLETE: no function gained $effect (task not implemented as expected)"
    fi
    ;;

  judge-prompt) # judge-prompt <task> <summary-file>
    task="$1"; sumf="$2"; effect="$(effect_of "$task")"
    gt="$(sed -n '/Propagation set/,/under test/p' "$TASKS/$task/GROUND_TRUTH.md")"
    # Redact the tool's identity so the judge is BLIND to condition (treatment summaries otherwise
    # say "candor's diff confirms…"). Substance — the propagation claims — is preserved.
    # Map every tool reference to a neutral "the analysis" — a phrase an agent could equally use for
    # its own manual call-graph reasoning — so the judge can't tell tool-assisted from manual.
    redacted="$(sed -E \
      -e 's#(cargo[ -]candor) diff [^ ]*#the analysis#g' \
      -e 's#cargo[ -]candor#the analysis#g' \
      -e 's#\.candor/baseline#the baseline#g' \
      -e "s#[Cc]andor'\''s( diff)?#the analysis#g" \
      -e 's#[Cc]andor#the analysis#g' "$sumf")"
    nl="$(nonlocal_of "$task")"; k="$(echo "$nl" | wc -w | tr -d ' ')"
    nl_lines="$(for f in $nl; do echo "  - $f"; done)"
    cat <<EOF
You are grading ONE software-engineering summary, BLIND, strictly and mechanically.

The engineer made a change that makes a low-level function gain the effect "$effect". That effect then
propagates transitively to these $k OTHER functions (the "propagation set"):
$nl_lines

PART 1 — COMPLETENESS (primary). For EACH function in the propagation set above, decide whether the
summary identifies it as now performing "$effect" — i.e. the function is named explicitly, OR it is
covered by an explicit blanket statement ("all callers", "every caller", "the whole call chain up to
main"). A function merely existing in the code does NOT count; the summary must indicate IT gains the
effect. Output one line per function, in order:
  <function>: yes
  <function>: no
Then a single line:
  COMPLETENESS: <number-of-yes>/$k

PART 2 — BINARY AWARENESS (secondary). One line:
  VERDICT: yes|partial|no
where yes = names >=1 specific non-local caller OR the full set; partial = notes a generic
"callers/performance are affected" WITHOUT naming a specific non-local caller; no = only the local change.

Output ONLY the per-function lines, the COMPLETENESS line, and the VERDICT line. No other text.

--- SUMMARY TO GRADE ---
$redacted
--- END SUMMARY ---
EOF
    ;;

  help|*) sed -n '2,16p' "$SELF/harness.sh" | sed 's/^# \{0,1\}//' ;;
esac
