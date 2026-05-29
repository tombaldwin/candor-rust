# Does the effect report actually help an agent? A pilot eval

**Status: pilot, not proof.** Single trial, one task, small battery, one orchestrator. Treat the
numbers as directional. The honest headline: **the JSON makes effect-scoping dramatically cheaper,
but it is only as accurate as candor's classifier — and this very eval exposed a real classifier
false-negative that the source-reading agent caught.**

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
