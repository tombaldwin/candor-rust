# effect_audit

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

The build produces `target/debug/libeffect_audit@<toolchain>-<platform>.dylib` (`.so` on Linux).

## Use

From any Rust project root, with `LINT` set to that dylib's absolute path:

```sh
# AUDIT (default): every function's transitive effect set. No code changes needed.
cargo dylint --lib-path "$LINT"

# JSON: machine-readable report, one file per crate+type: <prefix>.<crate>.<type>.json
EFFECT_AUDIT_JSON=/tmp/report cargo dylint --lib-path "$LINT"

# CONFORMANCE: enforce inferred ⊆ declared.
EFFECT_AUDIT_STRICT=1            cargo dylint --lib-path "$LINT"   # whole crate
EFFECT_AUDIT_STRICT=mymod::sub   cargo dylint --lib-path "$LINT"   # one module (incremental adoption)
```

Or register it in a project's `Cargo.toml` so plain `cargo dylint` finds it:

```toml
[workspace.metadata.dylint]
libraries = [{ path = "/abs/path/to/effect-audit" }]
```

## The capability discipline (conformance mode)

A function declares the effects it may perform by taking the matching **capability token** as a
parameter (`&Fs`, `&Env`, …). Tokens are unforgeable — a private field means they can only be
*received*, never constructed outside their defining module — and are minted once at the entry
point. See `sample/src/main.rs` for the pattern. The checker then flags:

- **AS-EFF-001** — a function performs an effect it does not declare.
- **AS-EFF-002** — a function declares a capability it never uses.

Adopt incrementally: scope `EFFECT_AUDIT_STRICT` to one module, thread tokens until it reports zero
violations, then move to the next.

## Extending the classifier

`classify()` in `src/lib.rs` is a curated table mapping crates/paths to effects. To recognise your
own effectful crates (a different HTTP client, in-house I/O wrappers), add their prefixes there and
rebuild. Match the actual I/O boundary, not the whole crate — e.g. only `.send()` for an SDK, only
`Command`/`Child` for `std::process` — or you will over-report.

## Known limitations

- **Escaping closures** captured into a returned closure / stored handler aren't propagated across
  function boundaries (the effect would need to ride in the function type).
- **Trait dynamic dispatch** resolves to the trait method (empty body), losing the impl's effects.
- Logging via macros is deduped per function but counts every function that logs.

## Status

Prototype. Validated on a real ~8k-line codebase (the `ebman` AWS Elastic Beanstalk TUI):
audit tagged 444 functions; a leaf module was converted to the capability discipline and brought to
zero conformance violations while still building on stable.
