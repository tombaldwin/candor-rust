# candor backlog

Honest priority order within each section. Sources: `CRITIQUE.md`, `EVAL.md`, hands-on findings.

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
- [ ] **4. Speed for a tight loop.** A full re-lint (~minutes on a big crate) is too slow per edit;
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
- [ ] **7. Effects → *risk* (argument provenance / taint-lite) — the high-value frontier.** The eval's
      real bug was an `Fs` effect whose path came from an untrusted parameter; candor knows the effect
      but not that its argument is attacker-derivable. Even a crude "this effect's key argument flows
      from a fn parameter, not a literal/validated value" moves candor from "you do Fs" to "you do Fs
      on caller-controlled input" — where the dangerous class lives. Biggest value, biggest effort
      (real data-flow); scope as a bet, not a quick win.
- [ ] **8. Speed for large codebases — the enabler, re-prioritized up.** candor is most valuable
      exactly where it's slowest (big repos). A persistent-report / changed-crate-only path (or a
      background daemon) makes the valuable case practical. Without it, 6 and 7 don't get used where
      they'd matter most.
- [ ] **9. Selectivity — surface only the *consequential* propagation.** Make `diff`/self-review lead
      with "an effect reached an entry point / a should-be-pure fn / a hot path" rather than listing
      every inheritor. Less noise, focuses the agent on the propagation that *matters*. Cheap; builds
      on the introduced/inherited split.

**Not worth doing:** more interactive-loop polish (call-site line, prettier output) — the eval says
that's the narrow, modest-value axis. Diminishing returns.

## P1 — correctness (silent wrong answers are the worst failure)

- [x] **Database clients.** `sqlx`/`rusqlite`/`postgres`/`tokio_postgres`/`diesel`/`redis`/… now
      classified `Db` (execution verbs only, not query building; best-effort, tune via CONFIG).
- [x] **`tokio::net` / `std::os::unix::net` Unix sockets** → `Ipc`, no longer conflated with `Net`.
- [x] **const/static initializers** now reported (a `static X = effectful()` performs its effect),
      with macro-generated items (e.g. tracing `__CALLSITE` statics) filtered out via
      `span.from_expansion()` — that filter was needed; without it the report flooded.
- [x] **memmap2** → `Fs`.
- [x] **Trait dispatch (dyn + generic) over local traits** — broke through with Class Hierarchy
      Analysis: a call to a locally-defined trait method now adds edges to all impls, so effects
      propagate through `dyn` and generic dispatch (sound over-approximation). On ebman this resolved
      the LLM feature and dropped `Unknown` 100→92, of which only 6 are now *purely* Unknown.
      `CANDOR_PARANOID` remains the opt-in for the residual *non-local* generic-dispatch gap.
- [ ] **Escaping closures / `impl Fn` callbacks** — the deep residue (the 6 purely-`Unknown` fns on
      ebman are mostly this). Needs effects to ride in *function types* (interprocedural closure
      flow) — a MIR-level engine, not an HIR patch. Partly *not a hole*: an effectful closure's
      effect is attributed to the function that lexically defines it, so it usually lands on the
      caller anyway. **Deferred** (small, characterized residue).
- [~] **stdio / println.** Decided *against* for now: `std::io::stdout`/`println!` is pervasive and
      low-signal (especially for TUIs); would add noise without authority-level value. Reconsider if
      a use case appears. `stdin` (real input) could be added later as its own effect.

## P2 — depth / precision

- [x] **Entry-point handling in strict mode.** `main` no longer raises AS-EFF-001 (it's the root
      that legitimately holds the whole capability bundle).
- [ ] **Reachability / dead-code elimination.** CHA made the call graph much more complete (it now
      has edges through local trait-object/generic dispatch), but it's still missing closure/std-`dyn`
      edges, so reachability would *still* mislabel some closure-reached code as dead. Closer to
      soundness than before, but not there yet — **deferred** until the closure-flow gap closes.
- [ ] **Finer `Fs` granularity (read vs write).** **Deferred — high cost, breaking:** needs an
      expanded effect vocabulary *and* a capability-token subtyping relation (`Fs ⊇ FsRead, FsWrite`)
      so existing `&Fs` declarations still satisfy them, AND it breaks every committed baseline
      (`Fs` → `FsRead`/`FsWrite` reads as a gained effect → spurious AS-EFF-005). A non-breaking
      JSON-only refinement (report read/write detail, keep the `Fs` effect) is possible if wanted.
      (Net-by-host is *not* statically knowable — won't do.)
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
      uses). Note: crates.io is **not** an option for the lint itself — it depends on `clippy_utils`
      via a git dependency, which crates.io forbids (true of every dylint lint). A separate, non-lint
      helper crate *could* be published, but the lint can't.
- [ ] Nightly fragility (`rustc_private` pins `nightly-2026-04-16`); document the bump process.
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
- [ ] **candor-java: adopt the v0.2 envelope + first tests/CI.** It still emits v0.1 bare arrays
      (accepted by readers, but it should self-describe), and has *no* automated tests — its
      `sample/`, `conf-sample/`, `spring-sample/`, `cha-sample/` dirs are ready-made fixtures.
- [ ] **Engine-level version-aware cross-crate trust** (candor-spec §2.1 SHOULD): when a sibling
      report's version differs from the running engine, downgrade its inherited effects to `Unknown`
      rather than trust them. Low priority — within one run all crates share a dylib so versions match,
      and `cargo candor guard` already skips on a baseline/engine mismatch; defense-in-depth.
- [ ] **De-duplicate the coverage `SUSPECT` heuristic** — currently copied in `candor-run.sh` and
      `cargo-candor`; factor into one sourced snippet so they can't drift.

## P5 — research (the thesis)

- [ ] Controlled eval of *edit quality* (not just analysis cost) with independent ground truth and
      multiple trials — **the gate on P0** (see P0 §5). The pilot (`EVAL.md`) only showed consumability
      + efficiency, and that a source-only agent can beat the report where the classifier has a gap.
- [ ] Effect-aware PR-review agent fed by the baseline diff (AS-EFF-005) — the *review-time* sibling of
      P0's *edit-time* loop; both are powered by `cargo candor diff` (P0 §1).
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

## Known limitations (confirmed by review 2026-05-29; documented, not all worth fixing)

- **declared_caps sees only direct parameters** — a capability behind a struct field / `Option` /
  `Vec` / generic isn't recognized as declared (would false-positive AS-EFF-001). Recursing into
  field types is the fix; non-trivial.
- **Macro-generated functions are skipped** (`span.from_expansion()`), so a fn generated by a user
  macro (async_trait/derive/decl-macro) is invisible in all modes. Filter is needed for tracing
  noise; distinguishing user-macro output from compiler noise is the open part.
- **const/static initializers don't propagate as callees** — their init effects are reported
  standalone but not inherited by code that references the const (we only follow `Fn`/`AssocFn`
  call edges, and a const reference isn't a Call expr anyway).
- **Baseline key is `def_path_str`** — not guaranteed unique; identical stringly-named items
  (rare) could collide in the guard. Names are the only stable cross-run key, so this is inherent.
- **`cargo-candor` picks `head -1`** of the dylib glob — a stale dylib from a previous toolchain
  could be chosen after a bump. Minor.
