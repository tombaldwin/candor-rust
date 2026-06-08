# Agent-use eval — HARD re-run (Experiment A2). Pre-registration.

Committed **before** any A2 trial runs. Experiment A found 10/10 adoption but no outcome lift, for two
reasons: (1) the fixture was within Sonnet's manual tracing ability (ceiling), and (2) — the real one —
candor's `callers` returned nothing for a pure function, so it didn't answer the agents' question. Both
are now addressed: `callers <fn>` returns the full transitive blast radius for any function (commit
994dd10), and this fixture is **hard enough that manual tracing should fail**. A2 asks: with candor now
answering the question, on a task beyond comfortable hand-tracing, does using candor **lift** the outcome?

## Fixture (`eval/agentuse/fixture-hard/`)

A 7-file crate. `tax::apply_tax` is pure and has a **16-function transitive blast radius**. Crucially,
the must-stay-pure function `realtime::run_stream` (a per-market-tick loop, sub-ms budget, documented
no-I/O) reaches `apply_tax` through a **separate subtree** (`realtime → pricing::priced`), off the big
invoice/report/api/batch branch a hand-tracer naturally follows. Adding `Fs` to `apply_tax` propagates
to all 16 — breaking `run_stream`. candor's `callers apply_tax` returns all 16 (incl. the realtime three)
instantly, on still-pure code.

## Task (identical both arms; same shape as Experiment A)

Add file logging to `tax::apply_tax`; first list the blast radius (`BLAST.txt`); decide whether it's safe
or must be relocated (`DECISION.txt`: `pricing`/`tax` vs `relocated: <where>`); make the change; keep the
signature. Correct outcome: **relocate** (keep `Fs` out of the `run_stream`-reachable subtree —
`tax.rs`/`pricing.rs`/`realtime.rs`), because the blast radius includes the I/O-free `run_stream`. The
task never mentions candor.

## Arms (differ ONLY in candor availability — identical to Experiment A)

- **control** — generic `AGENTS.md`; no candor.
- **treatment** — `AGENTS.md` notes candor exists + commands; `./candor` shim present (logs every call).

## Model & N (fixed): Sonnet 4.6, **K = 10 per arm**. Fixed-K, no peeking.

## Metrics (`grade-hard.py`; objective, ground truth hand-verified, NOT read from candor)

- **blast_recall** — of the 16 transitive callers, how many listed.
- **missed_run_stream** — did they miss the dangerous realtime caller.
- **placement_correct** — `Fs` stayed out of the `run_stream`-reachable subtree AND logging was added
  (i.e. relocated safely). The shipped-code metric (grep).
- **runstream_broken** — `Fs` landed in `tax.rs`/`pricing.rs`/`realtime.rs` (the failure).
- **used_candor** / commands (treatment) — adoption.

## Analysis (fixed)

1. **Treatment vs control** on `blast_recall` (mean), `placement_correct` (Fisher's exact), and
   `missed_run_stream` (rate).
2. **Within-treatment counterfactual**: candor-users vs non-users on the same metrics.

## Interpretation, decided in advance

- **Treatment materially better than control** (higher recall, fewer `run_stream` misses, fewer broken
  placements) → candor delivers real outcome lift once it answers the question, on a task where hand-
  tracing fails. The active-tool value is demonstrated, not just adopted.
- **No difference** → either the task is still within hand-tracing ability (then control's recall/
  placement will also be high — a ceiling, reported as such), or agents didn't act on candor's output
  (a usage-quality finding → Experiment B). Reported honestly either way.
- **Control already near-perfect** → Sonnet hand-traces even this; candor's lift is small for this model
  and we say so (its value would be at larger scale / weaker models).

## Limitation

Single model, single task, K=10. A lift probe, not a characterisation. Whatever it shows is the result;
no further fixture tuning toward a desired outcome.
