# candor

<p align="center"><img src="https://raw.githubusercontent.com/tombaldwin/candor/main/assets/beaky.svg" alt="Beaky, the candor canary" width="180"></p>

**Enforce the capability and architectural boundaries that AI-generated code silently crosses — as a
CI gate you can trust.** candor is a Rust capability/effect checker implementing
[candor-spec](https://github.com/tombaldwin/candor-spec) (the reference engine is
[candor-java](https://github.com/tombaldwin/candor-java)). **The user route is `candor-scan`** —
`cargo install candor-scan`, stable Rust, no nightly. This repository also holds a rustc
[dylint](https://github.com/trailofbits/dylint) lint: that is the family's **oracle instrument** (what the
soundness fuzzers, the censuses and the `strace` oracle measure the scanner against), not a second route
for users. candor knows which functions reach the network,
filesystem, a database, a subprocess, the clock, or the environment — *transitively, across crates* —
and turns invariants like *"this layer stays pure," "this service may only talk to Stripe," "the
domain layer must not depend on infra"* into rules that **fail the PR** when an edit breaks them.

**Site:** [candor.poly.io](https://candor.poly.io) — the measured case in five minutes: the
exhibits, the pre-registered evals, and the prove-it-on-your-own-repo path.

**Why this matters for AI-assisted development.** An agent's characteristic failure isn't a typo —
it's a locally-reasonable edit that crosses a boundary it never sees. It adds a feature in
`pricing.rs`, and the simplest way to get the data calls something that, three hops and one crate
away, opens a socket or hits the database. The file looks clean; a quick review looks clean. candor's
transitive, cross-crate analysis catches it and the gate blocks it — the one failure mode that gets
*worse*, not better, as agents write more code faster. In a
[pre-registered trial](eval/bet2/RESULTS.md), when the locally-simplest edit crossed a pure boundary,
candor took the shipped-violation rate from **80% to 0%**.

**A gate is only worth trusting if it says what it cannot see.** `candor-scan` is syntactic: it sees what is
written, not what the compiler resolves. It is calibrated never to fabricate an effect, and it marks `Unknown`
at the boundaries it can see but not see through (an invoked fn-value or callback, an FFI `extern` call, a
trait with no visible implementor, an untrusted chained report). What it can still **miss, silently**, is an
effect reached through external-trait dispatch, an uninferrable receiver, a desugared operator, `?` or
`.await`, or cross-crate identity when the dependency is not chained (`--deps`). So a clean scan is
necessary, never sufficient. What measures that gap is the oracle: the rustc lint in this repo, whose
"never silently pure" contract is held by an adversarial **soundness fuzzer in CI** that threads a known
effect through every way Rust can hide a call (operator overloads, `?`, `.await`, dynamic dispatch, closures
and callbacks, macros, cross-crate boundaries), plus `strace` ground truth. `candor-scan --policy` is the gate
itself: forbidden effects, network host allowlists and layer-dependency rules (AS-EFF-006/008/009), enforced
across a whole workspace as a **syntactic floor**.

**It maps, too.** The same analysis answers "what does this function transitively touch?" and "who
reaches `Net`?" instantly from a cached report — a cheap blast-radius tool for an agent or a human in
unfamiliar code. Handing an editing agent the *non-local* delta of its own change is real value too (a
[pilot](EVAL.md) had the agent report the full propagation **100% of the time vs 7% without it**) — but
as models get better at local call-graph tracing, candor's durable edge is the part a model *can't* do
for itself: hold the whole effect graph and **block the PR**.

### Get an agent using it — one paste, from nothing

Give your coding agent (Claude Code, Cursor, …) this:

```text
Read https://github.com/tombaldwin/candor-rust/blob/main/AGENTS.md and follow it to map this repo's effects.
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
desync). See its [README](integrations/claude-code/README.md) for the trust model and stated limits.

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

**Where it earns its keep — and where it doesn't.** candor is sharpest as a **CI gate** enforcing
capability and architectural boundaries transitively — fail the PR that makes a parser open a socket,
or routes the domain layer into infra ([CI guardrail](#ci-guardrail-lowest-friction-adoption)), no
token-threading or rewrite required — and as the *non-local* delta handed to an editing agent (above).
Its value is **conditional**, and stated plainly: it shows up when a codebase *has* boundaries worth
defending and an edit *would* cross one. If the code already affords a clean seam, a strong model
routes around the problem and candor is redundant — the same eval showed exactly that. It is
deliberately *not* a few things: not a security boundary ([SECURITY.md](SECURITY.md)); not a
codebase-quality grade (effect counts are domain-dependent — there is no "candor score" to chase);
this repo is Rust-only (the JVM — Java/Kotlin/Scala/Groovy — has
[candor-java](https://github.com/tombaldwin/candor-java), same spec, same report shape, same gate);
and the [scanner](#the-scanner-the-user-route-and-the-oracle-lint) is syntactic, so it can miss — silently — an
effect it cannot resolve. What it does disclose is marked by `Unknown` and coverage warnings; what it can
miss is listed under [Known limitations](#known-limitations). A sharp, narrow instrument that states its
limits, not a quality platform.

*Humans:* [Quick start](#quick-start-humans) · *Detail:* [what it detects](#what-it-detects) ·
[PRINCIPLES](PRINCIPLES.md) · [CRITIQUE](CRITIQUE.md)

## Layout

| Path | What |
|---|---|
| `src/lib.rs` | the rustc dylint lint — the family's **oracle instrument** (call-graph fixpoint over typechecked bodies); not a user route |
| `crates/candor-classify` | the effect classifier (`crate × path → effect`) — pure string logic, no `rustc`; the one source of truth the lint **and** the stable scanner both call |
| `crates/candor-scan` | **the user route**: a stable-Rust, `syn`-based scanner that produces the report JSON on stock `cargo`, no nightly/dylint (see below) |
| `crates/candor-report` | the report types + parsing, shared by every backend and the CLI (no `rustc_private`) |
| `crates/candor-query` | `cargo-candor`'s read-only queries (`audit`/`show`/`where`/`callers`/`map`/`diff`/`whatif`/`rewire`/`containment`/`reachable`/`path`/`impact`) as one typed binary — plus ⟨0.24⟩ `gate --report <locator> --policy <file>`, which applies a policy to an EXISTING report with no scan (the supply-chain verb; SPEC §3.1) |
| `cargo-candor` | the CLI wrapper — thin bash that runs `candor-scan` and dispatches queries to `candor-query`; it reaches the lint only under the `CANDOR_BACKEND=lint` oracle opt-in |
| `sample/` | a small crate written in the capability discipline, for trying conformance mode |
| `rust-toolchain` | pins the nightly the oracle lint links against (`rustc-dev`) |

## Setup

```sh
cargo install candor-scan          # the user route — https://crates.io/crates/candor-scan (stable Rust, no nightly)
```

That is the whole install. If you also want the `cargo candor` wrapper (queries over the report, `watch`,
`whatif`, `fix`) from a clone:

```sh
./install.sh                       # builds candor-scan + candor-query on stable, stashes them under ~/.candor,
                                   # and symlinks `cargo-candor` into ~/.cargo/bin so `cargo candor …` resolves
```

`install.sh` is one-shot and idempotent, and it needs **no nightly**: the stable tools survive a `cargo clean`
in this clone because they are stashed under `~/.candor`. Re-run it (or `cargo candor setup`) any time to
refresh; `cargo candor update` pulls and rebuilds. The rustc lint is built only on request
(`./install.sh --oracle`) and is for an operator running the oracle (below), never a prerequisite.

### The scanner (the user route) and the oracle lint

This repository holds two producers of the **same report JSON**:

- **`candor-scan`** — the user route. A syntactic scan on stock `cargo`: no nightly, no dylint, no rustc-dev.
  Every verb of the `cargo candor` wrapper runs on it by default.
- **the rustc dylint lint** (`src/lib.rs`, "rust-deep") — the family's **oracle instrument**. It is what the
  soundness fuzzers, the censuses, the `strace` oracle and candor's own Self-guard run, and what the scanner is
  measured against. It is **not a user route**: its report is not spec-current (SOUNDNESS R456 — it stamps
  `spec: 0.43` while writing a pre-⟨0.21⟩ envelope with no `analyzed` manifest, so `candor-query gate` answers
  its output with *"re-scan with a current engine"*), and it is calibrated and shipped as an instrument, not a
  product. An operator who runs it on purpose sets `CANDOR_BACKEND=lint`; nothing else reaches it through the wrapper.

The scanner is the friction-killer, and it's a **one-line install** — no clone, no nightly:

```sh
cargo install candor-scan          # https://crates.io/crates/candor-scan
candor-scan .                      # writes .candor/report.<crate>.scan.json   (or --json to stdout)
                                   # (a workspace root: one report per member, same prefix)
```

**Staying current:** check your installed version and upgrade — [candor/AGENTS.md §2a](https://github.com/tombaldwin/candor/blob/main/AGENTS.md#2a-staying-current--check-the-version-upgrade). `candor-scan --version` prints the build, the spec, and the upgrade one-liner (offline; candor never phones home).

It walks the crate's `.rs` files, parses them with [`syn`](https://docs.rs/syn), resolves `use`-aliased
call paths, and classifies them through the **same [`candor-classify`](crates/candor-classify) the lint
uses** — one source of truth, so the scanner and the oracle can't drift on what counts as an effect. It needs
nothing but a stable toolchain, so it runs anywhere `cargo` does (CI without a nightly, a locked-down
box). Within a clone install it's also `cargo candor scan`.

**One route — you don't pick a backend.** The read-only queries (`show`/`where`/`callers`/`map`/`audit`/`path`/
`impact`/`whatif`/`fix`/`diff`/`watch`) and the [Claude Code receipt](integrations/claude-code/) all run on
`candor-scan`, and the receipt says `· stable backend` so a reader knows which route produced it. The verbs
that exist only on the oracle lint — `strict`, `no-ambient`, `snapshot`, `guard`, `policy`, `risk`, `explain` and
`audit --all` — **refuse** (exit 2, naming the scanner equivalent) unless an operator sets
`CANDOR_BACKEND=lint`. A refused gate is fail-closed, as everywhere in the family: with `--gate-json` the sink
holds the §3.1 refusal document, never a previous run's verdict. The scanner equivalents are `candor-scan . --policy
<file>` for the policy gate and `candor-scan . --out <prefix>` then `CANDOR_BASELINE=<prefix> candor-scan .`
for the regression guard (both below).

The trade is **precision, disclosed**. The scanner is syntactic, so it sees what's *written*, not
what the compiler *resolves*. It catches path-qualified effect calls (`std::fs::read`, `Command::new`,
`reqwest::Client::execute`), `use`-aliases, intra-crate transitive propagation, macro bodies, and
**local-trait dispatch** (a `&dyn Store`/`impl Store`/`S: Store` receiver resolves to the trait's local
implementors — syntactic CHA, bounded like the JVM engine's — or reads `Unknown` when the trait has no
visible impl or too many). It **discloses `Unknown`** where it can see the boundary it can't see
through: an invoked fn-value/callback, an FFI `extern` call, an untrusted chained report. For
dependencies, the receipt **names what the classifier can't see** (the coverage ledger:
`candor's classifier doesn't cover N dependencies this code calls into…`) and `--deps` **closes it**:
scan the whole `Cargo.lock` tree once (unbuilt registry sources, ~0.23s/dep measured) and the root
scan chains over the reports — effects cross every crate boundary without the classifier knowing the
crates (spec §2 report chaining). It **misses** — and nothing in the report says so — effects reached only through
external-trait dispatch or an uninferrable receiver, desugared operators/`?`/`.await`, and cross-crate
propagation by stable identity (unless chained). So on resolution-heavy code it can **under-report**, and a
clean scan or a green `--policy` run is necessary, never sufficient. Use `scan` for triage and for CI on
stable, and treat a miss as possible wherever the code leans on those forms; the oracle (above) is how the
family measures how often that happens.

By default `scan` reports only the crate's **library/binary** source — it skips `tests/`, `benches/`,
`examples/`, `build.rs`, and `#[cfg(test)]` modules, so the report is what the *crate* does, not what its
harness does (`--include-tests` keeps them). A [calibration on 35 real crates](eval/calibration/CALIBRATION.md)
found that with this in place the scanner has **no false positives in library code** — every effect it
reports is real; its only errors are under-reports through FFI, method dispatch, and macros. E.g. it correctly catches chrono reading `/etc/localtime`+`$TZ` and `which` resolving `$PATH`,
and correctly shows `Net: 0` on reqwest (whose socket I/O hides behind hyper's resolved method calls).

## Quick start (humans)

After `install.sh`, use the wrapper from any Rust project. Every verb below runs on `candor-scan` unless it is
marked **oracle only** — those exist only on the rustc lint, and refuse (exit 2, naming the scanner equivalent)
unless an operator sets `CANDOR_BACKEND=lint`:

```sh
cargo candor scan                       # produce the report on stock cargo (no nightly)
cargo candor audit                      # at-a-glance effect profile of the whole project
cargo candor diff     .candor/baseline  # describe the per-function effect delta vs a candor-scan baseline (--json)
cargo candor watch                      # keep the report fresh in the background → instant queries
cargo candor show     my_function       # a function's effects, instant (read from the report)
cargo candor where    Net               # which functions perform an effect, instant
cargo candor callers  my_function       # which functions call this one, instant (who depends on it)
cargo candor containment [baseline]     # effect-leakage diagnostic; with a baseline, a CI ratchet
cargo candor reachable                  # what the program does at runtime (union over entry points)
cargo candor path     my_fn Net         # the call chain by which a fn comes to perform an effect
cargo candor impact   my_fn             # blast radius: transitive callers + downstream entry points
cargo candor whatif   my_fn Net         # PRE-EDIT: what would propagate, and would it break policy?

# the gates are candor-scan's own flags (the user route):
candor-scan . --policy .candor/policy                      # enforce effect boundaries (deny/pure/allow/forbid rules)
candor-scan . --out .candor/baseline                       # record a baseline (commit it) …
CANDOR_BASELINE=.candor/baseline candor-scan .             # … and fail on a function that gained an effect

# oracle only (CANDOR_BACKEND=lint):
cargo candor audit --all                # the lint's per-function diagnostics (spans in context)
cargo candor snapshot .candor/baseline  # the lint's baseline writer     (user route: candor-scan . --out)
cargo candor guard    .candor/baseline  # the lint's AS-EFF-005 guard    (user route: CANDOR_BASELINE=… candor-scan .)
cargo candor policy   .candor/policy    # the lint's policy gate         (user route: candor-scan . --policy)
cargo candor explain  my_function       # trace WHY a function has each effect (user route: `cargo candor path`)
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
allow Db  in billing  ledger.*               # billing may touch the database — but ONLY the ledger schema
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

- **`accept <fn> <Effect>… for <rule> because <reason>`** (SPEC §6.2 ⟨0.41⟩) — a *known* violation, written down
  where a reviewer reads it. The `for … because` span must be the rule line's own tokens, and the entry names ONE
  function by exact name (`candor-scan` and `candor-query gate --report` evaluate it; the oracle lint route does not
  and leaves the violation red). The row leaves `violations`, appears in the verdict's `accepted`
  list, and the human channel prints `N violation(s) accepted by policy; M new`. A function that now does *more*
  than was accepted stays red; an entry that matches nothing is itself a violation (`AS-EFF-012`, delete the line);
  a closure's `{closure#N}` key cannot be accepted — accept the enclosing function. `allow` and `only` rules
  cannot be accepted.
  `candor-query accepts` prints a *proposed* policy that refreshes these lines after an upgrade — stale entries
  deleted, over-wide ones narrowed to what they accepted, an exceeded one proposed in a comment (never widened) — and
  with `--adopt --because <reason>` appends a line, carrying your reason, per violation no entry names. It writes no
  file: `candor-query accepts --policy p > p.new && diff p p.new` (never `> p`, which empties it first).

`cargo candor policy` enforces all three across a whole workspace in one command — it snapshots every
crate, then enforces with the siblings loaded, so an effect or endpoint that lives in a *shared crate*
still gets caught at the boundary that forbids it. See [examples/candor-policy](examples/candor-policy)
and [eval/bet3](eval/bet3/RESULTS.md).

**Machine-readable verdict — `--gate-json` (candor-spec §3.3).** The gate's verdict as
JSON, from the *same* check that sets the exit code, for CI annotations / the PR-native SARIF reporter:

```sh
candor-scan . --policy .candor/policy --gate-json verdict.json   # the user route
CANDOR_BACKEND=lint cargo candor policy .candor/policy --gate-json verdict.json   # the oracle lint — same shape (also: guard)
# → { "spec": "0.43", "ok": false, "violations": [ { "rule": "AS-EFF-006", "fn": "…", "effects": ["Db"], "detail": "…" } ] }
```

`-` streams it to stdout. Exit semantics are pinned: violation → 1; a gate that could not run to
completion (build failure, unreadable policy, unwritable verdict) → 2 with **no** verdict file — never
a stale or clean-looking lie.

**Exit codes, everywhere a gate runs:** `0` = evaluated, clean · `1` = evaluated, violations ·
`2` = **not evaluated** (unreadable/empty policy, build failure, absent/stale baseline, unusable
config). A `2` is fail-closed by design — the gate refuses to pass green when it could not actually
check anything.

### AS-EFF-005 on the stable scanner — the zero-install regression guard

The [regression guard](#ci-guardrail-lowest-friction-adoption) runs on `candor-scan` with no nightly:
the AS-EFF-005 ratchet. Record a baseline, commit it,
and a PR that makes an *existing* function gain an effect fails:

```sh
candor-scan . --out .candor/baseline              # 1. record it (commit the .candor/baseline.* files)
CANDOR_BASELINE=.candor/baseline candor-scan .    # 2. in CI: exit 1 if a function gained an effect
```

Or check it in once — the `.candor/config` `baseline` key (below) activates the guard on every scan.
Semantics follow the reference engine (candor-java) exactly: a function that **gained** an effect vs
its baseline set → one `[AS-EFF-005]` line per function + exit 1, and the violations join the
`--gate-json` verdict; ⟨0.40⟩ a function **absent from the baseline** is compared against nothing
(SPEC §3, SOUNDNESS R932) — if it performs a real effect it is an `[AS-EFF-005]` violation too, the line
says it is ABSENT FROM THE BASELINE, and every verdict row carries `origin` (`existing`/`new`/`unknown`;
"new" means absent under this key, so a rename reads as absent); a new pure function passes and a new
`Unknown`-only one is named in an advisory note. Under a `--out` prefix, a crate with no file under a
prefix that has files for other crates is a crate absent from the baseline (R933). Before ⟨0.40⟩ new
functions were exempt ("reviewed as new code"), and code review does not read effects. **No baseline
file** (and no file under the prefix for any crate) → a stderr note, guard inactive, exit unchanged; a baseline that is **unparseable or produced by a different scanner build** (the envelope
`candor.version`) → exit 2 *without evaluating* — a stale baseline is invalid gate input (spec §2.1):
never a silent skip, never a stale compare. The same advisory-floor caveat as the scan policy gate
applies: the syntactic backend can miss an effect it cannot resolve, and says nothing when it does, so a
clean ratchet is necessary, never sufficient.

### `.candor/config` — check in the configuration

One checked-in file replaces the `CANDOR_*` env wiring (candor-spec §3.4), so CI is "point at the
repo" and the configuration travels with the code. Both backends discover it by walking **up from the
target** (`$CANDOR_CONFIG` overrides discovery; precedence: CLI arg → env var → config → default):

```text
# .candor/config
policy   .candor/policy      # the §6.2 policy file  (cargo candor policy / candor-scan --policy)
baseline .candor/baseline    # the AS-EFF-005 guard prefix (cargo candor guard/diff/snapshot AND candor-scan)
deps     .candor/deps        # CANDOR_DEPS report chaining (candor-scan)
```

Relative paths resolve against the config's **home directory** — the one containing `.candor/` —
never your shell's CWD. Fail-closed: an unusable config, or a bare `policy` line with no value, exits
2 (a silently dropped config could be a silently dropped gate); unknown keys warn (typo protection);
keys an engine recognizes but doesn't implement are disclosed, not silently inert.

`cargo candor risk` is an **advisory, heuristic** nudge toward the injection class — an effect whose
argument derives from a function parameter (`fs::read(format!("/var/cache/{key}"))`, `Command::new(name)`):

```text
[AS-EFF-007] `read_user_file` performs { Fs } on caller-derived input (an injection surface — …)
```

It is *not* sound taint analysis: a syntactic, intra-procedural check that over- and under-flags
(it misses flow through struct fields and across functions, and flags a parameter that's actually
validated). Use it to find surfaces worth reviewing — never as a gate.

### `cargo candor containment` — an architecture signal that isn't a "score"

Raw effect *counts* are domain-dependent (a database app has lots of `Db` — not a defect), so there is
no single "candor score". But the **dispersion** of a boundary effect across layers *is*
domain-independent: `Db` all in one data layer is well-architected; `Db` smeared across `model`,
`actions`, *and* `dao` is leaky — regardless of how much DB it does. `containment` measures that, per
boundary effect (`Db`/`Net`/`Exec`/`Fs`/`Ipc`); `Log`/`Clock` are ambient (reported, not scored). Layers
are inferred from the module after the common crate root, no config:

```text
  effect  contained  layers   owner  ← leaked into
  Db            55%       3   conn (11)  ← query:7, app:2      # pgman: DB is mostly in conn/query…
```

(Run on `pgman` — whose DB is *meant* to live in `conn` + `query` — candor independently found that
boundary **and** its one documented exception, the `app:2` leak.) Given a baseline prefix it's a
**ratchet** — gate on getting worse, note getting better:

```text
[containment] a boundary effect leaked into a layer it wasn't in:   ← exit 1, fail the PR
  Db → actions
✓ improved — a boundary effect left a layer:                        ← informational
  Db ⊘ legacy
```

`cargo candor containment` for the diagnostic, `cargo candor containment .candor/baseline` for the gate.
Deliberately a **diagnostic + trend gate, not a single grade** — the absolute level is domain-dependent
and gameable, but "did a boundary effect leak into a new layer?" is a real, enforceable quality signal.

## Oracle instrument: invoking the lint directly (operators only)

**This is not a user route.** It is how the soundness lanes, the censuses and Self-guard drive the rustc lint, kept
here as the operator's reference. Its report is not spec-current (SOUNDNESS R456), its policy route does not
evaluate `accept` lines, and nothing here should be read as a recommendation to install a nightly toolchain to
gate a project: use `candor-scan` (above). Build it with `./install.sh --oracle` (or `cargo build` in the clone).

From a Rust project root, with `LINT` set to the dylib's absolute path:

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
CANDOR_RULES=candor.rules  cargo dylint --lib-path "$LINT"   # extra classifier rules
CANDOR_PARANOID=1          cargo dylint --lib-path "$LINT"   # treat generic trait dispatch as Unknown
```

`CANDOR_PARANOID` changes what the lint reports, so it is a *result-moving switch* (SPEC §3.4): the report names it in
`switches`, and `candor-query gate --report` (and `unverified`, `fix-gate`, `whatif`) refuse over a report that carries
that key (exit 2). Read the PARANOID report with `candor-query` verbs that answer no `ok` (`where`, `show`, `fix`), or
re-run without the variable to gate.

Registering the lint in a project's `[workspace.metadata.dylint]` (clone-free, from git) is deliberately not
documented here: it would make the oracle a user route, which is what this section says it is not.

## What it detects

candor answers two questions about a codebase:

1. **What effects does each function perform?** — network (AWS SDK, `reqwest`/`ureq`/`isahc`, raw
   `std`/`tokio` sockets), databases (`sqlx`/`rusqlite`/`postgres`/…), local IPC (Unix sockets),
   filesystem, process spawn, env, clock, randomness, logging, clipboard — including effects
   inherited transitively through the functions it calls.
2. **Are the signatures honest?** — once you thread capability tokens (or use cap-std) through a
   module, it flags any function performing an effect it doesn't declare.

The scanner reads call paths syntactically — `use`-aliases, local call graph, local-trait dispatch — and
classifies the crate/path each lands in. The oracle lint instead resolves every call's `DefId`, and that
type resolution is why it is the oracle: a bare `.send()` is meaningless syntactically, but the resolved
method tells it the call belongs to `aws_sdk_*` → a network effect. The difference between the two is
exactly what the scanner can miss.

## The capability discipline (conformance mode — oracle only)

Conformance mode (`strict`, `no-ambient`; AS-EFF-001..004) exists only on the oracle lint
(`CANDOR_BACKEND=lint`); `candor-scan` has no equivalent. AS-EFF-005 and the policy rules run on both.

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
  PR that makes a previously-pure function start doing network/disk/etc. I/O. (⟨0.40⟩ a function ABSENT from the
  baseline that performs a real effect is flagged too, on the scanner route; the oracle lint's own guard is the
  older shape.)

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
candor-scan . --out .candor/baseline

# in CI: exit 1 if a function gained an effect (AS-EFF-005) — see examples/candor-guard.yml
CANDOR_BASELINE=.candor/baseline candor-scan .
```

Now a PR that makes a parser suddenly open a socket, or a render function start reading the
filesystem, fails review automatically — no tokens, no rewrite. Refresh the baseline deliberately
(re-run the snapshot command) when a new effect is intended. This is equally useful to a human
reviewer and to an AI agent reviewing a diff.

**The guard fails closed (exit 2) on a baseline it cannot trust** — it never silently compares one: a baseline
**unparseable or recorded by a different scanner build** (stale after an upgrade: re-record it with
`candor-scan . --out .candor/baseline`, review `git diff .candor/`, commit). With **no baseline at all** the guard is
inactive and says so on stderr (exit unchanged), so a typo'd prefix is a note, not a gate: keep the baseline checked
in, and put `baseline` in `.candor/config` so a missing file is visible. Exit 2 means "the gate could not evaluate",
distinct from exit 1 ("a function gained an effect"); see [AS-EFF-005 on the stable scanner](#as-eff-005-on-the-stable-scanner--the-zero-install-regression-guard).

## How well does it actually help an agent? (the measured version)

A controlled pilot ([EVAL.md](EVAL.md)) pitted a JSON-only agent against a source-only one on the
same scoping task. The JSON was ~3× cheaper and ~6.5× faster — *and* it surfaced a real lesson: the
source-only agent was more **accurate** in one spot, because candor had silently misclassified some
`reqwest` HTTP calls (a classifier gap, since fixed). So: the report is cheap and genuinely useful,
but **only as correct as its classifier** — which is exactly why `Unknown`/`unresolved` exists, and
why an agent should treat flagged-uncertain functions as "go read the source," not "trust me."

## Unresolved calls (soundness by disclosure)

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
own effectful crates without rebuilding, point `CANDOR_RULES` at a rules file — one rule per line,
`<Effect> <crate|path> <prefix>` (previously named `CANDOR_CONFIG` — that variable now means the
[`.candor/config`](#candorconfig--check-in-the-configuration) override path, per candor-spec §3.4):

```
# project effect rules
Net   crate  reqwest
Fs    path   mycrate::storage::
```

Match the actual I/O boundary, not the whole crate — e.g. only `.send()` for an SDK, only
`Command`/`Child` for `std::process` — or you will over-report.

## Known limitations

- **`candor-scan` can miss effects silently** — effects reached only through external-trait dispatch or an
  uninferrable receiver, desugared operators/`?`/`.await`/drop glue of external types, and cross-crate
  propagation by identity unless the dependency is chained (`--deps`). Nothing in the report marks a miss.
  What it *does* disclose: an invoked fn-value/callback, an FFI `extern` call, a trait with no visible implementor
  and an untrusted chained report read `Unknown`, and the coverage ledger names uncovered dependencies. The
  bullets below describe the oracle lint's own residuals.
- **Dynamic dispatch / fn-pointers / callbacks** can't be resolved to a concrete callee. These are
  surfaced as `Unknown` (→ AS-EFF-003) rather than silently dropped, but candor still can't
  tell you *which* effects hide behind them. Exception: `dyn` over conventionally-pure std traits
  (`Display`, `Debug`, `Error`, `ToString`, `Clone`, …) is treated as pure, not `Unknown` —
  otherwise ubiquitous patterns like `dyn Error` formatting would flood reports with false positives.
- **Generic static dispatch** (`t.method()` for `t: T: Trait`) is assumed to honour its bound — a
  deliberate residual unsoundness to keep the report readable (see `CRITIQUE.md`).
- **Advisory, not enforced**: a `&Fs` token doesn't actually gate `std::fs`; candor only reports.
  For real enforcement use [cap-std](https://github.com/bytecodealliance/cap-std).
- **Macro-generated consts/statics are skipped** (to drop noise like tracing's per-log-site
  `__CALLSITE` statics). Macro-generated *functions* (an `async_trait` method, a derive-impl method,
  a decl-macro fn) **are** analyzed and reported — the earlier blanket skip was a real under-report,
  fixed and held by a fuzzer lane (`macro_call` / macro-defined sinks).
- **Capabilities must be direct parameters.** `declared_caps` recognizes a capability (`&Fs`, a
  cap-std `&Dir`) only as a top-level parameter. A capability reached *through* a struct field
  (`fn f(ctx: &AppContext)` where `ctx` holds the `Dir`) is not counted as declared — that function
  would be flagged in strict mode despite holding the capability.
- **Generic static dispatch over non-local traits** is assumed to honour its bound (CHA only sees
  through *local* traits); `CANDOR_PARANOID` flags the rest at the cost of noise.
- Logging via macros is deduped per function but counts every function that logs.

## Documentation

- **[candor-spec](https://github.com/tombaldwin/candor-spec)** — the language-agnostic spec candor
  implements (effect vocabulary, report schema, trust contract). The
  [JVM engine](https://github.com/tombaldwin/candor-java) is the family's **reference engine**; this
  repo, the from-spec-alone [TS engine](https://github.com/tombaldwin/candor-ts) and the
  [Swift engine](https://github.com/tombaldwin/candor-swift) complete the four code engines, joined
  by the [candor-agents](https://github.com/tombaldwin/candor-agents) domain engine for agent
  fleets; a CI conformance suite holds all of them to the same answers.
- **[AGENTS.md](AGENTS.md)** — self-contained instructions for an AI agent (install → run → read).
- **[PRINCIPLES.md](PRINCIPLES.md)** — the ideas candor (and its development) are built on.
- **[CRITIQUE.md](CRITIQUE.md)** — a critical self-assessment + comparison to prior art
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

The **soundness contract** — "never silently pure" — is its own gate, not a hope. An adversarial
fuzzer ([`soundness/`](soundness/)) generates compilable crates that thread a *known* effect from a
leaf up through a random chain of call forms, and asserts every reachable function is reported with
that effect or `Unknown` (a pure/omitted function is the bug it hunts). The fuzzer lanes cover the
forms that have historically hidden a call — direct/closure/generic/boxed callbacks, `dyn` and
arbitrary-self dispatch, UFCS, overloaded operators, `?`, `.await`, macros, implicit `Drop`, opaque
`impl Trait` returns, and the cross-crate boundary — each *teeth-verified* (reverting the fix makes
the lane fail). Two more
lanes run each program under `strace` and confirm candor's static prediction over-approximates the
effects the kernel actually observed — ground truth that trusts nothing about how the test was generated.

## Status

Beta — the candor family's Rust engine is `candor-scan`, declaring **spec 0.43** (the same contract the
reference engine, [candor-java](https://github.com/tombaldwin/candor-java), declares; the
cross-engine conformance suite pins the agreement). The scanner is
[calibrated on 35 real crates](eval/calibration/CALIBRATION.md) (no false positives in library
code) and can still miss effects it cannot resolve. The oracle lint's "never silently pure" contract is held in
CI by the adversarial [soundness fuzzer](soundness/) plus `strace`/runtime oracles, which is how the scanner's misses
are found; both gate surfaces carry pinned exit-code contracts (0/1/2) with fail-closed negatives. Pre-1.0: minor versions may change behavior,
always in the soundness-increasing direction (see [CHANGELOG.md](CHANGELOG.md)).

candor also **guards itself**: CI runs the oracle lint over candor against `.candor/baseline` (Self-guard). Its effectful
surface — the lint's config / baseline / cross-report reads and report writes, `candor-report`'s report
I/O, `candor-classify`'s config discovery and debug-env reads (all `Env`/`Fs`), and the build script
(`Exec`/`Fs`) — can't gain a *new* effect unnoticed. ⟨0.40⟩ that now includes a function ABSENT from the
baseline (a newly added one, or a formerly pure one the baseline omits): it is compared against nothing,
so `Env` seeded into a pure `candor-classify` function fails the guard (SOUNDNESS R811, which the old
"new code is exempt" rule left open). Review with `CANDOR_BACKEND=lint cargo candor diff .candor/baseline`, then splice the one function's entry into
the affected `.candor/baseline.<crate>.<kind>.json` when a new effect is intended (do not bulk-refresh: those files
define the engine's permitted effect surface).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
