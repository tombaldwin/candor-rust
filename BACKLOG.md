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
- [ ] **Generic static trait dispatch is assumed pure.** `CANDOR_PARANOID` flags *all* trait
      dispatch (too noisy for default). A precise default needs whole-crate impl analysis ("does any
      in-crate impl of this trait have effects?") — real design work, not a patch. **Deferred.**
- [ ] **Escaping closures.** A capability captured into a returned closure / stored handler isn't
      propagated across function boundaries. This needs effects to ride in *function types*
      (interprocedural) — a capability candor fundamentally lacks; can't be patched in. **Deferred.**
- [~] **stdio / println.** Decided *against* for now: `std::io::stdout`/`println!` is pervasive and
      low-signal (especially for TUIs); would add noise without authority-level value. Reconsider if
      a use case appears. `stdin` (real input) could be added later as its own effect.

## P2 — depth / precision

- [x] **Entry-point handling in strict mode.** `main` no longer raises AS-EFF-001 (it's the root
      that legitimately holds the whole capability bundle).
- [ ] **Reachability / dead-code elimination.** **Deferred for a concrete reason:** candor's call
      graph is *intentionally incomplete* — dynamic dispatch / fn-pointers / callbacks produce no
      edges (they're `Unknown`). A reachability pass over that graph would mislabel dyn-reached code
      (event handlers, trait-object callbacks) as dead — actively misleading, which violates the
      "no lying" principle. Would only be sound atop a complete call graph.
- [ ] **Finer `Fs` granularity (read vs write).** **Deferred — high cost, breaking:** needs an
      expanded effect vocabulary *and* a capability-token subtyping relation (`Fs ⊇ FsRead, FsWrite`)
      so existing `&Fs` declarations still satisfy them, AND it breaks every committed baseline
      (`Fs` → `FsRead`/`FsWrite` reads as a gained effect → spurious AS-EFF-005). A non-breaking
      JSON-only refinement (report read/write detail, keep the `Fs` effect) is possible if wanted.
      (Net-by-host is *not* statically knowable — won't do.)

## P3 — real enforcement

- [ ] Recognise cap-std capability types in `declared_caps` (conformance against real, unforgeable
      capabilities). Compile-time enforcement stays out of scope — that's cap-std's job; candor is
      the visibility layer.

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
`cargo-candor` wrapper · CI + downstream guard workflow · self-guard.
