<!-- MAINTAINERS: this is the canonical doc. After editing it, re-sync the embedded crate copies in the SAME commit or CI's drift gate (embedded_agents_contract_matches_the_repo_doc) fails: cp AGENTS.md crates/candor-scan/AGENTS.md && cp AGENTS.md crates/candor-query/AGENTS.md -->
# Using candor (instructions for an AI coding agent)

You are working in a Rust project. **candor** tells you, for every function, which side effects it
performs — network, filesystem, database, subprocess, env, clock, IPC, logging, randomness,
clipboard — *including effects inherited transitively from functions it calls*. Use it instead of
tracing call chains by hand or guessing what code does. The language-agnostic consumption contract is
[candor-spec/AGENTS.md](https://github.com/tombaldwin/candor-spec/blob/main/AGENTS.md); this file is
the Rust-specific production + query surface.

> **If the repository is not Rust-only, start at the umbrella:**
> [candor/AGENTS.md](https://github.com/tombaldwin/candor/blob/main/AGENTS.md). `candor` is one
> command in front of every engine (Rust, JVM, TypeScript, Swift, agent fleets) — it picks the right
> one per target, `candor update` installs and upgrades them, and `candor doctor` checks that every
> installed engine agrees on a spec version. A polyglot repo scanned with this engine alone gets an
> answer about its Rust and nothing that says so.

> **This document ships inside the tools.** `candor-scan --agents` (and `candor-query --agents`)
> print the contract for the *installed* version — always prefer that over a vendored or fetched
> copy, which can describe a different candor than the one you are running. If the receipt or
> `-V` shows the engine version changed since you last read it, re-run `--agents`.

## 0. Check what's already installed — and tell the user

Before installing or scanning anything:

```sh
command -v candor-scan && candor-scan -V          # the published stable scanner?
ls -d /tmp/candor ~/.candor 2>/dev/null            # an existing engine clone?
command -v cargo-candor && echo "cargo candor available"
```

**If candor is already present** (a `.candor/` report dir, or `candor-scan` on PATH), do *not* just
silently scan with it — surface it to the user first:

1. Run `candor-scan --version` (offline) and **tell the user, plainly, which version this project is
   on** — e.g. "This project is on candor-scan 0.31.0 (spec 0.31)." On a build too old for the two-line
   `--version`, read `candor.version` / `candor.spec` from an existing `.candor/report*.json` and
   report those instead.
2. Then do the §1a currency check (candor can't phone home; *you* have network) and, if it's behind,
   **ask before upgrading** — never upgrade silently. See §1a.

If candor is current, or the user declines the upgrade, proceed with what's there. Otherwise pick a
path in §1 and install normally.

## 1. Get a report — two backends, one JSON shape

Both write the same report files; everything below reads either. Choose:

- **Path A (the floor)** — published scanner, stable Rust, one `cargo install`, seconds to run.
  Syntactic: it **under-reports** relative to the deep engine (external-trait dispatch, uninferrable
  receivers, desugared operators, cross-crate identity — those misses are *silent* by design). It
  emits `Unknown` only where it can see the boundary: an invoked fn-value/callback, an FFI `extern`
  call, an untrusted chained report. It never fabricates an effect. Right for a fast effect map,
  triage, and stable-only CI.
- **Path B (the certificate)** — the deep engine (a rustc-integrated lint on a pinned nightly,
  a few minutes to build once). Type-system-level resolution with the §4 trust contract: anything
  it can't resolve is marked `Unknown`, never silently pure. Right when you need to *rely* on
  "no effect reported" — soundness gates, purity claims, security review.

**Path A — from nothing to a report in two commands:**

```sh
cargo install candor-scan
candor-scan . --out /tmp/candor-report     # writes /tmp/candor-report.<crate>.scan.json (+ callgraph sidecar)
                                           # a workspace root writes ONE REPORT PER MEMBER under that prefix —
                                           # point the query tools at the prefix and they merge all of them
```

It can also enforce a policy file as a gate: `candor-scan . --policy .candor/policy` (exit 1 on
violation) — an **advisory floor**: a clean run is necessary, never sufficient; the deep engine is
the sound gate. The same floor applies to the AS-EFF-005 regression guard: with
`CANDOR_BASELINE=<saved report path or --out prefix>` (or the `.candor/config` `baseline` key) a
function that *gained* an effect vs the saved report exits 1; no baseline file → a note, guard
inactive; a baseline from a **different scanner build** (or unparseable) → exit 2 without
evaluating — never a stale compare, never a silent skip.

**Path B — clone + build the deep engine** (first build downloads a pinned nightly — expect a few
minutes; it's not stuck):

```sh
cargo install cargo-dylint dylint-link 2>/dev/null || true            # the lint runner
git clone --depth 1 https://github.com/tombaldwin/candor-rust /tmp/candor 2>/dev/null \
  || (cd /tmp/candor && git pull -q)
( cd /tmp/candor && cargo build && ./install.sh )                     # install.sh puts `cargo candor` on PATH
cargo candor audit          # first query generates the deep report (.candor/report.<crate>.<type>.json)
```

Or without `install.sh`, invoke the lint directly:

```sh
LIB=$(ls /tmp/candor/target/debug/libcandor@*.dylib \
         /tmp/candor/target/debug/libcandor@*.so 2>/dev/null | head -1)
CANDOR_JSON=/tmp/candor-report cargo dylint --lib-path "$LIB"
```

Either way you get one report file per crate: `<prefix>.<crate>.<type>.json`. Rust entries never
carry `unitKind` (every Rust unit is an ordinary function — the spec-0.7 field's default);
sibling reports under a merged prefix may carry it (an accessor, a `<clinit>`, a fleet's agents) —
it is informative only, read effects/edges identically.

## 1a. Staying current — candor can't check for you, *you* can

candor never makes a network call to see if it's out of date: its own policy is `deny Net` (it's an
effect auditor — phoning home would make it perform the exact effect it exists to forbid). So the
version check is *your* job, not the tool's. You have network access; it doesn't.

```sh
candor-scan --version          # offline: "candor-scan <ver> (spec <SPEC>)" + the upgrade line; no network.
# Then YOU (you have network; candor doesn't) compare against crates.io.
# crates.io REQUIRES a User-Agent header (it rejects requests without one):
curl -s -H 'User-Agent: candor-version-check' https://crates.io/api/v1/crates/candor-scan \
  | grep -o '"max_version":"[^"]*"'                                      # -> "max_version":"<version>"
```

If they differ, **ask the user before upgrading** — e.g. "candor-scan 0.7.2 is available (you're on
0.7.1) — upgrade before I scan?" — and only run the upgrade if they agree. Never upgrade silently:
the engine version is part of the result's provenance (the report's `version` field), so a bump the
user didn't sanction quietly changes what the scan means. Upgrade by the path you installed with:

- **Path A** (`cargo install candor-scan`): `cargo install candor-scan --force`.
- **Path B** (a clone at `/tmp/candor` or `~/.candor`): `cargo candor update` — it runs
  `git pull --ff-only`, rebuilds the engine + integration scripts + this AGENTS.md at one commit,
  then **restamps `.candor/baseline` and tells you whether the new engine classifies your own code
  differently** (the thing to actually check after a tool bump — a verdict change is the upgrade's
  doing, not your code's). Without `install.sh`, just re-run the `git pull` + `cargo build` from §1.

Pin for reproducibility: PROVE-IT.md requires **0.3.5 or later** (earlier published builds have
since-fixed resolution bugs). The report's `version` field records the exact engine build, so a report
you commit is traceable to the engine that produced it.

## 2. Read the report

Each entry:

```json
{ "fn": "app::App::handle_key", "loc": "src/app.rs:2987:5",
  "inferred":   ["Fs", "Net", "Unknown"],   // full TRANSITIVE effect set
  "direct":     ["Log"],                      // effects in this function's own body
  "fs":         ["read", "write"],            // (optional) Fs access kind, when the verbs reveal it
  "declared": [], "undeclared": [], "overdeclared": [],
  "unresolved": true }                        // true => some calls could not be traced
```

Effects: `Net`, `Fs`, `Db`, `Exec` (subprocess), `Env`, `Clock`, `Ipc`, `Log`, `Rand`, `Clipboard`.
`fs` refines `Fs` (read vs write) when statically knowable; `cargo candor show` renders it as
`Fs(write)` / `Fs(read,write)`. Absent when unknown or no `Fs` — it never changes the `Fs` effect.

**The report lists only effectful (or unresolved) functions — pure functions are omitted.** Alongside
it, both backends write `<prefix>.<crate>.<type>.callgraph.json`: **every** function (including pure
ones) mapped to its callees. So:

- in the callgraph sidecar, *absent* from the report → **pure** (as far as the backend can see; on
  Path A remember it under-reports),
- in the report → effectful and/or `unresolved` (§4),
- in *neither* → the backend never saw it (wrong crate? `#[cfg]`'d out? tests without
  `--include-tests`?) — do not conclude anything about it.

## 3. Query it

With the Path B clone installed, `cargo candor <cmd>` answers the common questions **instantly**
(reading the report, not recompiling):

- **What effects does a function have? / blast radius of editing it** → `cargo candor show <fn>`
  (`*` = performed directly).
- **Why does a function have an effect?** → `cargo candor explain <fn>` traces the call path to the
  source (`main → middle → leaf`, and `leaf` calls `std::net::TcpStream::connect`). Use it before
  editing to see what flows through a function, and to act on the trust rule (§4) — it shows you
  exactly which call is the `Unknown`.
- **Did I build an effect on untrusted input?** → `cargo candor risk` flags an effect whose argument
  comes from a function parameter (`fs::read(path_from_param)`, `Command::new(name)`) — the injection
  class. A *heuristic* nudge (it over- and under-flags): treat a hit as "validate this input or confirm
  its source is trusted," not as proof of a bug.
- **Which functions touch the network (or any effect)?** → `cargo candor where Net` (splits the
  direct sources from the functions that inherit it). Faster than grepping the codebase.
- **Blast radius of editing this function** → `cargo candor impact <fn>` — the `affected` list (every
  effectful fn that transitively calls it) + the downstream `entryPoints` (the runtime roots a change
  surfaces through). `cargo candor callers <fn>` is the lower-level raw direct+transitive callers.
  Either beats grepping for call sites.
- **If I add an effect here, what breaks?** → `cargo candor whatif <fn> Net` (pre-edit: what the
  effect propagates to, and whether it would violate the policy).
- **Safe to treat as pure (e.g. unit-test without mocks)?** → use the §2 rule: in the callgraph
  sidecar and absent from the report (or present with `inferred == []` and `unresolved == false`,
  which the engine normally elides). On Path A, "absent" is only as strong as the syntactic floor.

The same queries exist as a standalone binary, `candor-query`, built by the Path B clone
(`/tmp/candor/target/debug/candor-query`) — useful when `cargo candor` isn't on PATH. Positional
args; the trailing `0|1` is the want-JSON flag:

```sh
candor-query show     <prefix> <fn-query>  <0|1>
candor-query where    <prefix> <Effect>    <0|1>
candor-query callers  <prefix> <fn-query>  <0|1>
candor-query impact   <prefix> <fn-query>  [--json]   # the blast radius: {fn,affectedCount,affected,entryPoints}
candor-query map      <prefix>             <0|1>
candor-query whatif   <prefix> <fn> <Effect> [policy-file] [0|1]
candor-query path     <prefix> <fn> <Effect> [--json]
candor-query tour     [<N>] [--report <locator>] [--json]   # the N (default 10) most surprising transitive reaches
candor-query gains    <cur_prefix> <base_prefix> [--json]   # supply-chain alarm: effects a surface gained
candor-query diff     <cur_prefix> <base_prefix> <0|1> <baseline_ver> <engine_ver>
candor-query gate     --report <locator> --policy <file> [--json] [--gate-json <f>]
```

`gate` applies a policy to a report you ALREADY HAVE — no scan, no source. It is the supply-chain
verb (gate a dependency's published report without re-analysing code you do not have) and its exit
codes are `candor-scan --policy`'s exactly: 0 clean, 1 violation, 2 refused. It reads the report file
and nothing else: an entry ABSENT from the report is candor's purity claim, and `gate` will not
back-fill it from the callgraph sidecar or a chained dep. Rules whose evidence the report cannot
carry are REFUSED (exit 2) rather than evaluated on partial evidence — `forbid A -> B` (a report's
`calls` is effect-relevant, so a crossing into a pure unit is invisible), `only A -> B …` (it asks
whether EVERYTHING reached is on a list, so an omitted crossing turns a green into a claim of
completeness), `allow <E> …` (the
AS-EFF-008 completeness marker is not a gate-usable wire fact), and a class-scoped `deny` over an
entry whose scoping field is absent. Gate those at scan time instead. A report whose `analyzed.count`
is **0** is refused too: it judged nothing, so an absent entry in it is not a purity claim and a green
verdict would certify a signature that makes no claim about any unit. (`count > 0` with an empty
`functions` is the opposite — a package that WAS judged and found effect-free — and gates clean.)

`<prefix>` is the report path prefix (e.g. `/tmp/candor-report` — the part before
`.<crate>.<type>.json`). A prefix matching no report files fails loud (exit 2), so a silent `{}` is
never "wrong path". With **only Path A installed** there is no query binary — read the JSON directly
(`jq`/`python3` over the report + callgraph sidecar); the shapes above are all derivable from those
two files.

## 4. The trust rule — do not skip this

`inferred` is **authoritative for what candor resolved**. When `unresolved` is `true` (or `"Unknown"`
appears in the set), the effect list **may be incomplete** — read the source for *that* function
before relying on it. Never conclude a function is pure or effect-free if it is marked `unresolved`.
candor deliberately discloses what it cannot see; respect that boundary. (Path A emits `Unknown`
only for the boundaries it can see — an invoked fn-value/callback, an FFI `extern` call, an
untrusted chained report; its **other** misses are silent by design, so treat every Path A absence
as "not seen syntactically", never "proven absent".)

## 5. After you change code

Run `cargo candor diff .candor/baseline` (add `--json` to parse it). It lists, per function, the
effects your change *gained* or *lost* vs the baseline — including the **non-local** consequence: a
network call you add deep in a helper shows `+Net` on every function that calls it. (For a tight loop,
run `cargo candor watch &` once at the start of your session — it keeps the report fresh in the
background so `diff` reads it instantly instead of recompiling.) For each `+`:

- **Was it intended?** If a function gained `Net`/`Db`/`Exec`/`Fs`/`Env`/`Ipc` it didn't have,
  confirm it's necessary, and prefer threading a capability over reaching for ambient authority.
- **A gained `Unknown`** means candor can no longer prove that function's effect set is complete —
  read it (you likely added a callback or dynamic dispatch it can't see through).

An unintended gain is a regression in your change — fix it. If intended, say so.

If the project has a `.candor/policy`, an **`AS-EFF-006`** means your change made a function cross a
declared architectural boundary (e.g. domain logic now reaches the database, even through a helper).
Move the I/O to the layer that's allowed to perform it — do not just delete the rule.

### If candor prompts you (the opt-in `CANDOR_REVIEW` self-review)

When the maintainer has set `CANDOR_REVIEW=1`, candor's Stop hook hands you this delta automatically
after any turn that introduced a new effect ("your edits gave functions NEW effects: …"). Treat it as
a checkpoint, not a blocker: review each gained effect as above, fix anything unintended, then finish.
Each effect is surfaced once — you won't be re-prompted for the same one.

## Optional, separate: make step 5 automatic in CI

The check in step 5 can be a permanent guard rather than a manual habit: commit a baseline and let
CI fail any PR that makes a function gain an effect. That's a *team* decision (it changes everyone's
PR flow), so it's not part of this setup — suggest it to the maintainer if candor proves useful, and
point them at the project README's "CI guardrail" section and `examples/candor-guard.yml`.

- **⟨0.30⟩ A GREEN GATE CAN NOW EXIT 2 — read `outOfScope` before you trust a pass.** When a policy is
  configured, candor also reads the files the scan EXCLUDED (test files, build scripts, archives under the
  root, files outside the build's program) and reports any that perform an effect the policy DENIES, under
  the report's `outOfScope` key. A non-empty block makes the verdict `ok:false`, `incomplete:true`, exit 2
  — *"I could not see enough of this tree to certify it"*, which is NOT the same as "your code violates":
  those functions are never in `violations`, because the gate did not judge them. Branch on `incomplete`
  to tell the two apart. An absent key means the producer was never asked (no policy at scan time), and an
  empty one means asked-and-clear.
