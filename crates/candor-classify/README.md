# candor-classify

The curated effect classifier for [candor](https://github.com/tombaldwin/candor): a pure
`classify(crate, path) -> Option<Effect>` over calibrated third-party crates and standard-library/FFI
entry points (libc, libsqlite3, libgit2, libssl, …). Pure string logic, no `rustc` — one source of
truth shared by the nightly lint and the stable [`candor-scan`](https://crates.io/crates/candor-scan)
backend, so the two never drift on what counts as an effect.

Licensed under MIT OR Apache-2.0.
