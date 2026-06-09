# Does candor save AI tokens? — a measurement

The claim "candor saves tokens" was, until now, an inference. This makes it falsifiable and reproducible
for the one question where candor's value is real: the **transitive blast radius** — *"who is affected if I
add an effect to function X?"* Run it yourself: `python3 eval/token-cost/measure.py <crate-dir>`.

## What is measured

For a sample of functions in a real crate, the **information cost** of two ways to get the *complete*
blast radius:

- **candor** — the tokens of `candor-query callers <fn>` (one query → the full transitive caller set).
- **manual ceiling** — the tokens of the crate's source, i.e. what an agent reads to trace the same answer
  exhaustively by hand. (Callers are spread across the whole crate; to be *complete* you must read it.)

Token estimate = chars/4 (model-agnostic; the ratio is stable under any fixed tokenizer).

## Result (5 real crates)

| crate | source (tokens) | candor answer (tokens, avg) | compression |
|---|---:|---:|---:|
| fd | 63,000 | 74 | ~850× |
| gitoxide-core | 99,000 | 50 | ~1,970× |
| atuin-client | 104,000 | 149 | ~700× |
| helix-term | 312,000 | 218 | ~1,430× |
| zellij-utils | 696,000 | 371 | ~1,875× |

candor answers the complete blast-radius question in **~50–370 tokens** where reading the source for the
same complete answer is **~60K–700K tokens** — **roughly 700×–2000×** less, and the ratio *grows* with
codebase size (the answer stays small; the source doesn't).

## Honest caveats — what this does and doesn't show

1. **It's the COMPLETE-answer comparison.** The manual figure is the cost of being *exhaustive*. A cheap
   `grep` is far less — but `eval/scaled` shows that agents who don't pay the full cost get **~6%** of the
   blast radius (1 of 16), *even at the frontier*. So the real choice isn't "candor vs a cheap grep"; it's
   "candor's complete answer for ~0.2K tokens" vs "an incomplete grep, or ~100K tokens to be exhaustive."
   candor makes *completeness* cheap.

2. **It measures information cost, not reasoning tokens.** The *behavioral* question — does candor change
   outcomes — is answered separately in `eval/scaled` (completeness 6% → 79–100%) and `eval/agentuse`
   (10/10 adoption). This doc adds the missing token dimension; together they make the case.

3. **It is question-specific.** Blast radius / call-graph traversal is candor's strength. For *"what does
   this one function do,"* reading the function is cheap and candor saves ~nothing. The value is real but
   **narrow** — it shows up on graph questions over non-trivial codebases, not on local reading.

## Bottom line

For the blast-radius question on a real codebase, candor delivers the **complete** answer at **~2–3 orders
of magnitude** lower token cost than reading the source for it — *and* the complete answer is one agents
otherwise skip (6%). So: yes, it saves tokens — but specifically by making *exhaustive graph answers* cheap,
not by helping with code an agent can just read.
