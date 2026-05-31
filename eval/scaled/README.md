# Scaled edit-quality eval — multi-task, pre-registered

**Status: protocol + harness, pre-registered.** This extends [EVAL.md](../../EVAL.md) Trial 5 — which
showed, on **one** task (N=4/arm), that candor's edit-feedback turns *partial/local* awareness of an
edit's effect consequence into *complete, specific, non-local* awareness — to **multiple tasks** with
a **reproducible harness**. Trial 5's own limitation was the gate: *"N=4/arm, one task, one model… a
multi-task study would tighten the estimate."* This is that study's design and harness.

It is **pre-registered**: this file and the fixtures + ground truth are committed *before* any batch
runs (see git history), so the metric and falsification clause can't be retrofitted to the result.

## The question (unchanged from Trial 5)

When an agent makes an edit with a **non-local effect consequence** — the edited function gains an
effect (`Fs`/`Net`/`Exec`/…) that then propagates transitively to callers in other files — does
giving the agent candor's edit-feedback (`cargo candor diff`) make it **notice and report the full
propagation**, which capable agents otherwise tend to under-report?

This is the P0 thesis (candor changes what the agent *does*, not just how fast it analyses). It is
**not** "is candor's report accurate" (EVAL.md's earlier trials covered that, conditionally).

## Design

Each **task** is a small multi-file Rust crate where one natural edit makes a low-level function gain
a single effect that propagates to a known set of caller functions across files. The propagation set
is **deterministic ground truth**, verified by actually making the edit and running candor (see each
task's `GROUND_TRUTH.md`).

Two **conditions**, each agent on a fresh copy of the fixture, identical prompt except the candor
clause:

- **control** — the task only.
- **treatment** — the task **+** "candor is set up; after editing, run `cargo candor diff
  .candor/baseline` and address what it reports."

**Trials:** N per arm per task (the first batch reports its N explicitly; the harness takes N as a
parameter).

## Metrics

**Primary — non-local effect-awareness** (blind-judged, mechanical rubric). Does the agent's final
summary identify that callers **beyond** the edited function now perform the new effect?

- **yes** — names ≥1 specific non-local caller that gains the effect (e.g. `report::build`,
  `api::get_one`), **or** correctly states the full set / "all callers now perform X".
- **partial** — notes a generic consequence ("adds blocking I/O on every call", "performance
  impact", "callers are affected") **without** naming a specific non-local caller or the set.
- **no** — describes only the local change; no propagation awareness.

Awareness score: yes=1, partial=0.5, no=0. Reported per arm, per task, and pooled.

**Secondary — task completion** (objective, automatable): does candor's own `diff` on the agent's
edited copy show the edited function gained the expected effect? An agent that didn't implement the
task is excluded from the primary metric (recorded separately, not silently dropped).

**Secondary — cost** (objective): output tokens, tool calls, wall time per arm — re-confirming the
consumption-cost axis at scale.

## Blinding

The judge receives **only** the agent's final summary text with the condition redacted, plus the
task's ground-truth propagation set and the rubric. It returns `yes`/`partial`/`no` + a one-line
justification quoting the summary. The condition↔summary mapping is revealed only after all judgements
are in. One judge call per summary; the judge never sees which arm produced a summary, nor the other
arm's output.

## Falsification clause (committed before the run)

If **control** agents already reliably score `yes` (pooled control awareness ≥ 0.8), candor's marginal
value on this axis is low, and we report that as the headline. Trial 5 found control ≈ 0.63 on one
task; this batch tests whether that holds across tasks or was task-specific.

Equally, if **treatment** does *not* clear control by a clear margin (pooled treatment − control ≥
0.25), the mechanism does not generalise beyond Trial 5's single task, and we say so.

## Tasks (the first batch)

| Task | Crate | Effect gained | Edited fn | Natural impl (std-only, no deps) |
|---|---|---|---|---|
| `minicache` | TTL cache (5 files) | `Fs` (read) | `Cache::get` | `std::fs::read_to_string` on a miss |
| `geoip`     | geo-IP lookup (5 files) | `Net` | `Resolver::resolve` | `std::net::TcpStream::connect` on a miss |
| `renderer`  | template engine (5 files) | `Exec` | `Engine::expand` | `std::process::Command` for an `{{exec:…}}` token |

All effects are **std-only** so a fixture compiles fast and offline and candor classifies them
deterministically. Each task's `GROUND_TRUTH.md` lists the exact functions that gain the effect, and
was produced by making the canonical edit and running `cargo candor diff` — not by hand.

`minicache` is Trial 5's fixture, reused verbatim so this batch is comparable to it.

## Running it

`harness.sh` is the reproducible runner. It does **not** itself call an LLM (that's the one
non-scriptable part); it prepares each trial's fresh fixture copy + the exact prompt, and provides the
judge prompt and the scoring aggregation. See `harness.sh --help`. The first batch's prompts, raw
summaries, judgements, and the condition mapping are recorded under `runs/` and summarised in
`RESULTS.md`.

## Honesty constraints carried from EVAL.md

- Ground truth is **independent of candor** (it's what the source actually does post-edit; candor is
  only used to *enumerate* it mechanically, and the enumeration is human-checkable against the call
  graph in each fixture).
- The judge is blind to condition and scores ONE axis on a fixed rubric.
- The treatment is *told to run candor* — so part of any lift is "we pointed it at the propagation".
  The control has equal license to investigate callers; whether it does is exactly what's measured.
- Summary-as-proxy-for-edit-quality is a known limitation (an agent might know more than it writes);
  we measure what it *reports*, which is what a human reviewer would act on.
