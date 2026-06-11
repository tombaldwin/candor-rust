# Prove it on *your* repo — a 15-minute self-experiment

Our [evals](EVAL.md) show agents miss most of an effect's blast radius on *our* fixtures
(~6% completeness untooled on the hard case; [measured](eval/token-cost/FINDINGS.md), pre-registered,
[red-teamed](eval/scaled/RESULTS-speed.md)). You shouldn't care about our fixtures. This is the same
A/B, run by **your** agent on **your** codebase, with every claimed result verifiable by you at a
file:line. Either outcome is informative — including "candor didn't help here" (the prompt reports
that too, and says why).

**Requirements:** a Rust crate, `cargo install candor-scan` (stable toolchain, ~15s build), any
agentic coding tool (Claude Code, Cursor, …). JVM project (Java/Kotlin/Scala/Groovy)? Use the
[JVM variant of this prompt](https://github.com/tombaldwin/candor-java/blob/main/PROVE-IT.md).

**Paste this prompt into your agent at the repo root:**

---

```text
We're testing whether a static effect-analysis tool (candor) tells me things about MY codebase that
you'd otherwise miss or take longer to find. Follow these steps IN ORDER — the order is the
experiment's integrity (your manual answer must be committed before the tool's answer exists).

STEP 1 — Pick the target. Choose ONE function in this crate's PRODUCTION code (under src/ — not
tests/, examples/, benches/, or a `#[cfg(test)]` module, all of which the scan deliberately
excludes as harness code) that performs
I/O (network, filesystem, database, subprocess) and is called from more than one place — ideally one
I care about changing. If I named a function in my message, use that. State your choice.

STEP 2 — MANUAL TRACE (commit before looking at any tool output). From source alone, answer:
"Which functions in this crate would be affected if <target> changed its behavior — i.e. every
TRANSITIVE caller, across all files and call-graph layers?" Work as you normally would (grep, read).
Write the complete list to /tmp/candor-manual.txt — one function per line, named module-relative
the way a callgraph would key it (`module::Type::method`). Also note
roughly how many file-reads/searches it took you.

STEP 3 — Run candor. `cargo install candor-scan` if not present (version 0.3.5 or later — earlier
published versions have known resolution bugs fixed since), then run `candor-scan .` — it writes
.candor/report.<crate>.scan.json (per-function transitive effects) and
.candor/report.<crate>.scan.callgraph.json (every function's direct callees, pure ones included).

STEP 4 — Compute the tool's answer from the callgraph sidecar (it's plain JSON — no magic):
write a ~10-line script that loads the callgraph, builds the reverse edge map, and BFS's from the
target to collect every transitive caller. Save to /tmp/candor-tool.txt.

STEP 5 — Diff and VERIFY. Compare the two lists.
- For each function candor found that your manual trace MISSED: reconstruct the call chain from the
  callgraph (target ← caller ← caller …) and open ONE file per missed function to confirm the call
  site is real, quoting the file:line. These are real, verifiable edges — not tool assertions.
- For each function YOU listed that candor did not: check whether it's a real caller candor missed
  (candor-scan is deliberately conservative — it under-reports rather than fabricates; a miss here
  is worth reporting at https://github.com/tombaldwin/candor-rust/issues) or a mistake in the trace.
  Write both lists in the callgraph's own naming (crate-relative, typestate-erased — check a few of
  its keys first) so the diff compares like with like.

STEP 6 — Scorecard. Report, honestly:
- target function, and the size of its true radius
- manual trace: N of M found, the specific functions missed (with the verified file:line evidence),
  and the file-reads/searches it took
- candor: one scan + one script, complete set
- AND if the result is unflattering to candor, say so plainly: if your manual trace found everything
  (shallow radius, distinctive names — common in small crates), the honest conclusion is "on this
  codebase candor's value is speed/CI-gating, not completeness." If candor missed real callers
  (the documented blind spots — trait-object and Deref-coercion dispatch, generic-parameter fields,
  macro-generated calls — are listed in crates/candor-scan/README "Misses"), report that as
  candor's limitation.

Do not soften either direction. The point is what's true on THIS repo.
```

---

## Why this is a fair test

- **The commitment device** (manual answer written before the tool runs) is the same protocol as our
  pre-registered evals — the agent can't retrofit its trace to the tool's answer.
- **No circular trust:** every function the manual trace missed comes with a call chain whose every
  edge is a real call site in *your* code, opened and quoted. You check the evidence, not the tool.
- **The negative result is in-scope.** On small crates with distinctive names, a good agent traces
  completely by hand ([we measured exactly this](eval/scaled/RESULTS-speed.md) — and the speedup was
  still ~1.8×). Radii get missed where codebases get real: deep closures, common names, many files.
  If your repo is the former, the prompt says so instead of manufacturing a win.

## What we measured on our side (so you can compare)

| claim | measurement |
|---|---|
| completeness, implicit tracing (edit tasks) | untooled ~6% of a 16-fn radius → 79–100% with candor ([scaled eval](eval/scaled/RESULTS-v3.md)) |
| explicit blast-radius question, frontier agent | equal completeness, **~1.8× faster** (p≈.04), [red-teamed](eval/scaled/RESULTS-speed.md) |
| information cost of the complete answer | median **~17×** fewer tokens than a grep-trace ([measured](eval/token-cost/FINDINGS.md)) |

If your scorecard disagrees with these in either direction, we'd genuinely like to see it —
[open an issue](https://github.com/tombaldwin/candor-rust/issues) with the scorecard.
