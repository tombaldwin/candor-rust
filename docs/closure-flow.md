# Design note — closure / `impl Fn` flow (the "receiving side")

**Status:** the **bounded HIR slice is shipped**; the full MIR pass remains deferred (with a measured
trigger). This note states the problem, measures what's handled, and records the design. The shipped
slice (below, "What shipped") resolves the common case — a free higher-order fn only ever passed
*named* functions — without any MIR; the residue (closures, methods, mixed call sites) keeps the sound
`Unknown`.

## What shipped (the bounded slice)

A free fn that invokes a callback parameter (`fn apply(f: impl Fn()) { f() }`) no longer stamps a
blanket `Unknown` on the spot. Instead candor **defers** it and, in `check_crate_post`, resolves it
from the concrete functions passed at the HOF's call sites:

- every call site passes a **named fn** → edge the HOF to those targets; the redundant `Unknown` never
  appears (and effects — even the `Net` host detail — propagate *through* the HOF). `apply(net_cb)`
  makes `apply` report `{ Net }`.
- any call site passes a **closure / fn-pointer / generic value**, OR the HOF is **never called
  locally** (external callers pass who-knows-what) → the deferred `Unknown` stands. Sound.

Bounded to **free functions** (arg index == param index; a method's `self` would offset it) and **named
callbacks** (a passed closure's effects are already captured lexically on its definer, so nothing is
lost — only the HOF's own attribution stays `Unknown`). This is "CHA for callbacks": the HOF's effect
is the union over the named fns that flow to it, exactly as a trait call unions over its impls.

The rest of this note is the original scoping that justified doing *only* this slice and deferring the
general pass.

## The problem

candor resolves a call by pinning its callee's `DefId`. A call *through a callback parameter* can't be
pinned at HIR, because the parameter is generic:

```rust
fn apply(f: impl Fn()) { f(); }   // what does f() do? depends on the caller's argument
```

candor sees `apply` **once**, generically, so `f()` is an unresolvable call → `apply` gets `Unknown`
(`unresolved: true`). This is the *receiving* side of the closure problem. (The *passing* side — a
named fn or inline closure handed to a combinator — is already handled: see "What's already captured".)

## What's already captured (measured, not assumed)

Run candor on the canonical shape:

```rust
fn apply(f: impl Fn()) { f(); }
fn user()  { apply(|| { let _ = std::net::TcpStream::connect("h.internal:1"); }); }
fn named() { let _ = std::net::TcpStream::connect("n.internal:1"); }
fn user2() { apply(named); }
fn main()  { user(); user2(); }
```

candor reports:

| fn      | effects                                   | why |
|---------|-------------------------------------------|-----|
| `apply` | `{ Unknown* }`                            | invokes the callback param — unresolvable (the residue) |
| `user`  | `{ Net*(h.internal:1) Unknown }`          | **closure body charged lexically** → real `Net` captured; `Unknown` inherited from `apply` |
| `named` | `{ Net*(n.internal:1) }`                  | ordinary leaf |
| `user2` | `{ Net(n.internal:1) Unknown }`           | **named-fn-callback edge** → `named`'s `Net` captured; `Unknown` from `apply` |
| `main`  | `{ Net(h.internal:1,n.internal:1) Unknown }` | both real effects propagate up |

**The decisive observation: the effects are NOT missed.** `Net` correctly lands on `user`, `user2`,
and `main` — through two existing mechanisms:

- **inline closures** are charged lexically to the enclosing named fn (an effectful `|| …` body lands
  on whoever wrote it, invoked or not), and
- **named functions passed as values** add a `caller → fn` call edge (shipped earlier — the "passing
  side").

So the *only* residue is the `Unknown` that `apply` carries (and propagates to its callers). It is
**sound** (over-approximation in the safe direction — never silent-pure) and in cases like this one,
**redundant**: the real effect is already captured elsewhere, so the `Unknown` adds noise, not missing
information.

**Reframing the value.** "Closure-flow" is therefore not "capture missed effects" — it's "remove the
redundant `Unknown` that higher-order functions and their callers carry." Its worth is a function of how
noisy HOFs make `Unknown` in *real* codebases, not a soundness fix.

## Why HIR can't do better

`apply` has a single HIR identity. The effect of `f()` differs per instantiation (`apply::<closure_h>`
vs `apply::<named>`), but candor's report has one row per function, not per monomorphization. To do
better you must move below HIR. Three options:

1. **Effect-polymorphic signatures.** Treat `apply`'s effect set as a function of `F`'s effect set
   (`effect(apply) = effect(F)`), and instantiate it at each call site. Most principled; closest to how
   the capability-token story already works (a `&Fs` parameter *is* an effect in the signature). Large:
   needs an effect variable in the propagation lattice and per-call-site instantiation. Doesn't fit the
   one-row-per-fn report without a representation change.
2. **Per-monomorphization MIR analysis.** Walk `tcx.instance_mir` for each reachable `Instance` of a
   generic fn; `f()` in `apply::<named>` resolves via `Instance::try_resolve` (which candor already
   uses for trait devirtualization). Sound and concrete, but multiplies analysis by the monomorphization
   count, and the report must then *collapse* per-instance results back to one row per source fn (losing
   the per-instance precision again) — so it mostly removes the `Unknown`, at real cost.
3. **Closure-flow dataflow.** A whole-program pass tracking which closures/fns reach which callback
   params (a points-to / flow analysis over function values). Removes the `Unknown` without
   monomorphizing, but is the most code and the easiest to get subtly unsound.

All three are buildable *within* the dylint architecture (we have MIR and `Instance::try_resolve`); none
is a different project. (2) is the most tractable prototype: it reuses machinery already in the codebase.

## Recommendation: defer, with a trigger

**Don't build it yet.** The residue is sound, and the effects themselves are already captured by the
lexical-closure + named-fn-edge mechanisms — so the only payoff is reduced `Unknown` noise on HOF paths,
which is a precision win on a safe base. Spending a MIR pass (and a representation change) for that is a
poor trade *until the noise is shown to matter*.

**The trigger to revisit:** run candor's `audit --coverage` / `unresolved` counts on a few real
callback-heavy crates (async runtimes, iterator-combinator-dense code, plugin/registry designs). If a
large fraction of `Unknown` traces to HOF invokers (not to genuine dynamic dispatch / FFI / reflection),
the noise is real and (2) becomes worth prototyping. Until that measurement exists, this stays deferred —
not because it's impossible (it isn't), but because the evidence says the juice is thin.

**Cheapest prototype when triggered:** option (2), scoped to *non-generic-recursive* higher-order fns,
behind a flag (like `CANDOR_PARANOID`), measured against a fixture corpus of the three callback shapes
(combinator, stored-callback/registry, `spawn`) before going on by default.

## What is explicitly NOT the answer

- Assuming a callback param is pure (silent-pure) — forbidden by §4; the current `Unknown` is correct.
- Charging the HOF with *every* closure in the crate — unsound-by-overreach and floods the report.
