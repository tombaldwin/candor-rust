# Bet 4 spike — would a MIR core rewrite pay off?

You chose to spike a core rewrite. I built a minimal, env-gated MIR effect extractor
(`src/mir_spike.rs`, `CANDOR_MIR=1`, non-production) to test the roadmap's central premise —
*"MIR gives soundness by construction"* — against the actual IR, on the exact call forms that have been
candor's recurring soundness-hole source. Here is what the evidence says.

## The fixture

One crate threading a `Net` effect (`TcpStream::connect`) through every form the HIR engine needs bespoke
handling for — direct, inline closure, `Box<dyn Fn>`, `Arc<dyn Trait>` with arbitrary self type — plus an
**effectful `Drop`** guard (network I/O on scope exit), which HIR has no node for at all.

## What the MIR spike found

**1. ✅ MIR genuinely collapses the call *syntax* forms.** All 14 calls — direct, closure, boxed-dyn,
arc-dyn — appear as a single `TerminatorKind::Call`. A MIR engine iterates terminators; there is no
closure node, method-call node, or `Arc<dyn>` node to special-case and therefore none to *forget*. The
"per-syntax-form HIR handling" that produced the `Box<dyn Fn>` / non-local-callback / `Arc<dyn>` holes
does structurally disappear as a *source* of omissions.

**2. ⚠️ But dynamic dispatch does not vanish — it relocates.** The `Box<dyn Fn>` call lowers to
`Call { func: FnDef(std::ops::Fn::call, …) }` on a `dyn` receiver: *resolved to a `FnDef`*, but to the
trait method, not the real target. A MIR engine must still recognise `Fn::call` / `FnMut::call_mut` /
trait methods on `dyn`/generic receivers as `Unknown`. The handling moves from "match HIR call forms" to
"recognise dynamic MIR call constructs" — smaller and more uniform, but not zero.

**3. ⚠️ A naïve MIR engine would be *less precise* than candor is today.** The HIR engine, via its
closure-flow analysis, resolves `Box::new(sink); b()` all the way to `sink` (reports `Net`, precisely).
Naïve MIR sees `via_boxed -> Fn::call` (dynamic) → `Unknown`. To match today's precision you must rebuild
that value-flow analysis on MIR. MIR's explicit places/locals make such dataflow *more* tractable than on
HIR — but it is real work, not free.

**4. ✅✅ MIR uniquely catches implicit `Drop` — and that is a LIVE hole today.** MIR makes scope-exit
drops explicit `Drop` terminators (the spike counts 7 in the fixture). The production HIR engine is
structurally blind to them: it analyses `<Guard as Drop>::drop` (→ `Net`) but **never adds the edge** from
the function whose local goes out of scope. Verified directly: `via_drop` and `main` are reported
**effect-free** even though the program opens a socket on drop. This is a genuine §4 trust-contract
violation ("never silently pure"), and it is the spike's one clear, concrete soundness win for MIR.

**5. ❌ A MIR core does not touch the toolchain axis.** It is still `rustc_private` + a pinned nightly —
the *other* half of Bet 4's motivation (adoption friction, the recurring breakage). That fragility is
already mitigated elsewhere (no git deps → crates.io-ready; `nightly-bump.yml` auto-bumps weekly), so a
MIR rewrite would buy soundness it doesn't urgently need while leaving adoption untouched.

## Caveat on method

The spike reads `optimized_mir` at check-time (opt-level 0), where the MIR inliner is off, so the call
graph is faithful. A real engine would want to confirm an early MIR phase to avoid optimisation reshaping
the graph — itself a thing the spike flags rather than assumes.

## Recommendation: NO full rewrite — capture the one real win surgically

The premise the roadmap gated MIR on ("the hole-rate is structurally high") is **not met**: the syntactic
holes are closed and *stay* closed because Bet 1's fuzzer is the cheap safety net the roadmap predicted —
revert any fix and CI goes red. A rewrite would re-pay for dynamic-dispatch handling and precision
dataflow, leave the toolchain story unchanged, and risk regressions across a large, well-tested engine.

But the spike found a real, fixable hole. The right move is the *surgical* version of the rewrite's
benefit: use MIR **narrowly**, for the one thing it is uniquely good at — implicit `Drop` edges — and
keep the HIR engine for everything else. That captures MIR's concrete soundness win at a fraction of the
cost and validates "marginal beats rewrite." **That fix is implemented alongside this writeup** (see the
`Drop`-edge handling in `check_crate_post` via `mir_spike::drop_edges`, and the implicit-drop
integration test, §9c): `via_drop` now correctly inherits `Net`, while a pure function gains nothing.
Reverting it re-opens the hole — teeth, same as the rest of the harness.

**Follow-up (hardening the fix).** `drop_edges` first resolved only value-embedded drops (the dropped
type's own `Drop` impl plus its struct/tuple/array/enum fields), which *silently missed* a guard behind
a heap pointer — `Vec<Guard>` / `Box<Guard>` came back pure. That was itself a trust-contract hole, so
it's closed: `local_drop_impls` now also follows the curated std OWNING containers (Box/Vec/Rc/Arc/
HashMap/…) into their element types. And the whole thing is now a CI **gate**, not a single example: a
Drop-soundness fuzzer (`soundness/gen_drop.py` + `run_drop.sh`, 40 seeds/push) threads the effect
through a `Guard`'s `Drop` and wraps it in random container forms, asserting every dropping function
inherits the effect — teeth-verified (feed `drop_edges` an empty edge list and all forms fail
`(pure/omitted)`).

If a rewrite is ever revisited, do it for *real interprocedural taint* (where MIR's dataflow is the
genuine enabler), not for call-form soundness — that battle is already won and fenced by the fuzzer.
