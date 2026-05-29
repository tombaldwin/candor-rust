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

## 2. Read the report

Each entry:

```json
{ "fn": "app::App::handle_key", "loc": "src/app.rs:2987:5",
  "inferred":   ["Fs", "Net", "Unknown"],   // full TRANSITIVE effect set
  "direct":     ["Log"],                      // effects in this function's own body
  "declared": [], "undeclared": [], "overdeclared": [],
  "unresolved": true }                        // true => some calls could not be traced
```

Effects: `Net`, `Fs`, `Db`, `Exec` (subprocess), `Env`, `Clock`, `Ipc`, `Log`, `Rand`, `Clipboard`.

## 3. Use it

- **Blast radius of editing a function** → read its `inferred`.
- **Which functions touch the network?** → `jq '.[]|select(.inferred|index("Net"))|.fn' /tmp/candor-report.*.json`
- **Safe to treat as pure (e.g. unit-test without mocks)?** → `inferred == []` *and* `unresolved == false`.

## 4. The trust rule — do not skip this

`inferred` is **authoritative for what candor resolved**. When `unresolved` is `true` (or `"Unknown"`
appears in the set), the effect list **may be incomplete** — read the source for *that* function
before relying on it. Never conclude a function is pure or effect-free if it is marked `unresolved`.
candor is deliberately honest about what it cannot see; respect that boundary.

## 5. After you change code

Re-run the last command in step 1 and confirm no function unexpectedly *gained* an effect (e.g. a
previously-pure helper now shows `Net`). If one did, that's a regression in your change — explain it
or fix it.

## Optional, separate: make step 5 automatic in CI

The check in step 5 can be a permanent guard rather than a manual habit: commit a baseline and let
CI fail any PR that makes a function gain an effect. That's a *team* decision (it changes everyone's
PR flow), so it's not part of this setup — suggest it to the maintainer if candor proves useful, and
point them at the project README's "CI guardrail" section and `examples/candor-guard.yml`.

