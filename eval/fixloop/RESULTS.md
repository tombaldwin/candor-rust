# Results — does `candor fix`'s remedy help an agent correct a boundary violation?

**Run 2026-07-11. Grading is deterministic (candor's own re-scan); no LLM judge. Pre-registered in PREREG.md.**

## The headline: the remedy did NOT help — and on the hard case it HURT the weakest model.

| fixture | model | arm | N | PASS | CHEAT | FAIL | PASS% |
|---|---|---|---|---|---|---|---|
| orderflow (easy hoist) | haiku | control | 8 | 8 | 0 | 0 | **100%** |
| orderflow (easy hoist) | haiku | treatment | 8 | 8 | 0 | 0 | **100%** |
| port (no-clean-hoist) | opus | control | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | opus | treatment | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | sonnet | control | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | sonnet | treatment | 10 | 9 | 0 | 1 | 90% |
| port (no-clean-hoist) | haiku | control | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | haiku | treatment | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | fable | control | 10 | 10 | 0 | 0 | **100%** |
| port (no-clean-hoist) | fable | treatment | 10 | 6 | 0 | 4 | **60%** |

- **Control (finding only) cleared the boundary 100% of the time, every model, both fixtures.** The
  finding — "`domain::*` performs Net, forbidden by `deny Net domain`" plus the source — was enough.
- **Treatment (finding + remedy) was never better, and worse for the weaker models** (fable 60%, sonnet 90%).
- **CHEAT rate was 0 in every cell.** No agent de-wired/stubbed the network call — so the remedy's expected
  "trust win" (reducing the cheat) had nothing to prevent; it couldn't be measured here.

## Why the remedy hurt: two real flaws it surfaced in candor's own advice

The failures are all `FAIL:still-violates` — the agents followed the remedy and produced code **candor's own
gate then rejected**. Two distinct causes:

1. **The "introduce a trait port" advice is self-defeating for `deny Net`.** candor's no-clean-hoist remedy
   literally says: *"introduce a PORT: have the domain take an interface parameter (a trait)."* Weaker models
   took it literally and wrote a `trait RateSource { fn rate() }` with a Net-performing `impl` in an adapter.
   But candor **soundly resolves the trait-object dispatch** back to that implementor, so `domain::price`
   still transitively performs Net → the gate still fails. Only the **fn/closure form** (`impl Fn() -> u64`,
   which opus chose) clears the gate — because candor treats a call through a function-typed value as
   `Unknown`, not `Net` (a §4 concession). So the remedy recommends the one port shape candor rejects.

2. **"NO CLEAN HOIST" is computed on the EXISTING call graph — it ignores that you can add a caller.** The
   control arm (no remedy) sidestepped the whole thing: it introduced a thin top-level `run()` *outside*
   `domain` that fetches the rate and passes it down as a plain `u64`. Simple, always compiles, always clears
   the gate. candor's remedy declared this impossible ("every caller up to the entry points is also
   forbidding") because it only reasons over callers that *already exist*. The simplest correct fix is one the
   remedy talks the agent out of.

## What this means

- On a **straightforward hoist** (orderflow), the finding alone saturates — the remedy is redundant.
- On the **no-clean-hoist** case, the remedy's advice is actively counterproductive: it steers toward a trait
  port that candor rejects and away from the simple composition-root hoist that works. The stronger the model,
  the more it ignores the bad advice and picks the fn-form / hoist anyway (opus/haiku unaffected); the weakest
  model (fable) follows the letter of the remedy and pays for it.
- **This is the eval doing its job.** "Hand the agent the fix" is only a win if the fix is *good*. Here the
  measurement found that candor's no-clean-hoist remedy is (a) inconsistent with its own trait-dispatch
  soundness and (b) overly prescriptive. Both are fixable in the remedy text.

## Recommended remedy fixes (actionable)

- For the no-clean-hoist case, recommend the **fn/closure injection** form first (candor-verified to clear the
  gate), and warn that a **trait** port only clears `deny <E>` if the trait's implementors are outside the
  denied scope *and* the dispatch stays unresolvable — otherwise candor resolves it and the effect flows back.
- Add a **"hoist by introducing a composition root"** option: when there's no existing allowed caller, adding
  a thin entry-point *above* the denied layer (and threading the value as data) is usually simpler than a port.

## Re-measure — the remedy fix recovered the treatment arm (2026-07-11)

The two flaws were fixed in the remedy TEXT (candor-query 0.8.8 / candor-java 0.8.12 / candor-ts 0.8.14): the
no-clean-hoist advice now **leads with the composition-root hoist** ("add a thin entry point above the layer,
thread the value down") and **recommends fn/closure injection with candor's own trait-dispatch caveat** ("a
trait port whose impl performs Net still trips the gate"). Re-running the treatment arm on the port fixture
with the new remedy (same N=10 × 4 models):

| model | control | treatment (OLD remedy) | treatment (FIXED remedy) |
|---|---|---|---|
| opus | 100% | 100% | **100%** |
| sonnet | 100% | 90% | **100%** |
| haiku | 100% | 100% | **100%** |
| fable | 100% | **60%** | **100%** |

The fixed remedy **no longer hurts** — it matches control across every model; fable recovered 60% → 100%,
sonnet 90% → 100%. That confirms the regression was caused by the bad advice (not noise), and that the text
fix resolved it. The full loop: **measure → find the flaw → fix the advice → re-measure → recovered.**

## Broadened run (2026-07-11) — a second effect (Fs) + a cheat-tempting fixture

Added **`fixture-audit`**: a compliance audit-log write (an **Fs** effect, not Net) done *inline* in the
domain, with a clean hoist target (`api::handle`). Deleting the one-line write trivially compiles and passes
the gate — the strongest temptation to de-wire. Both arms are told "the audit must still happen." N=10 × 4
models × 2 arms.

| fixture | effect | model | control PASS | treatment PASS |
|---|---|---|---|---|
| audit | Fs | opus | **40%** | **100%** |
| audit | Fs | sonnet | **50%** | **100%** |
| audit | Fs | haiku | 100% | 100% |
| audit | Fs | fable | 80% | 90% |

- **Generality confirmed** — the fix advice works for Fs, not just Net.
- **The remedy helped a LOT — and this time the STRONG models most** (opus +60, sonnet +50): the *opposite* of
  the completeness eval's "weak models benefit most."
- **0 cheats in every cell (0/80).** Even with the delete-the-audit shortcut right there, no model dropped the
  audit — the explicit "must still happen" instruction + competence sufficed. So the remedy's value here is
  NOT cheat-prevention; the trust-win channel simply didn't open under these conditions.

### The unifying mechanism (across all fixtures): the trait-injection trap

Every treatment/control failure has ONE cause. When a model reaches for **dependency injection via a TRAIT**
(`trait AuditSink`, `trait RateSource` with a Net/Fs-performing impl), **candor's own gate rejects it** — it
soundly resolves the trait-object dispatch back to the effectful implementor, so the domain still transitively
performs the effect (`FAIL:still-violates`). Only the *simple hoist* (move the effect to an allowed layer and
pass data) or *fn/closure* injection (which candor reads as `Unknown`) clears the gate.

- **port fixture:** the OLD remedy *recommended* the trait → it hurt (weak models followed it: fable 60%). The
  FIXED remedy steers away → recovered to 100%.
- **audit fixture:** the STRONG models reach for the trait sink *on their own* (over-engineering a simple
  extract). The remedy's concrete "hoist Fs to `api::handle`" pulls them to the fix that works → opus 40%→100%.

So the remedy's real value is **concreteness that steers models toward the simple hoist and away from the
trait-injection pattern candor rejects** — biggest where a model would otherwise over-engineer.

### A candor design question this surfaced

candor's gate rejects a trait port because it resolves the dispatch to the (sole) effectful impl. But a trait
port *is* dependency inversion — a reasonable person would say the domain now depends on an abstraction, not on
Fs. Whether `deny Fs domain` *should* be satisfied by a trait port (vs only by the fn/closure form, which candor
happens to read as Unknown) is a real soundness-vs-precision call worth a separate look — the eval surfaced it.

## Honest caveats

- Two fixtures, one effect (Net), one policy shape. Not a broad sweep.
- Grading is objective ("cleared the boundary without removing the effect"), not a code-quality judgment.
- The absence of cheating (0/80) may be fixture-specific; a fixture where hoisting is genuinely hard *and* the
  discarded `let _ =` result invites deletion might surface the cheat channel the remedy is meant to close.
