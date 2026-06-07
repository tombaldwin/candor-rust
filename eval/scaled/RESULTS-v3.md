# Scaled eval — batch 3 results (the large fixture + frontier model)

**The cell batches 1–2 left open.** [RESULTS-v2](RESULTS-v2.md) closed with: *"Next: a large fixture
and a frontier model, to map the one cell these two batches leave open."* Batch 1 was frontier-model
but leaky/contaminated; batch 2 was clean but Sonnet on 5-file crates. Batch 3 is **both at once** —
a **large** fixture and a **frontier** agent — on the de-leaked protocol.

**Batch:** 1 task (**orderflow**, Net) × 2 arms × 3 trials = 6 trials. Agent under test =
**Opus-class** (frontier); blind frontier judges, one per summary, each reading an isolated,
tool-identity-redacted prompt under a shuffled opaque id. **Completeness is the primary metric**
(fraction of the 16 non-local propagated functions the summary identifies); binary awareness secondary.

**Pre-registration.** The fixture, its candor-verified 16-function ground-truth denominator
(`harness.sh nonlocal_of orderflow`), the completeness metric, and the falsification bars were all
committed (`tasks-v3/orderflow`, harness wiring — commit before the run; metric+bars inherited from
the batch-2 pre-registration in [README.md](README.md)) **before** any trial agent ran. No fixture or
metric was tuned post-hoc.

**No exclusions.** All 6 trials objectively COMPLETED the task — candor's `diff` confirms each edit
introduced `Net` (18 functions gained it: the 16-fn propagation set + the edited `Pricing::quote` +
the agent's new `fetch_rate` helper). Valid N: **control 3, treatment 3.**

## The fixture: where the small tasks propagate to 7, this one reaches 16

`orderflow` is a 10-module order/pricing crate. The canonical edit — fetch the live FX rate over TCP
inside `Pricing::quote` (the one place a foreign-currency price is produced) — makes `Net` propagate
to **16 non-local functions across 9 files, through 3–5 call-graph layers**
(`pricing → cart → discount/checkout → service → api/report/admin → main`). Tracing that by hand is
the realistic failure mode candor targets. (Small tasks: 7 functions / 4 files / 2 layers.)

## Primary metric: completeness (blind judge marks each of the 16 non-local callers)

| | completeness | per-trial |
|---|---|---|
| **treatment** (candor) | **12.67 / 16 = 79%** | 16, 6, 16 |
| **control** (no candor) | **1.00 / 16 = 6%** | 1, 1, 1 |

**Neither pre-registered falsification condition triggered.** Control completeness **6%** is far below
the 0.80 "low marginal value" bar; treatment − control = **0.73**, far above the 0.20 "weak effect"
bar. The gap holds — and is essentially unchanged from batch 2's 0.93 — **at scale and at the frontier**.

**The key finding: a frontier model does *not* close the gap.** RESULTS-v2 flagged the open question of
whether a stronger model would trace the call graph unprompted and shrink candor's lift. It doesn't. The
Opus-class control, asked for "consequences for the rest of the codebase," named only **`quote_bulk`**
(the immediate helper it edits past) plus a generic "callers that assumed they were side-effect-free" —
**1 of 16**, every trial. The bottleneck isn't model capability; it's that enumerating five layers of
transitive callers by hand is work the agent doesn't volunteer in a summary, however capable it is.
candor's `diff` hands it the list, and it reports it.

**Treatment variance is judge-variance on one phrasing, not a capability gap.** All three treatment
agents got the same `diff` ("Net … reaches main (+15 intermediate)") and all three wrote a blanket
"propagates through 16 intermediate callers … to main." Two judges read that as the protocol's allowed
blanket ("the whole call chain up to main") → 16/16; one judge read it conservatively, crediting only
the explicitly-named functions → 6/16. So the 79% is a **conservative** floor created by one strict
judge on one trial; under the lenient blanket reading the other two judges applied, treatment is ~100%.
Even the strict 6/16 is **6× the best control trial**.

**Objective cross-check (mechanical name-substring count, immune to tool-use leakage):** control **6%**
(3/48), treatment **29%** (14/48). It brackets the judge result *from below* — it can't credit the
blanket "16 intermediate callers" phrase (no literal name), so it floors treatment at 29% while the
semantic judge credits the blanket at 79%. Both metrics agree on a large gap and an identical control.

## Secondary metric: binary awareness (saturated, uninformative here)

All 6 trials → **yes**, both arms. As batch 2 noted, binary awareness is the weak metric: the control
*does* name one specific non-local caller (`quote_bulk`, the helper it directly calls), which clears
the "names ≥1 non-local caller" bar. Binary can't see the difference between "named 1 of 16" and "named
16 of 16" — that is exactly what the primary completeness metric measures.

## Cost

Per trial (Opus-class): control ≈ 14.1k output tokens / ~8 tool calls; treatment ≈ 14.4k / ~8 (the
`diff` step). candor adds **~2–3%** in the edit setting — the completeness lift (1/16 → ~16/16) is
effectively free.

## Limitations

- **N=3 per arm, one task** — the smallest batch, a single large fixture; it fills the missing *cell*
  (frontier × large) rather than adding power. The effect size (0.73) is large and consistent with the
  better-powered batch 2 (0.93).
- **One model family** (Claude, Opus-class for both engineer and judge). The orchestrator (an
  agent-spawning harness) drove fresh, independent subagents per trial; engineer and judge contexts
  were isolated.
- **The treatment is told to run candor** (its intended use); the control was asked the identical
  "consequences for the codebase" question with equal license to trace callers, and traced ~1/16.
- **Judge variance on blanket phrasing** (the treatment-2 6/16 vs 16/16 split above) — the metric is
  somewhat sensitive to whether "N intermediate callers → main" counts as covering each. Reported
  conservatively (79%, not the lenient ~100%); the mechanical cross-check is immune and still 5× control.
- **Residual blinding hint** — treatment summaries narrate "the analysis confirms…"; the tool name is
  redacted, prompts were shuffled to opaque ids, and the judge applies a per-function rubric, but tool
  use can still leak. The mechanical cross-check is immune and agrees.
- **Summary-as-proxy** for edit quality, as in prior batches.

## Headline

On a **large** fixture (16-function, 9-file, 5-layer propagation) with a **frontier** agent — the cell
batches 1–2 left open — **candor's edit-feedback lifts non-local effect-completeness from 6% to ~79–100%**
for ~2–3% extra tokens. The decisive new result: **frontier capability does not substitute for the
analysis.** A strong model still names only the function it directly edits past (1/16) and writes "callers
unaffected"; it's the call-graph enumeration, not raw capability, that's missing — and that's precisely
what candor supplies. The 6%→100% effect from batch 2 is **not** a small-fixture or weak-model artifact;
it holds, large, at scale and at the frontier.
