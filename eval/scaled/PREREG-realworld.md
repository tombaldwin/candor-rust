# Pre-registration: the blast-radius A/B on a REAL, large codebase (the un-leaky open cell)

**To be committed before any trial agent runs**, and before the target repo/symbol/ground-truth are
frozen (see "Freeze" below — that lands in its own pre-trial commit). Extends
[PREREG-speed.md](PREREG-speed.md) and [PREREG-speed-models.md](PREREG-speed-models.md): those
established, across Fable 5 / Opus / Sonnet (and Haiku in the agent-use track), that candor's answer is
model-invariant while manual tracing degrades as the model cheapens. Every one of those batches carries
the **same honest bound**: the fixture is small (≤16-function radius, 9–10 modules), single-screen, and
distinctively named — so "control can't trace it" is shown only on a toy. This batch removes that
asterisk.

## Question

On a **real, large, unfamiliar** Rust codebase — one whose call graph does not fit in comfortable
context and whose names were *not* authored to telegraph the call structure — does candor still deliver
the complete transitive blast radius faster and more completely than an agent working from source, and
how does that gap move across model tiers?

This is the regime the tool is *for*: the small-fixture batches showed frontier controls hit the
ceiling (they can trace 16 functions by hand). The claim under test is that on a real graph the ceiling
drops for **every** tier, widening candor's margin.

## The ground-truth problem (the methodological crux)

The small fixtures are graded against a candor-computed propagation set that is *independently
hand-verifiable* because they are tiny. On a real codebase that escape hatch is gone, and grading
treatment against candor's own output would be circular (the treatment agent's input becomes the answer
key). Ground truth here is therefore established **independently of candor**, by the same method used to
adjudicate the `ebman`/`mcfly` trials in [EVAL.md](../EVAL.md), made exhaustive:

- The graded set is the **exhaustive reverse-reachability** of the chosen symbol — every transitive
  caller — established by **grep-recurse over the symbol + hand-reading each call site**, NOT by candor.
  Tracing *one* symbol's caller tree is tractable to verify exhaustively (grep the name, recurse,
  read each hit) even in a large repo; what is *hard* — and what the agent is tested on — is doing it
  reliably and completely under context pressure across an unfamiliar codebase.
- It is **cross-checked by two independent strong-model source-only tracers** (no candor). Every
  disagreement between the hand trace, the two tracers, and candor's `callers` output is resolved
  **against source by hand** and the resolution recorded. Candor's set is reported alongside but is
  **not** the answer key; where candor and the adjudicated set differ, that difference is a finding
  (candor bug or independent-tracer miss), logged either way.
- The frozen `GROUND_TRUTH.md` (symbol, the adjudicated propagation set, and the candor-vs-truth diff)
  is committed **before any trial agent runs**.

If the adjudicated set cannot be made stable (persistent unresolved disagreement on >2 functions), the
symbol is rejected and the next candidate is chosen by the rule below — recorded, not hidden.

## Target selection (rules fixed now; the choice frozen pre-trial)

**Repo** — chosen by these rules, in order, first that qualifies:

1. A real, widely-used, **pure-safe-Rust** project **not in candor's calibration corpus** (excludes
   `ebman`, `mcfly`, and any crate added as a calibration dep — see EVAL.md). Confirmed un-seen.
2. **Large enough that the graph exceeds comfortable context**: ≥ ~8k source lines and ≥ ~30 modules,
   so an agent cannot hold the whole call graph at once.
3. **candor-scan analyzes it cleanly** — it produces a report with a *low* unresolved-source rate on
   the subtree of interest (a repo that is mostly `Unknown` would test the classifier, not the
   blast-radius thesis; disqualify and fall through).
4. Names are ordinary domain names, **not** authored to telegraph callers (the de-leak requirement that
   batch-2 added).

**Primary candidate: `ripgrep`** (multi-crate workspace — also exercises the cross-crate boundary that
`mcfly` exposed and the DefPathHash fix closed). **Backups, in order: `fd`, `bat`.** Whichever first
satisfies the four rules with a stable adjudicated ground truth is frozen; the rejects and why are
recorded in `RESULTS-realworld.md`.

**Symbol** — a deep, well-connected function with a **wide** reverse-reachability set (target ≥ ~25
transitive callers across ≥ 5 files — materially larger than the 16-function toy), reached through ≥ 4
call-graph layers. The effect probed is whichever is natural for that symbol (`Net`/`Fs`/`Db`/`Exec`);
the **analysis** framing is used (PREREG-speed style — "if `X` gained effect `E`, which functions
transitively perform `E`?"), so no edit needs to compile in the real repo and ground truth is a pure
reachability question.

## Design

Both arms get a fresh checkout of the frozen repo at the frozen commit, with any `.candor/` and this
eval's files removed; the **treatment** copy additionally gets a fresh `candor-scan` report + callgraph
sidecar in `.candor/`.

**Models:** `fable` / `opus` / `sonnet` / `haiku` — the agent harness's four tiers (the exact ids it
resolves recorded in RESULTS). This completes the 4×4 matrix the prior batches left at three.
**N = 8 per arm per tier** (64 trials), batched 8-concurrent with arms balanced within each batch
(4C + 4T, same tier), one shot per trial, single rerun only on a harness error (agent returns nothing),
noted. Tooling under test: the **published `candor-scan`** (version recorded in RESULTS) + the repo
`candor-query`.

**Identical prompt**, differing ONLY in the tool clause (verbatim shape from PREREG-speed.md):

> You are analyzing the Rust project at `<DIR>`. Question: if the function `<SYMBOL>` gained the
> `<EFFECT>` effect (it starts performing `<EFFECT>` I/O), which OTHER functions in this project would
> transitively perform `<EFFECT>` as a result — i.e. every transitive caller of `<SYMBOL>`? Be
> exhaustive: list EVERY affected function across the whole project. Return ONLY a list of function
> names (module::Type::method or module::function), one per line, no commentary.

- **control** — the prompt + "Work from the source code."
- **treatment** — the prompt + "candor is set up: an effect report and call-graph sidecar are in
  `<DIR>/.candor/`, and the query binary `<candor-query>` answers e.g.
  `candor-query callers <DIR>/.candor/report <fn>` (transitive callers) or
  `candor-query whatif <DIR>/.candor/report <fn> <EFFECT>` (the blast radius). Use them."

## Metrics (priority order)

1. **Completeness** (primary) — recall against the adjudicated ground-truth set (leaf-name match,
   `main` counts), per trial; per-arm-per-tier mean. The load-bearing statistic batch-2 settled on.
2. **Precision** — also reported this time (real graphs make over-listing possible in a way the toy
   did not): functions named that are *not* in the adjudicated set, hand-checked against source. A fast
   *complete-but-bloated* answer is a different failure than a complete one.
3. **Wall-clock** — `duration_ms` per trial; median(control)/median(treatment) within each tier.
4. **Tokens** — `subagent_tokens` per trial; same ratio.

## Pre-registered hypotheses & falsification bars

1. **Treatment completeness ≥ control at every tier, and the gap is larger than on the toy.**
   *Refuted for a tier* if control mean completeness ≥ treatment there. *Premise weakened* (and
   reported as such) if **control** mean completeness ≥ 80% at the frontier tiers — i.e. a strong model
   traces a real graph fine and the "exceeds context" premise didn't bite.
2. **The tier gradient holds**: control completeness falls as the tier cheapens; treatment stays high
   and roughly flat (tool answer model-invariant). Refuted if treatment completeness itself degrades
   materially across tiers (would mean weaker models can't *operate* the query — itself a finding).
3. **Treatment is faster** within each tier (median treatment wall-clock < control). Reported as
   *trivial* if faster but less complete/precise.
4. **Precision is not sacrificed**: treatment precision ≥ control precision. If treatment is complete
   but materially *less* precise (candor over-listing folded into the agent's answer), that is the
   honest cost and is reported, with the over-listed functions root-caused (classifier imprecision vs
   call-graph over-approximation, à la the EVAL.md CHA finding).

## Analysis & honesty rules

Per arm-per-tier: mean completeness + precision, median + min/max for duration and tokens, per-trial
raw numbers committed (as the prior batches do). Candor's own set is reported next to the adjudicated
truth so any candor miss/over-list on a *real* graph is visible — the most useful output of every prior
trial was a bug, and a real codebase is the likeliest place to find the next one. No exclusions beyond
the single-rerun harness-error rule. The cross-day caveat applies to any comparison with earlier
batches; within-this-batch, within-tier comparisons are the clean ones. Results in
`RESULTS-realworld.md`.

## Limitations (stated in advance)

One repo, one symbol, four tiers, K=8/arm/tier. It removes the "toy fixture" asterisk but is still a
single real-world point, not a survey; a second repo/symbol would be the natural follow-up. Ground
truth is human-adjudicated reverse-reachability — as good as the adjudication, which is why every
disagreement is logged against source rather than waved through. Whatever the batch shows is the
result; the target is frozen before trials and not retuned toward a desired outcome.
