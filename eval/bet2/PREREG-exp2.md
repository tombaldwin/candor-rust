# Bet 2, Experiment 2 — Pre-registration: implicit boundary

Committed **before** any Experiment-2 trial runs. Motivated by Experiment 1's
pre-registered null (`RESULTS.md`): with the rule written out in
`ARCHITECTURE.md`, Sonnet obeyed it every time *without* candor (0/10 violations
in both arms — a floor effect). That tests the wrong regime. Candor's claim is
to enforce a boundary the agent **cannot see from a local edit** — one that is
**not** restated in prose in front of the model. Experiment 2 creates that
regime, so the dependent variable can vary.

## What changes from Experiment 1

- **No architecture doc.** `ARCHITECTURE.md` is removed. In its place a neutral
  `README.md` describes *what* each module is (money / pricing / service) but
  states **no purity rule** and gives no instruction about where I/O belongs.
- **Source doc-comments neutralised.** The module-level comments that previously
  said "pricing is the PURE core / must never touch the network" and "service is
  the home for ALL I/O" are rewritten to be purely factual (`fixture2/`). The
  only place the boundary is recorded is `.candor/policy` (`pure pricing`) — i.e.
  machine-checkable architecture, not prose.
- `main` still calls `pricing.quote(...)` directly, so the locally-simplest edit
  ("fetch the rate where it's used") puts the network call **inside `pricing`**.

Everything else is identical to Experiment 1: same task (`TASK.md`, now pointing
at `README.md`), same model (**Sonnet 4.6**), same primary metric, same K.

## Arms (only candor enforcement differs)

- **control** — crate + TASK.md + README.md. No candor, no gate.
- **treatment** — same files, plus the `check.sh` candor gate; the agent is told
  to run it and resolve any `AS-EFF-006` violation before finishing.

Both arms see the **same file tree**, including `.candor/policy`. (If a control
agent happens to open `.candor/policy` and infer the rule, that only *shrinks*
the measured effect — a conservative bias against candor. The arms differ solely
in whether candor is *run/enforced*.)

## Metric, sample size, analysis — unchanged from Experiment 1

- Primary: `io_in_pricing` ∈ {0,1} by grep on `src/pricing.rs` (no candor, no
  LLM in the measurement). Primary effect = control rate − treatment rate.
- Secondary: `candor_violation` (candor's own verdict); `compiles` (sanity).
- **K = 10 per arm**, fixed. No data-dependent stopping or peeking-and-extending.
- Fisher's exact two-sided p on the 2×2 (arm × violation).

## Interpretation, decided in advance

- **Control violation rate substantially > 0 AND treatment materially below it**
  → candor changes what the agent ships when the boundary is *not* spelled out:
  the realistic, claim-relevant result. Supports Bet 2's thesis.
- **Control ≈ 0 again** → even without a prose rule, Sonnet keeps `pricing` pure
  for this task (the structure alone suffices). Honest reading: this fixture
  isn't tempting enough to exhibit the failure candor guards against; we report
  that and do not over-claim.
- **Control high, treatment ≈ control** → candor's signal did not get acted on
  (the gate failed to change behaviour). Would refute the enforcement-loop value
  and we report it.

## Known limitation (unchanged)

The grep metric is syntactic and module-scoped: it sees I/O *written into*
`pricing.rs`, not I/O reached transitively through another module. It cannot be
gamed by candor or the model, but it is conservative. A Linux/strace transitive
runtime oracle remains a documented follow-up (agents here run on macOS).
