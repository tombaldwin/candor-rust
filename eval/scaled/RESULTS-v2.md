# Scaled eval — batch 2 results

**Batch:** 3 tasks (minicache/Fs, geoip/Net, renderer/Exec) × 2 arms × 3 trials = 18 trials, on the
**de-leaked `tasks-v2/` fixtures**, agent under test = **Sonnet** (batch 1 was Opus-class), blind
Haiku judge. Pre-registered in [README.md](README.md) "Batch 2" (committed before the run).
**Completeness is the primary metric**; binary awareness is secondary.

**Exclusion (recorded, not dropped):** `renderer-treatment-2` — the trial agent's own safety
classifier blocked the `std::process::Command` edit (the Exec task) and asked for permission instead
of completing it. candor's objective check confirms no effect was introduced. Excluded from both
metrics per the protocol. Valid N: **control 9, treatment 8.** Other 17 trials implemented the effect
(objective candor `diff`).

## A contamination bug was found and fixed mid-batch — then the whole batch was re-run clean

The first batch-2 run was **invalid**: the harness copied each task's `GROUND_TRUTH.md` (which lists
the entire propagation set + the effect) into the agent's working directory. One treatment agent
quoted a phrase that exists *only* in that file, proving agents could read the answer key. The harness
now copies **only `Cargo.toml` + `src/`** into the work dir; the task reaches the agent through the
prompt alone. **Every batch-2 trial below was re-run from scratch on clean copies.** The same bug was
present in batch 1 — see the caveat at the end.

## Primary metric: completeness (blind judge marks each of the 6 non-local callers)

| | completeness | per-arm |
|---|---|---|
| **treatment** (candor) | **6.00 / 6 = 100%** | every valid trial named all 6 |
| **control** (no candor) | **0.44 / 6 = 7%** | 0,2,0 / 0,2,0 / 0,0,0 |

By task (control / treatment): geoip **11% / 100%**, minicache **11% / 100%**, renderer **0% / 100%**.

**Neither pre-registered falsification condition triggered.** Control completeness is **7%**, far below
the 0.80 "low marginal value" bar; treatment − control is **0.93**, far above the 0.20 "weak effect"
bar. On de-leaked fixtures with a weaker model, candor's edit-feedback is **decisive** for reporting
the full propagation.

**Objective cross-check (mechanical substring count):** control **20%**, treatment **90%**. It brackets
the judge result from the other side — it *over*-counts control (it credits a function named as
"unaffected") and *under*-counts treatment (it misses blanket "all callers in api and report" phrasing
that the judge correctly credits). Both metrics agree on a large gap; the judge (semantic) is the
pre-registered primary.

## Secondary metric: binary awareness

Control **0.167** (0 yes, 3 partial, 6 no) vs treatment **1.00** (8 yes). Unlike batch 1 — where the
binary metric *saturated* (control 0.83) — here it discriminates sharply, because the confounds that
inflated control are gone: de-leaked fixtures give the control agent nothing to read off, and Sonnet
doesn't trace the call graph unprompted (it typically concludes "signature unchanged → callers
unaffected").

## The headline: batch 1 *understated* candor's value; the clean number is large

| | control completeness | control binary |
|---|---|---|
| Batch 1 (Opus, leaky comments, **+ answer-key contamination**) | 42% (post-hoc) | 0.83 |
| **Batch 2 (Sonnet, de-leaked, clean)** | **7%** | **0.17** |

Removing the three confounds — the `GROUND_TRUTH.md` leak, the telegraphing doc comments, and the
frontier model — collapses control from "already mostly aware" to **~7% complete**, while treatment
holds at **100%**. So the batch-1 falsification ("capable control already flags a caller, candor's lift
is small") was an **artifact of those confounds**. On a realistic setup, candor's edit-feedback takes
an agent from naming ~0 of the non-local callers to naming all of them.

What's happening, concretely: a Sonnet agent that adds disk/network/subprocess to a leaf function and
is asked for "consequences for the rest of the codebase" overwhelmingly writes "the public signature
is unchanged, so callers are unaffected" — true for *compilation*, wrong for *effects*. `cargo candor
diff` hands it the exact list of functions that gained the effect, and it reports them. The control had
equal license to investigate callers; it mostly didn't.

## Cost

Per valid trial (Sonnet): control ≈ 16.3k output tokens / ~9 tool calls; treatment ≈ 16.9k / ~11 (the
`diff` step). As in batch 1, candor adds ~5% in the edit setting (both arms read+edit source) — the
completeness lift is essentially free.

## Limitations

- **N=9 control / 8 treatment** (one task lost a treatment trial to the agent's safety block) — a
  pilot-scale batch; tighter than batch 1 but still small per task.
- **One agent model** (Sonnet). Batch 1 (Opus) and batch 2 (Sonnet) bracket the model axis: the gap is
  *larger* for the weaker model, as predicted. A frontier model on a *large, un-leaky* codebase is the
  untested cell (batch 1 conflated frontier-model with leaky-fixtures-and-contamination).
- **The treatment is told to run candor** (its intended use), so part of the lift is "we pointed it at
  the propagation." The honest control comparison: the control was asked the same "consequences for the
  codebase" question and had equal license to trace callers; 7% completeness is what it did with it.
- **Fixtures are still 5 files.** De-leaked, but small; a larger codebase (where even a careful agent
  can't read everything) is batch 3.
- **Residual blinding hint** (treatment narrates "the analysis confirms…"); the tool name is redacted
  and the judge applies a mechanical per-function rubric, but tool use can still leak. The objective
  mechanical cross-check is immune to this and agrees.
- **Summary-as-proxy** for edit quality, as before.

## Headline

On de-leaked fixtures with a weaker, more realistic agent model, and with the batch-1 contamination
fixed, **candor's edit-feedback lifts non-local effect-completeness from 7% to 100%** (binary awareness
0.17 → 1.00), for ~5% extra tokens. Batch 1's "small lift" was an artifact of a leaked answer key,
telegraphing comments, and a frontier model; the clean, pre-registered result is a large and consistent
effect across all three tasks. Next: a *large* fixture and a frontier model, to map the one cell these
two batches leave open.
