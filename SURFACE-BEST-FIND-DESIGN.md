# candor scan — surface the best find (the cold-repo hook)

_Design doc. Status: **scoped**, from the 2026-07-12 top-of-funnel work. Makes the two-minute cold-repo demo
deterministic instead of lucky._

## Why

The top-of-funnel for a skeptical, busy, IDE-less senior dev lives or dies on the FIRST run surfacing
something true and non-obvious about *their* code — without them knowing which question to ask. Pressure-
tested cold on `atuin` (neither of us had scanned it): `scan` (40 ms) → `where Net` → `path
settings::Settings::needs_update Net` surfaced a **`Settings` method that phones home**
(`needs_update → latest_version → api_client::latest_version` @ `src/api_client.rs:142`). Real, verifiable,
the kind of thing a senior dev files away. But it needed the user to ask the right question. **This feature
makes `scan` END by handing them the single most surprising reach + the exact follow-up command** — so the
opener is guaranteed, not dependent on a lucky query.

## What

After the effect summary and the κ ledger, candor-scan emits ONE more line — the most surprising transitive
reach, with a ready-to-run `candor path` command:

```
candor: most surprising reach — `settings::Settings::needs_update` performs Net, 3 hops away via
        `api_client::latest_version` (src/api_client.rs:142). A "settings" method that reaches the network.
        See the chain:   candor path settings::Settings::needs_update Net
```

If nothing clears the bar: `candor: nothing hidden — every effect sits where its name says it should.`
(An honest, useful result — and it protects the "never wastes your time" promise. Never manufacture a surprise.)

## The heuristic — what "surprising" means (computable, deterministic, no LLM)

A candidate is a function that **INHERITS** an effect transitively (a *direct* source is obvious — excluded).
Score each; emit the top one.

`score = salience(effect) · benignity(name) · crossing · hops · liveness`

- **salience(effect):** Net / Exec / Db / Ipc = high (the boundary/security-relevant effects a reviewer
  cares about); Fs / Env = medium; Clock / Log / Rand = low.
- **benignity(name):** the function's leaf name and module read as *local/pure/config* — `settings`, `config`,
  `util`, `helper`, `model`, `dto`, `format`, `parse`, `get`, `load`, `new`, `default`, `validate`, `render`,
  `view`, `build`, `item`, `entry`, `record` … → HIGH. An effect-suggestive name — `fetch`, `http`, `client`,
  `api`, `sync`, `request`, `download`, `upload`, `query`, `exec`, `spawn`, `db`, `store`, `save`, `connect` …
  → ~0. **This is the core surprise signal: a benign name reaching a scary effect.**
- **crossing:** the effect SOURCE lives in a different module than the function → the reach crosses a
  boundary (more hidden). Bonus per module boundary the chain crosses.
- **hops:** distance to the source. 0 = direct (excluded). 2–4 = the sweet spot (hidden but real).
  >6 = damped (too deep reads as noise, not insight).
- **liveness:** reachable from an entry point / public API → bonus (it's live, not dead code).

**Anti-noise + soundness:** exclude direct sources, Unknown-only reaches (disclosed separately), functions
already IN an effect-named module reaching that effect (`api_client::* → Net` is obvious), and test code.
Fully deterministic — pure call-graph + name analysis, so the same repo yields the same opener every time.
The find is never *wrong*: `path` re-derives the chain and the gate is ground truth; worst case is a slightly
less-interesting but TRUE reach, never a false claim.

## Where it lives

candor-scan emits it at scan time — it already holds the report + callgraph in memory and already prints the
κ ledger and the provable-purity note, so this is the same seam. Plus a `candor tour <report>` query
(candor-query) for the top-N on demand without re-scanning. Cross-engine later: the report + callgraph is the
shared §2 contract, so the identical heuristic rides in candor-java/ts/swift — but candor-scan first, because
the frictionless cold-repo funnel is Rust-first (no build, no deps).

## Phasing

- **P1** — the scan-time note (top-1): the name-benignity lexicon + the score + the ready-to-run command +
  the honest fallback. candor-scan.
- **P2** — `candor tour` (top-N, on demand; the guided-poke experience).
- **P3** — cross-engine parity (java/ts/swift) + a conformance pin so the opener is consistent per engine.

The product point this closes: the two-minute demo becomes **deterministic** — every cold repo gets handed
its single most surprising reach, so the skeptic converts himself on his own code instead of needing to guess
the right question.
