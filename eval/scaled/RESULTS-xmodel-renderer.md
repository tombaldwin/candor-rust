# Cross-model completeness — 2nd codebase (renderer / Exec) — GENERALIZATION TEST

Pre-registered in [PREREG-xmodel-renderer.md](PREREG-xmodel-renderer.md). Same protocol as
[RESULTS-xmodel.md](RESULTS-xmodel.md) (orderflow), **only the fixture changes**: `renderer` — one edit adds
an `{{exec:CMD}}` directive so `Engine::expand` gains **Exec**, propagating to **6** non-local functions
(vs orderflow's Net / **16**). 4 models × 2 arms × 5 trials = **40 engineers** (all 40 objectively COMPLETED
— the edit introduced Exec), each summary scored by **3 blind Haiku judges** (120 judges). Dual metric, judge
reports facts, orchestrator computes STRICT (named) / LENIENT (blanket→6).

## Result — mean over 3 judges × 5 trials (/6)

| model  | STRICT ctl | tmt  | lift  | LENIENT ctl | tmt  | lift  |
|--------|-----------|------|-------|-------------|------|-------|
| opus   | 3.07      | 5.20 | +2.13 | 3.07        | 5.67 | +2.60 |
| sonnet | 3.13      | 5.20 | +2.07 | 3.13        | 5.27 | +2.13 |
| haiku  | 4.60      | 4.33 | −0.27 | 4.93        | 4.60 | **−0.33** |
| fable  | 3.80      | 6.00 | +2.20 | 3.80        | 6.00 | +2.20 |

## The finding: the lift does NOT blanket-generalize — it is a function of the enumeration BURDEN

Pre-registered hypotheses vs the orderflow run:

- **H1 (treatment > control at every tier): FALSIFIED** — haiku control (4.93) ≥ treatment (4.60).
- **H2 (lift largest for weaker models): FALSIFIED** — haiku has the SMALLEST lift (−0.33), the *opposite* of
  orderflow (where haiku had the LARGEST lift, +14).
- **H3 (treatment tier-flat): HOLD.**
- **H4 (control low at every tier): FALSIFIED** — haiku control = 4.93/6 (82%); on orderflow it was 0.0/16.
- **H5 (generalization): the qualitative pattern does NOT replicate on the small fixture.**

This is the mechanism from orderflow's hypothesis 2, **confirmed by refutation**: candor's completeness lift is
not a universal property — it scales with how TEDIOUS the transitive enumeration is.

- **orderflow (16-fn chain, 5 layers):** control 0–6/16, treatment ~14–15/16, lift **+9 to +14**, LARGEST for
  weak models — the chain is long enough that models won't volunteer the enumeration, so the tool carries them.
- **renderer (6-fn chain):** the chain is short enough that models — *including haiku* (4.93/6 unaided) — mostly
  volunteer it, so the lift shrinks to **+2** (opus/sonnet/fable) or **vanishes/inverts** (haiku −0.33).

The stronger models (opus/sonnet/fable) still gain ~+2 on renderer because they are *conservative* in control
(3.1–3.8 — they name the local change and stop), and the tool pushes them to name the full short chain. Only
haiku, which is verbose-and-enumerative on an easy task, is already near-complete unaided.

## What this means for the value proposition (honest scope)

candor's edit-completeness value is **real and large where it is hard** — deep, multi-layer transitive
propagation, which is exactly where real architecture violations hide in big codebases — and **small where it
is easy** (short chains a model handles anyway). This is a *defensible refinement*, not a retraction: the tool
earns its keep on the hard graph problems, not the trivial ones. A pitch that claims a universal completeness
lift is not supported; a pitch that claims candor makes agents complete on the *tedious transitive* cases they
otherwise botch IS supported (orderflow), and this run maps the boundary. The single-fixture orderflow headline
was burden-inflated; the two-fixture picture is the honest one.

Runs under `runs-xmodel-renderer/`; 120 judge records aggregated deterministically. Judge model = Haiku, blind.
