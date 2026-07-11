# Design note — should candor's gate accept a trait port as dependency inversion? (2026-07-11)

Surfaced by the fix-loop eval (RESULTS.md): agents that fix a `deny <E> <layer>` violation by giving the
layer a **trait** port (`trait Rates { fn get() }` with an effect-performing impl) find candor's gate STILL
rejects it. Is that a bug, or correct? Investigated empirically — three fix shapes for `deny Net domain`, and
how candor classifies `domain::price` in each:

| fix shape | how the domain gets the value | candor says `domain::price` performs |
|---|---|---|
| **trait port** | `price(r: &dyn Rates)`, `r.get()`; impl does Net | **`Net`** (candor resolves the dispatch to the impl) |
| **fn/closure port** | `price(f: &dyn Fn()->u64)`, `f()` | **`Unknown`** (candor can't resolve a function value) |
| **simple hoist** | `price(rate: u64)` — the value is passed as DATA | **pure** (no effect, no Unknown) |

And how each fares under three policies:

| policy | trait-port | fn-port | simple-hoist |
|---|---|---|---|
| `deny Net domain` | VIOLATES | CLEAN | CLEAN |
| `pure domain` | VIOLATES | CLEAN | CLEAN |
| `deny Net Unknown domain` | VIOLATES | **VIOLATES** | **CLEAN** |

## Conclusion — candor's behavior is CORRECT; rejecting the trait port is sound, not a bug.

A trait object with a KNOWN, effect-performing implementor **does reach that effect at runtime**. candor
resolves the dispatch (class-hierarchy analysis) and charges the effect to the caller. If candor instead
treated the trait port as "the domain is now clean," it would be **silently under-reporting** that the domain
reaches Net — the cardinal sin the whole tool exists to avoid. So the trait-port rejection is non-negotiable:
it is candor being sound, exactly where a naive reading of "dependency inversion" would hide a real effect.

The **fn/closure** port is different only because candor genuinely CANNOT resolve a function value — so it
marks `Unknown` (the §4 trust marker), not "clean." `deny Net` is Net-specific, so `Unknown` slips past it —
but this is DISCLOSED, not silent: candor says "the domain has an unresolvable call here." A policy that wants
the domain provably decoupled uses **`deny Net Unknown domain`**, which the fn port then fails (see the grid).

The **simple hoist** — pass the fetched value DOWN as plain DATA — is the only fix that makes the domain
**provably pure**: candor verifies it performs no effect and no Unknown, so it is CLEAN under every policy,
including `deny Net Unknown`. That is why the remedy leads with it.

## The purity hierarchy (the actionable takeaway)

For a layer that must be free of effect `E`, the three fixes are NOT equivalent — they buy different guarantees:

1. **Simple hoist (pass data)** → the layer is *provably pure* (candor verifies it). Clean under any policy.
2. **Fn/closure injection** → the layer clears `deny E` / `pure`, but candor can't SEE through the injected
   function, so it reads `Unknown`. A hole only `deny E Unknown` (or an explicit `deny Unknown`) closes.
3. **Trait injection** → does NOT clear the gate: candor resolves the dispatch back to the effectful impl.

## Actions taken

- **No gate change** — the resolution behavior is correct (soundness-preserving).
- The `fix` no-clean-hoist advice already steers to (1)/(2) and away from (3); refined it to name the
  provably-pure vs Unknown-hole trade-off between (1) and (2), so the advice is soundness-precise.
- Recorded the trait-port-vs-`deny E` interaction so it's not re-litigated as a bug.
- **New `candor unverified` query (candor-query, 2026-07-11)** — the policy-guidance follow-through. A
  `pure`/`deny <E>` layer that PASSES the gate but contains `Unknown` functions is disclosed as "not PROVABLY
  clean," with the `deny <E> Unknown <scope>` upgrade that makes the intent enforceable. `--strict` → exit 1
  (CI can require provable purity). Advisory — the gate's verdict is untouched; this only surfaces the gap the
  hierarchy above creates. Wired into `cargo candor unverified` + the `candor_unverified` MCP tool. Rust-only
  for now (the primary query engine); a java/ts/swift port is a natural follow-on.
