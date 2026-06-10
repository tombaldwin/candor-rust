# Announcement drafts (for distribution)

Reusable, honest copy for announcing candor. Pick a channel, trim to taste. The guiding rule: no hype,
lead with the concrete thing, be upfront about the limits — the audience is skeptical Rust developers.

---

## Show HN / r/rust title options

- `Show HN: candor – map which functions in a Rust crate touch the network/fs/db, transitively`
- `candor-scan: a stable-Rust tool that maps effects (net/fs/db/exec) and their blast radius`
- `I built an effect/capability checker for Rust. cargo install candor-scan`

## One-liner (tweet / toot / mastodon)

> `cargo install candor-scan` → a map of which functions in a Rust crate reach the network, filesystem,
> a database, or a subprocess — *transitively*, across the call graph — and the blast radius of editing
> any one of them. Stable Rust, no nightly. https://github.com/tombaldwin/candor-rust

---

## Body (Show HN / blog post)

**candor maps what every function in a Rust codebase actually *does*** — which ones reach the network,
the filesystem, a database, subprocesses, the clock, the environment — *transitively* across the call
graph, and where it honestly can't tell.

```sh
cargo install candor-scan
candor-scan .          # writes .candor/report.<crate>.scan.json   (or --json to stdout)
```

**Why I built it.** You edit one function without seeing the *non-local* consequence — a network call
added deep in a helper now propagates to every caller. A human misses it in review; an AI agent misses
it constantly. candor surfaces exactly that: "this edit gives `compute_price` a filesystem effect, which
propagates to these 6 callers — including the health probe that's supposed to stay I/O-free."

**It does two jobs:**

1. **A map / blast radius.** `where Net` lists every function that reaches the network; `callers <fn>`
   gives the transitive blast radius before you edit something. Instant — it reads a cached report.
2. **Enforcement at a scale nobody holds in their head.** A `.candor/policy` turns architecture into a
   CI gate: a shared library that must stay free of app-coupling I/O; a database tool that must keep
   every query in its data layer; a domain layer that must reach no I/O even through a helper. The tool
   blocks the PR — which is the thing a model can't do for itself.

**Honest about the limits.** There are two backends sharing one classifier:

- `candor-scan` (the one you `cargo install`) is **syntactic** — it parses source, never builds, runs
  anywhere `cargo` does, even on a dependency you haven't compiled. It's calibrated to **never fabricate
  an effect** but it **under-reports**: it can't see through some trait-object dispatch or cross-crate
  calls, and it does *not* emit `Unknown`.
- The full nightly lint (a [dylint](https://github.com/trailofbits/dylint)) adds the soundness contract:
  it marks what it can't resolve `Unknown` rather than guessing, and powers the conformance/policy gates.

**On the rigor** (because "trust me" isn't enough for a trust tool):

- Calibrated across **1,294 real crates** from crates.io — zero crashes, and **zero false positives** on
  a curated set of known-pure crates (encoders, parsers, hashing, data structures).
- In a **pre-registered** eval, an AI agent handed candor's effect delta reported the full propagation
  **100% of the time, vs 7% without it**.
- A pre-registered enforcement trial took a shipped-architecture-violation rate from **80% → 0%** when
  the locally-simplest edit crossed a "pure" boundary.

It's MIT/Apache-2.0, built for the agent era but useful to humans. The classifier is curated (it knows
~50 common effectful crates + the libc/libsqlite3/libgit2/libssl FFI tiers), so coverage is honest about
its gaps rather than guessing.

Repo: https://github.com/tombaldwin/candor-rust · crate: https://crates.io/crates/candor-scan

I'd genuinely like to know where it under-reports on *your* code — that's the feedback that moves it.

---

## Notes for posting

- **Lead with the install line and a screenshot/asciicast** of `candor-scan .` on a real crate (e.g.
  `tokio` or your own app) — the concrete output sells it more than the description.
- HN/Reddit reward honesty about limits — the "under-reports, never fabricates" framing is a feature,
  not an apology; keep it prominent.
- Have an answer ready for the obvious questions: "why not just grep for `reqwest`?" (transitive
  propagation + the policy gate is the point); "isn't the classifier a losing battle at 50 crates vs
  150k?" (yes for blanket coverage — but the value is the *boundary enforcement* and the *blast radius*,
  which don't need every crate; and coverage warns on what it doesn't know).
- The pre-registered evals (`eval/`) and the 1,294-crate calibration (`eval/calibration/CALIBRATION.md`)
  are your credibility — link them when someone pushes on "does it actually work."
