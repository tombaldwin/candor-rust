# candor

A type-aware **capability/effect checker for Rust**, built as a [dylint](https://github.com/trailofbits/dylint) lint.

It answers two questions about a Rust codebase:

1. **What effects does each function actually perform?** — network, filesystem, process spawn,
   env reads, clock reads, logging, clipboard — including effects inherited transitively through
   the functions it calls.
2. **Are the signatures honest?** — once you thread explicit capability tokens through a module,
   it flags any function that performs an effect it does not declare.

It works by resolving every call's `DefId` and classifying the crate/path it lands in. That type
resolution is the whole point: a bare `.send()` is meaningless syntactically, but the resolved
method tells us it belongs to `aws_sdk_*` → a network effect. A purely syntactic tool can't do this.

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | the entire lint — classifier, per-function call-graph fixpoint, the three modes |
| `sample/` | a small crate written in the capability discipline, for trying conformance mode |
| `rust-toolchain` | pins the nightly the lint links against (`rustc-dev`) |

## Setup

```sh
cargo install cargo-dylint dylint-link   # once per machine
cargo build                              # builds the lint; first build downloads the nightly + rustc-dev
```

The build produces `target/debug/libcandor@<toolchain>-<platform>.dylib` (`.so` on Linux).

## Use

From any Rust project root, with `LINT` set to that dylib's absolute path:

```sh
# AUDIT (default): every function's transitive effect set. No code changes needed.
cargo dylint --lib-path "$LINT"

# JSON: machine-readable report, one file per crate+type: <prefix>.<crate>.<type>.json
CANDOR_JSON=/tmp/report cargo dylint --lib-path "$LINT"

# CONFORMANCE: enforce inferred ⊆ declared.
CANDOR_STRICT=1            cargo dylint --lib-path "$LINT"   # whole crate
CANDOR_STRICT=mymod::sub   cargo dylint --lib-path "$LINT"   # one module (incremental adoption)

# ENFORCEMENT (cap-std-aligned): flag any DIRECT reach for ambient authority.
CANDOR_NO_AMBIENT=mymod    cargo dylint --lib-path "$LINT"   # AS-EFF-004 per direct ambient call

# Flags that combine with any mode:
CANDOR_CONFIG=candor.rules cargo dylint --lib-path "$LINT"   # extra classifier rules
CANDOR_PARANOID=1          cargo dylint --lib-path "$LINT"   # treat generic trait dispatch as Unknown
```

Or register it in a project's `Cargo.toml` so plain `cargo dylint` finds it:

```toml
[workspace.metadata.dylint]
libraries = [{ path = "/abs/path/to/candor" }]
```

## The capability discipline (conformance mode)

A function declares the effects it may perform by taking the matching **capability token** as a
parameter (`&Fs`, `&Env`, …). Tokens are unforgeable — a private field means they can only be
*received*, never constructed outside their defining module — and are minted once at the entry
point. See `sample/src/main.rs` for the pattern. The checker then flags:

- **AS-EFF-001** — a function performs an effect it does not declare.
- **AS-EFF-002** — a function declares a capability it never uses.
- **AS-EFF-003** — a function makes a call candor cannot resolve (dynamic dispatch, fn-pointer, or
  callback through `impl Fn`), so its effect set is not provably complete and cannot be certified.
- **AS-EFF-004** (`CANDOR_NO_AMBIENT`) — a function reaches for *ambient authority* directly
  (`std::fs`, `std::net`, `std::env`, `std::process`, the clock, …) instead of receiving a
  capability. This is the cap-std-aligned, *enforceable* alternative to the advisory tokens: it
  fires even on functions that hold a token, because holding `&Fs` doesn't stop you calling
  `std::fs`. The fix is to route the call through an injected capability (e.g. a cap-std handle).

Adopt incrementally: scope `CANDOR_STRICT` / `CANDOR_NO_AMBIENT` to one module, fix until it reports
zero, then move to the next.

## Machine-readable for agents

The JSON report is meant to be consumed by tools and AI agents, not just read. In one test, an agent
given **only** the JSON for an 8k-line codebase (and forbidden from reading source) scoped a
cross-cutting "add retry + logging to every network call" refactor — locating all 66 direct-network
functions by `file:line`, finding 66/66 unlogged, and — using the `unresolved` flag — correctly
listing the 18 functions where source review is still required. It did this in ~22k tokens without
opening a single `.rs` file. That is the point of `Unknown`/`unresolved`: it lets a consumer be
honest about the report's own blind spots.

## Unresolved calls (honest soundness)

A call candor cannot trace to a concrete callee — `dyn Trait` dispatch, a function pointer, a
closure reached through a generic `impl Fn` parameter — could perform *any* effect. candor records
these as an **`Unknown`** effect rather than silently assuming purity. You'll see `Unknown` in audit
output and the JSON `unresolved` flag; in conformance mode it raises AS-EFF-003. (Measured cost of
*not* doing this: on a real ~8k-line codebase, 22% of functions make at least one unresolved call.)

Residual gap: statically-dispatched **generic** trait calls (`t.method()` where `t: T: Trait`) are
assumed to honour their bound rather than marked `Unknown` — otherwise every `.clone()` /
`.to_string()` / iterator adaptor would drown the report. See `CRITIQUE.md`.

## Extending the classifier

`classify()` in `src/lib.rs` is a curated table mapping crates/paths to effects. To recognise your
own effectful crates without rebuilding, point `CANDOR_CONFIG` at a rules file — one rule per line,
`<Effect> <crate|path> <prefix>`:

```
# project effect rules
Net   crate  reqwest
Fs    path   mycrate::storage::
```

Match the actual I/O boundary, not the whole crate — e.g. only `.send()` for an SDK, only
`Command`/`Child` for `std::process` — or you will over-report.

## Known limitations

- **Dynamic dispatch / fn-pointers / callbacks** can't be resolved to a concrete callee. These are
  now surfaced honestly as `Unknown` (→ AS-EFF-003) rather than silently dropped, but candor still
  can't tell you *which* effects hide behind them.
- **Generic static dispatch** (`t.method()` for `t: T: Trait`) is assumed to honour its bound — a
  deliberate residual unsoundness to keep the report readable (see `CRITIQUE.md`).
- **Advisory, not enforced**: a `&Fs` token doesn't actually gate `std::fs`; candor only reports.
  For real enforcement use [cap-std](https://github.com/bytecodealliance/cap-std).
- Logging via macros is deduped per function but counts every function that logs.

See **[CRITIQUE.md](CRITIQUE.md)** for an honest, critical assessment and comparison to prior art
(Cackle, cap-std, the Rust effects initiative).

## Status

Prototype. Validated on a real ~8k-line codebase (the `ebman` AWS Elastic Beanstalk TUI):
audit tagged 444 functions; a leaf module was converted to the capability discipline and brought to
zero conformance violations while still building on stable.
