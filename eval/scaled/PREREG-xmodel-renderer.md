# Pre-registration: cross-model completeness on a 2nd codebase (renderer / Exec)

**Committed before any trial agent runs.** Generalization test for the cross-model completeness lift
([RESULTS-xmodel.md](RESULTS-xmodel.md), on `orderflow`): does the lift hold on a DIFFERENT codebase with a
DIFFERENT effect and a smaller propagation set? If it's an orderflow artifact, it won't.

## Design — IDENTICAL protocol to RESULTS-xmodel.md; only the fixture changes

- **Task:** `renderer` (batch-1/2, `tasks/renderer/`) — one natural edit adds an `{{exec:CMD}}` template
  directive that runs `sh -c CMD` → `Engine::expand` gains **Exec**, propagating to **6 non-local functions**
  (`Page::render_token Page::render api::render_one api::render_many report::build_all main` —
  `harness.sh nonlocal_of renderer`). Deliberately UNLIKE orderflow: Exec not Net, template-engine not
  pricing, 6-fn chain not 16 (a smaller/less-tedious graph — a direct test of hypothesis 2's mechanism).
- **Models:** `opus` (Opus 4.8) · `sonnet` (Sonnet 5) · `haiku` (Haiku 4.5) · `fable` (Fable 5). Resolved
  ids recorded in RESULTS.
- **Arms:** `control` (task only) · `treatment` (task + "run `cargo-candor diff .candor/baseline`, fold into
  your Summary"). Prompts are the harness's verbatim output; the engineer never sees GROUND_TRUTH.
- **N = 5/cell.** 4 models × 2 arms × 5 = **40 engineers**. The harness is already validated end-to-end by
  the orderflow run, so no separate pilot; a trial whose objective `verify` is INCOMPLETE/ERROR is excluded
  from the completeness mean with a note (never scored as a model false-negative).
- **Judges:** **3 blind Haiku judges per summary** (120 judges). Blind to condition — they read only the
  tool-redacted summary + the 6-fn propagation set. Each returns FACTS: `{ named: [fn leaf-names the summary
  explicitly says gain Exec], blanket: bool (an explicit "all callers / whole chain up to main" claim) }`.
  The orchestrator (not the judge) computes both metrics deterministically.

## Metrics (dual — same as RESULTS-xmodel.md)

- **STRICT** = |named ∩ propagation-set| — functions named explicitly (mean /6, over 3 judges then 5 trials).
- **LENIENT** = 6 if `blanket` else STRICT — credits an explicit whole-chain claim as complete.
- Secondary: objective `verify` COMPLETED rate.

## Pre-registered hypotheses (falsification bars) — same as orderflow, re-tested on renderer

1. **Treatment > control at every tier** (LENIENT). Falsified for a tier if treatment ≤ control.
2. **Lift largest for weaker models**: (treatment − control) at `haiku` ≥ at `opus`.
3. **Treatment high + tier-flat** (LENIENT): treatment at `haiku` ≥ 0.75 × treatment at `opus`.
4. **Control low + no frontier recovery**: no tier's control LENIENT mean ≥ 0.75 × 6 (= 4.5).
5. **Generalization (the point of this run):** the qualitative pattern (big tier-flat treatment, low control,
   biggest lift low-tier) replicates orderflow's. Falsified if the direction inverts or the lift vanishes on
   the smaller/Exec fixture.

Judge model = Haiku, blind. Orchestration = a Workflow (I am the harness). Runs under `runs-xmodel-renderer/`.
