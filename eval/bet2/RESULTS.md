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

**Result — the first positive:**

| arm        | n  | compiles | io_in_pricing (grep, primary) | candor_violation (transitive, secondary) |
|------------|----|----------|-------------------------------|------------------------------------------|
| control    | 10 | 10/10    | **5/10**                      | **8/10**                                 |
| treatment  | 10 | 10/10    | **0/10**                      | **0/10**                                 |

- **Primary effect (grep):** 5/10 − 0/10 = **0.50**. Fisher's exact two-sided
  **p = 0.033**.
- **Secondary effect (candor's own transitive verdict):** 8/10 − 0/10 = **0.80**.
  Fisher's exact two-sided **p = 0.0007**.
- Compile failures: 0. Both arms shipped working code; they differed in *where*
  the I/O landed.

**Interpretation: candor changed the code that shipped.** Removing the clean seam
exposed the failure candor exists to catch. Without candor, **8 of 10** control
submissions had `pricing` transitively performing network I/O — the architecture
boundary `.candor/policy` declares — and 5 of those put the raw socket
syntactically *inside* `pricing.rs`. With candor enforcing the same boundary,
**0 of 10** did: every treatment agent relocated the fetch into `main` or a
separate module that `pricing` never calls. Same task, same model, same files —
the only difference was the candor gate, and it moved the shipped-code violation
rate from 50–80% to 0%.

**The grep-vs-candor gap is itself the headline.** In control trials 03, 06, and
08 the agent factored the fetch into a new `rates.rs` module and had
`pricing::quote` *call* it. A file-scoped check — grep, a human skimming
`pricing.rs`, a file-level lint — sees no I/O syntax in `pricing.rs` and passes
it (`io_in_pricing = 0`). candor's **transitive** effect inference correctly
reports `pricing::Pricing::quote performs { Net }` and flags the violation
(`candor_violation = 1`). That is exactly candor's reason to exist: the boundary
breaks *across* a module edge a local read can't see. The conservative syntactic
metric undercounts the violation by 3/10; candor catches all of them.

The two control "clean" trials (04, 07) fetched in `main` and used `set_rate` —
proving the clean path was genuinely available, not foreclosed. So the effect is
candor changing behaviour, not the fixture forcing it.

---

## Synthesis — *when* candor changes what an agent ships

Three pre-registered experiments, identical except for one variable each, map the
boundary precisely:

| experiment | boundary stated in… | clean seam pre-built? | control violation (candor) | treatment | effect |
|------------|---------------------|-----------------------|----------------------------|-----------|--------|
| 1 | prose `ARCHITECTURE.md` | yes (`service::current_rate`) | 0/10 | 0/10 | none (floor) |
| 2 | only `.candor/policy` | yes (`service::current_rate`) | 0/10 | 0/10 | none (floor) |
| 3 | only `.candor/policy` | **no** | **8/10** | 0/10 | **p < 0.001** |

The honest, specific conclusion:

- **When the code already affords a clean place for the new I/O** (a seam in the
  right layer), a careful frontier model puts it there on its own — with or
  without a prose rule. candor is **redundant** in that common, well-structured
  case (Experiments 1–2).
- **When the locally-simplest edit lands the effect in a layer that must stay
  pure** — the agent doing a local change cannot see that the boundary is being
  crossed transitively — candor **changes what ships**, taking the violation rate
  from 80% to 0% (Experiment 3). This is the case file-level review misses, and
  the one candor's transitive analysis is built for.

This neither over- nor under-sells candor. It says: candor earns its keep exactly
when a boundary is crossed by a *non-local* consequence of a *local* edit — and
is redundant when the right structure is already in front of the model. That is a
falsifiable, measured claim, and it points straight at Bet 3: the value compounds
as the distance between the edit and the boundary grows past what fits in one
context — which these single-crate fixtures only begin to probe.

## Limitations (carried from the pre-registrations)

- Single model (Sonnet 4.6), single crate, K=10/arm. The effect is demonstrated,
  not yet characterised across models or at workspace scale.
- The primary grep metric is syntactic and conservative (it undercounts the
  transitive violations — see trials 03/06/08); candor's own verdict is the
  transitive measure but is not instrument-independent of candor. Both point the
  same way here.
- No Linux/strace transitive *runtime* oracle (agents ran on macOS) — the
  shipped-code boundary check is static. A runtime oracle remains a documented
  follow-up, as does a larger multi-crate fixture for the Bet-3 scale claim.
