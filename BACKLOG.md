# candor backlog

Consolidated open items. Honest priority order within each section. Sources noted:
`CRITIQUE.md` (self-assessment), `EVAL.md` (the agent pilot), and hands-on findings.

## P1 — correctness (silent wrong answers are the worst failure)

- [ ] **Classifier coverage: database clients.** `sqlx`, `rusqlite`, `postgres`/`tokio_postgres`,
      `diesel`, `redis`, `mongodb`, `mysql` are unrecognised → silent false negatives, exactly like
      the `reqwest` gap the eval caught. Add them *precisely* (match query execution, not query
      building) with unit tests. Highest-confidence next win.
- [ ] **Classifier coverage: stdio + more.** `std::io::{stdin,stdout,stderr}`, `println!`/`eprint!`
      (console I/O), `mmap`/`memmap2`. Decide whether console is its own effect or noise for TUIs.
- [ ] **Generic static trait dispatch is assumed pure.** `t.method()` for `t: T: Trait` isn't
      marked `Unknown` (only `CANDOR_PARANOID` does, and it's noisy). Residual unsoundness — look at
      a smarter heuristic (e.g. only flag when the bound's trait has *any* effectful impl in-crate).
- [ ] **Escaping closures.** A capability captured into a returned closure / stored handler isn't
      propagated across function boundaries. Needs the effect to ride the function type.
- [ ] **const/static initializers are skipped.** `enclosing_named_fn` returns `None` for `Const`/
      `Static` body owners, so effects in `static X: T = effectful();` / `lazy_static!` are dropped.
- [ ] **`tokio::net` Unix-domain sockets counted as `Net`.** Local IPC (`UnixStream`/`UnixListener`)
      isn't really network (noted in `EVAL.md`). Either exclude or split into an `Ipc` effect.

## P2 — depth / precision

- [ ] **Reachability / dead-code elimination.** Effects in never-called functions are still
      reported. Cackle does proper reachability; candor doesn't. Matters most for conformance.
- [ ] **Finer `Fs` granularity (read vs write).** Deferred deliberately: needs an expanded effect
      vocabulary *and* a capability-token subtyping relation (`Fs ⊇ FsRead, FsWrite`) so existing
      `&Fs` declarations still satisfy them. (Net-by-host is *not* statically knowable — won't do.)
- [ ] **Entry-point handling in strict mode.** `main` (and similar roots that legitimately hold the
      whole capability bundle) always flag AS-EFF-001. Consider special-casing roots.

## P3 — real enforcement

- [ ] **Recognise cap-std capability types** in `declared_caps`, so projects using cap-std get
      conformance for free against real (unforgeable, compile-enforced) capabilities.
- [ ] **Compile-time enforcement** is out of scope for a lint — document that cap-std is the answer
      when you need the bad call to fail to *compile*, and position candor as the visibility layer.

## P4 — packaging / maintenance

- [ ] **Distribution.** Repo is private; users reference candor by path. Decide: public mirror,
      crates.io publish, or a `cargo install candor` binary that vendors the lint.
- [ ] **Nightly fragility.** `rustc_private` pins `nightly-2026-04-16`; toolchain bumps will break
      it. Document the bump process; consider a CI matrix when bumping.
- [ ] **Behavioural (UI) test coverage.** compiletest_rs 0.11.2 has no bless support, so coverage is
      unit-tests on the pure logic only. Revisit if dylint_testing gains bless, or add an
      integration test that runs the lint on the sample and asserts on parsed output.
- [ ] **JSON output via serde.** Output is hand-rolled (escaping is minimal); switch to serde for
      correctness now that the dep is present (input already uses it).

## P5 — research (the thesis)

- [ ] **Controlled eval of *edit quality*.** The pilot (`EVAL.md`) showed the report is cheap to
      consume and ~3–6× faster than reading source, but a source-only agent was *more accurate*
      where the classifier had a gap. Still unproven: that an agent's actual *edits* improve with
      the report. Needs many tasks, multiple trials, and independent ground truth.
- [ ] **Effect-aware PR review agent.** Natural application: feed the baseline diff (AS-EFF-005) into
      an agent reviewing a PR — "this change adds Net to `handle_foo`; is that intended?"

## Done (recent, for context)

Unknown/AS-EFF-003 (no lying by omission) · CANDOR_CONFIG · CANDOR_NO_AMBIENT/AS-EFF-004 ·
CANDOR_PARANOID · CANDOR_BASELINE/AS-EFF-005 regression guard · ICE hardening · raw-socket + HTTP
(`reqwest`/`ureq`/`isahc`) + `Rand` classification · unit tests · `cargo-candor` wrapper · CI +
downstream guard workflow · self-guard (candor guards candor).
