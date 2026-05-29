# Contributing

Thanks for looking. candor is a small, opinionated tool; the opinions are in
[PRINCIPLES.md](PRINCIPLES.md) and they apply to contributions too — most of all: **never make
candor silently wrong.** A change that adds a false "this is pure" is worse than no change.

## Build & test

```sh
cargo install cargo-dylint dylint-link   # once
cargo build                              # builds the lint; the pinned nightly (rust-toolchain)
                                         # + rustc-dev are fetched automatically by rustup
cargo test                               # unit tests over the classifier + a load smoke-test
```

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

For a crate specific to *your* project, you usually don't need code at all — use a `CANDOR_CONFIG`
rules file (see the README).

## If you can't make it sound, make it honest

If a case genuinely can't be resolved (dynamic dispatch over a non-local trait, a closure through a
callback), it must surface as `Unknown` — not be silently dropped or guessed. If you're deferring
something, write it in [BACKLOG.md](BACKLOG.md) **with the concrete blocker**, the way the existing
entries do. "We didn't do X, here's exactly why" is the house style.

## Toolchain bumps

`rust-toolchain` (the nightly) and the `clippy_utils` git `rev` in `Cargo.toml` are coupled — bump
them together. Expect `rustc_private` API breakage; fix, then make sure `cargo test`, the CI
behavioural check, and the self-guard all pass.

## Before you open a PR

- `cargo test` is green, and you added a test for new classification behavior.
- If your change alters **candor's own** effect surface, refresh its baseline:
  `cargo candor snapshot .candor/baseline` (CI runs candor on candor and will fail otherwise).
- Diagnostics stay actionable and quiet — no new noise in audit output.
