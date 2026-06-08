# Agent-use eval (stable backend) — Results

Pre-registered in [PREREG-stable.md](PREREG-stable.md). Question: does the **stable** backend
(`candor-scan`, no nightly/dylint) deliver the same behavior-changing signal — and agent outcome — as
the nightly lint did in [Experiment A](RESULTS.md)? Model: the session model (Opus-class), 8 trials/arm.

## Part 1 — Signal-equivalence (the gate): **PASS**

On `eval/agentuse/fixture`, the stable backend reproduces the lint's behavior-changing output **exactly**
(both render through `candor-query`, so the agent can't tell them apart):

```
$ ./candor callers compute_price          # stable backend (CANDOR_BACKEND=scan)
  `pricing::compute_price` is reached by 6 function(s) (the blast radius if it gained an effect):
      invoice::line_item (direct)      invoice::render_invoice      main
      monitoring::health_probe (direct)   report::export_csv      report::monthly_report
$ ./candor where Fs                       # after adding Fs to compute_price → propagated to all 6
```

All **6** ground-truth functions, including `monitoring::health_probe` — the one the fixture is built to
make easy to miss. The treatment agent receives the same input regardless of backend.

## Part 2 — Agent behavior (N=8/arm)

| arm | blast_recall | missed health_probe | pricing_pure (relocated) | used candor |
|---|---|---|---|---|
| control          | **0.833** | 0/8 | 8/8 | 0/8 |
| treatment-stable | **1.000** | 0/8 | 8/8 | **8/8** |

Raw: [results-stable.tsv](results-stable.tsv). Every control agent missed exactly one function — `main`
(the entry point) — while finding the 5 "business" functions including `health_probe`. Every treatment
agent listed all 6: `./candor callers compute_price` / `audit` names `main` explicitly.

### Decision rule (pre-registered) — all three **PASS**

1. treatment recall 1.000 ≥ 0.90 **and** > control 0.833 ✓
2. treatment `pricing_pure` 1.00 ≥ control 1.00 ✓
3. 8/8 treatment agents invoked candor (adoption holds on the stable path) ✓

**Conclusion: the stable backend preserves the value. The friction-killer is not hollow** — adoption is
unchanged (8/8), and candor still closes the completeness gap (1.000 vs 0.833) with the *same* signal the
lint provides.

## Honest caveats

- **Ceiling effect — the lift is smaller than the original.** Experiment A (RESULTS.md, Sonnet-class)
  found control recall ≈ **0.07** and control *shipping the bug*. Here a stronger model traces this small
  4-module fixture by hand, lifting the control floor to 0.833 and getting the *decision* right unaided
  (0/8 missed `health_probe`, 8/8 relocated). So candor's marginal value here narrows to **completeness**
  (the last function, `main`), with **no decision-level separation** on this model+fixture. This is
  consistent with the original program's finding that the decision-level lift emerges with **weaker**
  models (the [A3 Haiku probe](RESULTS-weak.md)) — not a property of the stable backend.
- **This eval isolates the backend, not the model.** It was designed to answer "does stable == lint?",
  and the clean within-arm determinism (all control 0.833, all treatment 1.000) reflects the tiny fixture
  + strong model. The dramatic effect sizes live in the harder/weaker-model variants; what this shows is
  that **whatever value candor delivers, the stable backend delivers it identically** — which is exactly
  what Part 1 proves mechanically and Part 2 confirms behaviorally.
