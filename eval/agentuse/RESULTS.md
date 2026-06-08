# Agent-use eval (Experiment A) — Results

See `PREREG.md` (committed before the run). Question: when candor is **available but not instructed**,
do agents reach for it, and does using it improve the outcome? Sonnet 4.6, K=10/arm.

## Headline

| metric | control | treatment |
|---|---|---|
| **used candor** (adoption) | n/a | **10 / 10** |
| placement correct (`pricing` stays pure) | 10/10 | 10/10 |
| blast-radius recall (of 6) | 0.87 | 0.88 |
| missed the dangerous `health_probe` | 0/10 | 0/10 |
| compiles | 10/10 | 10/10 |

Two clear results, one expected, one not.

## 1. Adoption is total — and competent

**Every treatment agent (10/10) reached for candor**, unprompted (the task never mentions it; only
`AGENTS.md` notes it exists). And they chose the *right* queries for a blast-radius question: all 10 ran
`candor audit`, all 10 ran `candor callers compute_price`, 6 also ran `candor map`. So the answer to
"do agents reach for candor, and do they pick sensible commands?" is an unambiguous **yes**. The
active-tool framing (MCP / slash command / `cargo candor`) is validated on the adoption axis: given an
effect-tracing task and candor in the toolbox, agents use it, first, every time.

## 2. …but candor didn't actually answer their question

The outcome was **identical across arms** — both got placement right 10/10 and neither missed
`health_probe`. That isn't just a ceiling (Sonnet can trace this 6-function graph by hand). The deeper
reason, which only watching agents *use* candor surfaces:

> On the still-**pure** fixture — exactly the state when an agent asks *"who would be affected if I add
> an effect here?"* — candor's queries return nothing useful:
> - `candor audit` → "no effectful functions found (everything candor can see is pure)."
> - `candor callers compute_price` → **"callers of pure functions aren't tracked."**

The treatment agents ran `callers compute_price` — the *perfect* query for the blast radius — and hit a
dead end. candor only tracks callers of *already-effectful* functions; it has no answer for "what would
propagate if I introduce an effect." So the agents reached for the right tool with the right command,
got nothing, and recovered the blast radius by reading source manually (which is why their answers match
control). candor was *consulted* 10/10 times and *informative* ~0 times.

This is the actionable finding. It isn't a soundness bug — it's a **workflow gap**: candor's headline
value is the *consequence of an edit* (the `diff`/`audit` after you change something), but agents
naturally ask the *pre-edit, what-if* question — "I'm about to touch `X`; who depends on it?" — and
`callers <fn>` should answer that for **any** function, effectful or not, because "who calls X" is a
structural question independent of X's current effects.

## Interpretation (per the pre-registration)

The registered reading for "high adoption, no outcome difference" was: *agents use candor but it doesn't
change results here — the task is too easy or they misread the output.* The truth is a sharper third
option the prereg didn't enumerate: **agents used candor correctly, but candor's current query surface
doesn't serve the pre-edit blast-radius question** (`callers` is gated to effectful functions). Combined
with the prior evals — where candor's output, *when it answered the question* (the post-edit diff on a
harder 16-function task, `eval/scaled`), took completeness from 6% to 79–100% — the synthesis is:

- **Agents reliably reach for candor** (this eval: 10/10).
- **When candor answers the question, it helps a lot** (`eval/scaled`, hard task, handed diff).
- **But its query surface has a gap at the most natural entry point** — "who calls this pure function I'm
  about to make effectful" — and closing that (`callers`/blast-radius on any function) is the
  highest-leverage way to make the active-tool experience actually pay off, not just get invoked.

## Limitations

Single model, single task, K=10. The fixture is within Sonnet's manual call-graph-tracing ability, so it
can't measure outcome *lift* from candor — but it wasn't able to anyway, because candor didn't answer the
pre-edit question. The adoption result (10/10, right commands) is the robust finding; the workflow gap is
the actionable one. Next: close the gap (make `callers`/blast-radius work on pure functions), then re-run
on a harder fixture where manual tracing fails — to measure the lift candor *could* deliver once it
actually answers the question agents ask.
