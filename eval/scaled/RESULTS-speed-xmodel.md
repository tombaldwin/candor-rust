# Cross-model speed/token A/B — the blast-radius analysis question

Pre-registered in [PREREG-speed-xmodel.md](PREREG-speed-xmodel.md). Extends the Opus-only speed A/B
([RESULTS-speed.md](RESULTS-speed.md): 1.81× faster, fewer tokens, both 16/16) across the tier range.

## Methodology unblock (the headline for the harness)

The prior note held this eval was NOT runnable via subagents — no per-agent telemetry. **That is fixed:**
this session's Agent completion notifications carry `subagent_tokens` and `duration_ms` per agent. So the
speed/token A/B runs via subagents with clean per-agent tokens + the returned list for recall. (Wall-clock
is secondary/caveated — trials run concurrently, so absolute durations are serving-speed/contention noisy.)

## Result — N=1 per cell (opus/sonnet/haiku/fable × control/treatment), directed "list EVERY affected fn"

| model  | tokens c/t        | tok ratio | wall c/t (noisy) | tools c/t | recall |
|--------|-------------------|-----------|------------------|-----------|--------|
| opus   | 21705 / **15576** | 1.39×     | 27.1s / 14.2s    | 13 / 2    | 16/16  |
| sonnet | 26863 / **21657** | 1.24×     | 23.7s / 20.1s    | 3 / 3     | 16/16  |
| haiku  | 22856 / **16094** | 1.42×     | 48.8s / 11.2s    | 13 / 2    | 16/16  |
| fable  | 20979 / **15577** | 1.35×     | 42.4s / 24.3s    | 3 / 2     | 16/16  |

- **TOKEN saving is consistent across every tier: 1.24–1.42× (24–42% fewer), median 1.37×** — the clean,
  rigorous metric. Candor's query returns the transitive set directly; the source agent re-derives it.
- **RECALL is 16/16 for BOTH arms at every tier.** On a DIRECTED question ("list EVERY affected function"),
  even the control agent does the full trace — so here candor buys the SAME complete answer for fewer tokens
  and fewer tool calls (treatment 2–3 calls vs control 3–13), not more completeness. (Contrast the edit-task
  completeness eval, [RESULTS-xmodel.md](RESULTS-xmodel.md), where the control agent won't *volunteer* the set
  in a review summary — control ≈ 0, treatment ≈ 14. The two evals are complementary: candor makes the
  complete answer **cheaper** on a directed query, and **volunteered** on an edit.)
- **WALL-CLOCK: treatment faster at every tier (1.18–4.35×), but noisy** — e.g. haiku-control 48.8s (13 tool
  calls) vs 11.2s; fable-treatment 24.3s did the same 2-call query as haiku-treatment's 11.2s. Serving-speed
  + concurrency, not tool effect. Directionally consistent with the token/tool-call story; a clean serial
  wall-clock (matching the prior Opus 1.81×) is the follow-up if a tight number is wanted.

## Falsification bars

1. **Token claim** (median treatment tokens < control): ✅ held at every tier (1.24–1.42×).
2. **Recall floor** (treatment ≥ control): ✅ held — 16/16 = 16/16 every tier.
3. **Consistency** (token ratio > 1 every tier): ✅ held — no tier below 1.24×.

## Caveat

N=1/cell — a cross-model SNAPSHOT, not the full N=5 medians. The token saving is consistent and replicates
the prior Opus N=8, so the direction is solid; scaling to N=5 (and a serial-timed wall-clock pass) would
tighten the medians, especially for wall-clock. Per-agent telemetry: `speed-xmodel/runs/telemetry.tsv`.

---

## N=5 RESULT (2026-07-12, per PREREG amendment)

4 models × 2 arms × **5 trials** (40 agents), same blast-radius analysis task ("list EVERY function affected
if `Pricing::quote` gained Net" over orderflow's 16-fn graph). **Metric here = OUTPUT tokens** (workflow
`budget.spent()` deltas, sequential cells for clean attribution) — a DIFFERENT, larger lens than the N=1
run's *total* `subagent_tokens` (1.37×): control's cost is dominated by the long reasoning/list it GENERATES
while tracing the graph by hand, so the output-token gap is much wider than the total-token gap.

| model  | control tok/trial | treatment tok/trial | saving | recall (ctl / tmt) |
|--------|-------------------|---------------------|--------|--------------------|
| opus   | 1300              | 337                 | 3.9×   | 16/16 · 16/16 |
| sonnet | 1224              | 285                 | 4.3×   | 16/16 · 16/16 |
| haiku  | 2101              | 248                 | **8.5×** | 16/16 · 16/16 |
| fable  | 894               | 303                 | 2.9×   | 16/16 · 16/16 |

- **H1 (treatment < control tokens every tier): HOLD** — 2.9–8.5× fewer output tokens, every tier.
- **H2 (recall floor): HOLD** — **16/16 in every cell, both arms** (the directed task → both reach full
  recall; candor doesn't cost completeness, it costs far fewer tokens to get there).
- **H3 (consistency, ratio > 1 every tier): HOLD.**

**The saving is LARGEST for the weakest model (haiku 8.5×):** unaided, haiku burns the most output tokens
(2101/trial) flailing through the source to trace the graph; with the candor report it runs ONE
`candor-query callers` call and answers in 248. This mirrors the completeness mechanism on the analysis task —
the tool carries weak models most where the manual work is tedious. Direction replicates the N=1 snapshot and
the prior Opus N=8 (1.81× total-token); the output-token lens just makes the gap starker.
Runs: `speed-xmodel/` (this workflow measured per-cell output tokens; per-trial `subagent_tokens` would need
direct-agent notifications — the direction is unambiguous either way).
