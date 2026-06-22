# Ground truth — `git-delta`, `utils::process::calling_process` (Exec)

**Established INDEPENDENTLY of candor** and frozen before the trial matrix. See
[../PREREG-realworld.md](../PREREG-realworld.md) §"The ground-truth problem". The machine-readable list
the harness grades against is [delta-groundtruth.txt](delta-groundtruth.txt) (61 functions).

- **Effect gained:** `Exec`. **Probed function:** `utils::process::calling_process` — genuinely pure
  today (it reads a cached `Lazy` static `CALLER`; the real `sysinfo` parent-process inspection runs
  once in a background thread). Natural gain: querying the OS for the parent process on each call.
- **Question:** if `calling_process` performed process-inspection I/O, which OTHER functions in the
  crate would transitively perform it — i.e. the complete set of transitive callers.

## Method (anti-circularity)

Two independent strong-model **source-only** tracers (no candor, no call-graph tool) each computed the
reverse-reachability; their sets were diffed against each other **and** against candor's
`callers` output, and **every** disagreement was resolved against source. The graded set is what the
adjudication established — **independently** grounded: tracer **B** produced exactly this 61-set on its
own, and tracer **A**'s 6 differences were all resolved at the source (below). candor's result is then
reported as a *finding* against this set, not used to define it.

## The adjudicated set — 61 source functions

See [delta-groundtruth.txt](delta-groundtruth.txt). Spans (by file): `utils/path` (2), `handlers/grep`
(8), `handlers/hunk` (5), `handlers/hunk_header` (4), `handlers/diff_header` (5), plus
`diff_header_diff`/`diff_header_misc`/`diff_stat`/`commit_meta`/`submodule`/`blame`/`git_show_file`/
`merge_conflict` handlers, `paint` (7), `features/line_numbers` (3), `features/side_by_side` (5),
`config::from`, `delta::consume`+`delta::delta`, `subcommands/*` (4), `run_app`, `main`. Convergence hub:
`delta::StateMachine::consume` (dispatches ~12 `handle_*` methods) → `delta::delta` → `run_app` → `main`.

## Adjudication log (the 6 disagreements + the granularity caveat)

Resolved against source:
- **`get_filename`** — `match &*process::calling_process()` → **direct caller, INCLUDE**. (Tracer A
  missed it; candor + B had it.)
- **`paint_left_panel_minus_line`, `paint_right_panel_plus_line`** — both call
  `paint_minus_or_plus_panel_line`, which calls `Painter::paint_line` (a confirmed target-reacher) →
  **INCLUDE** both. (Tracer A missed both; candor + B had them.)
- **`handle_diff_header_file_operation_line`, `should_write_generic_diff_header_header_line`** — the
  latter only calls `write_generic_diff_header_header_line`, which writes via `painter.writer` /
  `draw_fn` and reaches **no** target-path function; the former's only candidate edge is that call →
  **EXCLUDE** both. (Tracer A over-included; candor + B excluded.)
- **`handle_hunk_header_line`** — parses the header and sets `self.state`; no target-reaching call →
  **EXCLUDE**. (Tracer A over-included; candor + B excluded.)

Net: tracer **B** = the 61-set exactly; tracer **A** = 58/61 (missed 3, over-included 3) — the
completeness gap a strong, unbounded source tracer still shows on a 30k-LOC crate.

## Candor's result, recorded as a finding (NOT the key)

`candor-query callers <deep-report> utils::process::calling_process 1` returned **all 61** source
functions — **recall 61/61** against the adjudicated truth, matching tracer B exactly and catching the 3
callers tracer A missed. candor additionally surfaces **8 `lazy_static` init pseudo-nodes**
(`<OUTPUT_CONFIG as Deref>::deref::__static_ref_initialize`, the `CACHED_IS_WORD_DIFF` equivalents,
etc.). These are **genuine** members of the call path — the lazy_static initialization really does call
`make_output_config` / `compute_is_word_diff`, which reach the target — but they are macro-generated and
below the source-function granularity a developer (or the agents) would list, so they are excluded from
the graded set. They are a transparency/granularity detail, not a false positive: candor sees *more* of
the real call graph than a hand trace, not less. So on both the easy (ignore, 32/32) and hard (delta,
61/61) targets, candor's deep-engine blast radius equals the independently-adjudicated source truth; the
treatment ceiling is established and the trials measure the control gap across model tiers.
