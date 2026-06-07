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

## Phase 2 — dynamic oracle (ground truth from the kernel)

The construction checker trusts the generator's labels. The **dynamic oracle** trusts nothing but
reality: `oracle.sh` RUNS each generated program under `strace`, confirms the effect actually executed
(its distinctive marker — a seed-specific path, `127.0.0.1`, an `echo` arg — appears in the trace,
which filters out the runtime's own startup syscalls), and then asserts candor's static prediction for
the program (`main`'s transitive `inferred`) contains that effect or `Unknown`. A program that
*demonstrably* performs an effect candor predicts nowhere is a silent under-report — caught against the
kernel's own record. (`Env` isn't syscall-observable — it reads process memory — so the oracle skips
it; the construction checker still covers it.) `oracle_check.py` is the pure decision logic, unit-able
without strace; `oracle.sh` is Linux-only and skips gracefully elsewhere.

## Run it

```sh
bash soundness/run.sh            # construction fuzzer: 40 seeds (builds candor first)
bash soundness/run.sh 200        # more seeds = more coverage
SEEDS="1 4 7" bash soundness/run.sh   # specific seeds (reproducible by seed)
bash soundness/oracle.sh 40      # dynamic oracle (Linux + strace; no-op elsewhere)
```

CI runs 60 construction seeds + 40 oracle seeds on every push. The construction checker is verified to
have teeth: reintroducing the historical `resolve_callee` `_ => None` hole makes every `recv_boxed`
seed fail with `recv_boxed(pure/omitted)`.

## Next (not yet built)

- **Per-function attribution in the oracle:** instrument each fn to emit a marker at runtime, interleave
  with the syscall trace, so the oracle catches a *specific* function under-reporting (today it's a
  whole-program check on `main`).
- More forms: cross-crate boundaries, macro-generated bodies, trait objects with arbitrary self types.
