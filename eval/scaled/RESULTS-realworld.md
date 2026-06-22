# Real-world blast-radius A/B — results (hardened target: git-delta)

Pre-registered in [PREREG-realworld.md](PREREG-realworld.md); target frozen in
[realworld/MANIFEST.md](realworld/MANIFEST.md) + [realworld/GROUND_TRUTH.md](realworld/GROUND_TRUTH.md)
**before** any trial. Target: `git-delta` @ `f85c46b` (30k-LOC single crate), symbol
`utils::process::calling_process` gaining `Exec`; the question is the analysis variant ("which functions
transitively perform `Exec` if `calling_process` does?"). Graded mechanically (pre-registered leaf-name
match) against the independently-adjudicated **61-function** ground truth (60 distinct leaf names — one
leaf, `paint_file_path_with_line_number`, is shared by two modules). Instrument: the deep engine
(`cargo candor` snapshot) + `candor-query callers`. The 8 `lazy_static` init pseudo-nodes candor surfaces
are stripped before scoring (genuine path members below source granularity — see GROUND_TRUTH).

## Why this target (and the ignore pilot it replaced)

The first freeze (ripgrep `ignore`, 6.7k LOC, 32-fn tree) proved the harness but was too easy — sonnet
control reached 97.7% by reading the whole crate (efficiency/reliability win, but completeness didn't
bite; see [realworld/ignore-pilot/NOTE.md](realworld/ignore-pilot/NOTE.md)). delta is large enough that
exhaustive reading is infeasible, so a control agent must grep-and-reason — exactly the regime the cell
exists to test.

## Sonnet tier (N=8/arm)

| arm | mean recall | mean precision | perfect (recall=100%) |
|---|---:|---:|---:|
| **treatment** (candor) | **100.0%** | 100.0% | **8 / 8** |
| **control** (source) | **90.6%** | 98.8% | **1 / 8** |

Per-trial control recall: 77%, 87%, 90%, 92%, 93%, 93%, 93%, 100%. Each treatment agent answered with a
single `candor-query callers` call and returned the exact 61.

**What control missed (systematically).** The transitive tail that isn't reachable by a quick grep for
`calling_process`: the lazy_static-cached `handlers::grep::_emit_classic_format_code` and
`hunk_header::write_to_output_buffer`, `paint::Painter::syntax_highlight_and_paint_line`,
`handlers::hunk::{maybe_raw_line,new_line_state}`, the `subcommands::show_*` entry points, `main`, and (one
agent) the entire `features::side_by_side` panel-painter sub-tree. These are 2–5 call-graph layers from the
symbol — the depth where a manual trace runs out of patience.

**What control got wrong (false positives).** 3 of 8 control agents listed
`handlers::hunk_header::handle_hunk_header_line`, and one added
`handlers::diff_header::handle_diff_header_file_operation_line` — both adjudicated **out** of the ground
truth (they set state / write decorations but reach no target-path function). So control erred in both
directions: incomplete (90.6%) *and* imprecise (98.8%), while candor was exactly complete and exactly
precise.

**Reading.** On the easy crate the gap was efficiency; on this 30k-LOC crate the **completeness** gap
bites — sonnet control drops to 90.6% and is perfect only 1/8, and you cannot tell which ~10% is missing
(or which entries are spurious) without the tool. candor returns the full, correct 61 in one query.

## Cross-tier matrix — IN PROGRESS

Sonnet done (above). opus / haiku / fable pending (N=8/arm each). Expectation (pre-registered hypothesis
2): control completeness falls further at haiku, holds higher at opus/fable; treatment stays ~100% at
every tier (the tool answer is model-invariant — it's one deterministic query). This section will be
completed as the tiers land.

## Candor vs the adjudicated truth (recorded finding)

On both targets candor's deep-engine `callers` set equalled the independently-adjudicated source truth
(ignore 32/32; delta 61/61), matching the stronger of the two independent human-equivalent tracers and
catching callers the other missed — plus the lazy_static granularity detail noted above. So the treatment
ceiling is established by construction; the trials measure how far control falls short of it per tier.
