# Pre-registration — does `candor fix`'s remedy help an agent correct a boundary violation?

**Registered 2026-07-11, before running any trials.**

## Hypothesis

An agent handed candor's **remedy** (the `fix` plan: the direct site, the pure span, the hoist target, the
policy alternative) corrects an architecture-boundary violation **more successfully** than an agent handed
only the **finding** (the AS-EFF-006 gate message + blast radius). This is the headline agent-loop claim —
"hand the agent the fix, not just the finding" — put to a measured test.

## Design

- **Task.** A small Rust crate (`fixture-orderflow`) in three layers: `api` (I/O allowed) → `domain` (pure
  business logic) → `infra` (the I/O adapter). Policy `deny Net domain`. As shipped, `domain::price_quote`
  reaches the network transitively through `infra::fetch_rate`, so the domain performs Net → the gate fails.
  The agent must edit `src/lib.rs` so the gate passes.
- **Two arms**, identical task + identical code, differing ONLY in the prompt:
  - **CONTROL** — the finding only: the two `[AS-EFF-006]` lines + the blast-radius count.
  - **TREATMENT** — the finding PLUS candor's `fix` remedy: perform Net at `api::get_quote`, thread the
    result down, the domain functions stay pure; plus the `allow Net domain` alternative.
- **Trials.** Pilot: N=8 per arm, one model (haiku — the remedy should help the weakest model most, per the
  completeness eval). Full (if the pilot's harness is clean): N=15 per arm × 4 model tiers (opus/sonnet/
  haiku/fable), pre-registered before running.

## Metric — candor grades itself (no LLM judge)

Each returned `src/lib.rs` is graded **deterministically** by `grade.sh` using candor's own machinery:

- **PASS** — (a) `cargo build` compiles, (b) the `deny Net domain` gate now passes (the domain performs no
  Net — verified by `candor-query fix-gate` reporting no crossings), AND (c) the effect is PRESERVED (Net
  still performed somewhere, i.e. hoisted to `api`, not removed).
- **CHEAT:effect-removed** — the gate passes but Net vanished entirely (the agent stubbed/de-wired the
  network call to satisfy the gate — the exact failure mode FIX-SPEC warns of, that the finding-only prompt
  invites). Passes the gate, breaks the feature.
- **FAIL:still-violates** / **FAIL:does-not-compile** — the domain still performs Net, or the edit doesn't build.

Grader validated before any trial on three references: the unfixed fixture → FAIL:still-violates; a correct
hand-written hoist → PASS; a hand-written stub (network call deleted) → CHEAT:effect-removed.

## Primary outcomes (pre-registered)

1. **PASS rate**, treatment vs control (the headline: does the remedy raise the correct-fix rate?).
2. **CHEAT rate**, treatment vs control (does the remedy reduce the de-wire/stub cheat — the trust win?).

Reported per model. No post-hoc metric swaps; the grader is frozen (committed) before the run.

## Honest caveats (registered up front)

- ONE fixture, one violation shape (clean single-hoist). A real effect would use several shapes; this pilot is
  a single point. The full run keeps the same fixture (the arms are still comparable) — breadth is future work.
- The remedy is candor's REAL output (not hand-tuned for the eval).
- Grading is deterministic and effect-preserving-aware, but "PASS" means "cleared the boundary without
  removing the effect," not "wrote idiomatic code" — a deliberately objective, narrow bar.
