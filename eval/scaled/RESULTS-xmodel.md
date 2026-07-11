# Cross-model completeness — full run (opus / sonnet / haiku / fable)

Pre-registered in [PREREG-xmodel-completeness.md](PREREG-xmodel-completeness.md) (+ its pilot amendment).
**4 models × 2 arms × 5 trials = 40 engineers**, each verbatim `## Summary` scored by **3 blind Haiku judges**
returning structured `{namedYes, blanket}` (120 judges). Orchestrated by a Workflow (160 agents, **2.65M
tokens**, ~8 min). Task: `orderflow` — one edit makes `Pricing::quote` gain **Net**, propagating to 16
non-local functions across 9 files / 5 layers.

- **Objective check:** all **40/40** trials COMPLETED (`harness.sh verify`: the edit introduced Net). No
  exclusions.
- **Judges:** 15/cell as designed, except opus-treatment 14 (one judge returned no structured output — that
  trial is meaned over its 2 valid judges; noted per the prereg).
- **Two metrics** (Tom's call — the judge reports facts, the orchestrator computes both, no judge discretion):
  **STRICT** = functions the summary names explicitly; **LENIENT** = 16 if the summary asserts the effect
  reaches the whole caller chain up to `main`, else = STRICT.

## LENIENT — "does the engineer convey the effect reaches the whole chain?" (mean /16)

| model  | control | treatment | lift  |
|--------|---------|-----------|-------|
| opus   | 4.47    | **15.20** | +10.73 |
| sonnet | 1.07    | **14.27** | +13.20 |
| haiku  | 0.00    | **14.00** | +14.00 |
| fable  | 6.33    | **15.00** | +8.67 |

## STRICT — "does the engineer NAME each of the 16?" (mean /16)

| model  | control | treatment | lift  |
|--------|---------|-----------|-------|
| opus   | 1.47    | 3.63      | +2.17 |
| sonnet | 1.07    | 4.40      | +3.33 |
| haiku  | 0.00    | 1.20      | +1.20 |
| fable  | 1.27    | 3.07      | +1.80 |

## Hypotheses

Evaluated on both metrics (LENIENT is the natural reviewer reading; STRICT is the harsh enumeration bar).

1. **Treatment > control at every tier.** ✅ **HELD on BOTH metrics** — every lift is positive at every tier
   (LENIENT +8.7…+14.0; STRICT +1.2…+3.3).
2. **The lift is LARGEST for weaker models** (tool carries the weak). ✅ **HELD under LENIENT** — haiku lift
   **+14.0** ≥ opus **+10.7** (the weakest model gains the most; fable's smaller +8.7 is because *fable's
   control* was the strongest at 6.3, not because treatment dipped). ❌ Under STRICT the ordering inverts
   (haiku smallest, +1.2) — weak models, even handed candor's output, don't transcribe it into per-function
   names.
3. **Treatment is high and tier-flat (~14–16).** ✅ **HELD under LENIENT** — treatment **14.0–15.2 at every
   tier**, essentially flat from Haiku to Fable 5. ❌ Under STRICT treatment is low (1.2–4.4): candor does
   **not** drive per-function enumeration.
4. **Control low everywhere; no frontier recovery.** ✅ **HELD** — control 0.0–6.3, every tier below the 8/16
   bar; the frontier (opus 4.5 LENIENT) does not close the gap unaided.

## Reading

**Under the natural metric (LENIENT), candor's completeness lift is large, consistent, and tier-flat across
the entire capability range — treatment ≈ 14–15/16 whether the engineer is Haiku 4.5 or Fable 5, while
control stays low at every tier. The tool carries completeness *most* for the weakest model** (haiku control
0 → treatment 14). This is the value-proposition claim confirmed across four tiers, not just the frontier:
the bottleneck is not capability, it's that no model volunteers 5 layers of transitive enumeration unaided —
and candor supplies exactly that, so the engineer states "the effect reaches the whole chain up to `main`"
regardless of tier.

**The STRICT metric is the honest caveat:** candor makes the engineer assert the *complete-chain claim*, it
does NOT make them list all 16 functions by name (treatment names ~2–4). If a downstream use needs the
explicit per-function set (e.g. auto-generating a policy allowlist), that comes from candor's *output itself*
(the `diff`/`callers` query), not from the engineer's prose summary. The value is the tool's map, surfaced;
the summary conveys its shape.

Raw per-trial scores + summaries: the Workflow result (recorded in this commit's run). Judge protocol +
metric definitions: the prereg. Cost: 2.65M tokens / 160 agents / ~8 min wall.
