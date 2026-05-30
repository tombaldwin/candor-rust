# Does the effect report actually help an agent? A pilot eval

**Status: two trials, not proof.** Directional, not statistical. The honest headline: **the JSON
makes effect-scoping dramatically cheaper, and it is only as accurate as candor's classifier.** The
original pilot (below) exposed a real classifier false-negative the source agent caught — *fast but
blind*. A re-run after the classifier was hardened (17 gap classes fixed) showed the other half:
*fast and accurate* — candor's answer matched an independent source-reading agent to within ~2%, at
~1/47th the wall-clock. See [Re-run after hardening](#re-run-after-hardening--the-accuracy-loop-closed).

## Design

A real methodological trap: if you grade agents against candor's own JSON as "ground truth," the
JSON-equipped agent is graded on its own input — trivially 100% and meaningless. So this eval does
**not** measure accuracy-vs-candor. It gives two agents the *same* realistic task and measures
**cost** and **completeness**, then adjudicates disagreements by hand against source.

- **Task** (identical to both): "Add retry-with-backoff to every network call and ensure each is
  logged. Report: functions with network I/O transitively; directly; of the direct ones, how many
  have no logging; list the direct ones with file:line; state confidence."
- **Condition A** — agent may read *only* the candor JSON, no source.
- **Condition B** — agent may read *only* the Rust source, no candor / no effect report.
- Target codebase: `ebman` (~8k lines).

## Results

| | A (JSON only) | B (source only) |
|---|---|---|
| Output tokens | **20,146** | 59,371 |
| Tool calls | **3** | 25 |
| Wall time | **55 s** | 359 s |
| Direct-network count | 69 | 69 |
| "No logging" count | 68 | 68 |

**Cost: clear win for the JSON** — ~3× fewer tokens, ~8× fewer tool calls, ~6.5× faster. B had to
build its own call-graph parser in Python and trace chains by hand; A answered from a single file.

**Completeness: a draw that exposed a candor bug.** Both reported 69 direct-network functions and
agreed on the 66-function `AwsClient` core and the near-total logging gap. But the *sets differed*:

- A's 69 included 3 `tokio::net` **Unix-domain sockets** (local IPC in `control.rs`/`cli`) and
  **missed** the HTTP calls in `llm.rs`/`audit.rs`.
- B's 69 **excluded** the Unix sockets (arguing local IPC ≠ network — a defensible call) and
  **included** `llm::call_anthropic`, `llm::call_ollama`, `audit::fire_webhook` — real outbound
  HTTPS via `reqwest`.

Adjudicating against source: **B was right about `reqwest`.** ebman depends on `reqwest 0.12` and
calls the Anthropic API and a webhook — yet candor reported `llm::call_anthropic` as `['Env']`,
`unresolved=false`: confidently *not* network. candor's classifier knew only the AWS SDK and raw
sockets, so it **silently misclassified real HTTP as network-free** — the worst failure mode (false
confidence), and exactly the "curated allowlist" weakness in `CRITIQUE.md`, now demonstrated.

## What the eval changed

The gap is fixed: `reqwest`/`isahc` (`.send()`/`.execute()`) and `ureq` (`.call()`) are now
classified `Net`, matching only the dispatch (not the builder chain), with unit tests. After the fix:

- `llm::call_anthropic` → `[Env, Net]`, `audit::fire_webhook` → `[Log, Net]` (previously network-free).
- Transitive network functions: **279 → 299** — the entire LLM-explain feature and webhook paths
  were invisible before.

## Honest conclusions

1. **Efficiency: supported.** For whole-codebase effect questions the JSON is several times cheaper
   and faster than reading source. This is real and repeatable in principle.
2. **Accuracy: conditional.** The JSON is exactly as correct as the classifier behind it. A
   source-reading agent can *out-perform* the JSON where the classifier has a blind spot — and here
   it did. The mitigation already in candor (`unresolved`/AS-EFF-003) does **not** cover this case:
   the reqwest calls were resolved, just unrecognised, so they weren't even flagged as uncertain.
3. **Not yet shown:** that an agent's *edits* (not just its analysis) improve with the JSON. That
   needs many tasks, multiple trials, and independent ground truth. This pilot only shows the
   artifact is cheap to consume and surfaced — and stress-tested — a real coverage gap.

The most useful thing this eval produced was not a number; it was a bug.

## Re-run after hardening — the accuracy loop closed

The pilot's headline caveat was conclusion #2: *accuracy is conditional — the JSON is only as good
as the classifier, and here the classifier was blind to reqwest.* That kicked off a hardening
campaign that fixed **17 silent-false-negative classes** — reqwest, tokio-postgres, aws-config, log,
git2, legacy tokio sockets, redis, async-nats, lapin, mongodb, mysql, sea_orm, lettre, tungstenite,
elasticsearch, tonic, rdkafka — and ground-truthed the coverage net (it now reports the crates candor
actually saw called, so workspace-member blind spots like git2-in-a-member surface automatically).

Same A/B shape, re-run on `ebman`, with the cleaner checkable question *"how many functions perform
network I/O transitively?"*:

| | A (candor report) | B (source only) |
|---|---|---|
| Answer | **298** Net functions | ~305 (73 direct + 233 transitive) |
| Output tokens | **19,137** | 79,540 |
| Tool calls | **1** | 40 |
| Wall time | **11.9 s** | 562 s (9.4 min) |

**Efficiency: ~4× cheaper, 40× fewer tool calls, ~47× faster** — more lopsided than the pilot,
because the source agent had to trace the whole AWS-driven call graph by hand (the 562 s).

**Accuracy: a genuine agreement, not a disagreement.** candor's 298 matched the source agent's
*independent* estimate of ~305 — within ~2%. The pilot's failure mode (candor confidently wrong) did
not recur; the source agent even confirmed candor's precision calls (it noted the
`aws_config::SdkConfig::builder()` stub paths do *no* network and should be excluded — exactly what
candor's `::load`-only rule does). The residual ~7-function gap is **definitional, not a candor
error**: the source agent counted two functions that shell out to `curl` as "network", while candor
classifies those as `Exec` (a subprocess) — both defensible.

So the loop the pilot opened is closed: **pilot = fast but blind; re-run = fast *and* accurate.** The
JSON's accuracy tracks the classifier, and the classifier is now broad enough that, on a real
AWS/HTTP app, an independent source-reading agent agrees with it to within noise — at a fraction of
the cost. Conclusion #3 (that an agent's *edits* improve, not just its analysis) remains unshown and
would need a multi-task, multi-trial study.
