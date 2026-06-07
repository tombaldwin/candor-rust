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

_Results below, populated after the pre-registered run._
