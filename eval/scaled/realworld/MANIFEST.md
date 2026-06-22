# Frozen target — real-world blast-radius A/B (HARDENED)

Committed before the trial matrix runs (the pre-trial freeze required by
[../PREREG-realworld.md](../PREREG-realworld.md)). The first target (ripgrep `ignore`) proved the
harness but was too easy — sonnet control reached 97.7% by reading the whole 6.7k-LOC crate (see
[ignore-pilot/NOTE.md](ignore-pilot/NOTE.md)). This hardened target is a 30k-LOC crate whose call graph
exceeds comfortable reading.

| field | value |
|---|---|
| **repo** | `dandavison/delta` (`https://github.com/dandavison/delta`) — `git-delta`, a git-diff syntax highlighter |
| **commit** | `f85c46b` |
| **scope** | the whole `delta` binary crate (~30k LOC, single crate, ~30 modules under `src/`) |
| **symbol** | `utils::process::calling_process` (`pub fn calling_process()` in `src/utils/process.rs`) |
| **effect probed** | `Exec` — natural framing: *delta inspects its parent process to adapt rendering; `calling_process` currently returns a **cached** value (the real `sysinfo` inspection runs once in a background thread). If it queried the OS for the parent process on each call, it would perform process-inspection I/O.* (Genuinely pure now — candor correctly reports it pure — so the "gains an effect" framing is clean.) |
| **instrument** | deep engine (`cargo candor`, nightly rustc/MIR) — see the prereg amendment |
| **ground truth** | the adjudicated **61-function** set in [GROUND_TRUTH.md](GROUND_TRUTH.md) / [delta-groundtruth.txt](delta-groundtruth.txt) |

## Why this target (against the selection rules)

1. **Real, widely-used, un-seen** (not in candor's calibration corpus). ✓
2. **Exceeds comfortable context.** 30k LOC / ~30 modules; the symbol's caller tree is **61 source
   functions** spanning `handlers/*` (≈12 files), `paint`, `features/{line_numbers,side_by_side}`,
   `subcommands/*`, `utils/path`, `config`, `delta`, `main` — through 4-6 call-graph layers (incl. the
   `StateMachine` trait dispatch hub `delta::StateMachine::consume`). Not enumerable by eye; even a
   strong unlimited-effort source tracer landed 58/61 with 3 false positives (see GROUND_TRUTH §log). ✓
3. **Deep engine analyzes it cleanly** — 1030 nodes / 1311 edges; the symbol's tree resolves with the
   only granularity caveat being the `lazy_static` init pseudo-nodes (noted in GROUND_TRUTH). ✓
4. **Un-leaky names** — ordinary domain names; call structure not telegraphed. ✓

## Reproduce

```sh
git clone https://github.com/dandavison/delta checkout && git -C checkout checkout f85c46b
( cd checkout && cargo candor snapshot ../work/baseline )        # nightly deep engine
candor-query callers ../work/baseline utils::process::calling_process 1
```
