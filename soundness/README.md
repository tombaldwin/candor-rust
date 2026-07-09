# Soundness fuzzer

candor's one inviolable promise is the **trust contract**
([SPEC.md §4](https://github.com/tombaldwin/candor-spec/blob/main/SPEC.md#4-the-trust-contract--the-core-of-candor)):
"**an implementation must never report a function as effect-free when it could not actually determine
that.** A call it cannot resolve to a concrete target […] MUST contribute `Unknown` to that function's
effect set […]. It must not be silently assumed pure." A silent under-report — candor's cardinal sin —
is the worst possible bug, because the whole point of the tool is that you can trust a clean result.

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
- `macro_call` (the call lives inside a `macro_rules!` expansion),
- `arc_dyn` (dispatch through `Arc<dyn Trait>` whose method takes `self: Arc<Self>` — arbitrary self type),
- **operator overloads:** `op_add` / `index` / `deref` — the effect is reached through an overloaded
  `a + b` / `v[i]` / `*p` whose `Add`/`Index`/`Deref` impl performs the I/O. In HIR these are
  `Binary`/`Index`/`Unary` nodes, NOT Call/MethodCall, so `resolve_callee` must devirtualize them to
  the concrete impl or the edge is invisible and the caller looks pure.
- **`?` error conversion:** `try_from` — the effect is reached through a custom `From<Ea> for Eb` impl
  invoked by the `?` desugar's error path. candor sees the std `FromResidual::from_residual` call but
  not the LOCAL `From::from` it dispatches to internally, so the edge is recovered from the call's
  residual/Self types (`from_residual_local_edge`).
- **`.await` over a custom Future:** `await_poll` — the effect is in a custom `Future::poll`, reached
  via `.await`. The desugar emits a `Future::poll(..)` Call dispatched statically to the LOCAL impl,
  which candor devirtualizes through the Call. CONSTRUCTION-ONLY (the future is never executor-driven,
  so it never runs) — excluded from the default form set so it can't make the dynamic oracle vacuous;
  reach it via `CANDOR_FUZZ_FORMS="await_poll"`.
- **UFCS dynamic dispatch:** `ufcs_dyn` — `Trait::method(obj)` on a `&dyn Trait` is a `Call` (not a
  MethodCall), so the structural `is_dyn_receiver` check never runs and `dynamic` starts false.
  Resolving the trait method on a `dyn` Self yields a VIRTUAL instance whose `def_id()` is the bodyless
  trait method — candor must report it as still-virtual (`Devirt::StillVirtual`) and CHA the local
  impls, not edge to the bodyless method. (The same fix closes the exotic nightly case of a custom
  `DispatchFromDyn` smart pointer with an arbitrary self type.)
- **receiving side:** `recv_boxed` / `recv_impl` — a function that takes a `Box<dyn Fn>` / `impl Fn`
  parameter and *invokes* it (where the real bugs lived).

It also sometimes DEFINES `sink` via a macro (testing the macro-generated-fn visibility fix — a
macro-gen fn that performs I/O must still be reported, not omitted).

`CANDOR_FUZZ_FORMS="op_add index deref"` restricts the generator to a chosen subset of forms — CI uses
it to run a dedicated operator-overload lane (`Soundness fuzzer (operators)`).

## Cross-crate variant (`run_cross.sh`)

`gen_cross.py` emits one package compiled as lib+bin (the dylint-friendly way to get two linted crates
that reference each other): the lib performs the effect in `dep_sink`; the bin chains across the crate
boundary into `xc::dep_entry()` using the same call forms. One `cargo dylint` lints both; the bin must
inherit the lib's effect **across the boundary** — directly (precise) or as `Unknown` (sound). This
exercises candor's `DefPathHash` cross-crate propagation, a distinct bug-prone surface the single-crate
fuzzer can't reach. Teeth-verified: disabling `load_cross_reports` fails the seeds whose effect depends
on cross-crate inheritance.

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

**Per-function attribution** (`oracle_pf.sh`) goes further than the whole-program oracle (which only
checks `main`). Generating with `CANDOR_FUZZ_INSTRUMENT=1` brackets each chain function with `eprintln`
entry/exit markers — visible to strace (`write(2,…)`) but invisible to candor (`eprintln` routes
through the free fn `std::io::_eprint`, not a classified effect). `oracle_pf_check.py` reconstructs the
CALL STACK at the moment the effect syscall fires (interleaving the markers with the effect syscall in
the trace) and asserts every function ON THE STACK is effect-or-`Unknown` — pinning a runtime-proven
effect to the *exact* function, not just the program. Restricted to Fs/Net (single-process: a clean,
fork-free stack to reconstruct).

## Run it

```sh
bash soundness/run.sh            # construction fuzzer: 40 seeds (builds candor first)
bash soundness/run.sh 200        # more seeds = more coverage
SEEDS="1 4 7" bash soundness/run.sh   # specific seeds (reproducible by seed)
bash soundness/run_cross.sh 40   # cross-crate variant (lib→bin boundary)
bash soundness/oracle.sh 40      # dynamic oracle, whole-program (Linux + strace; no-op elsewhere)
bash soundness/oracle_pf.sh 40   # dynamic oracle, per-function (Linux + strace)
```

CI runs 60 construction + 40 cross-crate + 40 whole-program-oracle + 40 per-function-oracle seeds/push. The construction checker is verified to
have teeth: reintroducing the historical `resolve_callee` `_ => None` hole makes every `recv_boxed`
seed fail with `recv_boxed(pure/omitted)`.

## Fabrication probe (`fabrication_probe.py`) — the OTHER direction

candor's cardinal sin is the SILENT UNDER-REPORT (what the fuzzer above guards). This probe guards the
OPPOSITE direction: **FABRICATION** — a minted effect on a PURE function, the precision failure that
poisons report trust (a spurious `deny` violation on innocent code). It is never *the* cardinal sin. The
classifier's crate rules used to be whole-crate (one effect on EVERY path of an effect-bearing crate),
which fabricated on those crates' pure accessors/builders/data-types (`Mmap::len`, `Level::as_str`,
`Error::to_string`, `Rng::with_seed`, `CommandBuilder::get_cwd`, …). Those rules are now **verb-precise**;
this probe pins that narrowing so it can never silently regress to whole-crate.

For each effect-bearing crate it emits a tiny self-contained crate naming that crate's types + methods,
with two kinds of `pub fn`:

- **PURE** — a single bare call to a member that is provably free of I/O / entropy / clock-read. candor
  MUST omit it from the report (effect-free fns are omitted). An `inferred` effect on it ⇒ FABRICATION.
- **CTRL** — a single bare call to a genuinely-effectful member. candor MUST still report the effect; a
  pure/omitted result ⇒ LOST CONTROL (an under-report — the other direction, also gated).

candor-scan is **syntactic** (syn-based): it resolves a call's receiver type from the fixture's `use`
imports + parameter type annotations *without compiling against the real crate*, so the probe ships
**zero third-party dependencies** — `use memmap2::Mmap; fn f(m: &Mmap) { m.len(); }` classifies on the
names alone. Two discipline rules keep it false-alarm-free: (1) a method is asserted pure ONLY when its
semantics are verified pure (rationale in a comment beside each), else it's left out entirely; (2) every
fixture body is a SINGLE bare call on a PARAMETER — no method chaining, because chaining `.map()`/
`.to_owned()` onto a call's result makes the syntactic scanner re-resolve the trailing method against the
same receiver type (an inference artifact unrelated to the classifier rule under test).

Crates covered (10, 45 probe fns): **memmap2** (`len`/`is_empty`/`as_ptr`/`MmapOptions::new` pure vs
`map`/`flush`=Fs), **tracing** (`Level::as_str`/`Span::is_disabled`/`metadata`/`id` pure vs `enter`=Log),
**arboard** (`Error::to_string` pure vs `Clipboard::get_text`/`set_text`=Clipboard), **fastrand**
(`with_seed`/`fork`/`clone` pure vs `u32`/`usize`=Rand), **portable_pty** (`get_argv`/`get_cwd`/
`PtySize::default`/`CommandBuilder::new` pure vs `spawn_command`=Exec), **chrono** (`year`/`month`/`hour`/
`timestamp` pure vs `Utc::now`/`Local::now`=Clock), **time** (`Date::year`/`ordinal` pure vs
`now_utc`=Clock), **tempfile** (`TempDir::path`/`Builder` setters pure vs `TempDir::new`/`tempfile`=Fs),
**reqwest** (`Client::new`/`header`/`query` pure vs `send`=Net), and **url** (`parse`/`host_str`/`path`/
`scheme` pure — url has NO effectful surface candor models, so it's PURE-ONLY: a control there would be a
lost-control false alarm, and the whole point is that a URL crate must never fabricate Net).

```sh
python3 soundness/fabrication_probe.py                 # build candor-scan if needed, run, gate
CANDOR_SCAN=./target/debug/candor-scan python3 soundness/fabrication_probe.py   # prebuilt binary
```

Exits non-zero (and lists each `FABRICATION …` / `LOST CONTROL …`) on any failure, so it gates CI.
Teeth-verified: temporarily widening the memmap2 rule back to whole-crate (`return Some("Fs")` at the
top of its branch) makes the probe fail with 4 fabrications (`Mmap::len`/`is_empty`/`as_ptr`/
`MmapOptions::new`) while the control still passes. The mirror probe for the JVM port lives at
`candor-java/soundness/fabrication_probe.py`.

(The formerly-planned extensions are all built and documented above: per-function oracle attribution =
`oracle_pf.sh`; cross-crate boundaries = `run_cross.sh`; macro-generated bodies = the `macro_call` form +
the macro-defined `sink`; arbitrary self types = `arc_dyn`/`ufcs_dyn`.)
