# candor-scan

**A stable-Rust effect scanner for Rust code.** It maps which functions perform side effects —
filesystem, network, subprocess, database, clock, env, randomness, IPC — and **how those effects
propagate transitively across the call graph**, including the *blast radius* of editing any function.

No nightly, no `rustc_private`, no dylint. It parses your `.rs` files with [`syn`](https://docs.rs/syn)
and runs anywhere `cargo` does — CI, a locked-down box, or `cargo install`. It does **not build your
crate**, so it can even analyze a dependency's source without compiling it.

```sh
cargo install candor-scan
candor-scan path/to/crate              # writes <crate>/.candor/report.<crate>.scan.json
candor-scan . --json                   # print the report to stdout instead
```

The report is the same JSON the full [candor](https://github.com/tombaldwin/candor-rust) nightly lint
produces, so the candor CLI's read-only queries (`show`/`where`/`callers`/`map`) read it identically.

## What it does well, and where it stops

`candor-scan` is **syntactic** — it sees what's *written*, not what the compiler *resolves*. It is
calibrated to **never fabricate an effect** (validated across 1294 real crates: zero false positives on
a curated-pure set). When it can't see an effect, it stays silent rather than guessing — an honest
under-report, never a wrong label.

- **Catches:** path-qualified effect calls (`std::fs::read`, `Command::new`, `reqwest::Client::execute`),
  `use`-aliases, transitive propagation, calls hidden in macros (`try_call!`, `println!`), four C-library
  FFI tiers (libc, libsqlite3, libgit2, libssl), and method dispatch via light local type inference
  (struct fields, params, constructors, factory return types).
- **Honest `Unknown`:** when the body invokes a callable the scan can't see through — a closure or
  fn-pointer held in a field, dispatch table, or index (`(self.handler)()`, `arr[i]()`) — it marks the
  function `Unknown` rather than silently certifying it pure, and propagates that like any effect (so
  the receipt's unresolved count is truthful, not a hardcoded 0). A LOCAL closure whose body IS visible
  (`let f = |..| ..; f()`) is NOT flagged — its effects were already walked lexically.
- **Misses (silently):** effects reached only through trait-object (`dyn`) dispatch on an uninferrable
  receiver, **`Deref`-coercion receivers** (`self.agent.run()` where the field is a wrapper that derefs
  to the type owning `run`), **generic-parameter fields** (`struct B<S>(S)` — `self.0.method()` can't be
  typed without instantiating `S`), overloaded operators / `?` / `.await` desugars, RAII drops (an
  effectful `Drop::drop` has no call expression — it runs at scope end), a custom `Iterator::next`
  reached only via a `for`-loop desugar, and cross-crate propagation by stable identity. These need the
  semantic resolution only the nightly lint has (the soundness fuzzer locks the desugar forms against
  the lint: `op_add`/`index`/`deref`/`try_from`/`await_poll`/`drop`/`iterator`/`eq`/`add_assign`).
  The Deref and generic-field shapes are measured in the wild (the PROVE-IT dogfood on `ureq`): 14 of a
  16-function blast radius found, those two shapes the remainder — under-reported, never fabricated.

**The policy gate floor.** `candor-scan <dir> --policy <file>` (or `CANDOR_POLICY=…`) enforces a
spec-§6.2 policy (`deny`/`pure`/`allow`/`forbid` — parsed by the same shared grammar as the nightly and
JVM gates) over the scan and exits 1 on violation. It is the **advisory floor**: the syntactic backend
under-reports, so a missed effect can pass — a clean run is necessary, never sufficient. It still
catches every boundary crossing the scan *can* see, deterministically, with zero extra install.

For the soundness contract (`Unknown` over-approximation), conformance checking, and the **sound**
policy/guard CI gates, use the full nightly [candor](https://github.com/tombaldwin/candor-rust) lint. The
two share one classifier and one policy parser, so they never disagree on what counts as an effect or
what a rule means.

## Usage

```
candor-scan [<dir>] [--out <prefix>] [--json] [--include-tests]
```

- `<dir>` — crate root to scan (default `.`).
- `--out <prefix>` — report path prefix (default `<dir>/.candor/report`); writes
  `<prefix>.<crate>.scan.json` plus a call-graph sidecar.
- `--json` — print the report to stdout instead of writing files.
- `--include-tests` — also scan `tests/`/`benches/`/`examples/` and `#[cfg(test)]` modules (off by
  default, so the report describes the crate, not its test harness).

Requires a reasonably recent stable Rust (edition 2024 → Rust 1.85+).

Licensed under MIT OR Apache-2.0. Part of [candor](https://github.com/tombaldwin/candor-rust); see the repo
for the calibration (`eval/calibration/`) and the full effect model.
