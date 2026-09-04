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

## Property gates for the stable scanner (`run_q.sh`, `run_macro.sh`)

Everything above tests the *deep* (dylint/HIR) engine. These two test **candor-scan**, the stable
syntactic scanner, and they check a *property* rather than an expected answer.

**Why they exist.** Six cardinal-sin regressions were introduced and later caught during the ⟨0.35⟩
round — SOUNDNESS **R187, R194, R199, R203, R204, R210**, all in the `?`-veto / drop-glue family.
The 1,504-crate wide-key corpus A/B caught **zero** of them: every one measured 0 corpus incidence.
The corpus is this family's *fabrication* gate. Every one of the six silences was caught by somebody
hand-writing the right panel fixture — i.e. by guessing the shape first. These gates check a property
over **generated** shapes, so nobody has to guess.

Each generated pair is **two spellings of one program**, emitted from a single description so the
two cannot drift apart the way a fixture and its hand-written "control" do:

| gate | reference spelling | variant spelling | claim |
|---|---|---|---|
| `run_q.sh` *hoist* | `let t = EXPR; t?;` | `EXPR?;` | identical evaluation ⇒ identical charge |
| `run_q.sh` *looprot* | `loop { CTOR …?; }` | `loop { …?; CTOR }` | a `?` in a loop body is live for everything that body builds ⇒ variant ⊇ reference |
| `run_macro.sh` | the construction written directly | the same construction through a single-arm `macro_rules!` | one program ⇒ identical charge |

Both spellings of every pair are **compiled and executed** (`examples/gt.rs`) and their in-frame drop
counts printed, so no verdict rests on comparing two absences: a pair whose spellings never drop is
reported `NO-GROUND-TRUTH` and not judged (§E3).

### The property that does NOT work, and why it is worth recording

The first formulation tried was the obvious one — *"insert a `?` that can never error; the set of
in-frame drops can only grow"*, compared against a `?`-free twin. Measured over 40 seeds it
rediscovered **0 of 6**. It is **vacuous on exactly the shapes the bugs live in**: with the
constructed value escaping by return, nothing drops in frame until a `?` creates an early exit, so
the `?`-free twin is charged *nothing* and no under-report can violate `⊇ ∅`. The R187 fixture's own
`collect_loop_noq` is that twin, and its documented expected answer is ABSENT. Two spellings that
both really drop is what makes the comparison bite.

### Calibration — retro-rediscovery (§I)

**A gate is trusted only after it is shown able to fail**, so this comes before any clean result.
A binary was built at each row's fix commit and at its PARENT, and the gates run against both. The
attribution is the **adjacent-commit** difference, not "red on a pre-fix binary": every pre-fix
binary in this chain also carries every defect fixed *later*, so a gate going red at `cd2d436` says
nothing about which row it found. What follows is the set of violating shapes that the named commit —
and only that commit — closes, measured in `--all` mode over the whole shape space.

| row | fix commit | shapes closed, `run_q.sh` | shapes closed, `run_macro.sh` | first random seed to re-find it |
|---|---|---|---|---|
| R187 | `a155a69` | **12** | 0 | `run_q` seed **14** |
| R194 | `7d9a970` | **18** | 0 | `run_q` seed **1** |
| R199 | `75053f1` | **6** | **6** | `run_macro` seed **6** (`run_q` 21) |
| R203 | `5cefa62`+`c22a31d` | 0 | **24** | `run_macro` seed **1** |
| R204 | `535022b` | 0 | **9** | `run_macro` seed **18** |
| R210 | `f991ff1` | 0 | **8** | `run_macro` seed **12** |

**6 of 6 rediscovered, and all six within 18 random seeds when both gates are run.** CI runs 60.

Neither gate alone gets past 3 of 6, and the split is structural, not luck: `run_q.sh` owns the
`?`-position rows (R187 is its `looprot` mode alone — 12 shapes that appear at `cd2d436` and at no
later build; R194 and R199 are its `hoist` mode), and `run_macro.sh` owns the macro-visibility rows
(R203/R204/R210), which `run_q.sh` cannot reach because they need the ending where a same-leaf value
escapes by return, and there its `?`-hoisted *reference* spelling is itself silent at HEAD, so the
comparison has nothing to lose. Only R199 is found by both. **They are complements; running one
without the other buys about half.**

The concrete programs, one per row (reference spelling charged, variant ABSENT under the pre-fix
build and charged after it):

```rust
// R187 — run_q, looprot: a `?` before vs after the construction in a loop body
for _i in 0..9u32 { out.push(H::new("a")); let _v = tick(&mut c)?; }   // charged
for _i in 0..9u32 { let _v = tick(&mut c)?; out.push(H::new("a")); }   // ABSENT under cd2d436

// R194 — run_q, hoist: the `?` on its own operand
let t = { out.push(H::new("a")); gen(n) }; t?;   // charged
{ out.push(H::new("a")); gen(n) }?;              // ABSENT under 70fd624

// R199 — the same, with the construction macro-borne
let t = { out.extend(vec![H::new("a")]); gen(n) }; t?;   // charged
{ out.extend(vec![H::new("a")]); gen(n) }?;              // ABSENT under 7d9a970

// R203 — run_macro: a template in the `?` operand, same leaf built after it
(if m > 0 { out.push(H::new("a")); gen(n) } else { Ok(0u32) })?;   // charged
(if m > 0 { out.push(mE!("a"));    gen(n) } else { Ok(0u32) })?;   // ABSENT under 75053f1

// R204 — a statement-position macro inside re-parsed block tokens
idm!({ { let x = H::new("a"); out.push(x); } gen(n) })?;   // charged
idm!({ mS!(out, "a");                       gen(n) })?;    // ABSENT under c22a31d

// R210 — a `macro_rules!` defined inside re-parsed block tokens and used there
(if m > 0 { { out.push(H::new("a")); } gen(n) } else { Ok(0u32) })?;   // charged
(if m > 0 { idm!({ macro_rules! mB { ($p:expr) => { H::new($p) } }
                   out.push(mB!("a")); }); gen(n) } else { Ok(0u32) })?;  // ABSENT under 7e0c90b
```

Each of those functions ends `let h = H::try_new(m, "b")?; let _ = out; Ok(h)` or `Ok(out)`, and
`examples/gt.rs` records the in-frame drops both spellings actually perform.

### The known-open register (`known_open.tsv`)

**candor-scan does not satisfy either property today.** `soundness/known_open.tsv` records the shapes
where it fails, measured **exhaustively** (every point of both shape spaces, not a sample — a sampled
baseline would mark a shape "new" the first time a later seed happened to reach it). The gates
subtract it, so a NEW instance fails while the standing ones are printed every run.

**Every line in that file is a silent under-report that is still open.** It is a debt register, not a
list of acceptable behaviours, and re-running `baseline.sh` to turn a red gate green is how a cardinal
sin gets accepted as a low residual. At `c5dae3d` it holds 216 entries:

- **163 macro-equivalence violations.** 89 are R203's declared residuals (tokens readable neither as
  an expression list nor as statements: `unparsed`, `match_arms`, `repetition`). 50 are `body_local` /
  `blocktok_local` — a `macro_rules!` defined in a function body or inside re-parsed macro tokens and
  used there. R206/R207/R210 closed those for the `?`-interior **veto**; the collector's own
  resolution is still open, which is the same split the R142/R143/R144 rows describe. The remaining
  24 are plain crate-level templates losing in three contexts only: the **hoisted** `?`
  (`let t = …; t?`) and both loop bodies.
- **53 `?`-position drifts**, every one with the ending where a same-leaf value escapes by return.
  There, `let t = <operand>; t?;` is reported PURE while the byte-equivalent `<operand>?;` is charged,
  whenever either the construction or the operand is macro-borne. One mechanism: the collector never
  sees the macro-borne construction, and the `?`-interior machinery patches over that only when the
  `?` sits syntactically on the operand.
- Separately, `run_q.sh` reports **102 `BOTH-PURE` pairs** at HEAD — neither spelling charged though
  the run dropped. Those are silent under-reports too, but they are not this gate's differential, so
  they are counted and printed rather than gated.

### Running them

```sh
bash soundness/run_q.sh 60                    # ?-position property, 60 seeds  (~30s)
bash soundness/run_macro.sh 60                # macro equivalence, 60 seeds    (~30s)
CANDOR_SCAN_BIN=/path/to/candor-scan bash soundness/run_q.sh 40   # measure a chosen arm
bash soundness/baseline.sh                    # re-measure known_open.tsv (exhaustive)
bash soundness/baseline.sh --check            # fail if the register is out of date
```

Exit 0 clean · 1 a real finding · 2 harness/build error · 3 SELFSKIP with a stated reason; each ends
with a `RESULT:` line. A failing seed's crate is copied to `soundness/.last-run.<gen>.seed<N>/`, so a
finding arrives with a reproduction.

### What these gates CANNOT catch

- **Anything already in `known_open.tsv`.** A regression that lands on an already-broken shape is
  invisible. The register is 216 of the ~756 shapes; the gates are blind on that 29%.
- **A defect both spellings share.** Both are self-differential. If the drop model loses an effect for
  the reference spelling too, the pair agrees and passes — that is what the `BOTH-PURE` counter is for,
  and it is reported, not gated.
- **Fabrication.** They only look for a spelling charging *less*. Over-charging in both spellings is
  invisible; that is the corpus A/B's job, and the two are complements.
- **One effectful `Drop` on one guard type.** No trait objects, generics, cross-crate edges, `async`,
  threads, or non-`Drop` effect routes. The other fuzzers in this directory cover those, and none of
  them covers these two properties.
- **The deep (dylint) engine.** These drive `candor-scan` only.
- **Shapes outside the composed space** — the ~13 construction forms, ~11 macro forms, ~11 contexts and
  3 endings enumerated in the two generators. It is a bigger space than any fixture set, and it is
  still a list somebody wrote down.
