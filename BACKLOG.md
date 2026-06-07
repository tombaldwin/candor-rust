# candor backlog

Honest priority order within each section. Sources: `CRITIQUE.md`, `EVAL.md`, hands-on findings.

## Direction — where the value actually concentrates (a critical read)

candor's measured, durable value is narrow and worth protecting from scope creep. Concentrate effort
on the two places the evidence is strongest:

1. **Agent edit-time blast-radius feedback** — the diff / `CANDOR_REVIEW` self-review / MCP / Claude
   Code hook. The pre-registered eval's decisive result (full-propagation reporting **7% → 100%**) is
   here. This is the north star (P0); everything that sharpens the *delta back to the agent* wins.
2. **The effect-regression CI ratchet** (AS-EFF-005) and **policy boundaries** (AS-EFF-006). Low
   adoption cost (no token threading), real felt need ("fail the PR that makes a parser open a
   socket"), unambiguous. Under-emphasised relative to its deployability — promote it.

**De-prioritise** the capability/conformance discipline (token threading, `&Fs`/cap-std migration):
elegant, but realistically almost no team rewrites a real codebase for it, so it stays the deep,
optional tier — not a place to spend polish.

**Honest ceiling to keep in view (don't paper over):** correctness is bounded by a *curated
classifier*, so the failure mode is a silent *under*-report ("network-free" when it isn't) — the most
dangerous direction. The mitigation (`Unknown` / coverage-gaps / version-trust) is candor's best
asset; keep widening classifier coverage and never trade the honesty for a cleaner-looking number.
The efficiency claim ("cheaper than reading source") depreciates as models get cheaper/longer-context;
lead with blast-radius, which is a reasoning gap, not a cost gap.

### Non-goal: a "candor score" / cross-codebase grade (decided — do not build)

A single aggregate score (incl. the tempting "resolvability = 1 − unresolved%") was considered and
**rejected.** It conflates *analyzability* with *quality* (penalises DI / plugins / event-driven —
good designs); drifts with the *engine* version, not just the code (this session's resolution work
would move it for unchanged source); is gameable in the harmful direction (de-abstract a seam to
satisfy the analyzer); and collapses the actionable signal (the *list* of unresolved fns) into an
ambiguous digit that cross-codebase comparison would read as "team A writes worse code" when it means
"team A uses Spring." The right primitives are a **ratchet** (don't regress) + a **map** (the
descriptive profile) + opt-in **discipline counts → 0** (conformance/policy) — candor already has all
three. Resolvability is legitimately useful **inward**, as a metric candor tracks about *its own*
coverage release-over-release — not a developer-facing grade.

## P0 — agent coding: make candor change what an agent *does* (the north star)

This is the point of the rest. The bet: candor's value to a coding agent is **verification, not
context**. An agent can read source and infer most effects itself — the A/B eval (`EVAL.md`) showed a
source-only agent matching/beating the report wherever the classifier has a gap, and over-trusting a
blind spot makes it *worse*. What an agent *cannot* cheaply compute is the **transitive effect delta
of its own edit** across the call graph and crate boundaries — exactly the failure mode agents have: a
*local* edit with a *non-local* consequence (add a `reqwest::get` in a helper → twelve callers now
transitively perform `Net`). candor computes that for free. **Lead with the delta, not the dump.**
(Rests on the P1 correctness foundation — a feedback signal is only worth acting on if it's right.)

- [x] **1. Agent-facing effect diff — `cargo candor diff`.** v1 ships: describes the per-function
      delta vs a baseline (`+ worker { +Net }`) *including the transitive blast radius* (a network
      call added in `worker` also shows `+Net` on its caller `main`), flags a new `Unknown`, and has
      `--json` for the agent. v2 ships too: the diff now separates **introduced** (the new effect is
      in the function's own `direct` set — the source) from **inherited** (transitive), with a
      headline `Fs: introduced in Cache::get → inherited by 6 caller(s)`. Remaining polish: the exact
      call-site location (`@ foo.rs:12`) — `explain` (§3) has it; emitting it in the report is the step.
- [x] **2. Close the loop in the agent's edit cycle.** Opt-in `CANDOR_REVIEW=1`: the Stop hook diffs
      the fresh report vs the baseline and, on a newly-introduced effect, feeds the delta *back to the
      agent* (`decision:block` + `additionalContext`) as a self-review checkpoint; `AGENTS.md` §5 tells
      the agent how to respond. Triple loop-guard: a once-per-effect `review-seen` marker,
      `stop_hook_active`, and Claude's 8-block cap; off by default. This is what makes candor *change
      behaviour*, not just inform. (Tested: candor-run exit-11 + stop-hook block/no-block.)
- [x] **3. `explain <fn>` — effect provenance** (the P2 item below). `cargo candor explain <fn>` traces
      the call path to where each effect originates: `main → middle → leaf` with `leaf via
      std::net::TcpStream::connect at main.rs:1`. For scoping (what flows through here before I edit)
      and to answer the diff's "why". Engine records effect *sites* (callee + location) under
      `CANDOR_EXPLAIN`; a BFS finds the nearest source.
- [x] **4. Speed for a tight loop** — done as P0′ §8 (fast `diff`, `watch`, instant queries,
      incremental re-lint). A full re-lint (~minutes on a big crate) is too slow per edit;
      need an incremental path, or at least a diff against the cached report that re-lints only the
      changed crate(s).
- [x] **5. Measure it — don't assume** (pilot; `EVAL.md` Trial 5). Pre-registered with/without eval on
      a non-local-effect trap (`eval/minicache`), blind-judged. Result: **treatment 4/4 vs control 1/4**
      fully identified the transitive blast radius (control 3/4 only gestured "callers/perf"). Honest
      bounds: candor makes non-local propagation *complete & explicit* (the axis it targets), but did
      NOT catch what agents miss entirely — control independently found path-traversal/TTL/error bugs
      candor doesn't. Pilot caveats: N=4/arm, one task, one capable model. **Still to do:** multi-task,
      multi-model study quantifying end-to-end *edit-quality* gains (not just awareness) before scaling.

### P0′ — where to take it next (post-eval reassessment)

The eval reframed the goal. AI agents fail at code in a way candor is positioned for: they **don't
hold the global architecture in their head**, so they put I/O in the wrong layer, break a purity
boundary, or give a function an effect it was never meant to have. candor sees the whole effect graph;
the agent doesn't. Lean into *that* asymmetry — not into restating effects the agent already sees
locally. (The eval also showed the *guard* is the dependable value, that candor misses the
security/correctness bugs that often matter most, and that its value scales with codebase size.)

- [x] **6. Effect policy / architectural invariants — shipped.** `CANDOR_POLICY` / `cargo candor
      policy` enforces a declarative `.candor/policy`: `deny Net Db Fs domain`, `pure parse`,
      `deny Exec`. Each rule checks a function's **transitive** effect set, so it catches a layer
      reaching an effect *through a helper* (`AS-EFF-006`) — the architectural violation an agent
      can't see from a local edit. Tested: parser unit test + integration (transitive violation fires,
      genuinely-pure fn doesn't). Spec'd (AS-EFF-006 in SPEC/SEMANTICS); `examples/candor-policy`.
- [~] **7. Effects → *risk* (argument provenance) — v1 shipped (heuristic).** `CANDOR_TAINT` /
      `cargo candor risk` flags `AS-EFF-007`: an injection-class effect (Fs/Exec/Db/Net/Env/Ipc) whose
      argument *syntactically derives from a function parameter* — `fs::read(format!("/x/{key}"))`,
      `Command::new(name)`. Catches the path-traversal/command-injection class the eval exposed; a
      literal-arg effect is not flagged. **Honest limits (the `~`):** intraprocedural + syntactic — it
      misses flow through struct fields and across functions, and over-flags a validated parameter; it's
      advisory (exit 0), never a gate. Tested (param-derived fires, literal doesn't). **The real frontier
      remains:** interprocedural, field-sensitive data flow (a MIR-level pass) for sound taint.
- [x] **8. Speed — separate the slow analysis (one compile per change) from instant queries.** The
      principle: the analysis only changes when the code does, so compile once off the critical path
      and serve queries from the cached report. **Done:** `cargo candor diff` now reads the kept-fresh
      `.candor/report.*` (when its source-hash matches `.candor/state`, maintained by the Stop hook)
      instead of recompiling — ~30s → **0.26s** in the common case; falls back to a re-lint when stale
      (content-hash, so never wrong). Also **done:** `cargo candor watch` — a background poller that
      re-lints on a real source change and stamps `.candor/state` only on a successful build, keeping
      the report fresh off the critical path so `diff` is instant even without the Stop hook (the
      compile runs concurrently with editing). Also **done:** instant read-only queries served from
      the fresh report (no recompile) — `cargo candor show <fn>` (its effect set, `*`=direct) and
      `cargo candor where <Effect>` (functions performing it, split direct-source vs inheritor),
      both `--json`. This is the net speed *win*: the agent answers "what does X do / what touches
      Net" in one ~0.5s call instead of grepping and tracing source. Also **done:** a forced re-lint
      (explain/policy/risk) is now incremental — `lint_fresh` tries an incremental build and only
      clears `target/dylint` on a pure cache hit (detected via cargo's "Checking"/"Compiling" line),
      so a re-lint recompiles just the changed crate instead of the whole tree.
- [x] **9. Selectivity — surface only the *consequential* propagation.** `cargo candor diff` no longer
      lists every inheritor: it computes, per effect, the **top-level** gainers (those not called by any
      other gainer — the entry point / public API where the effect actually surfaces) from the report's
      `calls` graph, and leads with `Fs: introduced in Cache::get → reaches main (+5 intermediate)`. The
      list shows the source and the top-level endpoints; the in-between plumbing is collapsed to a count.
      `--json` still carries everything. Cuts the noise on a wide blast radius to the functions that
      matter. (Could extend to flag "reaches a policy-forbidden fn" — ties to §6.) Tested (3-hop chain:
      source shown, `main` tagged top-level, `mid` collapsed).

- [x] **10. Realize the speed/cost savings — make the agent *use* the fast queries.** §8 made queries
      instant; this is about the agent reflexively reaching for them instead of grepping/reading.
      **Done:** `cargo candor callers <fn>` — instant reverse-dependency lookup ("who calls this?", the
      most common pre-edit grep), served from a new effect-relevant `calls` field in the report. Also
      **done:** an **MCP server** (`integrations/mcp/candor-mcp.py`, no SDK) exposing the query set
      (`candor_effects`/`where`/`callers`/`diff`) as native tools, so an MCP agent calls candor
      reflexively in one cheap call (CLI is the fallback) — the leverage point converting "candor *can*
      answer fast" into "the agent *skips* reading files". Also **done:** `cargo candor map` — a
      compact module→effects overview (`app { … } (80 fns)`) to front-load understanding at session
      start without grepping. Caveat: keep the tool surface small — over-querying adds round-trips.

**Not worth doing:** more interactive-loop polish (call-site line, prettier output) — the eval says
that's the narrow, modest-value axis. Diminishing returns.

## P1 — correctness (silent wrong answers are the worst failure)

- [~] **Soundness fuzzer — "never silently under-reports" is now a CI gate, not a hope** (`soundness/`,
      **Bet 1 phase 1** of the improvement roadmap). A hand review found multiple trust-contract
      violations the unit/integration suite missed (`Box<dyn Fn>` callbacks, non-local callbacks,
      `Arc<dyn>` dispatch — candor reported them PURE instead of `Unknown`). The fuzzer generates crates
      that thread a known effect through every call form known to under-report (closures, `dyn`
      dispatch, generic/boxed callbacks, the receiving side) and asserts every reachable function is
      `effect`-or-`Unknown`, never silent-pure. **Verified to have teeth:** reintroducing the historical
      `resolve_callee` `_ => None` hole makes every `recv_boxed` seed fail. **Phase 2 shipped — a
      dynamic oracle** (`soundness/oracle.sh`): RUNS each generated program under `strace`, confirms the
      effect executed (its marker in the trace, filtering runtime startup syscalls), and asserts
      candor's static prediction (`main`'s transitive `inferred`) over-approximates the kernel-observed
      effect — ground truth that trusts nothing about the generator. CI runs 60 construction seeds + 40
      oracle seeds/push (oracle is Linux-only; `oracle_check.py` decision logic unit-validated).
      **Extended:** a **cross-crate variant** (`run_cross.sh` — a lib→bin boundary exercising
      `DefPathHash` cross-crate propagation; teeth-verified by disabling `load_cross_reports`), plus
      **macro forms** (a call inside a `macro_rules!` expansion, and a macro-DEFINED `sink` testing the
      #5 macro-fn-visibility fix; teeth-verified by reverting it). CI now runs 60 construction + 40
      cross + 40 oracle seeds/push. **Still TODO:** per-function attribution in the oracle (instrument
      fns + interleave with the trace) and arbitrary-self-type trait-object forms.
- [x] **Database clients.** `sqlx`/`rusqlite`/`postgres`/`tokio_postgres`/`diesel`/`redis`/… now
      classified `Db` (execution verbs only, not query building; best-effort, tune via CONFIG).
- [x] **`tokio::net` / `std::os::unix::net` Unix sockets** → `Ipc`, no longer conflated with `Net`.
- [x] **`tokio::process` → `Exec`.** The async mirror of `std::process` (which was covered) — spawning
      a subprocess via `tokio::process::Command`/`Child` was classified pure, a silent under-report of
      subprocess execution (the dangerous direction). Closed to match the existing `tokio::fs`/
      `tokio::net` coverage; `tokio::process` has no pure data types to over-flag. Unit-tested.
- [x] **`rand` over-report fixed — now verb-precise.** Whole-crate `rand` → `Rand` flagged its *pure*
      distribution constructors (`Uniform::new`, `Normal::new`) and deterministic-seed constructors
      (`seed_from_u64`) as effectful. Now matched to the calls that actually consume randomness — the
      entropy sources (`OsRng`/`thread_rng`/`rng`/`from_entropy`) and generation verbs
      (`gen*`/`random*`/`fill*`/`sample*`/`next_u*`). `getrandom`/`fastrand` stay wholesale (effectful
      end-to-end). Removes the false positives without losing the generation coverage. Unit-tested.
- [x] **const/static initializers** now reported (a `static X = effectful()` performs its effect),
      with macro-generated items (e.g. tracing `__CALLSITE` statics) filtered out via
      `span.from_expansion()` — that filter was needed; without it the report flooded.
- [x] **memmap2** → `Fs`.
- [x] **Trait dispatch (dyn + generic) over local traits** — broke through with Class Hierarchy
      Analysis: a call to a locally-defined trait method now adds edges to all impls, so effects
      propagate through `dyn` and generic dispatch (sound over-approximation). On ebman this resolved
      the LLM feature and dropped `Unknown` 100→92, of which only 6 are now *purely* Unknown.
      `CANDOR_PARANOID` remains the opt-in for the residual *non-local* generic-dispatch gap.
- [~] **Closures / callbacks — the statically-resolvable half is closed.** A **named** function
      passed as a value (handed to a combinator — `iter().map(parse)` — `thread::spawn`, stored as a
      callback, registered) now adds a `caller → fn` edge, so its effects propagate even though the
      *call* happens inside unseen library code. Previously dropped (the `Path`-to-`FnDef` value
      wasn't a Call), a silent under-report inconsistent with inline closures (already charged
      lexically). Over-approximates in the safe direction; local targets only; ui-tested. **Residue
      (deferred, the genuinely MIR-hard part):** the *receiving* side — an `impl Fn`/fn-pointer
      **parameter** invoked inside a function — keeps an honest `Unknown`, because pinning its
      concrete target needs interprocedural closure-flow (effects riding in function types), a
      MIR-level pass. An effectful *inline* closure already lands on its lexical owner, so the live
      hole is now just "effect arrives only through an unknown callback parameter." Small, characterized.
- [~] **stdio / println.** Decided *against* for now: `std::io::stdout`/`println!` is pervasive and
      low-signal (especially for TUIs); would add noise without authority-level value. Reconsider if
      a use case appears. `stdin` (real input) could be added later as its own effect.

## P2 — depth / precision

### From the critical-assessment pushback — "fundamental" was too strong (these are buildable)

These were dismissed as hard limits; they're really *expensive or risky*, not impossible. The one
genuine floor is undecidability (no sound+complete effect set in general) — and the answer there is
sound over-approximation (`Unknown`), which candor already does.

- [~] **Closure-flow — the `impl Fn` *receiving* side: BOUNDED SLICE SHIPPED; full MIR pass deferred**
      ([docs/closure-flow.md](docs/closure-flow.md)). **Shipped (no MIR needed):** a free fn that
      invokes a callback param defers its `Unknown`; `check_crate_post` resolves it from the concrete
      fns passed at the HOF's call sites — all **named** → edge the HOF to them and drop the redundant
      `Unknown` (effects + host detail propagate *through* the HOF, "CHA for callbacks"); any
      **closure / fn-ptr**, or **never called locally** → the sound `Unknown` stands. Bounded to free
      fns (arg index == param index) and named callbacks (a closure's effects are already captured
      lexically on its definer). Integration-tested (resolved drops Unknown; closure-passed keeps it).
      **Residue (deferred, the genuinely MIR-hard part):** methods (self offset), and removing the
      `Unknown` for closure callbacks (needs un-folding the lexical charging / per-instance MIR). The
      original scoping + the trigger for the full pass are in the note. A function that *invokes* a
      callback parameter keeps an honest `Unknown` (its target isn't pinned at HIR). Scoping it
      surfaced the decisive fact, **measured** not assumed: the effects are **not missed** — an inline
      closure's body is charged lexically, and a named fn passed as a value gets a call edge (the
      passing side, already shipped), so `Net`/`Fs`/etc. land correctly on the definers and their
      callers. The *only* residue is the `Unknown` the HOF (and its callers) carry — **sound**, and in
      the common case **redundant** (the real effect is already captured elsewhere). So this isn't a
      soundness fix; it's *reducing redundant `Unknown` noise* on higher-order call paths. Three
      buildable routes exist (effect-polymorphic signatures / per-monomorphization MIR via the
      `Instance::try_resolve` we already use / closure-flow dataflow), none a different project — but
      the payoff is precision on a safe base. **Recommendation: don't build yet**; the trigger is a
      measurement — if `audit`'s `Unknown` count on real callback-heavy crates traces mostly to HOF
      invokers (not genuine dynamic dispatch / FFI), prototype the per-mono route behind a flag against
      a 3-shape fixture corpus. *(Pushback was right that "needs a MIR engine" framed a road as a wall;
      scoping then showed the road isn't worth walking until the noise is shown to matter.)*
- [ ] **Macro-generated effects — narrow the blanket filter.** Today `span.from_expansion()` skips
      *all* macro output (added because compiler-internal/`tracing __CALLSITE` expansions flooded the
      report). But every expansion carries its macro's identity (`ExpnData`/`DefId`), so we can filter
      **only known-noise macros** and analyze the rest (so an `async_trait`/derive/decl-macro that
      generates effectful code becomes visible). The blocker is *re-flooding risk*, which needs a
      real-codebase corpus to tune — a **validation problem, not an impossibility**. *(Pushback: the
      user-vs-noise distinction is decidable from the expansion's identity.)*
- [x] **Literal `Net` host detail — shipped (engine + report + spec).** Full host-by-runtime-value is
      undecidable, but the **literal** subset is extractable. **Done:** a directly-classified `Net`
      call carrying a string-literal address/URL records the host (`host[:port]`, scheme/path/userinfo
      stripped), propagated on the same call graph as `fs` detail (`propagate` is now generic over the
      element), surfaced as the report's optional `hosts` field and rendered by `show` as
      `Net*(api.example.com)`. Honest by construction: a runtime address yields `Net` with **no** host
      (absent ≠ "no network"); a header value / verb is filtered out (`net_host_literal` requires a
      dotted name or `host:port`). Spec'd in candor-spec §2 (never a completeness claim — `Net` keeps
      the I/O claim; `hosts` only narrows it with what's provably visible). Unit + integration tested.
      **Follow-up:** candor-java parity; more host-bearing APIs (e.g. URL builders) if a real case wants
      it. *(Pushback was right: "not statically knowable" only held for the runtime subset.)*
- [ ] **Smarter generic-dispatch over foreign types** (currently assumed pure to avoid flooding;
      `CANDOR_PARANOID` is the opt-in). A *choice*, not a limit: could assume-pure only when the
      dispatch's trait bounds **exclude** all known-effectful traits, and mark `Unknown` otherwise —
      tightening the default without the paranoid-mode noise. *(Pushback: a tunable tradeoff, not a
      wall.)*

- [x] **Entry-point handling in strict mode.** `main` no longer raises AS-EFF-001 (it's the root
      that legitimately holds the whole capability bundle).
- [ ] **Reachability / dead-code elimination.** CHA + the new named-fn-callback edges made the call
      graph much more complete (edges through local trait-object/generic dispatch, and through fns
      passed by name to combinators/`spawn`/registries), but it's still missing the *unknown-callback*
      `impl Fn`/std-`dyn` edges, so reachability would *still* mislabel some callback-reached code as
      dead. Closer to soundness than before, but not there yet — **deferred** until that residual
      closure-flow gap closes.
- [~] **Finer `Fs` granularity (read vs write).** **Non-breaking refinement shipped:** each report
      entry now carries an optional `fs: ["read"|"write"]`, derived from the verb of every
      directly-classified `Fs` call (`fs::write`→write, `File::open`→read, `fs::copy`→both;
      `OpenOptions::open` left unannotated since its direction is runtime-flag-decided) and propagated
      through the call graph in a separate fixpoint that never touches the effect set. The `Fs` effect
      itself is unchanged, so **no baseline regresses** (verified: the self-guard stays clean) and the
      field is omitted when unknown. `cargo candor show` renders it (`Fs*(write)`, `Fs(read,write)`)
      and `show --json` exposes it. **Still deferred (the breaking part):** splitting `Fs` into
      first-class `FsRead`/`FsWrite` *effects* with a capability-subtyping relation (`Fs ⊇ FsRead,
      FsWrite`) — that needs the vocabulary + token-subtyping work and *does* break committed baselines
      (`Fs`→`FsWrite` reads as a gained effect → spurious AS-EFF-005). Cross-crate `Fs` carries no
      detail (the dependency's report doesn't record it). (Net-by-host is *not* statically knowable —
      won't do.)
- [x] **Cross-crate effect propagation** (CRITIQUE §8 — closed). Each report entry carries a stable
      `DefPathHash`; a dependent crate loads its dependencies' reports keyed by it (surviving
      reexport-shortened paths) and inherits their *already-transitive* effects. Fixed a real consumer
      whose `bin` under-reported the `Db`/`Net`/`Exec` it performs through its `lib`.
- [x] **Devirtualize concrete trait calls** (CRITIQUE §9 — closed). A method call on a concrete
      (non-`dyn`) receiver resolves to its single impl instead of CHA-expanding to *every* impl —
      removing the over-report where a pure `self.applies()` inherited a sibling rule's effect
      (104 fns de-over-reported on gitui, no soundness loss).
- [x] **`cargo candor explain <fn>` — effect provenance.** Traces the call path that gives a function
      each effect: `main` has `Net` *because* `main → middle → leaf`, and `leaf` calls
      `std::net::TcpStream::connect` at `main.rs:1`. Turns an effect *set* into a story you can follow
      to its source. Engine records effect *sites* (callee + span) under `CANDOR_EXPLAIN`; a BFS over
      the call graph finds the nearest source per effect. (Cross-crate and unresolvable calls are
      labelled as such — the path stops at the boundary.) Used by P0 §3.

## P3 — real enforcement

- [x] **Recognise cap-std capability types** in `declared_caps` (and its operations in `classify`):
      a project on cap-std now gets conformance against real, unforgeable capabilities for free,
      with candor as the visibility layer. Validated in `sample-capstd/`. Compile-time enforcement
      stays cap-std's job. (Mapped: Dir→Fs, Pool/TcpStream→Net, SystemClock→Clock, UnixStream→Ipc;
      extend the small `capstd_cap` table for more.)

## P4 — packaging / maintenance

- [x] Distribution: repo is **public** (git is the channel — `--git` / `git clone`, as AGENTS.md
      uses).
- [x] **crates.io distribution — vendored `span_lint`, dropped the only git dep (the old "can't" was
      wrong).** The prior note said crates.io is impossible because `clippy_utils` is a git dependency —
      but candor used clippy_utils for **exactly one function** (`diagnostics::span_lint`, a thin
      wrapper over `LintContext::emit_span_lint`). **Done:** vendored those few lines (minus clippy's
      docs-page link) into `src/lib.rs`, removed the dep. Verified `0` git sources in `Cargo.lock`;
      `cargo package` now packages candor and stops only on `candor-report` not yet being on crates.io
      (routine multi-crate release ordering), with no git-dep blocker. Added a `version` to the
      `candor-report` path dep so the manifest is publish-ready. ui tests confirm byte-identical
      diagnostics. The nightly + rustc-dev toolchain is still needed to *build* (`rustc_private`) — that
      was never the crates.io blocker. *(Pushback was right: I'd called this fundamental; it wasn't.)*
- [x] Nightly fragility (`rustc_private` pins `nightly-2026-04-16`) — the bump process is now a
      step-by-step in `CONTRIBUTING.md` (pick matching nightly+clippy_utils rev, fix rustc_private
      breakage, re-bless ui, re-baseline the self-guard).
- [x] **Automated nightly bump — shipped** (`.github/workflows/nightly-bump.yml`). The pin can't be
      removed while we're a dylint lint, but the *migration* is now a bot: a weekly (+ manual) workflow
      that pins a candidate nightly, runs the full build/test, **re-blesses the ui fixtures** and
      **re-baselines the self-guard**, and opens a PR if green — or fails loudly and files a tracking
      issue if the nightly broke `rustc_private` (the case that needs a human). Vendoring clippy_utils
      already removed the old "pick a matching clippy_utils rev" step, so a bump is now just "pick a
      nightly, fix any rustc_private breakage". *(Pushback item: "expensive maintenance", not a hard
      limit — now a notification.)*
- [x] Test coverage — unit (pure logic) + `ui_test` fixtures with blessed `.stderr` (copied from the
      framework-saved file, since compiletest has no bless) + scripted `tests/integration.sh`
      (AS-EFF modes, cross-crate, version stamping, audit) + `test-receipt.sh` (the bash receipt).
      **23 unit · 5 ui · 15 integration · 10 receipt**, all gated in CI.
- [x] JSON output via serde (correct escaping for any path/loc; replaced the hand-rolled escaper).
- [x] **Report provenance / versioning.** `build.rs` stamps the source commit + toolchain into the
      dylib (a `#[used]` `candor-build-version=` tag), the report envelope, and the calibrated sidecar;
      `cargo-candor` and the receipt read the *true* dylib version (not the source tree's HEAD), so a
      pulled-but-not-rebuilt engine can no longer masquerade as current and mask a stale baseline.
- [x] **v0.2 self-describing report envelope** `{ candor: {version, toolchain}, functions: [...] }`.
      All readers accept the legacy v0.1 bare array during migration (candor-spec §2).
- [x] **`cargo candor audit` at-a-glance profile** — effect tally, unresolvable-call list, coverage
      gaps, broadest-surface functions; `--all` keeps the full per-function lint.
- [x] **`cargo candor audit --coverage` — make the classifier ceiling auditable** (the principled
      partial-fix for "silent under-report via an uncovered dep", the most dangerous gap). The default
      coverage check only warned for crates matching the `candor-suspect` *name* heuristic, so an
      effectful crate whose name doesn't look effectful slipped through silently. `--coverage` now lists
      **every** external crate candor saw called but has no effect rules for — calls into them are
      assumed pure, so any I/O they perform is under-reported. Can't *eliminate* the ceiling (candor
      can't know an unanalyzed crate is effectful), but converts it from silent to visible — candor's
      honesty thesis applied to its own coverage. Fixed a latent false-positive en route: path-matched
      runtimes (`tokio`/`async_std`/`mio`, matched by module path not crate name) were absent from
      `CALIBRATED_CRATES` and would have been mislabeled blind spots; a new `path_crates` field in the
      `calibrated.json` sidecar marks them covered. Default also gained a one-line count hint;
      `candor-suspect` widened (subprocess/dns/serial/socket families). Integration-tested.
- [x] **candor-java: adopt the v0.2 envelope + first tests/CI — done.** It now emits the
      `{ candor: {version, toolchain}, functions }` envelope (git hash baked in at build time via
      `build-info.properties`, readers still accept v0.1 bare arrays), and ships a 26-check `test/
      smoke.sh` behavioural suite (over real bytecode fixtures) gated in `.github/workflows/ci.yml`
      (JDK 21). Reached near-parity with the Rust impl: cross-jar hash/inheritance + version-trust,
      `calls`, the `show/where/callers/map/diff` query layer (`--json`), fs read/write detail,
      lambdas/method-refs, and constructors.
- [x] **Engine-level version-aware cross-crate trust** (candor-spec §2.1 SHOULD): `load_cross_reports`
      now reads each sibling report's `candor.version`; on a mismatch with the running engine it
      downgrades the inherited effects to `Unknown` (can't trust analysis by rules this engine may have
      changed). Legacy v0.1 reports have no version → trusted as before. Tested (mismatch → Unknown,
      match → effects as-is).
- [x] **De-duplicated the coverage `SUSPECT` heuristic** — now a single `candor-suspect` file at the
      clone root, read by both `candor-run.sh` (via `CANDOR_HOME` / its own location) and `cargo-candor`
      (via `CANDOR_DIR`), with a graceful skip if missing. No more two-copy drift.
- [x] **Ported the tooling/query layer from bash+Python to a Rust CLI binary.** The engine (`lib.rs`)
      *must* be Rust (a `rustc_private` dylint lint), but the wrapper — `cargo-candor`'s diff /
      show / where / callers / audit logic — was bash with embedded Python for JSON. That was the
      fast, zero-install choice, but a recurring *glue*-bug source (the sidecar/report glob collision,
      quoting, state-hash matching) with **duplicated logic** (report-reading re-implemented in nearly
      every Python snippet; the `SUSPECT` heuristic copy-pasted).
      **Done:** a Cargo **workspace** now holds the lint plus two no-`rustc_private` crates —
      `crates/candor-report` (the report structs + envelope-or-bare-array parsing, the single source
      of truth the lint and CLI both depend on) and `crates/candor-query` (the read-only
      `audit`/`show`/`where`/`callers`/`map`/`diff` commands, one typed binary over those structs).
      `cargo-candor` dispatches to it and is now **python-free** (606 → 355 lines, 251 lines of inline
      Python deleted). The port was verified **byte-for-byte** against the Python it replaced
      (identical human output for every command; `diff --json` identical content, now deterministically
      ordered instead of Python's hash-order). Thin bash remains only for genuine shell glue
      (orchestrating `cargo dylint`, the fast-path freshness check, `watch`). *(The Stop-hook receipt
      `candor-run.sh` and the MCP stdio server stay in Python — they're hook orchestration / a
      protocol server that already delegates report logic to `cargo-candor`, not duplicated query
      logic; folding them onto `candor-query` is a possible follow-up, not required for DRY.)*

## P5 — research (the thesis)

- [~] Controlled eval of *edit quality* (not just analysis cost) with independent ground truth and
      multiple trials — **the gate on P0** (see P0 §5). The pilot (`EVAL.md`) showed consumability +
      efficiency, that a source-only agent can beat the report where the classifier has a gap, and (Trial
      5) that candor's edit-feedback lifts non-local awareness on one task.
      **Now scaled — first batch shipped:** `eval/scaled/` is a pre-registered, reproducible 3-task ×
      2-arm × 2-trial harness (fixtures with candor-verified ground truth, blind judge, falsification
      clause). Batch 1 (`eval/scaled/RESULTS.md`) found a *completeness* gap but was confounded — it also
      exposed its own metric mis-spec, fixture leakage, and an answer-key contamination bug (all fixed).
      **Batch 2 (`eval/scaled/RESULTS-v2.md`) — clean, pre-registered:** completeness as primary,
      de-leaked fixtures, weaker model (Sonnet), N≈9/arm, contamination fixed and fully re-run. Result:
      **control completeness 7% vs treatment 100%** (binary 0.17 vs 1.00); neither falsification
      condition triggered — candor's edit-feedback takes a realistic agent from naming ~0 of the
      non-local callers to all of them, for ~5% extra tokens, a large lift consistent across 3 tasks.
      **Batch 3 (`eval/scaled/RESULTS-v3.md`) — the untested cell, shipped:** a *large* fixture
      (`tasks-v3/orderflow` — Net propagating to **16** non-local fns across 9 files / 3–5 call-graph
      layers, vs 7/4 in the small tasks) × a **frontier** agent (Opus-class), 2 arms × 3 trials, blind
      frontier judges, pre-registered (fixture + 16-fn denominator committed before the run). Result:
      **control completeness 6% vs treatment 79%** (conservative; ~100% under the lenient blanket
      reading 2 of 3 judges applied), gap 0.73 — neither falsification condition triggered. **Decisive
      new finding:** a frontier model does **not** close the gap — the Opus control still named only the
      one helper it edits past (1/16) and wrote "callers unaffected"; the missing thing is call-graph
      *enumeration*, not capability, which is exactly what candor supplies. The 6%→100% effect is not a
      small-fixture / weak-model artifact. **Remaining:** more tasks/power at this scale, and
      edit-quality-beyond-summary measures (does the agent *act* on the propagation, not just report it).
- [x] Effect-aware PR review — `examples/candor-pr-review.yml`: a workflow that POSTS the per-function
      effect delta vs the baseline as a PR comment + step summary (the *review-time* sibling of P0's
      *edit-time* loop; both powered by `cargo candor diff`). It informs rather than blocks (pair with
      `candor-guard.yml` to also fail); an AI reviewer can consume the same via `diff --json`.
- [x] **Formal semantics** — `candor-spec/SEMANTICS.md`: the effect lattice, call-site resolution
      rules (CLASSIFY/CROSS/DEVIRT/CHA/EXEMPT/UNKNOWN), the transitive least-fixpoint, cross-crate
      composition, the conformance predicates, and the conditional-soundness properties (with the two
      honesty caveats). The implementation was then verified against it clause-by-clause.

## Done (recent, for context)

Unknown/AS-EFF-003 · CANDOR_CONFIG · CANDOR_NO_AMBIENT/AS-EFF-004 · CANDOR_PARANOID ·
CANDOR_BASELINE/AS-EFF-005 · ICE hardening · raw-socket + HTTP + Rand + **Db + Ipc** classification ·
**const/static initializers (macro-filtered)** · **main entry-point exemption** · unit tests ·
`cargo-candor` wrapper · CI + downstream guard workflow · self-guard ·
**CHA: see through dyn/generic dispatch over local traits**.

_Latest pass:_ cross-crate propagation (DefPathHash) · devirtualization · report provenance &
versioning · v0.2 self-describing envelope · `cargo candor audit` at-a-glance profile · formal
`SEMANTICS.md` + a clause-by-clause code↔spec audit · remediated a real consumer's stale baseline.

_Hardening pass:_ Python→Rust query port (`candor-report` + `candor-query`) · Fs read/write detail ·
frictionless `install.sh` + stable `~/.candor` home · dylib/query resolution by newest-mtime (was the
`head -1` glob) · self-guard trusts the baseline's own siblings (no spurious cross-engine downgrade) ·
fixpoint profiled on ripgrep (negligible).

## Known limitations (confirmed by review 2026-05-29; documented, not all worth fixing)

- [~] **declared_caps now peels common wrappers** — a capability behind `Option<&Fs>` / `Vec<&Fs>` /
  `Box` / `Result` / a tuple is recognised (was a false AS-EFF-001). `caps_in_ty` recurses into type
  arguments (bounded depth). **Residual:** a cap behind a user STRUCT FIELD (a `Caps { fs: &Fs }`
  bundle) still isn't seen — that needs recursing into ADT field types, not just generic args.
- **Macro-generated functions are skipped** (`span.from_expansion()`), so a fn generated by a user
  macro (async_trait/derive/decl-macro) is invisible in all modes. Filter is needed for tracing
  noise; distinguishing user-macro output from compiler noise is the open part.
- **const/static initializers don't propagate as callees** — their init effects are reported
  standalone but not inherited by code that references the const (we only follow `Fn`/`AssocFn`
  call edges, and a const reference isn't a Call expr anyway).
- **Baseline key is `def_path_str`** — not guaranteed unique; identical stringly-named items
  (rare) could collide in the guard. Names are the only stable cross-run key, so this is inherent.
- **Effect/fs fixpoint is naive iterate-to-convergence**, not a worklist (`O(rounds × V × E)` per
  crate). Measured negligible: on ripgrep (52k lines) the largest crate (`rg`: 1179 fns / 3302 edges)
  fixpoints in 0.42 ms, ~0.7 ms total across the workspace — ~0.017% of the run. Per-crate cost is
  dominated by call resolution + rustc, not propagation, so a worklist rewrite isn't worth it.
