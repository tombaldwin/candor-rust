# Pre-registration: cross-model completeness of the edit-feedback lift

**Committed before any trial agent runs.** Extends the batch-3 orderflow completeness eval
([RESULTS-v3.md](RESULTS-v3.md), Opus-class only) across the model tier range, to separate what the TOOL
contributes from what the MODEL contributes. Tracks the value-proposition claim: candor's completeness lift
holds — and is *largest* — for weaker models, because the bottleneck is not capability but that models won't
volunteer 5 layers of tedious transitive enumeration.

## Design (and nothing else changes from RESULTS-v3's protocol)

- **Task:** `orderflow` (batch-3, `tasks-v3/`) — one natural edit (`Pricing::quote` fetches a live FX rate
  over TCP → gains **Net**) propagates to **16 non-local functions** across 9 files / 5 layers
  (`harness.sh nonlocal_of orderflow`). The hard graph task where the value concentrates.
- **Models (subagent tier names → resolved id recorded in RESULTS):** `opus` (Opus 4.8), `sonnet` (Sonnet 5),
  `haiku` (Haiku 4.5), `fable` (Fable 5). Fable access confirmed this session (a trivial probe resolved to
  `claude-fable-5`).
- **Arms:** `control` (task only) · `treatment` (task + "run `cargo-candor diff .candor/baseline`, fold it
  into your Summary"). Prompts are the harness's verbatim output — the engineer never sees GROUND_TRUTH.
- **N:** a **1/cell PILOT first** (4 models × 2 arms = 8 engineers + 8 blind judges) to validate the harness
  end-to-end via subagents and get a real per-trial token number; then **5/cell** for the full run
  (40 engineers + 40 judges) — reconfirmed with Tom on the pilot's measured cost before launch.
- **Orchestration:** I am the harness (Agent/Workflow tools). Engineer = the tier model, working in the
  de-leaked `runs-xmodel/<runid>/work/` with the harness PROMPT.md; its `## Summary` → `summary.md`.
  Objective check = `harness.sh verify` (did the edit introduce Net?). Judge = **Haiku, blind to condition**,
  one per summary, reading only `harness.sh judge-prompt orderflow <summary>` (tool-identity redacted).

## Metric

**Completeness** = of the 16 non-local functions, how many the agent's `## Summary` names as gaining Net
(blind-judge leaf-name match; `main` counts). Reported per (model, arm) as mean/16. Secondary: the objective
`verify` COMPLETED/INCOMPLETE rate (a trial whose edit didn't introduce Net is an infra/task miss, excluded
from the completeness mean with a note — never scored as a model false-negative).

## Pre-registered hypotheses (falsification bars)

1. **Treatment > control at every tier.** Falsified for a tier if its mean treatment completeness ≤ mean
   control completeness.
2. **The lift is LARGEST for weaker models.** Concretely: (treatment − control) at `haiku` ≥ (treatment −
   control) at `opus`. The tool "carries" weaker models. Falsified if the gap shrinks as the tier drops.
3. **Treatment completeness is high and tier-flat** (~14–16/16 at every tier — the tool does the
   enumeration regardless of model). Falsified if treatment at `haiku` < 0.75 × treatment at `opus`.
4. **Control completeness is low at every tier and does NOT recover at the frontier** (RESULTS-v3: Opus
   control = 1/16). Falsified if any tier's control mean ≥ 8/16 — that would mean a model volunteers the
   enumeration unaided, eroding the tool's value.

## Exclusions

A harness error (empty engineer return, build-failure that blocks `verify`, judge non-response) → one rerun,
noted. No other exclusions. Runs recorded under `runs-xmodel/`. Cross-day / cross-serving-speed caveats do
not apply (completeness is model-internal, not wall-clock).

---

## Amendment (pilot-driven, committed before the full run)

The 1/cell pilot ([RESULTS-xmodel-pilot.md](RESULTS-xmodel-pilot.md)) validated the pipeline but showed the
completeness metric at N=1 judge is dominated by blanket-credit variance (identical "reaches 16 callers up to
`main`" summaries scored 5 vs 15). Fixes, pre-registered here:

- **DUAL metric, reported separately** (Tom's call — removes the judge's weighting discretion):
  - **STRICT** = number of the 16 functions the summary names EXPLICITLY (or in an unambiguous "all N of:
    <list>").
  - **LENIENT** = 16 if the summary makes a valid whole-set blanket claim ("the effect reaches all callers /
    the whole call chain up to `main`"), else = STRICT.
- **Deterministic judge protocol** (no discretion on the blanket's weight): each blind judge outputs, per
  function, `named: yes|no` (explicit name only), plus one line `BLANKET: yes|no` (does the summary assert
  the effect reaches the ENTIRE caller chain / all callers up to `main`?). The orchestrator computes STRICT =
  Σ named, LENIENT = BLANKET ? 16 : STRICT. Judges never weight the blanket themselves.
- **3 blind judges per summary**, mean each metric (averages residual judge variance).
- **Verbatim summaries**: the engineer's full returned text is scored (the pilot hand-abridged; the Workflow
  run captures the exact text).
- **N = 5 trials/cell** (40 engineers). Total agents = 40 engineers + 120 judges = 160.
- Baseline regenerated with the shipped **candor-scan 0.8.8** (the pilot saw a cross-build baseline warning).
- Hypotheses H1–H4 unchanged, now evaluated on BOTH metrics (a claim that holds only under LENIENT is
  reported as such).
