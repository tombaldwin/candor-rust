# Cross-tier speed A/B results — Fable 5 / Opus / Sonnet

Pre-registered in [PREREG-speed-models.md](PREREG-speed-models.md) (`e895ba4`, before any trial).
Fixture: `orderflow`; published **candor-scan 0.3.2**; N=8/arm/model (48 trials, zero exclusions,
zero reruns); arms balanced 4C+4T within every concurrent batch. Tier names are the agent harness's
`fable`/`opus`/`sonnet` (Fable 5 / Opus 4.8 / Sonnet 4.6 per the harness's model table).

## Pre-registered statistics

| tier | control wall (median) | treatment wall (median) | **speed ratio** | control tokens | treatment tokens | tok ratio | control recall | treatment recall |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **Fable 5** | 28.7s | 14.5s | **1.98×** | 17,682 | 12,432 | 1.42× | 128/128 (8/8 perfect) | 128/128 |
| **Opus** | 22.0s | 17.0s | **1.30×** | 18,921 | 13,170 | 1.44× | 128/128 (8/8 perfect) | 128/128 |
| **Sonnet** | 38.8s | **6.5s** | **5.97×** | 14,049 | 13,322 | 1.05× | **123/128 — 3/8 trials incomplete** (14–15 of 16; missed `main`, `quote_bulk`, `OrderService::checkout`) | 128/128 (8/8 perfect) |

Raw per-trial numbers: see the table below.

## Hypothesis verdicts (as pre-registered)

1. **Treatment faster at every tier — HOLDS.** No tier's falsification bar was hit. The ratio is
   tier-dependent: 1.3× (Opus) to 1.98× (Fable) to **6×** (Sonnet).
2. **Control completeness degrades as tier drops; treatment doesn't — HOLDS, at exactly the
   predicted tier.** Fable and Opus controls were 8/8 perfect (replicating the original frontier
   finding); **Sonnet control dropped functions in 3 of 8 trials** while Sonnet *treatment* stayed
   perfect in all 8. This is the missing mid-tier support for the "tool carries completeness for
   weaker models" claim (previously only the Haiku probe + batch-3): with candor, Sonnet's answer is
   indistinguishable from Fable's; without it, Sonnet sometimes silently loses `main` or a
   service-layer caller — the worst kind of miss, since the answer *looks* complete.
3. **No cross-tier absolute-time prediction was made** — and rightly: Opus (fast-mode serving) ran
   its 11–13 manual-trace rounds *faster* than Fable ran 2–3, so absolute times reflect serving
   speed at least as much as work.

## What the gradient means

- **The tool's answer is model-invariant** (16/16 at every tier, ~13k tokens, mostly a single
  query); the manual trace's cost and reliability are model-dependent. So candor's value
  *increases* as the model gets cheaper: at Sonnet it's both ~6× faster **and** the difference
  between a complete and a silently-incomplete answer.
- **The economic case writes itself:** Sonnet + candor beats Sonnet alone on correctness and beats
  bigger models on cost for this question class — the tool substitutes for model capability on graph
  traversal, which is exactly the design thesis (deterministic completeness instead of model effort).
- Fable's treatment outliers (28.9s, 32.8s) were agents that ran *both* `whatif` and `callers` to
  cross-check the tool against itself — unprompted verification, a frontier-model behavior worth
  knowing about (it costs a round but is epistemically sound).

## Raw trials

| tier | arm | wall-clock (s) | tokens | recall/16 |
|---|---|---|---|---|
| fable | control | 25.3, 23.8, 33.9, 24.5, 31.2, 27.1, 31.9, 30.4 | 17676, 17631, 17872, 17687, 17763, 17655, 17943, 17608 | 16 ×8 |
| fable | treatment | 13.6, 28.9, 12.1, 14.7, 13.2, 14.4, 15.5, 32.8 | 12447, 13124, 12391, 12423, 12416, 12441, 12404, 13177 | 16 ×8 |
| opus | control | 20.3, 22.0, 23.1, 23.0, 22.1, 22.1, 18.4, 21.3 | 18890, 18987, 18977, 19006, 18952, 18167, 18874, 18145 | 16 ×8 |
| opus | treatment | 16.3, 10.8, 17.6, 16.8, 15.5, 17.2, 19.0, 18.8 | 13088, 12668, 13167, 13204, 13160, 13172, 13175, 13177 | 16 ×8 |
| sonnet | control | 60.3, 44.3, 65.2, 32.1, 36.9, 26.2, 34.9, 40.8 | 13411, 13779, 14402, 13941, 14284, 13442, 14157, 14303 | **15, 14**, 16, 16, 16, **14**, 16, 16 |
| sonnet | treatment | 6.4, 6.5, 8.8, 7.7, 6.1, 6.5, 6.5, 6.4 | 13316, 13322, 13316, 13357, 13322, 13317, 13355, 13355 | 16 ×8 |

## Honest bounds

1. **Same easy fixture as the original** — 10 modules, distinctive names. Sonnet-control already
   drops functions *here*; on real codebases expect worse (and bigger treatment ratios at every tier).
2. **Serving-speed confound across tiers**: absolute times mix model work with provider serving
   (Opus fast-mode is visibly quick per round); the within-tier ratio is the clean statistic, as
   pre-registered.
3. **Sonnet's token ratio (~1.05×) is misleading in isolation**: Sonnet-control read the whole crate
   in one batched call (1 tool use), so its token bill is low — it paid in *wall-clock and recall*
   instead. The three metrics must be read together.
4. **Treatment completeness remains "tool trusted and tool correct"** (the original red-team's
   circularity note) — rescued again by control unanimity at the frontier tiers: 16 independent
   manual traces (Fable+Opus) all reproduce candor's exact answer.
5. Cross-day comparisons with the original batch are dirty (different day, load, binary); note the
   original Opus-class numbers (30.0s/16.5s, 1.81×) vs today's Opus (22.0s/17.0s, 1.30×) differ
   mostly in *control* speed — consistent with a serving-speed change, not a protocol change.
