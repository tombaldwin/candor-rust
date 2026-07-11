# Cross-model completeness — 1/cell PILOT (invalidated the metric at N=1 judge)

Pre-registered in [PREREG-xmodel-completeness.md](PREREG-xmodel-completeness.md). Ran the full 4-model × 2-arm
matrix at **N=1** to validate the pipeline and get a cost number before the 5/cell run. It validated the
pipeline — and surfaced a metric-reliability problem that must be fixed before scaling.

## Pipeline: works end-to-end via subagents

- **Fable 5 reachable** as a subagent model (`model: "fable"` → `claude-fable-5`); the prior blocker is gone.
- 8 engineer subagents (opus/sonnet/haiku/fable × control/treatment) each did the real Rust edit; **all 8
  objectively COMPLETED** (`harness.sh verify`: the edit introduced Net across the 16-fn set). No exclusions.
- 8 blind Haiku judges scored each summary under shuffled opaque ids (no model/arm leaked).
- Cost: ~19–27k tok/engineer, ~16k/judge → **≈310k tokens for the 8-cell pilot**. Full 5/cell ≈ **1.5–1.7M**.

## Scores (completeness /16, N=1)

| model  | control | treatment |
|--------|---------|-----------|
| opus   | 1       | 5         |
| sonnet | 0       | 15        |
| haiku  | 0       | 0         |
| fable  | 1       | 6         |

## The finding: the metric is judge-variance-dominated at N=1

The treatment summaries were near-identical in shape — candor tells the agent the effect "propagates through
16 intermediate callers up to `main`", and the agent folds that sentence + ~4 named examples into its
Summary. **opus-treatment and sonnet-treatment wrote essentially the same thing, and scored 5 vs 15** —
purely because one blind judge credited "16 callers up to `main`" as a full-set blanket and the other counted
only the explicitly-named functions. The judge prompt lists blanket triggers ("all callers"/"every caller"/
"the whole call chain up to main"), but "propagate through 16 intermediate callers up to main" sits on the
boundary and the judges split on it. (The prior batch-3 had the same tell: per-trial 16, 6, 16 — the "6" was
a non-crediting judge.) **haiku-treatment scored 0** for a related reason: the haiku engineer summarized
candor's output as categories ("admin, API, checkout functions") with no specific names — a real
weaker-model tendency, but also un-creditable under the strict rule.

**What DOES hold cleanly:** control is uniformly low (0–1/16 at every tier, incl. the frontier — H4), and
every treatment ≥ its control. So the *direction* of the lift is real. But the *magnitude* and the
cross-model-consistency hypotheses (H1–H3) can't be tested at N=1 — the blanket-credit coin-flip swamps the
signal.

## Fixes required before the 5/cell run (do NOT scale as-is)

1. **≥3 blind judges per summary, mean the completeness** — averages out the blanket-credit variance (the
   dominant noise). Cheap (judges are ~16k tok, ~4s).
2. **Crisp the blanket rule**: state explicitly whether "the effect reaches all N callers up to `main`"
   counts as covering the whole set. (Recommend: YES — it's a correct, complete claim — matching how a
   reviewer would read it. This is a scoring *decision* to pre-register, not an engineer/model difference.)
3. **Capture the engineer's FULL verbatim Summary** as `summary.md` (the pilot hand-transcribed abridged
   versions; the programmatic Workflow run captures the exact return text).
4. Consider N=5 trials/cell on top of the multi-judge, so both trial- and judge-variance are averaged.

Status: **paused for a methodology decision before the full run** (per the prereg's "reconfirm before
launch"). The pilot's real deliverable is this: the completeness metric needs multi-judge + a pre-registered
blanket rule, or it measures judge coin-flips, not the tool's lift.
