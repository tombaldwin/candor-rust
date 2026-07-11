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
