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

---

## Red-team addendum (self-review, same day)

The result was given a deliberate kicking. What survives, what's bruised:

**1. CORRECTION — the marginal-ratio gloss was wrong.** The body says the marginal token cost
(~5,555 vs ~58) is "consistent with the per-question ~17×". It isn't — that ratio is **~96×**, which
sits *above* token-cost's typical band (1–30×) for a fixture this small/distinctive. Likely cause:
the two measurements count different quantities — token-cost's grep-trace estimator counts the *text
an agent must ingest*, while the marginal here also includes the agent's reasoning/output tokens for
the trace. Favorable to candor, but mis-stated precision is mis-stated precision.

**2. Circularity in treatment completeness.** The ground truth is candor-derived, and 7/8 treatment
agents returned candor's output verbatim — treatment's 16/16 is near-tautological. It is rescued
**only** by the control arm: 8/8 *independent source-only traces* converged on exactly the same 16,
which is simultaneously the strongest external validation of candor's answer this eval produced.
Treatment completeness should be read as "tool trusted and tool correct," not "agent verified."

**3. Best-case adoption.** The treatment prompt handed the agent the literal commands. This measures
the tool's ceiling, not discovery-and-adoption in the wild (the 33.1s outlier — an agent exploring
before querying — hints at the variance the spoon-feeding suppressed). Deployment-realistic speed
sits somewhere between the arms.

**4. Measurement conditions.** Durations were taken under 8-way concurrent agent load (balanced
across arms within each batch, so the *ratio* is fair, but absolute seconds are not isolated-run
times). A visible batch effect (batch 2 faster in both arms) suggests warm caches.

**5. Statistics.** Exact U = 59/64 control>treatment pairs; permutation test on the median gap
(13.5s) gives **p ≈ 0.036**. Significant, but N=8/arm on ONE fixture in two correlated batches —
adequate for the direction, thin for the magnitude.

**6. The control-100% finding inherits fixture cleanliness.** "A frontier model traces correctly when
explicitly asked" was demonstrated on a synthetic, perfectly-layered, distinctively-named 10-module
crate with no distractors. It does NOT establish that control stays at 100% on real codebases (common
names, dead code, macro noise) — where the speed gap should also widen. Both headline numbers are
easy-case numbers.

**7. Tool footnote the fixture masked.** `whatif pricing::Pricing::quote` substring-matched
`quote_bulk` too, seeding the radius from both. Harmless here (quote_bulk is in quote's radius
anyway) but on other names whatif's substring matching could inflate a blast radius — worth an
exact-match flag on the tool.

**8. Token accounting.** `subagent_tokens`' exact composition (input/output/cache split) was not
verified; within-metric ratios are sound, absolute interpretations less so.

**Post-kicking verdict.** The defensible sentence shrinks to: *on the easy case, under best-case
adoption, candor answered ~1.8× faster (p≈.04) at equal completeness, by replacing a 2–3-round serial
trace with one query — and the 8/8 control convergence independently validated the report itself.*
The magnitude is condition-specific; the mechanism and direction are solid; the strongest new fact is
arguably the control arm validating candor's ground truth, not the speedup.
