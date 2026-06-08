# Agent-use eval (stable backend) — Pre-registration

Committed **before** the treatment/control trial runs below. Question: *does the **stable** backend
(`candor-scan`, no nightly/dylint) deliver the same behavior-changing signal — and the same agent
outcome — as the nightly lint did in the original Experiment A ([RESULTS.md](RESULTS.md))?* The stable
scanner under-reports vs the lint (no `Unknown`; misses some method/macro/cross-crate effects), so the
worry is that the friction-killer is hollow: zero-install but value-free.

Same fixture, task, and grader as Experiment A ([PREREG.md](PREREG.md)) — only the **backend** changes.

## Part 1 — Signal-equivalence (deterministic, the gate)

The agent-facing interface is `./candor callers <fn>` / `where <Effect>` / `audit`, all rendered by
`candor-query` reading the report — so an agent cannot tell which backend produced it. The only question
is whether the stable backend produces the *same report* on this fixture.

**Result (run before the agent trials, on `eval/agentuse/fixture`):** PASS.
- `candor callers compute_price` (stable) → all **6** ground-truth functions, including
  `monitoring::health_probe (direct)` — the one this fixture is built to make easy to miss.
- After adding `Fs` to `compute_price`, `candor where Fs` (stable) → the effect propagated to all 6.

This is byte-identical to the lint's output (both flow through `candor-query`), so the treatment agent
receives the **same input** either way. Part 2 confirms agents *behave* the same.

## Part 2 — Agent behavior (confirmation)

Two arms, run now under identical conditions (same model, same fixture):

- **control** (N=8): `AGENTS-control.md` (no candor) — must trace the blast radius by reading code.
- **treatment-stable** (N=8): `AGENTS-treatment.md` + the `./candor` shim **forced to the stable
  backend** (`CANDOR_BACKEND=scan`, so the lint is never used even though it's installed).

Task (identical, from PREREG.md): add file-based `Fs` logging to `pricing::compute_price`; first
determine the blast radius (`BLAST.txt`, one fn/line); decide whether it's safe there or must be
relocated (`DECISION.txt`); `health_probe` must stay I/O-free, so the correct action is to **relocate**.

### Metrics (grade.py, none consult candor's live output)

- `blast_recall` — of the 6 transitively-affected functions, the fraction listed in `BLAST.txt`.
- `missed_health_probe` — did they miss the dangerous one.
- `pricing_pure` — did the shipped `src/pricing.rs` stay `Fs`-free (correct = relocate the logging).
- `used_candor` — treatment only, from the shim usage log.

### Decision rule (pre-registered)

The stable backend **preserves the value** iff, replicating the original lint result's direction:
1. `treatment-stable` mean `blast_recall` ≥ 0.90 **and** materially exceeds `control`; and
2. `treatment-stable` `pricing_pure` rate ≥ `control` (the shipped-code/decision metric); and
3. ≥ 7/8 treatment agents actually invoke candor (adoption holds on the stable path).

Anchors for comparison (prior run, nightly lint, N=10, [RESULTS.md](RESULTS.md)): treatment
`blast_recall` ≈ 1.00 vs control ≈ 0.07. We expect `treatment-stable` ≈ the lint treatment.

Results: [RESULTS-stable.md](RESULTS-stable.md), [results-stable.tsv](results-stable.tsv).
