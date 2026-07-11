# Pre-registration: cross-model speed/token A/B (the blast-radius ANALYSIS question)

**Committed before any trial runs.** Extends [PREREG-speed.md](PREREG-speed.md) / RESULTS-speed.md (Opus-only:
control 30.0s/15.8k tok, treatment 16.5s/10.5k tok, 1.81× faster, both 16/16) across the tier range, mirroring
the completeness eval ([RESULTS-xmodel.md](RESULTS-xmodel.md)).

## Methodology UPDATE (unblocks this eval)

Prior note held the speed A/B was NOT runnable via subagents — the Agent tool exposed final text but not
per-agent telemetry. **That is no longer true:** this session's Agent completion notifications carry
`<usage><subagent_tokens>…</subagent_tokens><duration_ms>…</duration_ms></usage>` per agent. So tokens and
duration ARE capturable via subagents.

- **TOKENS** (`subagent_tokens`) and **RECALL** (the returned function list vs the 16-fn set) are CLEAN —
  each agent's own usage, unaffected by concurrency. These are the primary, rigorous metrics.
- **WALL-CLOCK** (`duration_ms`) is reported as SECONDARY and caveated: trials run concurrently, so absolute
  durations are inflated by shared-throughput contention vs the prior serial human run. Arms are balanced
  (equal N/arm at each tier), so the treatment/control RATIO is a conservative directional estimate (if the
  bottleneck is tokens/sec, fewer-token treatment still finishes proportionally sooner). A clean serial
  wall-clock remains a separate follow-up if the ratio is close.

## Design

- Task = the PREREG-speed blast-radius question (verbatim), read-only (no editing) → control & treatment each
  use ONE shared de-leaked `orderflow` source dir (`speed-xmodel/control|treatment`); treatment's has a fresh
  `.candor/report` + callgraph.
- Models: `opus`, `sonnet`, `haiku`, `fable`. **N = 5/arm/model** (40 trials). Same prompt, differing only in
  the tool clause: control = "Work from the source code"; treatment = "use `candor-query callers|whatif`".
- Metrics: median `subagent_tokens` and median `duration_ms` per cell, ratio control/treatment; recall/16.

## Falsification bars

1. **Token claim**: refuted for a tier if median(treatment tokens) ≥ median(control tokens).
2. **Recall floor**: treatment recall ≥ control recall at every tier (candor must not cost completeness).
3. **Consistency**: the token ratio > 1 at every tier (the saving isn't frontier-only). Reported per tier.
