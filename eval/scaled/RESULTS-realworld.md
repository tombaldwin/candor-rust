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

## Cross-tier matrix (complete — N=8/arm/tier, 64 trials)

| tier | control recall | control precision | control perfect | treatment recall | treatment perfect |
|---|---:|---:|---:|---:|---:|
| **haiku** 4.5 | 60.4% | 83.5% | 0 / 8 | **100%** | **8 / 8** |
| **sonnet** 4.6 | 90.6% | 98.8% | 1 / 8 | **100%** | **8 / 8** |
| **opus** 4.8 | 97.3% | 100.0% | 4 / 8 | **100%** | **8 / 8** |
| **fable** 5 | 99.0% | 100.0% | 5 / 8 | **100%** | **8 / 8** |

**The pre-registered gradient holds, cleanly and monotonically.** Control recall tracks model capability
— 60% → 91% → 97% → 99% — and so does the perfect-answer rate (0 → 1 → 4 → 5 of 8). Treatment is **flat at
100% recall, 100% precision, 8-of-8 perfect at every tier**: the tool's answer is model-invariant because
it is one deterministic `candor-query callers` call that every model copied faithfully (haiku included).

What this means for each end of the curve:
- **Weak model (haiku):** without candor it is *both* badly incomplete (60% recall, 0/8 perfect) *and*
  badly imprecise — one agent carpet-bombed **235 false positives** (precision 17.8%), i.e. listed most of
  the crate. With candor it is perfect. Here candor substitutes for capability: it is the difference
  between an unusable answer and an exact one.
- **Frontier model (fable):** even at 99% recall it is perfect only 5/8 — it still drops the occasional
  deep transitive edge (the lazy_static-cached emitters, a side-by-side painter) and you cannot tell which
  trial is the incomplete one. candor closes that last gap deterministically.

So candor adds value at **every** tier on a real 30k-LOC crate — most at the cheap end (where it rescues
both completeness and precision), and still measurably at the frontier (the last few percent + guaranteed
completeness). Treatment cost is ~constant (one query, ~24–28k tokens); control cost rises with capability
as stronger models do more tracing work to approach — but never reach — the tool's answer.

## Second target — bottom (Fs), the cross-target check

A second real-world point to tighten the single-target estimate, varying repo + effect + architecture:
`ClementTsang/bottom` @ `b3694fc` (a 37k-LOC TUI system monitor), symbol
`app::data::store::DataStore::get_data` gaining `Fs` (natural framing: a cached store that could read
live OS data), graded against an independently-adjudicated **26-function** tree (see
[realworld/bottom/](realworld/bottom/)). Same 4-tier N=8 matrix.

| tier | control recall | ctl precision | ctl perfect | treatment recall | tr perfect |
|---|---:|---:|---:|---:|---:|
| **haiku** | 81.7% | 97.3% | 2 / 8 | 99.0% | 6 / 8 |
| **sonnet** | 99.5% | 100% | 7 / 8 | 100% | 8 / 8 |
| **opus** | 99.5% | 100% | 7 / 8 | 100% | 8 / 8 |
| **fable** | 100% | 100% | 8 / 8 | 100% | 8 / 8 |

### What the two targets together show

Both crates are large (delta 30k, bottom 37k), but the **shape of the blast radius differs**, and that —
not crate size — is what governs the gap:

- **delta / `calling_process`** — a **61-fn, ~6-layer** tree threaded through `StateMachine` dispatch and
  a lazy_static cache; **not** greppable. Control falls at *every* tier: **60 / 91 / 97 / 99%**.
- **bottom / `get_data`** — a **26-fn** tree with **16 direct callers** (every widget `draw_*` pulls the
  store) — easy to grep and trace. Control is high except at the weak tier: **82 / 99.5 / 99.5 / 100%**.

So candor's marginal value scales with **how hard the radius is to trace by hand** (depth × non-grep-
ability), not raw LOC. On a deep tangled radius it bites at every tier; on a shallow greppable one it
concentrates at the cheap models. **In both, treatment is ~model-invariant and ~complete** (100% at every
tier except haiku-on-bottom 99%, where a couple of weak agents fumbled copying the query output) — one
deterministic `candor-query` call. And in both, candor's own report equalled the independently-adjudicated
truth (delta 61/61, bottom 26/26), confirming the treatment ceiling on two real codebases.

**Engine note:** the bottom run used the deep engine on the `nightly-2026-06-14` port (branch); delta was
re-verified byte-identical (61/61) on that engine. bottom's snapshot exits non-zero from a cosmetic rustc
shutdown delayed-bug, but the report is written and complete (see realworld/bottom/MANIFEST.md) — a
separate candor robustness item, not an analysis gap.

## Honest bounds

Two repos, two symbols, four tiers, N=8/arm. The graded GT is human-adjudicated reverse-reachability (two
independent source-only tracers + source resolution of every disagreement; see GROUND_TRUTH). Grading is
leaf-name match (pre-registered); the 8 lazy_static init pseudo-nodes candor surfaces are stripped before
scoring (genuine path members below source granularity). The treatment arm was *told* to run
`candor-query callers` — part of the lift is "we pointed it at the tool" — but the control arm had equal
license to trace callers and, below the frontier, systematically didn't (or did so incompletely). A second
repo/symbol would tighten the estimate; this is one strong real-world point, with the toy-fixture asterisk
removed.

## Candor vs the adjudicated truth (recorded finding)

On both targets candor's deep-engine `callers` set equalled the independently-adjudicated source truth
(ignore 32/32; delta 61/61), matching the stronger of the two independent human-equivalent tracers and
catching callers the other missed — plus the lazy_static granularity detail noted above. So the treatment
ceiling is established by construction; the trials measure how far control falls short of it per tier.
