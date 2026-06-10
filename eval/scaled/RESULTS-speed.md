# Speed A/B results — does the token saving correspond to a speed increase?

Pre-registered in [PREREG-speed.md](PREREG-speed.md) (committed `0ef0089`, before any trial). Fixture:
`orderflow` (16-fn non-local ground truth). N=8/arm, Opus-class agent both arms, identical prompt
except the tool clause. **No exclusions; no reruns** — all 16 trials returned a parseable list.

## Raw trials

| arm | trial | wall-clock | tokens | tool calls | recall (of 16) |
|---|---|---:|---:|---:|---:|
| control | 1 | 27.8s | 15,537 | 2 | 16/16 |
| control | 2 | 39.6s | 17,207 | 3 | 16/16 |
| control | 3 | 37.3s | 16,252 | 3 | 16/16 |
| control | 4 | 38.4s | 17,358 | 3 | 16/16 |
| control | 5 | 26.5s | 16,788 | 2 | 16/16 |
| control | 6 | 31.2s | 15,858 | 3 | 16/16 |
| control | 7 | 28.7s | 15,606 | 2 | 16/16 |
| control | 8 | 24.4s | 15,527 | 2 | 16/16 |
| treatment | 1 | 16.5s | 10,510 | 1 | 16/16 |
| treatment | 2 | 16.8s | 10,502 | 1 | 16/16 |
| treatment | 3 | 19.3s | 10,610 | 1 | 16/16 |
| treatment | 4 | 16.5s | 10,504 | 1 | 16/16 |
| treatment | 5 | 13.6s | 10,507 | 1 | 16/16 |
| treatment | 6 | 14.4s | 10,642 | 1 | 16/16 |
| treatment | 7 | 33.1s | 11,208 | 3 | 16/16 |
| treatment | 8 | 14.7s | 10,607 | 1 | 16/16 |

## Pre-registered statistics

| | control | treatment | ratio |
|---|---:|---:|---:|
| wall-clock, median | **30.0s** | **16.5s** | **1.81×** |
| wall-clock, range | 24.4–39.6s | 13.6–33.1s | |
| tokens, median | 16,055 | 10,558 | **1.52×** |
| completeness, all trials | 16/16 | 16/16 | 1.0 |

**The speed claim holds:** median treatment wall-clock is well under control's (falsification bar not
hit), at identical — perfect — completeness (the "fast but wrong" bar not hit either).

## The bar that DID fire, reported as pre-registered

**Control reached 100% completeness — far above the 80% bar** — so on *this* question, asked *this*
way, the "agents don't volunteer the full trace" premise does **not** apply. Reconciliation with
batch 3's 6%-vs-79–100% finding: there the tracing was *implicit* in an edit task (the agent had to
volunteer the propagation analysis); here exhaustive tracing **is the entire stated job**, and a
frontier model does it correctly from source on a 10-module crate. The completeness value of candor
lives in the implicit case (and in weaker models — the A3 Haiku probe); the **speed/cost** value is
what survives in the explicit case. The two evals now bracket the behaviour cleanly.

## Honest bounds

1. **End-to-end agent ratios compress the per-question ratio.** The ~17× of `eval/token-cost` is the
   *marginal information cost of the question*. At the agent level, ~10.5k tokens of fixed overhead
   (system prompt, orientation) sit under both arms; the **marginal** cost here was ~5,555 (control)
   vs ~58 (treatment) tokens — consistent with the per-question measurement. Expect the end-to-end
   ratios (1.5×/1.8×) on small crates, and growth toward the per-question ratios as closures deepen:
   control's time scales with closure size; treatment's is ~flat (one query regardless).
2. **This is the easy case for control:** 10 modules, distinctive names, all source in-context after
   2–3 reads. Common-named/deep closures (where the token measurement showed 75–225×) were not
   exercised here.
3. One treatment outlier (33.1s, 3 tool calls — exploratory poking before the query) shows adoption
   variance; 7/8 treatment trials were a single `whatif`/`callers` call.

## Bottom line

Yes — the saving is real in wall-clock, not just tokens: **~1.8× faster at the median** (30s → 16.5s)
with identical perfect completeness, on the easiest fixture for the manual arm. The mechanism is
serial-round elimination (control: 2–3 read/trace rounds; treatment: 1 query). And the honest caveat
cuts the other way for the headline claims: a frontier model asked the explicit blast-radius question
gets it right from source — candor's completeness value is for *implicit* propagation awareness and
weaker models; its speed/cost value is what remains for explicit questions.
