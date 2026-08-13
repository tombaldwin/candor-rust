# Contributing

Thanks for looking. candor is a small, opinionated tool; the opinions are in
[PRINCIPLES.md](PRINCIPLES.md) and they apply to contributions too — most of all: **never make
candor silently wrong.** A change that adds a false "this is pure" is worse than no change.

## Build & test

```sh
cargo install cargo-dylint dylint-link   # once
cargo build --workspace                  # the lint + the tooling crates (candor-report, candor-query);
                                         # the pinned nightly (rust-toolchain) + rustc-dev are fetched
                                         # automatically by rustup. `--workspace` because candor-query
                                         # is a member, not a dependency of the lint.
cargo test --workspace                   # classifier unit tests + a load smoke-test + the tooling crates
cargo clippy --all-targets -- -D warnings # the WHOLE workspace, on the pinned nightly (which carries
                                         # clippy — see rust-toolchain). This is the one to run: it is
                                         # the only command that lints the rustc_private dylint lib and
                                         # `build.rs`, neither of which the stable gate below can see.
cargo +stable clippy -p candor-report -p candor-query -p candor-classify -p candor-scan \
  --all-targets -- -D warnings           # the other CI lint gate: STABLE clippy over the four stable
                                         # crates — what an adopter actually builds. Scoped with -p
                                         # because stable clippy cannot compile the rustc_private lib.
```

Both are gated in CI, so neither result needs qualifying — **provided you ran both.** Running only the
first is not evidence, and the failure is quiet: the pinned nightly's clippy exits 0 on two adjacent
`#[test]` attributes where CI's STABLE leg errors with `duplicate-macro-attributes`. That cost a CI
round-trip on 2026-08-03, and the same commit carried the worse half — a stranded `#[test]` that
SILENTLY DISABLED a liveness test, which local `cargo test` could not see either, because a duplicated
attribute registers the test twice and the total stays flat.

So: a green nightly leg means the rustc_private lib and `build.rs` are clean, and nothing more. **The
stable leg is the authority on what an adopter actually compiles**, and a local run that skipped it
should be reported as "nightly clippy clean", never as "clippy clean".

If a nightly bump adds a lint, the whole-workspace leg is where it surfaces.


Try it on a real project with the wrapper (put this repo on `PATH`): `cargo candor audit`.

## The most common contribution: teaching the classifier a new crate

`classify()` in `src/lib.rs` maps a resolved callee (crate + path) to an effect. If candor reports
a function as effect-free when it actually does I/O, the classifier is usually missing a crate.

Two rules, both load-bearing:

1. **Match the I/O boundary, not the crate.** Builder-pattern crates (AWS SDK, `reqwest`, DB
   clients) are mostly *pure* construction; only the dispatch (`.send()`, `.execute()`, a query
   verb) is the effect. Tag that, not the whole crate — over-reporting erodes trust as much as
   under-reporting hides danger. (See the `aws_sdk_*` / `reqwest` / `DB_CRATES` arms for the shape.)
2. **Add a unit test pinning the precision** — both a positive case *and* a negative one (e.g.
   `std::net::TcpStream` is `Net` but `std::net::SocketAddr` is not). The `tests` module is full of
   these; copy the pattern.

For a crate specific to *your* project, you usually don't need code at all — use a `CANDOR_RULES`
rules file (see the README). (`CANDOR_CONFIG` now names the spec-§3.4 `.candor/config` override path.)

## If you can't make it sound, make it honest

If a case genuinely can't be resolved (dynamic dispatch over a non-local trait, a closure through a
callback), it must surface as `Unknown` — not be silently dropped or guessed. If you're deferring
something, write it in [BACKLOG.md](BACKLOG.md) **with the concrete blocker**, the way the existing
entries do. "We didn't do X, here's exactly why" is the house style.

## Toolchain bumps

`rust-toolchain` (the nightly) and the `clippy_utils` git `rev` in `Cargo.toml` are coupled — bump
them together. Step by step:

1. **Pick a nightly + matching `clippy_utils` rev.** They must agree: open the rust-clippy repo at the
   candidate `rev` and read *its* `rust-toolchain` — that's the nightly this rev expects. Use that
   nightly. (`dylint_linting`'s release notes also state the nightly each version targets.)
2. **Update both pins:** `channel = "nightly-YYYY-MM-DD"` in `rust-toolchain`, and the `rev = "…"` for
   `clippy_utils` in `Cargo.toml`. `rustup toolchain install nightly-YYYY-MM-DD --component rustc-dev llvm-tools-preview`.
3. **`cargo build`, fix the `rustc_private` breakage.** HIR/`TyCtxt`/`LateContext` APIs drift between
   nightlies — function renames, signature changes, moved items. The compiler errors are the to-do list.
4. **The dylib name changes** (`libcandor@<toolchain>-<triple>.dylib`). Nothing to do in-repo (build.rs
   reads `rust-toolchain` and re-stamps automatically), but a consuming project's `.candor/config`
   `CANDOR_LIB` path must be repointed at the new filename (or just rebuilt).
5. **Re-bless the `ui_test` fixtures** if diagnostics shifted: run the ui tests, copy each framework-saved
   `.stderr` over the committed one (compiletest has no `--bless`).
6. **Re-baseline the self-guard:** the engine version changed, so `cargo candor snapshot .candor/baseline`
   here (and in any consuming project) — the version-aware guard otherwise (correctly) refuses to
   compare across engines.
7. **Verify:** `cargo test --workspace`, `bash tests/integration.sh`, and the CI self-guard all green.

## Before you open a PR

- `cargo test --workspace` is green, and you added a test for new classification behavior.
- If your change alters **candor's own** effect surface, refresh its baseline:
  `cargo candor snapshot .candor/baseline` (CI runs candor on candor and will fail otherwise).
- Diagnostics stay actionable and quiet — no new noise in audit output.
