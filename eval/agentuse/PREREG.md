# Agent-use eval (Experiment A) — Pre-registration

Committed **before** any trial runs. Question: *when candor is **available** as a tool but not
**instructed**, do AI agents reach for it on a task where it would help — and does using it improve the
outcome?* Every prior candor eval removed the agent's choice (forced to read candor JSON; handed the
diff; told to run the gate). This one leaves the choice to the agent and measures it.

## Fixture (`eval/agentuse/fixture/`)

A 4-module Rust crate. `pricing::compute_price` is **pure** and is transitively called by exactly six
functions across the other files: `line_item`, `render_invoice`, `monthly_report`, `export_csv`,
`health_probe`, `main` (hand-verified from the call graph — this is the ground-truth blast radius).
`monitoring::health_probe` is documented as needing to stay I/O-free (a per-health-check latency
budget); critically, it calls `compute_price` **directly from its own module**, so it's easy to miss
when tracing the invoice chain by hand. Adding a filesystem effect to `compute_price` propagates `Fs`
to all six — breaking the health probe.

## Task (identical in both arms)

> Add file-based debug logging (write the SKU to `/tmp/pricing.log`) to `pricing::compute_price`. First
> determine the **blast radius**: every function that would transitively gain a filesystem effect.
> Then decide whether it's safe to add the logging there; if it breaks a constraint anywhere in the
> blast radius, put the logging somewhere safe instead. Deliverables in the project root: `BLAST.txt`
> (one affected function per line) and `DECISION.txt` (`pricing` or `relocated: <where>`). Make the
> change; keep `compute_price`'s signature. Read `AGENTS.md` first.

The task **does not mention candor.** The correct outcome is to keep `pricing` pure (relocate the
logging), because the blast radius includes the I/O-free `health_probe`.

## Arms (differ ONLY in candor availability)

- **control** — `AGENTS.md` lists generic conventions; no candor.
- **treatment** — `AGENTS.md` additionally states candor is installed and what it does (effect map +
  transitive propagation across the call graph), with the commands (`./candor audit|callers|map|diff`).
  A `./candor` shim is present that runs real candor and **logs every invocation** (objective adoption
  measure — not a self-report). The agent is told candor exists; it is **not** told to use it here.

So the agent *knows* candor is available; whether it *chooses* to use it for this effect-tracing task is
the measurement.

## Model & sample size (fixed)

- Model: **Sonnet 4.6** (consistency with the Bet 2 eval).
- **K = 10 per arm** (20 agent runs). Fresh fixture copy per trial; isolated directories. Fixed-K, no
  data-dependent stopping.

## Metrics (objective; `grade.py`)

- **adoption** (treatment): fraction of agents that invoked candor at all (from the usage log), and
  which commands.
- **blast_recall**: of the 6 ground-truth transitive callers, how many the agent listed in `BLAST.txt`
  (the *awareness* metric). Plus `missed_health_probe` (the dangerous miss) and `direction_err` (wrongly
  listing a *callee* — `base_price`/`margin` — a call-direction confusion).
- **pricing_pure**: did `src/pricing.rs` stay free of I/O syntax (the *shipped-code* metric — correct =
  relocated). Computed by grep; consults neither candor nor an LLM.
- **compiles**: sanity.

The blast-radius ground truth is hand-verified from the source, **not** read from candor's output — so
a candor-using agent is not graded against its own input (the methodological trap `EVAL.md` flags).

## Analysis (fixed)

1. **Adoption rate** in treatment (the headline: do agents reach for candor?).
2. **Treatment vs control** on `blast_recall` and `pricing_pure` (mean per arm; Fisher's exact for the
   binary `pricing_pure`).
3. **Within-treatment counterfactual**: candor-**users** vs candor-**non-users** on `blast_recall` and
   `pricing_pure` — controls for the task difficulty (same arm, same prompt), isolating the effect of
   actually using the tool.

## Interpretation, decided in advance

- **High adoption AND candor-users outperform** → agents reach for candor and it helps; the active-tool
  framing (MCP / slash command / `cargo candor`) is validated.
- **Low adoption** → agents don't spontaneously use candor even when told it's available; candor's
  value should be delivered passively (the Stop hook / CI gate), and the active-tool surface is
  secondary. A real, deflating-but-useful finding.
- **High adoption but no outcome difference** → agents use candor but it doesn't change results here —
  either the task is too easy or they misread the output (→ run Experiment B, the interpretation audit).

## Known limitations

Single model, single task, K=10 — an existence/rate probe, not a characterisation. The shim makes
adoption objective, but "used candor" counts any invocation; whether each use was *well-timed* is read
qualitatively from the logged command sequence, not scored.
