# Agent-use eval — WEAKER-MODEL probe (Experiment A3). Pre-registration.

Committed **before** any A3 trial runs. A2 (Sonnet, hard fixture) showed: agents reach for candor 10/10
and candor gives a significant *completeness* lift (full 16-fn radius, 9/10 vs 0/10), but the *decision*
was at ceiling (10/10 both) — a frontier model traces a 16-function graph well enough to decide right
without candor. The honest reading: what candor can resolve statically, a strong model can usually trace
by hand; candor's *decisive* edge should appear where manual tracing degrades. The most decision-relevant
such regime is the **weaker / cheaper model** population (the agents many teams actually run at scale).

**A3 holds the fixture and task fixed and changes one variable: the model — Sonnet 4.6 → Haiku 4.5.**

## Fixture / task / arms / metrics

Identical to A2 (`PREREG-hard.md`): the 7-file crate, `tax::apply_tax` with a 16-function blast radius and
the must-stay-pure `realtime::run_stream` in a separate subtree; the add-logging-then-decide task;
control (no candor) vs treatment (candor available via `AGENTS.md` + the logging `./candor` shim); graded
by `grade-hard.py` (blast_recall, missed_run_stream, placement_correct, runstream_broken, used_candor).

## Model & N (fixed): **Haiku 4.5**, K = 10 per arm. Fixed-K.

## Hypotheses & analysis (fixed)

1. **Does the weaker model still reach for candor?** Adoption rate in treatment (an open question — weaker
   models may under-use available tooling, itself a finding about *how well* agents use candor).
2. **Does control degrade vs the Sonnet A2 baseline?** If Haiku control's `blast_recall`/`placement_correct`
   drop (more missed callers, some broken `run_stream`), the graph is now beyond comfortable manual reach.
3. **Does candor lift the weaker model?** Treatment vs control on `blast_recall`, `placement_correct`
   (Fisher's exact), `missed_run_stream`; and within-treatment candor-users vs non-users.

## Interpretation, decided in advance

- **Haiku control degrades AND treatment recovers** → candor is a real equalizer: it delivers the
  model-independent blast radius a weaker agent can't reliably trace, lifting the *decision*, not just
  completeness. The active-tool value generalises beyond frontier models — the strongest pro-candor result.
- **Haiku ceilings too** (control already high) → even a cheaper model handles a 16-fn graph; candor's
  blast-radius value needs a larger-than-context regime, not just a weaker model. Reported plainly.
- **Haiku doesn't use candor / can't do the task** → a weaker agent under-leverages the tool (a real
  "how well do agents use candor" finding) and/or the task floors. Reported as the honest outcome.

## Limitation

One weaker model, one task, K=10. Probes whether the A2 ceiling is model-strength-bound. Whatever it
shows is the result; no fixture tuning toward a desired outcome.
