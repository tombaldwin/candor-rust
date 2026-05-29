# candor

**A cheap, honest map of what every function in a Rust codebase actually does** — which functions
reach the network, the filesystem, a database, the clock, the environment; and, crucially, where it
*can't* tell. A type-aware capability/effect checker built as a
[dylint](https://github.com/trailofbits/dylint) lint.

## Why it's built for AI coding agents

An AI agent has no memory between sessions and pays per token to read code, so it re-derives "what
does this do?" from scratch every time it touches a codebase — slowly, and sometimes wrongly. candor
precompiles that into a structured report it can read in seconds instead of tracing call chains
across thousands of lines. In a controlled pilot ([EVAL.md](EVAL.md)) a JSON-only agent scoped a
cross-cutting refactor **~3× cheaper and ~6.5× faster** than one reading source. Two properties make
it safe for an agent to *rely* on:

- **It's transitive.** A handler that does no I/O itself still reports `{ Net, Fs }` if something
  five calls deep does — so the agent sees an edit's real blast radius without following the chain.
- **It's honest about its blind spots.** A call candor can't resolve is marked `Unknown`, never
  silently assumed pure — so the agent knows exactly where the report is authoritative and where it
  must read source. The same report is also a deterministic guard on agent-*written* code: it catches
  a change that makes a previously-pure function start hitting the network.

Concretely, it answers two questions about a codebase:

1. **What effects does each function actually perform?** — network (AWS SDK, `reqwest`/`ureq`/`isahc`
   HTTP, raw `std`/`tokio` sockets), databases (`sqlx`/`rusqlite`/`postgres`/…), local IPC
   (Unix-domain sockets), filesystem, process spawn, env reads, clock reads, randomness, logging,
   clipboard — including effects inherited transitively through the functions it calls.
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

## Quick start (humans)

Put this repo on `PATH` (or symlink `cargo-candor` into one) and use the wrapper, which finds and
builds the dylib for you:

```sh
cargo candor audit                      # report each function's effect set
cargo candor snapshot .candor/baseline  # write a JSON report
cargo candor guard    .candor/baseline  # fail on functions that gained an effect
cargo candor strict   my_module         # conformance, scoped to a module
cargo candor no-ambient my_module       # flag direct ambient-authority use
```

## Quick start (AI agents)

Generate a machine-readable report, then *query it* instead of reading source. One JSON file is
written per crate, named `<prefix>.<crate>.<type>.json`:

```sh
cargo candor snapshot /tmp/candor
# or explicitly:  CANDOR_JSON=/tmp/candor cargo dylint --lib-path "$LINT"
```

Each entry:

```json
{ "fn": "app::App::handle_key", "loc": "src/app.rs:2987:5",
  "inferred": ["Fs", "Net", "Unknown"],   // full TRANSITIVE effect set
  "direct":   ["Log"],                      // effects in this function's own body
  "declared": [], "undeclared": [], "overdeclared": [],
  "unresolved": true }                      // true => some calls could not be resolved
```

How to use it — the agent contract:

- **Blast radius of editing a function** → read its `inferred`.
- **Which functions touch the network?** → `inferred` contains `"Net"`:
  `jq '.[] | select(.inferred | index("Net")) | .fn' /tmp/candor.*.json`
- **Safe to treat as pure (e.g. test without mocks)?** → `inferred == []` *and* `unresolved == false`.
- **Trust boundary (read this):** `inferred` is authoritative for what candor resolved. When
  `unresolved` is `true` (or `"Unknown"` is in the set), the list may be incomplete — read the source
  for *those* functions. candor never claims certainty it doesn't have; lean on that honesty.

## All modes (explicit invocation)

From any Rust project root, with `LINT` set to the dylib's absolute path:

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

# REGRESSION GUARD: fail if any function gained an effect since a saved snapshot.
CANDOR_JSON=.candor/baseline cargo dylint --lib-path "$LINT"      # 1. snapshot (commit it)
CANDOR_BASELINE=.candor/baseline cargo dylint --lib-path "$LINT"  # 2. in CI: AS-EFF-005 on regressions

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
- **AS-EFF-005** (`CANDOR_BASELINE`) — an existing function *gained* an effect it didn't have in a
  saved snapshot. The lowest-friction adoption path: no token threading, no rewrite — just catch the
  PR that makes a previously-pure function start doing network/disk/etc. I/O. (New functions are not
  flagged; they're reviewed as new code.)

Adopt incrementally: scope `CANDOR_STRICT` / `CANDOR_NO_AMBIENT` to one module, fix until it reports
zero, then move to the next.

### Or use real capabilities: cap-std

candor recognises [cap-std](https://github.com/bytecodealliance/cap-std) capability *types* as
declarations and its operations as the matching effect. A function that takes a `&Dir` and reads
through it (`dir.read_to_string(..)`) is conformant — its declared `Fs` matches its inferred `Fs` —
while a sibling that reaches for ambient `std::fs` is flagged. Unlike candor's own advisory tokens,
cap-std capabilities are unforgeable and compile-enforced; candor just makes the effect surface
*visible* on top. See `sample-capstd/`. Mapped today: `Dir`→Fs, `Pool`/`TcpStream`→Net,
`SystemClock`→Clock, `UnixStream`→Ipc.

## CI guardrail (lowest-friction adoption)

You don't have to adopt the capability discipline to get value. The cheapest win is the regression
guard: snapshot the effect report, commit it, and fail CI when a function's effect surface grows.

```sh
# once, on a known-good commit — then `git add .candor/`
CANDOR_JSON=.candor/baseline cargo dylint --lib-path "$LINT"

# in CI: fail only on AS-EFF-005 (a function gained an effect) — see examples/candor-guard.yml
out=$(CANDOR_BASELINE=.candor/baseline cargo dylint --lib-path "$LINT" 2>&1); echo "$out"
echo "$out" | grep -q AS-EFF-005 && { echo "effect surface grew"; exit 1; } || true
```

Now a PR that makes a parser suddenly open a socket, or a render function start reading the
filesystem, fails review automatically — no tokens, no rewrite. Refresh the baseline deliberately
(re-run the snapshot command) when a new effect is intended. This is equally useful to a human
reviewer and to an AI agent reviewing a diff.

## How well does it actually help an agent? (the honest version)

A controlled pilot ([EVAL.md](EVAL.md)) pitted a JSON-only agent against a source-only one on the
same scoping task. The JSON was ~3× cheaper and ~6.5× faster — *and* it surfaced a real lesson: the
source-only agent was more **accurate** in one spot, because candor had silently misclassified some
`reqwest` HTTP calls (a classifier gap, since fixed). So: the report is cheap and genuinely useful,
but **only as correct as its classifier** — which is exactly why `Unknown`/`unresolved` exists, and
why an agent should treat flagged-uncertain functions as "go read the source," not "trust me."

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
- **Macro-generated functions are invisible.** Items whose span is from a macro expansion are
  skipped (to drop noise like tracing's `__CALLSITE` statics) — so a function *generated* by a macro
  (`async_trait`, derive, a declarative macro) is not analyzed in any mode. Code you wrote by hand is
  unaffected.
- **Capabilities must be direct parameters.** `declared_caps` recognizes a capability (`&Fs`, a
  cap-std `&Dir`) only as a top-level parameter. A capability reached *through* a struct field
  (`fn f(ctx: &AppContext)` where `ctx` holds the `Dir`) is not counted as declared — that function
  would be flagged in strict mode despite holding the capability.
- **Generic static dispatch over non-local traits** is assumed to honour its bound (CHA only sees
  through *local* traits); `CANDOR_PARANOID` flags the rest at the cost of noise.
- Logging via macros is deduped per function but counts every function that logs.

## Documentation

- **[PRINCIPLES.md](PRINCIPLES.md)** — the ideas candor (and its development) are built on.
- **[CRITIQUE.md](CRITIQUE.md)** — an honest, critical self-assessment + comparison to prior art
  (Cackle, cap-std, the Rust effects initiative).
- **[EVAL.md](EVAL.md)** — a controlled pilot of whether the report actually helps an AI agent.
- **[BACKLOG.md](BACKLOG.md)** — what's done, what's deferred, and the concrete reason for each.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — build/test, and how to teach the classifier a new crate.
- **[SECURITY.md](SECURITY.md)** — why candor is *not* a security boundary, and how to report a
  false-negative (the bug class that matters most).

## Tests

`cargo test` runs unit tests over the *classifier* precision rules (e.g. `std::net::TcpStream` is
`Net` but `std::net::SocketAddr` is not) plus a load smoke-test. The **stateful core** (call-graph
fixpoint, CHA, conformance) isn't unit-tested — it needs the dylint harness, which has no bless
support — so it's covered instead by the `sample/`+`sample-capstd/` crates and a CI *behavioural*
check that asserts real audit output (so a "candor emits nothing" regression fails CI). The lint also
fails *gracefully* (never an ICE) on expressions outside a typechecked body.

## Status

Prototype. Validated on a real ~8k-line codebase (the `ebman` AWS Elastic Beanstalk TUI):
audit tagged ~445 functions; a leaf module was converted to the capability discipline and brought to
zero conformance violations while still building on stable.

candor also **guards itself**: CI runs candor over candor against `.candor/baseline`, so its three
existing effectful functions (config/baseline reads + the report write, all `Env`/`Fs`) can't gain a
*new* effect unnoticed. Note the guard's scope, honestly: per AS-EFF-005's design it flags
*regressions in existing functions*, not brand-new functions (those are reviewed as new code), so a
newly-added effectful function wouldn't trip it. Refresh with `cargo candor snapshot .candor/baseline`
when a new effect is intended.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
