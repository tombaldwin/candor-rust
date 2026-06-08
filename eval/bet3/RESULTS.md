# Bet 3 — Enforce effect boundaries at a scale nobody holds in their head

Bet 2 (exp3) pinned candor's irreplaceable value: a boundary crossed by a **non-local consequence of
a local edit** — invisible to the model, missed by file-level review. Bet 3 leans all the way into
that: turn it into a real **architecture-as-code** enforcement layer, and make it hold at **workspace
scale**, where the offending code lives in a different crate the editor never opens.

## What shipped

**1. Literal-allowlist enforcement (AS-EFF-008).** A new policy directive, over the three effects that
carry a literal surface:

```
allow Net  [in <scope>] <host>...     # billing may only talk to Stripe
allow Exec [in <scope>] <cmd>...      # build may only run git
allow Fs   [in <scope>] <path>...     # config may only read /etc/app
```

"In `<scope>`, that effect may reach ONLY these values." A function in scope that reaches any other —
or whose value candor can't see at all (a fully dynamic one) — is flagged. Checked against the
**transitive** literal surface, matched per effect (host by name, command by basename, path by prefix).
This is the supply-chain boundary a model structurally cannot self-check: editing `billing`, it has no
way to know which host a helper three frames down actually `connect`s to. candor already tracked
transitive Net hosts; Bet 3 adds command/path tracking and turns all three into enforced boundaries.

The semantics deliberately certify the **visible host surface**: a function that reaches a known,
allowed host but also makes an `Unknown` call is *not* flagged by AS-EFF-008 (that residual risk is
AS-EFF-003/006's job) — folding `Unknown` in would fire on essentially every real network function
(they all do dynamic-dispatch `write!`), making the allowlist unusable.

**1b. Module-layering rules (AS-EFF-009).** The dependency-direction boundary, complementing the effect
rules:

```
forbid <A> -> <B>
```

"A function in scope `A` must not transitively call into scope `B`" — *the domain layer must not reach
into infra, even through a chain of helpers.* This is the one check that reads the **call graph**, not
the effect lattice: a layer can be forbidden from *depending on* another even when neither performs an
effect. It's computed by a forward **reach** over the call graph, seeded from each function's direct
callees whose path matches a target scope (local *and* cross-crate) — so it catches three cases in one
mechanism: within-crate layers, a direct dependency on a whole sibling **crate** (`forbid app -> infra`),
and a dependency **laundered through a third crate** (`app -> util -> infra`). The laundered case works
because, under the workspace gate, each crate writes a `layerreach` sidecar — which target scopes each of
its functions reaches — and dependent crates (linted later, dependency-first) read it; so `util`'s
sidecar tells `app` that `util::store` reaches `infra`. (Computing reach during *util*'s own lint uses
its full local call graph, so pure intermediates are handled too.) Together the three rule kinds make
`CANDOR_POLICY` a real architecture-as-code layer: `deny`/`pure` (*what* a layer does), `allow
Net/Exec/Fs` (*which* values), `forbid ->` (*who* it depends on).

**2. Cross-crate host propagation + workspace-scale enforcement.** Two gaps blocked enforcing any
boundary across a workspace:

- Sibling reports carried only *effects* cross-crate, not *hosts*. Now `load_cross_reports` also
  threads each sibling function's `hosts` (keyed by `DefPathHash`), and a cross-crate call seeds the
  caller's host set — so within-crate propagation carries an endpoint that physically lives in another
  crate up to every transitive caller.
- Cross-resolution only fired under `CANDOR_JSON` (which *suppresses* enforcement) or `CANDOR_BASELINE`
  (the guard). Added **`CANDOR_REPORTS`** — a read-only cross-resolution prefix usable in *enforcement*
  modes. Workflow: snapshot every workspace crate once, then enforce with the siblings loaded.

**3. One-command workspace gate.** `cargo candor policy` now does the whole dance itself: it snapshots
every crate's report, then enforces the policy with the siblings loaded read-only (`CANDOR_REPORTS`) —
so cross-crate boundaries hold with a single invocation, no manual two-pass. It gates on AS-EFF-006,
**008**, and **009**.

## The teeth (`eval/bet3/host-allowlist/`, `verify.sh`)

A two-crate workspace. The forbidden host literal lives in a **shared `httpkit` crate**; the `billing`
module (in crate `app`) names no host at all — it just calls `httpkit::stripe_charge` and
`httpkit::track_event`. Policy: `allow Net in billing api.stripe.com hooks.stripe.com`.

| billing fn          | reaches (transitively, cross-crate) | on allowlist? | candor verdict |
|---------------------|-------------------------------------|---------------|----------------|
| `charge_customer`   | `api.stripe.com:443` (in httpkit)   | yes           | **clean**      |
| `record_activity`   | `metrics.growthtracker.io:443` (in httpkit) | no    | **AS-EFF-008** |

Verified with teeth (`verify.sh`, in CI): **without** cross-crate resolution (drop `CANDOR_REPORTS`),
`billing`'s calls into `httpkit` don't resolve, the forbidden host is invisible, and **AS-EFF-008 never
fires** — the violation is silently missed. The cross-crate machinery is exactly what closes that gap.
The script also drives the **single-command** path (`cargo candor policy`) and asserts it blocks (exit 1)
on the same cross-crate violation. This is the case a developer (or their agent) cannot catch by reading
`billing.rs`: the endpoint isn't there.

## Tests

- Unit: `host_allowlist_parses` (the `allow Net [in <scope>] …` grammar, unsupported-effect rejection,
  hostname-vs-port matching); `layering_rule_parses` (the `forbid <A> -> <B>` grammar, malformed-rule
  rejection); `load_cross_reports_filters_and_maps` extended to assert a sibling's host crosses the
  boundary into the cross host map.
- Integration (`tests/integration.sh`): §9a — AS-EFF-008 flags the off-allowlist host reached
  transitively through a helper, not the allowed-host path; §9b — AS-EFF-009 flags the forbidden
  cross-layer dependency reached transitively, not a sibling that doesn't reach it. (70/70 pass.)
- Soundness harness: unchanged and green (20/20) — the enforcement additions don't touch inference.
- End-to-end teeth (all in CI): `eval/bet3/verify.sh` (host allowlist cross-crate + single-command
  gate), `eval/bet3/verify-layering.sh` (direct cross-crate `forbid` on a sibling crate), and
  `eval/bet3/verify-laundered.sh` (a three-crate `app -> util -> infra` dependency caught through util's
  `layerreach` sidecar) — each also asserting a pure function is *not* flagged.

## Why this is the right Bet 3

It races *away* from what models are getting better at (local call-graph tracing) and toward what they
structurally cannot do: know, from a local edit, that a transitive — possibly cross-crate — call
reaches an un-sanctioned endpoint. A model advises; only a tool, holding the whole-workspace effect
surface, can *block the PR*. That is the repositioning: from "help your agent understand effects" to
"enforce effect boundaries at a scale nobody holds in their head."

## Honest limits

- The host surface candor sees is literal endpoints; a fully dynamic host is reported as
  "can't certify," not resolved. That's the correct conservative answer, but it means an allowlist is
  strongest where endpoints are literals (the common case for SDK calls).
- AS-EFF-008 certifies the visible host surface only (see semantics above); pair it with `deny Unknown
  <scope>` if you also want to forbid unverifiable Net in a scope.
- **Layering (AS-EFF-009) follows dependencies through any number of crates** under the workspace gate
  (via `layerreach` sidecars). Plain `cargo dylint` with only `CANDOR_POLICY` (no workspace snapshot)
  still catches within-crate and *direct* cross-crate dependencies, but a *laundered* one (`A -> util ->
  B`) needs the sidecars and so requires `cargo candor policy` (or `CANDOR_REPORTS`). Stale sidecars in a
  reused report dir could over-report; the gate uses a fresh temp dir each run, so this only affects
  hand-rolled `CANDOR_REPORTS` setups.
