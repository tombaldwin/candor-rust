# Soundness fuzzer

candor's one inviolable promise is the **trust contract**: it must never report a function as
effect-free when it can't actually prove it — a call it can't resolve contributes `Unknown`, never a
silent "pure". A silent under-report is the worst possible bug, because the whole point of the tool is
that you can trust a clean result.

This harness makes that promise **testable and CI-enforced** instead of hoped-for. (It exists because a
hand review found several violations — `Box<dyn Fn>` callbacks, non-local callbacks, `Arc<dyn>`
dispatch — that the unit/integration suite missed; this catches that entire class automatically.)

## How it works (phase 1 — construction-based)

`gen.py <seed> <dir>` emits a compilable Rust crate that threads a **known** effect (`Fs`/`Net`/`Exec`/
`Env`) from a `sink` function up through a random chain, where each call edge uses a randomly chosen
**call form** — exactly the forms that produce silent under-reports:

- `direct`, `iife`/`stored` closures,
- `generic` (a named fn handed to `fn apply<F: Fn()>(f: F)`),
- `boxed_val` (a named fn as a `Box<dyn Fn>` value),
- `dyn_method` (a fn in a generic struct dispatched via `&dyn Trait`),
- **receiving side:** `recv_boxed` / `recv_impl` — a function that takes a `Box<dyn Fn>` / `impl Fn`
  parameter and *invokes* it (where the real bugs lived).

Every emitted function transitively reaches the effect, so candor MUST report each one with the effect
in `inferred` **or** with `Unknown` (a sound over-approximation). `truth.json` records which functions
those are. `check.py` asserts it. A reachable function reported pure — or omitted from the report
(candor omits effect-free fns) — is a **FAIL**: a silent under-report.

`Unknown` is a PASS. This harness tests **soundness** (never silent-pure), not precision.

## Run it

```sh
bash soundness/run.sh            # fuzz 40 seeds (builds candor first)
bash soundness/run.sh 200        # more seeds = more coverage
SEEDS="1 4 7" bash soundness/run.sh   # specific seeds (reproducible by seed)
```

CI runs 60 fixed seeds on every push. It's verified to have teeth: reintroducing the historical
`resolve_callee` `_ => None` hole makes every `recv_boxed` seed fail with `recv_boxed(pure/omitted)`.

## Next (not yet built)

- **Phase 2 — dynamic oracle:** run each generated program under a syscall tracer (strace/seccomp) and
  assert candor's static prediction over-approximates the *observed* effects. That closes the loop on
  *any* under-report, including forms the generator doesn't construct.
- More forms: cross-crate boundaries, macro-generated bodies, trait objects with arbitrary self types.
