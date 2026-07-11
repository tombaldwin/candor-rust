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

## Honest caveats

- Two fixtures, one effect (Net), one policy shape. Not a broad sweep.
- Grading is objective ("cleared the boundary without removing the effect"), not a code-quality judgment.
- The absence of cheating (0/80) may be fixture-specific; a fixture where hoisting is genuinely hard *and* the
  discarded `let _ =` result invites deletion might surface the cheat channel the remedy is meant to close.
