# Bet 2 — Pre-registration: does candor change the code an agent SHIPS?

Committed **before** any trial is run. The hypothesis, arms, metric, sample
size, and analysis below are fixed in advance to prevent post-hoc tuning.

## Question

The edit-quality evals so far measure what the agent **reports** about effects.
Bet 2 asks the harder question: does candor change the code the agent **ships**?
Specifically — when a task tempts an agent to put I/O in a layer that is
supposed to be pure, does having candor enforce the boundary reduce the rate at
which the violation lands in the shipped code?

## Fixture (`eval/bet2/fixture/`)

A small Rust crate with a deliberate architectural boundary:

- `pricing` — the pure domain core. Computes quotes from a catalogue and an FX
  rate **already supplied to it**. Must perform no effects.
- `service` — the I/O layer. All network/fs/env/subprocess work lives here.
- `ARCHITECTURE.md` states the rule (pricing is pure; service does I/O and feeds
  the rate to `Pricing` via `set_rate`). Present in **both** arms.
- `.candor/policy`: `pure pricing` — candor's machine-checkable form of the same
  rule.

The task (`TASK.md`, identical in both arms): make quotes use a **live** FX rate
from `rates.internal:7070` over TCP. `main` calls `pricing.quote(...)` directly,
so the locally-simplest edit ("fetch the rate where it's used") puts the network
call **inside `pricing`** — the violation. The correct edit fetches in `service`
and threads the rate in. Both paths are available; the architecture doc forbids
the tempting one.

## Arms

Identical task, fixture, and model. The **only** difference is the enforcement
signal:

- **control** — agent gets the crate + TASK.md + ARCHITECTURE.md. No candor.
- **treatment** — same, plus: the project ships an architecture **gate**
  (`check.sh`) that runs candor against `.candor/policy` and fails on an
  `AS-EFF-006` violation. The agent is told to run it before finishing and to
  resolve any violation it reports (mirroring candor as a CI gate / edit hook).
  The treatment instruction is purely operational — "a gate exists, run it, fix
  what it flags" — and does **not** restate the architectural principle (that
  lives in ARCHITECTURE.md, which both arms see). So the manipulation is
  "enforcement signal present," not "extra reminder to stay pure."

## Primary metric (objective, instrument-independent)

`io_in_pricing` ∈ {0,1}: does the shipped `src/pricing.rs` textually contain I/O
syntax (`TcpStream|std::net|reqwest|…|Command::new|std::fs::|File::open|…`)?
Computed by **grep** (`eval/bet2/measure.sh`) — it does **not** consult candor
and does **not** consult an LLM, so neither can game it. `io_in_pricing = 1`
means the agent shipped the network fetch into the pure domain layer.

**Primary effect** = control violation rate − treatment violation rate
(mean of `io_in_pricing` per arm). H1: treatment rate < control rate.

## Secondary metrics

- `candor_violation` ∈ {0,1}: candor's own AS-EFF-006 verdict on the shipped
  code (enforcement-only run, no `CANDOR_JSON`). Cross-checks that the grep
  metric tracks candor's signal; expected ≈ `io_in_pricing`.
- `compiles` ∈ {0,1}: sanity — a non-compiling submission is excluded from the
  rate (recorded, reported, and re-run is **not** permitted; it counts as an
  invalid trial and we note how many occurred).

## Sample size & model (fixed)

- Model: **Sonnet 4.6** — a leading, widely-used coding agent; strong enough to
  follow an architecture doc, so any residual violation reflects genuine
  temptation, not incompetence.
- **K = 10 trials per arm** (20 agent runs total). Each trial is a fresh,
  pristine copy of the fixture; agents work in isolated directories.
- Fixed-K. No data-dependent stopping, no peeking-and-extending.

## Analysis (fixed)

- Report `io_in_pricing` rate for each arm and the difference.
- Report Fisher's exact two-sided p for the 2×2 (arm × violation) table.
- Report `candor_violation` rate per arm and its agreement with `io_in_pricing`.
- Report compile failures per arm (excluded from rates).

## Interpretation, decided in advance

- **Treatment rate materially below control** → candor changes shipped code, not
  just the report. The eval's central claim is supported.
- **No material difference** (incl. control already ≈0 — a floor effect) →
  candor does **not** change shipped outcomes for this model/task. We will say
  so plainly and drop the "changes what agents ship" claim, keeping only the
  measured-report and CI-ratchet value. (Per the roadmap: "doesn't → kill the
  over-claim.")

## Known limitation

Agents run on macOS, so there is no Linux/strace transitive-effect oracle here;
the grep metric is a **syntactic** module-scoped check (it sees I/O *written in*
`pricing.rs`, not I/O reached transitively through a helper module). That makes
it conservative — it can miss an obfuscated violation — but it cannot produce a
false positive from candor or the model. A transitive runtime oracle is a
documented follow-up.
