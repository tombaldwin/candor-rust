# candor backlog

Honest priority order within each section. Sources: `CRITIQUE.md`, `EVAL.md`, hands-on findings.

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

## P3 — real enforcement

- [x] **Recognise cap-std capability types** in `declared_caps` (and its operations in `classify`):
      a project on cap-std now gets conformance against real, unforgeable capabilities for free,
      with candor as the visibility layer. Validated in `sample-capstd/`. Compile-time enforcement
      stays cap-std's job. (Mapped: Dir→Fs, Pool/TcpStream→Net, SystemClock→Clock, UnixStream→Ipc;
      extend the small `capstd_cap` table for more.)

## P4 — packaging / maintenance

- [ ] Distribution: repo is private, path-referenced. Decide public mirror / crates.io / installable.
- [ ] Nightly fragility (`rustc_private` pins `nightly-2026-04-16`); document the bump process.
- [ ] Behavioural (UI) test coverage — compiletest_rs 0.11.2 has no bless; coverage is unit tests
      on pure logic only. Revisit if dylint_testing gains bless.
- [ ] JSON output via serde (input already uses it; output is hand-rolled).

## P5 — research (the thesis)

- [ ] Controlled eval of *edit quality* (not just analysis cost) with independent ground truth and
      multiple trials. The pilot (`EVAL.md`) only showed consumability + efficiency — and that a
      source-only agent can beat the report where the classifier has a gap.
- [ ] Effect-aware PR-review agent fed by the baseline diff (AS-EFF-005).

## Done (recent, for context)

Unknown/AS-EFF-003 · CANDOR_CONFIG · CANDOR_NO_AMBIENT/AS-EFF-004 · CANDOR_PARANOID ·
CANDOR_BASELINE/AS-EFF-005 · ICE hardening · raw-socket + HTTP + Rand + **Db + Ipc** classification ·
**const/static initializers (macro-filtered)** · **main entry-point exemption** · unit tests ·
`cargo-candor` wrapper · CI + downstream guard workflow · self-guard ·
**CHA: see through dyn/generic dispatch over local traits**.
