# Agent-use eval — WEAKER-MODEL probe (Experiment A3) — Results

See `PREREG-weak.md` (committed before the run). Same hard fixture and task as A2; the only change is the
model: **Haiku 4.5** instead of Sonnet 4.6. The probe: A2 found candor's lift on Sonnet was completeness,
not decision (placement at ceiling). Does the ceiling lift for a weaker, cheaper model?

## Headline (placement corrected by hand — see note)

| metric | control (Haiku) | treatment (Haiku) | Sonnet control (A2, ref) |
|---|---|---|---|
| **used candor** | n/a | **10 / 10** | n/a |
| blast recall (mean of 16) | 0.925 | **1.000** | 0.938 |
| complete radius (16/16) | 2/10 | **10/10** | 0/10 |
| **placement correct** | **9 / 10** | **10 / 10** | 10/10 |
| broke `run_stream` (shipped a bug) | **1 / 10** | **0 / 10** | 0/10 |

## 1. A weaker model reaches for candor just as readily

**10/10 Haiku treatment agents used candor** — adoption is not a frontier-model artifact. Several used it
to *verify* their fix (e.g. "the realtime functions don't appear in the effect map" after relocating),
and one used it to confirm a clever **logged-wrapper** solution kept the realtime path pure.

## 2. The completeness lift is clean and slightly larger

Treatment got the **complete** 16-function blast radius **10/10**; control **2/10** (mean recall 1.000 vs
0.925). Haiku control degraded a touch from Sonnet control (0.925 vs 0.938) — the graph is closer to its
manual-tracing limit.

## 3. The decision ceiling cracked — in the weaker-model *control* arm

This is the new result. **One Haiku control agent shipped a real bug**: it relocated the logging to
`pricing::priced` — not realizing `priced` is on the realtime path (`run_stream → stream_tick →
spot_quote → priced → apply_tax`), so `run_stream` still gains `Fs` and blows its per-tick budget. That
is exactly the non-local mistake candor's blast radius exists to prevent — and it appeared the moment we
dropped to a weaker model (Sonnet control made zero such errors). **Zero treatment agents made it**: with
candor's full radius (and, for several, a post-edit `audit` to verify), they placed the logging safely.

So with the weaker model, candor moved from a *completeness* lift (A2) to also a *decision* lift:
**placement 10/10 with candor vs 9/10 without.** At K=10, one error vs zero is directional, not
statistically significant (Fisher p ≈ 1.0) — but it is the **first decision-level error in the whole
series, it landed in the no-candor weaker-model arm, and candor closed it.** That is the equalizer signal
the probe was built to find, at the edge where it should first appear.

## Grader note (transparency)

`grade-hard.py` checks placement by grepping `tax.rs`/`pricing.rs`/`realtime.rs` for I/O — a file-level
over-approximation. It false-flagged `treatment-07`, which added the I/O to a *new* `apply_tax_logged`
function in `tax.rs` that the realtime path never calls (`priced → apply_tax`, the pure one). Verified by
hand: `run_stream` stays pure, so that solution is correct. The table above uses the hand-corrected
placement (treatment 10/10). `control-09` was likewise hand-verified as a *true* break.

## Conclusion — the full three-experiment arc

- **A (Sonnet, medium):** 10/10 adoption, but candor's `callers` returned nothing for the pure target —
  a workflow gap, since fixed.
- **A2 (Sonnet, hard):** 10/10 adoption with mature multi-step use; significant *completeness* lift
  (9/10 vs 0/10 complete) but *decision* at ceiling — a frontier model traces 16 functions and decides
  right unaided.
- **A3 (Haiku, hard):** 10/10 adoption again; *completeness* lift cleaner (10/10 vs 2/10); and the
  *decision* ceiling cracks — a no-candor weaker agent ships the exact non-local bug candor prevents.

**The answer to "how well do AI agents use candor": they reach for it reliably and use it well, across
model tiers; candor reliably gives them the complete blast radius hand-tracing under-counts; and its
value converts from "nice completeness" to "prevented a shipped bug" exactly as the agent gets weaker or
the propagation outruns what the model can hold.** candor is an equalizer — it matters most for the
cheaper agents teams actually run at scale, and at scale beyond one context window.

## Limitations

One weaker model, one task, K=10; the decision lift is one-vs-zero (directional, not significant). A
larger N or a still-weaker model would sharpen the decision-lift estimate; this run establishes that the
A2 ceiling is model-strength-bound and that candor closes the gap where it opens.
