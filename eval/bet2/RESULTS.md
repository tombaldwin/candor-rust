# Bet 2 — Results: does candor change the code an agent SHIPS?

See `PREREG.md` for the pre-registered design (committed before any trial ran).

---

## Experiment 1 — explicit architecture doc (pre-registered)

**Design as registered:** identical task/fixture/model across two arms; the only
difference is candor enforcing `.candor/policy` (`pure pricing`) in the treatment
arm. Primary metric: `io_in_pricing` (grep for I/O syntax in `src/pricing.rs` —
no candor, no LLM in the measurement). Model: Sonnet 4.6. K = 10 per arm.

**Result:**

| arm        | n  | compiles | io_in_pricing (violation) | candor_violation |
|------------|----|----------|---------------------------|------------------|
| control    | 10 | 10/10    | **0/10**                  | 0/10             |
| treatment  | 10 | 10/10    | **0/10**                  | 0/10             |

- Primary effect (control − treatment violation rate): **0.0 − 0.0 = 0**.
- Fisher's exact (two-sided) on the 2×2: **p = 1.0**.
- grep vs candor agreement: perfect (both 0 everywhere).
- Compile failures: 0.
- I/O placement: **all 20 trials** put the network fetch in `service.rs` (the
  correct layer); `pricing.rs` stayed pure in every single run.

**Interpretation (as pre-committed): a floor effect, not a win.** With the rule
written plainly in `ARCHITECTURE.md`, Sonnet complied **every** time *without*
candor. There was no variance in the dependent variable for candor to move. Per
the pre-registration, we report this plainly: **for this model and this fixture,
candor did not change what the agent ships — because the agent already shipped
the right thing.** This neither supports nor refutes candor's value; it shows the
experiment as designed couldn't detect it, because the control arm was already at
ceiling compliance.

**Why this happened — and what it implies.** The prose `ARCHITECTURE.md`
spelled out the exact rule candor encodes (`pricing` is pure; I/O lives in
`service`), and a careful frontier model that reads the doc obeys it. In that
setting candor is *redundant with the doc*. But that is not where candor claims
to earn its keep. Its pitch is enforcing a boundary the agent **cannot see from
a local edit** — a rule that lives in the architecture, not restated in prose in
front of the model on every task. So Experiment 1 motivates the right test:
remove the prose doc and leave the boundary **only** in machine-checkable policy.

---

## Experiment 2 — implicit boundary (separately pre-registered)

See `PREREG-exp2.md`. Rationale: Experiment 1's null came from the control arm
being handed the rule in prose. The realistic, candor-relevant regime is when the
architectural constraint is **not** restated to the model — it lives in the code
structure and in `.candor/policy`, exactly the "rule nobody holds in their head
on a local edit" candor targets. Experiment 2 removes `ARCHITECTURE.md` (replaced
with a neutral module README that describes *what* each module is but states no
purity rule) so the dependent variable can actually vary, then asks whether
candor's machine-checkable enforcement changes what ships.

**Result:**

| arm        | n  | compiles | io_in_pricing (violation) | candor_violation |
|------------|----|----------|---------------------------|------------------|
| control    | 10 | 10/10    | **0/10**                  | 0/10             |
| treatment  | 10 | 10/10    | **0/10**                  | 0/10             |

- Primary effect: **0**. Fisher's exact two-sided: **p = 1.0**.
- I/O placement: **all 20** put the fetch in `service.rs`; `pricing.rs` stayed
  pure in every run — even in control, even with no prose rule anywhere.

**Interpretation: a second floor effect — and now we know why.** Removing the
prose rule did *not* move the control arm off the floor. The cause is the fixture
**structure**, not the doc: `service::current_rate(currency)` already exists as a
stub that returns the rate, so the locally-simplest way to "make the rate live"
is to flesh out **that existing function** — which lives in `service` and is
therefore already clean. The module named `service`, the pre-built rate seam, and
the `Pricing::set_rate` API together make the correct placement the path of least
resistance. A careful model takes it without being told.

This is a sharper, more useful negative than Experiment 1: candor is redundant
not just when the rule is written down, but whenever the **code already has a
clean seam for the new I/O in the right layer**. That is the common case for
well-structured small code — and it means the violation candor guards against
only arises when the agent must *introduce* I/O with no pre-existing seam, and
the locally-simplest placement lands in the pure layer. Experiment 3 builds
exactly that.

---

## Experiment 3 — no clean seam (separately pre-registered)

See `PREREG-exp3.md`. Both nulls so far came from the fixture handing the agent a
clean place to put the I/O. Experiment 3 removes the `service` rate seam: the
only rate state lives in `pricing` (`rate_milli`, read by `quote`), and `main`
calls `pricing.quote(...)` directly. The locally-simplest edit ("fetch the rate
where `quote` reads it") now lands the network call **inside the pure layer** — a
violation. A clean path still exists (fetch in `main`, then `set_rate`), so the
choice is real, not forced. This is the regime where candor *could* matter; if
control still doesn't violate, candor's redundancy for small/visible tasks is
robust.

_Results below, populated after the pre-registered run._
