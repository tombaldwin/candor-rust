# candor-report

Shared report types and parsing for [candor](https://github.com/tombaldwin/candor-rust) — the v0.2
self-describing effect-report envelope (`{candor:{version,toolchain}, functions:[...]}`) and the
`ReportEntry`/`ReportMeta` types. Pure, stable Rust (no `rustc_private`); used by the candor lint, the
stable [`candor-scan`](https://crates.io/crates/candor-scan) backend, and the CLI.

Licensed under MIT OR Apache-2.0.
