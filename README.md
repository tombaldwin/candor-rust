# candor

**A cheap, honest map of what every function in a Rust codebase actually does** — which functions
reach the network, filesystem, a database, subprocesses, the clock, the environment; *transitively*;
and where it honestly *can't* tell. A capability/effect checker built as a
[dylint](https://github.com/trailofbits/dylint) lint — the Rust reference implementation of
[candor-spec](https://github.com/tombaldwin/candor-spec) (the same idea, specified across languages).

**Built for AI coding agents.** An agent edits one function without seeing the *non-local*
consequence — a network call added deep in a helper now propagates to every caller. candor's diff
surfaces exactly that. In a pre-registered eval ([EVAL.md](EVAL.md)) an agent handed candor's delta
reported the **full propagation 100% of the time, vs 7% without it** — and scoped whole-codebase
effect questions several times cheaper than reading source. The map is only as accurate as its
classifier, so candor marks what it can't resolve `Unknown` rather than guessing, and stays honest
about the gap (coverage warnings, version-stamped reports).

**And it enforces, at a scale nobody holds in their head.** Surfacing effects helps an agent; *blocking
the PR* is what a model can't do for itself. As models get better at local call-graph tracing, candor's
edge moves to the boundary a local edit can't see — an effect, an endpoint, or a layer dependency
reached *transitively, across crates*. `cargo candor policy` makes that an architecture-as-code gate
over a whole workspace: forbidden effects, network host allowlists, and layer-dependency rules
(AS-EFF-006/008/009). A [pre-registered trial](eval/bet2/RESULTS.md) found that when the locally-simplest
edit crossed a pure boundary, candor took the shipped-violation rate from **80% to 0%**.

### Get an agent using it — one paste, from nothing

Give your coding agent (Claude Code, Cursor, …) this:

```text
Read https://github.com/tombaldwin/candor/blob/main/AGENTS.md and follow it to map this repo's effects.
```

[AGENTS.md](AGENTS.md) is self-contained — it installs candor, runs it on this project, and explains
the report and the trust rule (`inferred` is authoritative; `unresolved`/`Unknown` → read the
source). Single source of truth for agents.

### Claude Code: see it work, automatically

The paste above asks the agent to use candor — but you can't *see* whether it did. The
[Claude Code integration](integrations/claude-code/) gives you a deterministic, un-fakeable
**receipt** in your transcript whenever your Rust changes — function count, effect breakdown, a
freshness hash, and a coverage warning when a dependency looks effectful but isn't calibrated:

```text
candor · 143 fns · 54 Db, 16 Net, 27 Fs · 0 unresolved · fresh @8c4c9053 · coverage ✓
```

A `Stop` hook auto-refreshes it on every turn that touches Rust (silent otherwise); `/candor` shows
it on demand. Install: `integrations/claude-code/install.sh` from your project — it installs thin
stubs that delegate to this clone, so `cargo candor update` refreshes the engine, the scripts, and
`AGENTS.md` together (every receipt is stamped with the engine commit, so they can't silently
desync). See its [README](integrations/claude-code/README.md) for the trust model and honest limits.

**Opt-in edit-time self-review.** Set `CANDOR_REVIEW=1` (in `.candor/config`) and the Stop hook does
more than inform the human: when the agent's edits give a function a *new* effect vs your committed
baseline, it hands that delta *back to the agent* as a self-review checkpoint — "your edits gave
`foo` a new `Net` (which propagates to its callers); intended?". Each effect prompts once, it never
loops, and it's off by default. This is the difference between candor *informing* an agent and
*changing what it does* — see [BACKLOG.md](BACKLOG.md) P0.

**MCP server.** [`integrations/mcp/`](integrations/mcp/) exposes candor's instant queries
(`candor_effects` / `candor_where` / `candor_callers` / `candor_diff`) as native MCP tools, so an
agent calls candor reflexively — in one cheap call instead of grepping and reading source. Pair it
with `cargo candor watch` so every call serves from a fresh report.

**Where it earns its keep — and where it doesn't.** candor is sharpest at two things: handing an
editing agent the *non-local* consequence of a change (above), and a low-friction **CI ratchet** —
fail the PR that makes a parser open a socket ([CI guardrail](#ci-guardrail-lowest-friction-adoption)),
no token-threading or rewrite required. It is deliberately *not* a few things: not a security boundary
([SECURITY.md](SECURITY.md)); not a codebase-quality grade (effect counts are domain-dependent — there
is no "candor score" to chase); and only as sound as a curated classifier, with two known holes
(generic dispatch is assumed to honour its bound; macro-generated code is invisible) — `Unknown` and
coverage warnings mark the rest. A sharp, narrow, trustworthy instrument, not a quality platform.

*Humans:* [Quick start](#quick-start-humans) · *Detail:* [what it detects](#what-it-detects) ·
[PRINCIPLES](PRINCIPLES.md) · [CRITIQUE](CRITIQUE.md)

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | the entire lint — classifier, per-function call-graph fixpoint, the three modes |
| `crates/candor-classify` | the effect classifier (`crate × path → effect`) — pure string logic, no `rustc`; the one source of truth the lint **and** the stable scanner both call |
| `crates/candor-scan` | the **stable-Rust** backend: a `syn`-based scanner that produces the same report JSON on stock `cargo`, no nightly/dylint (see below) |
| `crates/candor-report` | the report types + parsing, shared by every backend and the CLI (no `rustc_private`) |
| `crates/candor-query` | `cargo-candor`'s read-only queries (`audit`/`show`/`where`/`callers`/`map`/`diff`) as one typed binary |
| `cargo-candor` | the CLI wrapper — thin bash that orchestrates the backend (`cargo dylint` or `candor-scan`) and dispatches queries to `candor-query` |
| `sample/` | a small crate written in the capability discipline, for trying conformance mode |
| `rust-toolchain` | pins the nightly the lint links against (`rustc-dev`) |

## Setup

```sh
cargo install cargo-dylint dylint-link   # once per machine
./install.sh                             # build + install — then `cargo candor` works in any project
```

`install.sh` is one-shot and idempotent: it builds the lint (rustup auto-fetches the pinned nightly +
`rustc-dev` from `rust-toolchain` — you never manage the toolchain by hand), stashes the dylib +
the `candor-query` binary under `~/.candor` (a stable home that survives a `cargo clean` in this
clone), and symlinks `cargo-candor` into `~/.cargo/bin` so `cargo candor …` resolves everywhere. Re-run
it (or `cargo candor setup`) any time to refresh; `cargo candor update` pulls + rebuilds + refreshes.
The pinned nightly is inherent to dylint (it links rustc internals) and runs only for the lint — it
does not touch your projects' toolchains.

### Two backends: stable scanner (zero-friction) vs the nightly lint (soundness)

candor produces the **same report JSON** two ways, and every read-only query (`show`/`where`/`callers`/`map`)
reads either one identically:

```sh
cargo candor scan      # STABLE: a syntactic scan on stock `cargo` — no nightly, no dylint, no rustc-dev
cargo candor audit     # NIGHTLY lint: the full rustc-backed analysis with the soundness contract
```

The stable scanner is the friction-killer, and it's a **one-line install** — no clone, no nightly:

```sh
cargo install candor-scan          # https://crates.io/crates/candor-scan
candor-scan .                      # writes .candor/report.<crate>.scan.json   (or --json to stdout)
```

It walks the crate's `.rs` files, parses them with [`syn`](https://docs.rs/syn), resolves `use`-aliased
call paths, and classifies them through the **same [`candor-classify`](crates/candor-classify) the lint
uses** — one source of truth, so the two backends can't drift on what counts as an effect. It needs
nothing but a stable toolchain, so it runs anywhere `cargo` does (CI without a nightly, a locked-down
box). Within a full candor install it's also `cargo candor scan`.

**Stable by default — you don't pick the backend.** The read-only queries (`show`/`where`/`callers`/`map`/
`audit`) and the [Claude Code receipt](integrations/claude-code/) prefer the nightly lint when it's
installed (for the soundness contract) and **automatically fall back to the stable scanner when it
isn't** — so candor works with zero install on any machine, and the receipt says `· stable backend` when
it's using the syntactic path. Enforcement (`guard`/`policy`/`snapshot`/`diff`) still requires the lint:
blocking a PR needs the soundness guarantee.

The trade is **precision, stated honestly**. The scanner is syntactic, so it sees what's *written*, not
what the compiler *resolves*. It catches path-qualified effect calls (`std::fs::read`, `Command::new`,
`reqwest::Client::execute`), `use`-aliases, and intra-crate transitive propagation. It **misses** —
*silently*, without emitting `Unknown` — effects reached only through a method call on a non-path-qualified
receiver, trait-object dispatch, closures/fn-pointers, macros, and cross-crate propagation by stable
identity. So on resolution-heavy code it **under-reports** relative to the lint. Use `scan` for
zero-friction triage and CI on stable; use the nightly lint when you need the soundness contract
(`Unknown` over-approximation, conformance, the policy/guard gates).

By default `scan` reports only the crate's **library/binary** source — it skips `tests/`, `benches/`,
`examples/`, `build.rs`, and `#[cfg(test)]` modules, so the report is what the *crate* does, not what its
harness does (`--include-tests` keeps them). A [calibration on 35 real crates](eval/calibration/CALIBRATION.md)
found that with this in place the scanner has **no false positives in library code** — every effect it
reports is real; its only errors are under-reports through FFI, method dispatch, and macros (the lint's
job). E.g. it correctly catches chrono reading `/etc/localtime`+`$TZ` and `which` resolving `$PATH`,
and honestly shows `Net: 0` on reqwest (whose socket I/O hides behind hyper's resolved method calls).

## Quick start (humans)

After `install.sh`, use the wrapper from any Rust project (it self-heals — rebuilding the dylib if it
ever goes missing):

```sh
cargo candor scan                       # STABLE backend: produce the report on stock cargo (no nightly)
cargo candor audit                      # at-a-glance effect profile of the whole project (nightly lint)
cargo candor audit --all                # the full per-function lint (spans in context)
cargo candor snapshot .candor/baseline  # write a JSON report
cargo candor guard    .candor/baseline  # fail on functions that gained an effect
cargo candor diff     .candor/baseline  # describe the per-function effect delta (--json)
cargo candor watch                      # keep the report fresh in the background → instant `diff`
cargo candor show     my_function       # a function's effects, instant (read from the report)
cargo candor where    Net               # which functions perform an effect, instant
cargo candor callers  my_function       # which functions call this one, instant (who depends on it)
cargo candor explain  my_function       # trace WHY a function has each effect (the call path)
cargo candor policy   .candor/policy     # enforce effect boundaries (deny/pure rules)
cargo candor risk                       # heuristic: effects on caller-derived input (advisory)
cargo candor strict   my_module         # conformance, scoped to a module
cargo candor no-ambient my_module       # flag direct ambient-authority use
```

`cargo candor audit` aggregates the project's crates into a one-screen profile — how many
functions perform each effect, which make calls candor can't resolve, any uncalibrated
dependencies, and the functions with the broadest reach into the outside world:

```text
candor @62a9383
143 effectful functions  ·  7 pgman.Executable · 136 pgman.Rlib

  effects   56 Db · 53 Clock · 47 Log · 37 Env · 27 Fs · 23 Exec · 21 Clipboard · 18 Net

  broadest effect surface
    app::App::run   { Clipboard Clock Db Env Exec Fs Log Net }
    main            { Clipboard Clock Db Env Exec Fs Log Net }
    run_batch       { Clock Db Env Exec Fs Log Net }
    …
```

`cargo candor policy` is candor's **architecture-as-code** layer — and the part that earns its keep as
models get better at local reasoning. A model advises; only a tool, holding the whole effect graph,
*blocks the PR*. It enforces the failure mode AI agents have most — editing one function without seeing
the whole effect graph — and does it at a scale nobody holds in their head: one command snapshots every
crate in the workspace, then enforces with the siblings loaded, so a boundary catches a violation whose
cause lives in *another crate*. A policy file declares invariants and candor flags any *transitive* one:

```text
# .candor/policy
deny Net Db Fs  domain                       # the domain layer must reach no I/O — even through a helper
pure            parse                        # parsing must be side-effect-free
deny Exec                                    # nothing may spawn a subprocess
allow Net in billing  api.stripe.com         # billing may reach the network — but ONLY Stripe
allow Exec in build   git                    # the build layer may run subprocesses — but ONLY git
forbid domain -> infra                       # the domain layer must not depend on infrastructure
```

```text
[AS-EFF-006] `domain::checkout` performs { Db }, forbidden by policy (scope `domain`): `deny Net Db Fs domain`
[AS-EFF-008] `billing::record_activity` reaches { metrics.growthtracker.io:443 } outside the allowlist, forbidden by policy (scope `billing`): `allow Net in billing api.stripe.com`
[AS-EFF-009] `domain::checkout` reaches into a forbidden layer (via `infra::db::save`), violating policy: `forbid domain -> infra`
```

Three boundary kinds, all checked *transitively* so they catch what a local diff hides:

- **`deny` / `pure`** (AS-EFF-006) — *what* a layer may do. `checkout` need not touch the database
  directly; candor catches it reaching `Db` through any callee.
- **`allow <Effect> in <scope> <value>…`** (AS-EFF-008) — *which values* an effect may reach: `Net`
  hosts ("billing may only talk to Stripe"), `Exec` commands ("build may only run git"), `Fs` paths
  ("config may only read /etc/app"). A supply-chain boundary a model can't self-check, because the
  literal is buried in a transitive, often **cross-crate**, callee (matched per-effect: host by name,
  command by basename, path by prefix).
- **`forbid <A> -> <B>`** (AS-EFF-009) — *who* a layer may depend on. The domain layer must not reach
  into infra, even through a chain of helpers.

`cargo candor policy` enforces all three across a whole workspace in one command — it snapshots every
crate, then enforces with the siblings loaded, so an effect or endpoint that lives in a *shared crate*
still gets caught at the boundary that forbids it. See [examples/candor-policy](examples/candor-policy)
and [eval/bet3](eval/bet3/RESULTS.md).

`cargo candor risk` is an **advisory, heuristic** nudge toward the injection class — an effect whose
argument derives from a function parameter (`fs::read(format!("/var/cache/{key}"))`, `Command::new(name)`):

```text
[AS-EFF-007] `read_user_file` performs { Fs } on caller-derived input (an injection surface — …)
```

It is *not* sound taint analysis: a syntactic, intra-procedural check that over- and under-flags
(it misses flow through struct fields and across functions, and flags a parameter that's actually
validated). Use it to find surfaces worth reviewing — never as a gate.

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

Or register it in a project's `Cargo.toml` so plain `cargo dylint` finds it — by local path,
or **straight from git with no clone** (dylint fetches and builds it against candor's pinned
toolchain). This is dylint's equivalent of a dependency; dylint loads libraries only from `git` or
`path` sources, not crates.io, so candor is **not** (and need not be) published there.

```toml
[workspace.metadata.dylint]
# clone-free — pin a tag/rev for reproducibility:
libraries = [{ git = "https://github.com/tombaldwin/candor", tag = "v0.3.0" }]
# …or a local checkout:
libraries = [{ path = "/abs/path/to/candor" }]
```

## What it detects

candor answers two questions about a codebase:

1. **What effects does each function perform?** — network (AWS SDK, `reqwest`/`ureq`/`isahc`, raw
   `std`/`tokio` sockets), databases (`sqlx`/`rusqlite`/`postgres`/…), local IPC (Unix sockets),
   filesystem, process spawn, env, clock, randomness, logging, clipboard — including effects
   inherited transitively through the functions it calls.
2. **Are the signatures honest?** — once you thread capability tokens (or use cap-std) through a
   module, it flags any function performing an effect it doesn't declare.

It resolves every call's `DefId` and classifies the crate/path it lands in. That type resolution is
the point: a bare `.send()` is meaningless syntactically, but the resolved method tells us it belongs
to `aws_sdk_*` → a network effect.

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
  surfaced honestly as `Unknown` (→ AS-EFF-003) rather than silently dropped, but candor still can't
  tell you *which* effects hide behind them. Exception: `dyn` over conventionally-pure std traits
  (`Display`, `Debug`, `Error`, `ToString`, `Clone`, …) is treated as pure, not `Unknown` —
  otherwise ubiquitous patterns like `dyn Error` formatting would flood reports with false positives.
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

- **[candor-spec](https://github.com/tombaldwin/candor-spec)** — the language-agnostic spec candor
  implements (effect vocabulary, report schema, trust contract; the Java/C#/Go ports share it).
- **[AGENTS.md](AGENTS.md)** — self-contained instructions for an AI agent (install → run → read).
- **[PRINCIPLES.md](PRINCIPLES.md)** — the ideas candor (and its development) are built on.
- **[CRITIQUE.md](CRITIQUE.md)** — an honest, critical self-assessment + comparison to prior art
  (Cackle, cap-std, the Rust effects initiative).
- **[EVAL.md](EVAL.md)** — a controlled pilot of whether the report actually helps an AI agent.
- **[BACKLOG.md](BACKLOG.md)** — what's done, what's deferred, and the concrete reason for each.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — build/test, and how to teach the classifier a new crate.
- **[SECURITY.md](SECURITY.md)** — why candor is *not* a security boundary, and how to report a
  false-negative (the bug class that matters most).

## Tests

`cargo test --workspace` runs unit tests over the *classifier* precision rules (e.g. `std::net::TcpStream`
is `Net` but `std::net::SocketAddr` is not) plus a load smoke-test, and the `candor-report` /
`candor-query` tooling tests (report parsing/discovery, the query commands). The **stateful core** (call-graph
fixpoint, CHA, conformance) isn't unit-tested — it needs the dylint harness, which has no bless
support — so it's covered instead by the `sample/`+`sample-capstd/` crates and a CI *behavioural*
check that asserts real audit output (so a "candor emits nothing" regression fails CI). The lint also
fails *gracefully* (never an ICE) on expressions outside a typechecked body.

## Status

Prototype. Validated on a real ~8k-line codebase (the `ebman` AWS Elastic Beanstalk TUI):
audit tagged ~445 functions; a leaf module was converted to the capability discipline and brought to
zero conformance violations while still building on stable.

candor also **guards itself**: CI runs candor over candor against `.candor/baseline`. Its effectful
surface — five functions in the lint (config / baseline / cross-report reads + the report write, all
`Env`/`Fs`), plus `candor-report`'s `report_files` (`Fs`) and the build script (`Exec`/`Fs`) — can't
gain a *new* effect unnoticed. Note the guard's scope, honestly: per AS-EFF-005's design it flags
*regressions in existing functions*, not brand-new functions (those are reviewed as new code), so a
newly-added effectful function wouldn't trip it. Refresh with `cargo candor snapshot .candor/baseline`
when a new effect is intended.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
