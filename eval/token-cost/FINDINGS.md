# Does candor save AI tokens or time? — a measurement

The claim "candor saves tokens" was an inference. This makes it falsifiable and reproducible for the one
question where candor's value is real — the **transitive blast radius** (*"who is affected if I add an
effect to X?"*). Run it: `python3 eval/token-cost/measure.py <crate-dir>`.

## The real value is reliable COMPLETENESS, not a token ratio

Lead with this, because it's the load-bearing result (`eval/scaled`): on a blast-radius task whose effect
propagates to 16 functions across 9 files, agents who trace by hand get **~6%** of the radius (1 of 16) —
**even at the frontier** — because enumerating five layers of transitive callers is tedious work they don't
volunteer. candor's `callers`/`diff` hands over the complete set, taking completeness to **79–100%**. The
value is *getting the complete, correct answer at all*; tokens are secondary.

## Token cost, measured honestly (vs a realistic grep-trace)

For the *complete* blast radius, candor is one query; the manual equivalent is a recursive `grep` over every
function in the transitive closure. Across a 10-function sample of `atuin-client`:

| | candor | grep-trace | ratio |
|---|---:|---:|---:|
| typical (distinctive names, small closure) | 20–120 tok | 30–4,000 tok | **~1–30×** |
| common-named / deep closure | 0.1–1K tok | 8K–78K tok | **~75–225×** |
| **median** | | | **~17×** |

So: **single-digit to ~30× for typical functions, more when the closure has common names** — *exactly where
grep is also noisiest and least reliable* (it can't distinguish a real call from a coincidental name match,
so a grep-trace there is both expensive and wrong). candor's cost is name-independent: it has the real call
graph.

> An earlier version of this doc compared candor against reading the **entire crate** (~700–2,000×). That's
> a strawman denominator — no competent agent reads the whole crate; they grep. That number is *information
> compression* (the answer is ~3 orders of magnitude denser than the source), **not** a token-savings claim.
> Pass `--ceiling` to see it, clearly labelled as such.

## Honest bounds — what this does and doesn't show

1. **It's the COMPLETE-answer comparison.** A cheap one-level grep is far less than the grep-trace above —
   but it gets you the ~6% incomplete answer. The honest framing isn't "candor vs grep on equal work"; it's
   "candor is the only **cheap *and* complete** option" (grep = cheap+incomplete; read-all = complete+dear).
2. **It measures information cost, not reasoning.** The behavioural question — does candor change outcomes —
   is `eval/scaled` (6% → 79–100%). This doc adds the token dimension; together they make the case.
3. **It's question-specific.** Blast radius / call-graph traversal is candor's strength. For *"what does this
   one function do,"* reading the function is cheap and candor saves ~nothing. The value is real but narrow.

## Bottom line

candor's durable value is **reliable, complete blast-radius answers** that agents otherwise get ~6% of — at
a token cost typically **~1.5 orders of magnitude** below a manual grep-trace, and most lopsided exactly
where the manual trace is also least reliable. It is *not* a broad token-saver; it wins on graph questions
over non-trivial codebases, where being exhaustive is the whole point.
