# Agent-use eval — HARD re-run (Experiment A2) — Results

See `PREREG-hard.md` (committed before the run). The re-run with `callers` fixed (commit 994dd10) and a
16-function blast radius across 7 files, with the must-stay-pure `run_stream` reachable only through a
separate realtime subtree. Sonnet 4.6, K=10/arm.

## Headline

| metric | control | treatment |
|---|---|---|
| **used candor** | n/a | **10 / 10** |
| **complete blast radius (16/16)** | **0 / 10** | **9 / 10** |
| blast recall (mean of 16) | 0.938 | 0.994 |
| placement correct (relocated; `run_stream` kept pure) | 10/10 | 10/10 |
| broke `run_stream` | 0/10 | 0/10 |
| missed the dangerous `run_stream` | 0/10 | 0/10 |

## 1. Adoption is total — and now sophisticated

10/10 treatment agents used candor again, and the **usage matured** now that it answers the question:
they ran `candor callers apply_tax` (the full blast radius), several then ran `callers line_total` /
`callers priced` to **vet the relocation target** ("is the place I'm moving the logging to also on the
realtime path?"), and ran `audit` again **after** the edit to confirm `run_stream` stayed pure. That's
candor used as a reasoning instrument across the whole change, not a single lookup.

## 2. A real, significant completeness lift…

Treatment listed the **complete** 16-function blast radius **9/10** times; control **0/10** (Fisher's
exact **p = 0.00012**). candor reliably surfaces the full set; hand-tracing systematically under-counts.

## 3. …but the missed function was `main`, and placement was at ceiling

The honest detail: **every** control agent missed exactly one function — `main`, the entry point agents
reflexively skip — and **nothing else**. Every control agent caught all 15 substantive callers,
*including the buried realtime branch* (`spot_quote`/`stream_tick`/`run_stream`) and the dangerous
`run_stream` itself. So a 16-function, 7-file, separate-subtree blast radius is **still within a frontier
model's manual tracing ability.** And the shipped decision was at ceiling: **10/10 in both arms**
relocated correctly and kept `run_stream` pure. candor's completeness lift (catching `main`) did not
change the outcome, because the one function it added over hand-tracing was the harmless one.

## Honest conclusion

- **Agents reach for candor 100% of the time and use it well** (multi-step: vet the target, vet the
  destination, verify after) — the active-tool framing is robustly validated on adoption and usage
  quality.
- **candor delivers a measurable, statistically-significant completeness lift** (9/10 vs 0/10 complete) —
  it gets the *whole* blast radius where hand-tracing drops the entry point.
- **But for a strong model on a tractable graph, that lift didn't change the decision** — Sonnet traces a
  16-function graph (including a buried sub-branch and the critical function) well enough to decide
  correctly without candor. candor's *decisive* value therefore lives where this experiment's ceiling
  lifts: a **weaker model**, a **larger** propagation than one context comfortably holds, or a case where
  **the systematically-missed function is the critical one** (here it happened to be harmless `main`).

This squares with the rest of the evidence: the scaled eval — bigger propagation, candor's diff handed in
— moved completeness from 6% to 79–100%; here, smaller and within reach, candor moved completeness 0/10→
9/10 but not the decision. candor's lift scales with how far the consequence outruns what the model can
hold; at a frontier model's comfortable range it's real but marginal.

## Limitations

Single model, single task, K=10. The placement metric hit its ceiling (10/10 both), so this run measures
the *completeness* lift, not a decision lift — and shows, honestly, that a frontier model doesn't need
candor to get a 16-function blast radius's *decision* right. A decision-level lift would need a harder
regime (weaker model, or a graph where the overlooked function is load-bearing) — a clean next probe, but
one this experiment deliberately doesn't claim.
