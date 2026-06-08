# Using candor (instructions for an AI coding agent)

You are working in a Rust project. **candor** tells you, for every function, which side effects it
performs — network, filesystem, database, subprocess, env, clock, IPC, logging, randomness,
clipboard — *including effects inherited transitively from functions it calls*. Use it instead of
tracing call chains by hand or guessing what code does.

## 1. Set up and run it — one block, from nothing

Run this from the root of the project you're analyzing. (First run is slow: it downloads a pinned
Rust nightly and builds the lint — expect a few minutes; it's not stuck.)

```sh
cargo install cargo-dylint dylint-link 2>/dev/null || true            # the lint runner
git clone --depth 1 https://github.com/tombaldwin/candor /tmp/candor 2>/dev/null \
  || (cd /tmp/candor && git pull -q)
( cd /tmp/candor && cargo build )
LIB=$(ls /tmp/candor/target/debug/libcandor@*.dylib \
         /tmp/candor/target/debug/libcandor@*.so 2>/dev/null | head -1)
CANDOR_JSON=/tmp/candor-report cargo dylint --lib-path "$LIB"
```

This writes one report file per crate: `/tmp/candor-report.<crate>.<type>.json`.

**No nightly? Use the stable scanner.** If `cargo install cargo-dylint` or the nightly build fails
(locked-down box, stable-only CI), candor has a stable backend that needs nothing but stock `cargo` —
it produces the same report JSON the rest of this guide reads:

```sh
( cd /tmp/candor && cargo build -p candor-scan )                       # stable Rust, no nightly/dylint
/tmp/candor/target/debug/candor-scan . --out /tmp/candor-report       # writes /tmp/candor-report.<crate>.scan.json
```

It is *syntactic*, so it under-reports relative to the lint (it misses method-style effects, trait
dispatch, macros, and cross-crate propagation, and does **not** emit `Unknown`) — good for fast triage,
not for the soundness contract. Everything below works identically against its report.

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

## 3. Use it

- **What effects does a function have? / blast radius of editing it** → `cargo candor show <fn>`
  (instant — its full effect set; `*` = performed directly).
- **Why does a function have an effect?** → `cargo candor explain <fn>` traces the call path to the
  source (`main → middle → leaf`, and `leaf` calls `std::net::TcpStream::connect`). Use it before
  editing to see what flows through a function, and to act on the trust rule (§4) — it shows you
  exactly which call is the `Unknown`.
- **Did I build an effect on untrusted input?** → `cargo candor risk` flags an effect whose argument
  comes from a function parameter (`fs::read(path_from_param)`, `Command::new(name)`) — the injection
  class. A *heuristic* nudge (it over- and under-flags): treat a hit as "validate this input or confirm
  its source is trusted," not as proof of a bug.
- **Which functions touch the network (or any effect)?** → `cargo candor where Net` (instant — splits
  the direct sources from the functions that inherit it). Faster than grepping the codebase.
- **Who calls this function? (before editing it)** → `cargo candor callers <fn>` (instant — its direct
  callers, from the report's call graph). Faster than grepping for call sites.
- **Safe to treat as pure (e.g. unit-test without mocks)?** → `inferred == []` *and* `unresolved == false`.

## 4. The trust rule — do not skip this

`inferred` is **authoritative for what candor resolved**. When `unresolved` is `true` (or `"Unknown"`
appears in the set), the effect list **may be incomplete** — read the source for *that* function
before relying on it. Never conclude a function is pure or effect-free if it is marked `unresolved`.
candor is deliberately honest about what it cannot see; respect that boundary.

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

