# Scaled eval — batch 1 results

**Batch:** 3 tasks (minicache/Fs, geoip/Net, renderer/Exec) × 2 conditions × 2 trials = **12 trials**,
pooled **N=6 per arm**. One capable model (Opus-class) as the agent under test; a separate model
(Haiku) as the blind judge. Run with the pre-registered `harness.sh` + protocol in
[README.md](README.md). Raw prompts, summaries, and verdicts are under `runs/`.

**Task completion (objective, candor `diff`):** 12/12 trials implemented the effect — every edited
crate's low-level function gained the expected `Fs`/`Net`/`Exec`, propagating to the 7-function set in
each `GROUND_TRUTH.md`. No trial excluded.

## Primary metric (pre-registered): binary non-local awareness

`yes`=1, `partial`=0.5, `no`=0, blind-judged on the redacted summaries.

| | awareness | breakdown |
|---|---|---|
| **treatment** (candor) | **1.00** | 6 yes |
| **control** (no candor) | **0.83** | 4 yes, 2 partial |

By task (N=2/arm): minicache **1.00 / 1.00**, geoip **1.00 / 1.00**, renderer **0.50 / 1.00**
(control / treatment).

**Against the pre-registered falsification clause, this is a (qualified) falsification.** Control
≥ 0.80 (it's 0.83), and treatment − control = **0.17 < 0.25**, the bar set for "the mechanism
generalises beyond Trial 5's single task." On this binary axis, capable control agents already reach
high non-local awareness, so candor's marginal lift is real but small.

## The binary metric was mis-specified — the completeness gap (exploratory, post-hoc)

The binary rubric scores "named **one** non-local caller" identically to "enumerated **all** of them."
Counting how many of each task's **6 non-local callers** each summary actually names tells a different
story:

| | non-local callers named | |
|---|---|---|
| **treatment** | **5.5 / 6  (92%)** | per-trial 6,5,5,5,6,6 |
| **control** | **2.5 / 6  (42%)** | per-trial 2,2,3,2,3,3 |

A **2.2× gap**. Capable agents on small fixtures notice that *something* propagates (so binary
awareness is high in both arms), but they enumerate it **partially** — typically the one caller a doc
comment flags ("a periodic dashboard… assumes it's cheap"). Candor makes the **complete** propagation
explicit: every treatment summary listed ~all six callers across all four files; no control summary
did. *This is exploratory (not pre-registered) and the count is substring-based, so treat the exact
numbers as indicative — but the separation is large and consistent across all three tasks.*

**The honest reading:** candor's contribution here is **completeness of the propagation, not whether
the agent notices it at all.** The pre-registered binary metric — inherited from Trial 5, where a
weaker separation made it discriminating — is too coarse for a capable model on small fixtures, which
clear the "named ≥1 caller" bar easily. The right primary metric is the completeness fraction; this
batch should be read as pre-registering *that* metric for the next one.

## Cost (objective)

Per-trial averages (6 each): **control ≈ 18.5k output tokens / 12 tool calls / 63 s; treatment ≈ 19.3k
/ 14 / 58 s.** Unlike EVAL.md's *analysis-only* A/B trials (where the candor report was ~4× cheaper
than reading source), in the **edit** setting both arms read and edit source, so candor is **not**
cheaper — it adds ~5% tokens and ~2 tool calls (the `diff` step) for the completeness lift. The
edit-feedback is roughly *free* on top of normal editing, not a saving.

## What the pilot exposed (the value of running it)

1. **Metric mis-specification** (above): binary awareness saturates for capable models; completeness
   is the discriminating axis. Fixed for the next batch by pre-registering completeness.
2. **Fixture leakage:** each fixture's `report` module carries a doc comment ("periodic dashboard…
   assumes it's cheap") that *hands* control agents one specific non-local caller — inflating binary
   control to 0.83 and contributing 1 of their ~2.5 named callers. A cleaner batch needs fixtures
   whose callers are **not** telegraphed by comments (and ideally larger than 5 files, so the call
   graph can't be read in full).
3. **The renderer/Exec task** was the only one to separate on the binary axis (control 0.50 vs 1.00):
   both control agents were pulled to the *security* angle ("arbitrary command execution", "any caller
   with untrusted input") and stayed generic about **which** callers inherit it — while candor kept
   treatment on the propagation axis. Suggestive: candor helps most when another salient concern
   (here, injection) competes for the agent's attention.

## Limitations

- **N=6/arm pooled, N=2/arm per task** — a pilot; the per-task cells are too small for per-task claims.
- **One agent model** (capable). A weaker model would likely widen both metrics' gaps (Trial 5's
  control was 0.63); the ceiling effect on the binary metric is partly a strong-model artifact.
- **Summary-as-proxy** for edit quality — we grade what the agent *reports*, which is what a reviewer
  acts on, but an agent may know more than it writes.
- **Residual blinding hint:** treatment summaries narrate "the analysis confirms…"; the tool *name* is
  redacted but the fact that *some* analysis ran can leak. Mitigated by the mechanical rubric (it
  scores which callers are named, not how they were found), but not eliminated.
- **Completeness is post-hoc** and substring-counted; pre-register and tighten it next.

## Headline

On this batch, **the Trial 5 mechanism reproduces** — every treatment agent made the full propagation
explicit — but the **pre-registered binary metric falsifies the easy "big lift" claim**: capable
control agents on small, comment-leaky fixtures already flag *a* non-local caller (0.83). The honest,
load-bearing result is the **completeness gap: control 42% vs treatment 92% of the propagation set.**
Candor's edit-feedback doesn't make a capable agent *notice* non-local effects so much as make it
report them **completely** — for ~5% extra tokens. The next batch should pre-register completeness as
the primary metric and use fixtures that don't telegraph their callers.
