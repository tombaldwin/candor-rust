# Pre-registration: the speed A/B across model tiers (Fable 5 / Opus / Sonnet)

**Committed before any trial agent runs.** Extends [PREREG-speed.md](PREREG-speed.md): the original
batch inherited one (Opus-class) session model; this re-runs the identical protocol at three tiers to
separate what the TOOL contributes from what the MODEL contributes.

## Changes from the original (and nothing else)

- **Models:** `fable` (Fable 5), `opus` (Opus 4.8), `sonnet` (Sonnet 4.6) — the agent harness's tier
  names; the exact ids the harness resolves are recorded in RESULTS.
- **N = 8/arm/model** (48 trials). Batched 8-concurrent with arms balanced within every batch (4C+4T,
  same model), so concurrent-load bias is fair *within* each model's comparison.
- **Tooling under test is the PUBLISHED candor-scan 0.3.2** (the version PROVE-IT requires) + the
  repo `candor-query`; the treatment prompt's binary path is updated for the candor-rust rename.
  Note 0.3.2's match ladder means the whatif target seeds from exactly one function (the original
  run's substring-widening footnote no longer applies).
- Prompts otherwise **verbatim** from PREREG-speed.md.

## Pre-registered hypotheses

1. **Treatment is faster at every tier** (per-model falsification: median treatment wall-clock ≥
   median control wall-clock refutes for that tier).
2. **Control completeness degrades as tier drops; treatment does not.** The standing claim
   (batch-3 + the A3 Haiku probe) is that the tool carries completeness for weaker models. Concretely:
   treatment recall stays 16/16 at every tier; control recall at sonnet < control recall at fable/opus
   is the expected direction. If sonnet-control also hits 16/16, the "tool carries weaker models"
   claim loses its mid-tier support and must be reported as weakened.
3. **No prediction is made comparing across tiers' absolute times** (different models have different
   serving speeds); the ratio *within* each tier is the statistic.

## Metrics, scoring, exclusions

Identical to PREREG-speed.md: wall-clock `duration_ms`, `subagent_tokens`, recall vs the 16-function
non-local set (leaf-name match, `main` counts). Median ratios per tier. No exclusions except a
harness error (empty return → one rerun, noted). Cross-day caveat applies to any comparison with the
original batch (different day/load/binary); within-this-batch comparisons are the clean ones.
