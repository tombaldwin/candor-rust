# Announcement drafts (for distribution)

Reusable, honest copy for announcing candor. Pick a channel, trim to taste. The guiding rule: no hype,
lead with the concrete thing, be upfront about the limits — the audience is skeptical Rust developers.
Every number below is from a pre-registered eval or a measured run in `eval/`; if you can't point at
the file, don't say it.

---

## Show HN / r/rust title options

- `Show HN: candor – map which functions in a Rust crate touch the network/fs/db, transitively`
- `candor-scan: a stable-Rust tool that maps effects (net/fs/db/exec) and their blast radius`
- `Show HN: an effect map + CI gate for Rust/JVM, built for coding agents (with the evals to show why)`

## One-liner (tweet / toot / mastodon)

> `cargo install candor-scan` → a map of which functions in a Rust crate reach the network, filesystem,
> a database, or a subprocess — *transitively*, across the call graph — the blast radius of editing any
> one of them, and a `--policy` gate that fails the build when an edit crosses a declared boundary.
> Stable Rust, no nightly. https://github.com/tombaldwin/candor-rust

---

## Body (Show HN / blog post)

**candor maps what every function in a codebase actually *does*** — which ones reach the network,
the filesystem, a database, subprocesses, the clock, the environment — *transitively* across the call
graph, and where it honestly can't tell.

```sh
cargo install candor-scan
candor-scan .                       # writes .candor/report.<crate>.scan.json (+ a call-graph sidecar)
candor-scan . --policy .candor/policy   # CI gate: exit 1 when an edit crosses a declared boundary
```

**Why I built it.** You edit one function without seeing the *non-local* consequence — a network call
added deep in a helper now propagates to every caller. A human misses it in review; an AI agent misses
it constantly. candor surfaces exactly that: "this edit gives `compute_price` a filesystem effect, which
propagates to these 6 callers — including the health probe that's supposed to stay I/O-free."

**It does two jobs:**

1. **A map / blast radius.** `where Net` lists every function that reaches the network; `callers <fn>`
   gives the transitive blast radius before you edit something; `whatif <fn> Net` answers "if I add a
   network call here, what propagates and does it break the architecture?" *before* any code is
   written. Instant — they read a cached report.
2. **Enforcement at a scale nobody holds in their head.** A `.candor/policy` turns architecture into a
   CI gate: a shared library that must stay free of app-coupling I/O; a domain layer that must reach no
   I/O even through a helper. The tool blocks the PR — which is the thing a model can't do for itself.

**Honest about the limits.** There are two backends sharing one classifier:

- `candor-scan` (the one you `cargo install`) is **syntactic** — it parses source, never builds, runs
  anywhere `cargo` does, even on a dependency you haven't compiled. It's calibrated to **never fabricate
  an effect** but it **under-reports**: it can't see through some trait-object dispatch or cross-crate
  calls, and it does *not* emit `Unknown`. Its `--policy` gate is accordingly an **advisory floor** —
  a clean run is necessary, never sufficient.
- The full nightly lint (a [dylint](https://github.com/trailofbits/dylint)) adds the soundness
  contract: anything it can't resolve is marked `Unknown`, never silently pure — enforced by a
  construction-based fuzzer that generates effect chains through the call forms that historically hide
  them (closures, `dyn` dispatch, operator overloads, `?` conversions, `.await`, RAII drops, opaque
  `impl Trait` returns) and fails if any function comes back silently pure.

And one limit that matters more than either: **a green effect gate is not a green feature.** In a
behavioral probe, a weak model under gate pressure "passed" the policy by de-wiring the feature it was
supposed to keep. That finding is committed in the repo (`eval/whatif-behavior/`), and it's why
`candor rewire` exists — it diffs call graphs and flags dropped edges, catching exactly that gaming.
Pair the gate with your tests; it's a seatbelt, not a driver.

**On the rigor** (because "trust me" isn't enough for a trust tool):

- Calibrated across **1,294 real crates** from crates.io — zero crashes, and **zero false positives** on
  a curated set of known-pure crates (encoders, parsers, hashing, data structures).
- **Pre-registered blast-radius eval**: agents asked to enumerate an edit's transitive consequences got
  **6% of the affected functions by hand; 79–100% with candor** — and a frontier model does *not*
  close that gap (the bottleneck is that models won't volunteer five layers of tedious enumeration).
- **Pre-registered speed A/B across three model tiers** (N=8/arm/tier): the tool's answer is
  **model-invariant** (16/16 functions at every tier, one query); manual tracing is not — ~2× faster
  at the frontier tier, **~6× at the Sonnet tier, where the manual arm also silently dropped
  functions in 3 of 8 trials**. The cheaper the model, the more the tool carries.
- A pre-registered enforcement trial took a shipped-architecture-violation rate from **80% → 0%** when
  the locally-simplest edit crossed a "pure" boundary.
- Token cost, measured then **self-corrected**: our first comparison (vs reading the whole crate,
  ~700–2000×) was a strawman and we say so in the repo; against a realistic grep-trace the median is
  **~17× fewer tokens per blast-radius question** (range 1–225×).

**Not just Rust.** The effect vocabulary, report shape, query names, and policy grammar are a
[spec](https://github.com/tombaldwin/candor-spec); a JVM engine
([candor-java](https://github.com/tombaldwin/candor-java), ASM over bytecode — Java, Kotlin, Scala,
Groovy validated on real apps) answers them identically, and a CI conformance suite makes the
cross-language consistency machine-checked, not aspirational. As an existence proof that the spec is
the product: a [TypeScript engine](https://github.com/tombaldwin/candor-ts) written from the spec text
alone — without reading either reference implementation — passes the same 20-case oracle, live in the
suite's CI.

**Don't take the evals' word for it — run the experiment on your own repo.**
[PROVE-IT.md](https://github.com/tombaldwin/candor-rust/blob/main/PROVE-IT.md) is a copy-paste prompt
for any agentic tool: your agent commits a manual trace first, computes candor's answer from the raw
JSON, then diffs them with mandatory file:line verification of every discrepancy — honest-outcome
branches in both directions. If candor loses on your code, that's exactly the report I want.

It's MIT/Apache-2.0, built for the agent era but useful to humans. The classifier is curated (it knows
~50 common effectful crates + the libc/libsqlite3/libgit2/libssl FFI tiers), so coverage is honest about
its gaps rather than guessing.

Repo: https://github.com/tombaldwin/candor-rust · crate: https://crates.io/crates/candor-scan ·
site: https://candor.poly.io

I'd genuinely like to know where it under-reports on *your* code — that's the feedback that moves it.

---

## Hostile-question crib sheet

The questions a skeptical HN/r-rust thread will ask, with the honest answers. Don't dodge; the
honesty *is* the positioning.

- **"Why not just grep for `reqwest`?"** Grep finds the leaf. The value is the *transitive closure*
  (who inherits the effect, five layers up) and the deterministic gate. Measured: agents hand-tracing
  a closure got 6% of it; grep-tracing costs a median ~17× more tokens and is least reliable exactly
  when names are common.
- **"Why not CodeQL/Semgrep?"** You could build a reachability query in CodeQL per language, per
  vocabulary, per policy — nothing off-the-shelf does effect-set + policy-gate + agent-facing instant
  queries, and nothing gives the same answer in Rust and on the JVM. candor's cross-language
  consistency is machine-checked in CI (same fixtures, both engines, same verdicts — plus a
  from-spec-alone TS engine as the derivability proof). Also: setup weight; CodeQL's license excludes
  closed-source CI.
- **"Isn't the classifier a losing battle — 50 crates vs 150k?"** For blanket coverage, yes, and we
  say so. The value rides on the call *graph* and the *gate*, which don't need every crate classified;
  the coverage check warns on the crates it doesn't know instead of guessing, and `Unknown` marks what
  the engine can't see. Under-report-and-say-so beats fabricate-and-look-complete.
- **"Soundness? Static analysis always lies."** Two different promises, stated separately: the deep
  engines (nightly lint, JVM) never report a function pure when they couldn't resolve its calls —
  that's fuzzer-enforced (effect chains generated through closures, dyn dispatch, operators, `?`,
  `.await`, drops, opaque returns; any silent-pure = red build). The quick-install scanner makes a
  deliberately weaker promise — under-report rather than fabricate — and its gate is documented as a
  floor. Treat the scanner as triage, the deep engines as the certificate.
- **"`Unknown` is a cop-out."** It's the honesty marker. The alternative is silently guessing pure
  (unsound) or flooding every generic call with warnings (unusable). `Unknown` is bounded — it
  appears where dispatch is genuinely unresolvable — and `explain` shows you exactly which call it is.
- **"Can't an agent just game the gate?"** Yes — measured, committed, and tooled: a weak model under
  gate pressure de-wired the feature to get green (`eval/whatif-behavior/`). `rewire` diffs call
  graphs and flags dropped edges to catch exactly that. Gate + rewire + your tests; the gate alone is
  necessary, never sufficient.
- **"Your evals are self-scored / circular."** The protocols are pre-registered (hypotheses +
  falsification bars committed before trials, in `eval/`), the raw per-trial numbers are published,
  and the strongest fact is external: 16 *independent* manual traces at frontier tiers converge on
  exactly the tool's answer. Where an eval embarrassed us we published it (the token-cost strawman
  correction, the behavioral-probe gaming, the control hitting 100% on an easy fixture) — and
  PROVE-IT.md exists so you can run the A/B on your own repo and keep the result.
- **"What about build.rs / proc macros / unsafe FFI?"** Effects in macro-*expanded* code are seen
  (it's the same HIR); the libc/libsqlite3/libgit2/libssl FFI tiers are classified. Build-time
  effects (`build.rs` running at compile time) are out of scope of the runtime report. `unsafe`
  per se isn't an effect — `cargo-geiger` measures that; candor measures what code *reaches*.
- **"Why a curated classifier instead of types/capabilities in the language?"** An effect system in
  the language would be better and doesn't exist (and would be one language). cap-std constrains at
  runtime; candor maps and gates statically, today, on unmodified code, in two languages.
- **"The AI angle is hype."** The AI claims are the most-measured part: pre-registered A/Bs at three
  model tiers, completeness and wall-clock and tokens, with the negative results in the repo (frontier
  models don't need the tool for *easy* questions — value is 0 there and we say so; it's the deep
  transitive questions and the cheaper tiers where it carries).

## Notes for posting

- **Lead with the install line and a screenshot/asciicast** of `candor-scan .` on a real crate (e.g.
  `tokio` or your own app) — the concrete output sells it more than the description.
- HN/Reddit reward honesty about limits — the "under-reports, never fabricates" framing is a feature,
  not an apology; keep it prominent.
- The pre-registered evals (`eval/`) and the 1,294-crate calibration (`eval/calibration/CALIBRATION.md`)
  are your credibility — link them when someone pushes on "does it actually work."
- PROVE-IT.md is the conversion path for skeptics: "run it on your repo, post the result" is a better
  reply than any argument. It requires candor-scan ≥0.3.2 (published).
