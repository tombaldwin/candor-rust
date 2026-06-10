# Pre-registration: does candor's token saving correspond to a SPEED increase?

**Committed before any trial agent runs** (see git history). Motivated by a direct question
("we talked about a ~17× token reduction — does that correspond to a speed increase?"). The token
claim is `eval/token-cost` (median ~17× vs a grep-trace, for the complete blast radius); the
completeness claim is `eval/scaled` batch 3 (control ~6% of the radius, treatment 79–100%). This
measures the missing axis: **wall-clock**.

## Question

On the blast-radius ANALYSIS question (not an edit task), does an agent with candor's report answer
**faster** than an agent working from source alone — and at what completeness?

## Design

Fixture: `tasks-v3/orderflow` (the batch-3 large fixture; 16 non-local propagated functions across
9 files, 3–5 call-graph layers; candor-verified ground truth in its `GROUND_TRUTH.md`). Both arms get
a fresh copy with `GROUND_TRUTH.md`/`TASK.md`/`target`/`.candor` removed; the treatment copy then gets
a fresh `candor-scan` report + callgraph sidecar.

**N = 8 per arm.** Agent under test: this session's default subagent model (Opus-class), identical
for both arms; one shot per trial, no retries except a harness error (agent returns nothing — rerun
once and note it).

**Identical prompt** (verbatim below), differing ONLY in the tool clause.

> You are analyzing the Rust crate at `<DIR>`. Question: if the function `pricing::Pricing::quote`
> gained the `Net` effect (it starts performing network I/O), which OTHER functions in this crate
> would transitively perform `Net` as a result — i.e. every transitive caller of `Pricing::quote`?
> Be exhaustive: list EVERY affected function across the whole crate. Return ONLY a list of
> function names (module::Type::method or module::function), one per line, no commentary.

- **control** — the prompt + "Work from the source code."
- **treatment** — the prompt + "candor is set up: an effect report and call-graph sidecar are in
  `<DIR>/.candor/`, and the query binary `/Users/tom/git/candor-rust/target/debug/candor-query` answers
  e.g. `candor-query callers <DIR>/.candor/report <fn>` (transitive callers) or
  `candor-query whatif <DIR>/.candor/report <fn> Net` (the blast radius). Use them."

## Metrics (in priority order)

1. **Wall-clock** — `duration_ms` reported by the agent harness per trial. Primary statistic:
   **median(control) / median(treatment)**.
2. **Tokens** — `subagent_tokens` per trial; same ratio.
3. **Completeness** — recall against the 16-function non-local ground-truth set (leaf-name match,
   `main` counts; the edited `quote` itself and the agent's inclusion/exclusion of `quote_bulk`'s
   own row are not penalized in either direction beyond the 16-set).

## Falsification bars

- **Speed claim refuted** if median(treatment wall-clock) ≥ median(control wall-clock).
- **Speed claim trivial** (and to be reported as such) if treatment is faster but its median
  completeness < control's — a fast wrong answer is not a win.
- The batch-3 completeness pattern (control low, treatment near-complete) is expected to replicate;
  if control reaches ≥80% median completeness here, the "agents don't volunteer the full trace"
  premise weakens and must be reported.

## Analysis

Per-arm: median + min/max for duration and tokens; per-trial completeness. No exclusions besides the
single-rerun harness-error rule. Results in `RESULTS-speed.md`; raw per-trial numbers committed.
