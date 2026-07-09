# eval/ — the experiment ledger

Pre-registered experiments and field findings behind candor's claims. Each directory is
self-contained (PREREG → trials → RESULTS where applicable); one line each:

| dir | what it establishes |
|---|---|
| [`agentuse/`](agentuse) | does an agent, given candor's report, answer scoping questions better/cheaper than from source? (control/treatment, weak/stable/hard variants) |
| [`bet2/`](bet2) | the headline gate result: with a pure-boundary policy, the shipped-violation rate went 80% → 0% on locally-simplest edits |
| [`bet3/`](bet3) | workspace-scale enforcement: cross-crate host allowlists (AS-EFF-008) + layering, incl. laundered-through-a-third-crate (CI runs its verify scripts) |
| [`bet4/`](bet4) | findings from the implicit-Drop spike (the drop-glue edge work) |
| [`calibration/`](calibration) | the 35-real-crate scanner calibration: no false positives in library code; under-report classes named |
| [`minicache/`](minicache) | a small fixture crate used by experiments |
| [`realworld/`](realworld) | real-world blast-radius trials on git-delta / ripgrep-ignore (matrix across model tiers) |
| [`scaled/`](scaled) | the scaled agent-task harness (speed/models variants) + runs |
| [`token-cost/`](token-cost) | what the report costs/saves in agent tokens |
| [`unknownwhy-sweep/`](unknownwhy-sweep) | the unknownWhy origin-tag sweep findings |
| [`whatif-behavior/`](whatif-behavior) | does the pre-edit `whatif` verdict change what an agent writes? |

Also here: [`dogfood-pgman.md`](dogfood-pgman.md) — the first real-codebase dogfood notes. The
`bet3` verify scripts are CI gates; everything else is a frozen record — treat RESULTS as
append-only history, not living docs.
