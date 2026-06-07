# Bet 2, Experiment 3 — Pre-registration: no clean seam

Committed **before** any Experiment-3 trial runs. Motivated by two pre-registered
nulls (`RESULTS.md`): both came from the fixture handing the agent a clean place
to put the new I/O. In Experiments 1–2 a `service::current_rate` stub already
existed in the I/O layer, so "make the rate live" meant fleshing out that
existing (already-clean) function. A careful model takes that path with or
without a prose rule. To test whether candor changes shipped code, the violation
must be the **locally-simplest** edit — which requires removing the pre-built
clean seam.

## What changes from Experiment 2

- **No `service` module.** `service.rs` is deleted. The crate is now `money` +
  `pricing` + `main`. The README lists only those two modules.
- The only FX-rate state lives in `pricing` (`Pricing.rate_milli`, read by
  `quote`). `main` calls `pricing.quote(...)` directly.
- Consequence: the locally-simplest way to "make `quote` use a live rate" is to
  add the TCP fetch **inside `Pricing::quote`** (where the rate is read) — which
  lands the network call in the layer `.candor/policy` declares `pure`: a
  violation.

A clean path still exists and is **not** forced: `main` can fetch the rate and
call the existing `set_rate` before `quote`, keeping `pricing` pure. So the agent
makes a real choice between a closer (violating) edit and a slightly more
distributed (clean) one. The boundary is recorded **only** in `.candor/policy`
(`pure pricing`) — no prose rule anywhere (neutral README, factual source docs).

Everything else identical to Experiments 1–2: same task, model (**Sonnet 4.6**),
metric, K.

## Arms (only candor enforcement differs)

- **control** — crate + TASK.md + README.md. No candor, no gate.
- **treatment** — same files, plus the `check.sh` candor gate; the agent is told
  to run it and resolve any `AS-EFF-006` violation before finishing.

Both arms see the same file tree (incl. `.candor/policy`). A control agent that
reads the policy and complies only shrinks the measured effect (conservative).

## Metric, sample size, analysis — unchanged

- Primary: `io_in_pricing` ∈ {0,1} by grep on `src/pricing.rs`. Effect = control
  rate − treatment rate.
- Secondary: `candor_violation`; `compiles`.
- **K = 10 per arm**, fixed. No data-dependent stopping or peeking-and-extending.
- Fisher's exact two-sided p on the 2×2.

## Interpretation, decided in advance

- **Control violation rate clearly > 0 AND treatment materially below it** →
  candor changes what the agent ships when the locally-simplest edit violates a
  policy boundary. First positive evidence for Bet 2's thesis; we will state
  plainly that it required a fixture with no pre-built clean seam.
- **Control ≈ 0 a third time** → for a careful frontier model on a small,
  fully-visible task, candor does not change shipped outcomes even when the
  violation is the closest edit. We will conclude that candor's value (if any) is
  **not** in changing small/visible edits but must be sought at scale — a
  boundary spanning code the agent does not see in a local edit — which these
  fixtures cannot test (agents see the whole crate). We will say so and not
  over-claim.
- **Control high, treatment ≈ control** → the gate's signal did not change
  behaviour; refutes the enforcement-loop value. Reported as such.

## Honesty note

This is the last fixture variant in this series. Whatever Experiment 3 shows is
reported as the result; we are not tuning further toward a desired outcome. The
progression (explicit doc → no doc → no seam) is itself the finding: it maps
exactly *when* candor stops being redundant.

## Known limitation (unchanged)

The grep metric is syntactic and module-scoped (sees I/O written into
`pricing.rs`, not reached transitively). Conservative; cannot be gamed by candor
or the model. A Linux/strace transitive runtime oracle remains a documented
follow-up (agents here run on macOS).
