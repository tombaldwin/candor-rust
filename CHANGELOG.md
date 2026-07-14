# Changelog

All notable changes to candor are recorded here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); candor is pre-1.0, so minor versions may include
behavioural changes (always in the soundness-increasing direction — see the §4 trust contract).

## spec 0.12 — the gains origin field (2026-07-14) — current floor

candor-scan and candor-query now declare **spec `0.12`** (both at crate **0.12.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.12.0**). **0.12 is the current spec
floor** — the ratchet from 0.11. No report-schema or verdict change — a 0.11 report and a 0.11
`--gate-json` verdict are byte-identical under 0.12 — the bump pins the gains `origin` field and the
comparative-verb loud-fail completion into the contract.

### 🧬 gains `--json` carries `origin` — existing | new | unknown

Every gained effect in `candor gains --json` now says whether the function **existed at the
baseline**: a fn that shipped pure and now does Net (the supply-chain attack signal) is a different
alarm from a brand-new fn that does Net (a feature). Reports omit pure functions (§2), so existence
is keyed on the **baseline callgraph** sidecar (caller keys + callees), and the ladder never guesses:
baseline report/graph hit → `existing`; no sidecar or a **partial** graph (a disclosed-and-dropped
corrupt file) → `unknown` — never a downgrade to `new`. JSON-only: the human TSV is a pinned consumer
surface (the `candor-run.sh` seen-file dedup) and stays byte-stable; `byFunction` keys are emitted
alphabetically (`effect`, `fn`, `origin`). The JSON also carries **`baseline_version` /
`engine_version`** provenance (envelope `candor.version`, `meta.version` fallback, empty when
unknown) plus the §2.1 version-mismatch ⚠ stderr disclosure. Pinned four-way by conformance
**PART 5b**.

### 🔊 the comparative verbs complete the loud-fail rule

`gains`, `diff`, and `containment` loaded reports through a quiet path, so a FOUND-but-corrupt
report yielded an exit-0 empty answer — a supply-chain all-clear over corrupt input, the same §4
cardinal sin the 0.11 rung closed for the single-report verbs. The loud rule now covers **both
locators** of the comparatives (and containment's current AND baseline): found-but-corrupt →
**exit 2** with a disclosure; a well-formed empty report is still valid. The quiet loader is
deleted, so no future verb can reach for it. (The plural-`packages` `tour` header shipped inside
0.11.0 and is recorded under the 0.11 entry below.)

## spec 0.11 — the surprising-reach surface (2026-07-13)

candor-scan and candor-query now declare **spec `0.11`** (both at crate **0.11.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.11.0**). **0.11 is the current spec
floor** — the ratchet from 0.10. No report-schema or verdict change — a 0.10 report and a 0.10
`--gate-json` verdict are byte-identical under 0.11 — the bump pins the surprising-reach surface and the
corrupt-report loud-fail rule into the contract.

### ✨ the surprising-reach surface — scan-time opener + `tour` top-N + `candor path` suggestions

After the effect summary and coverage ledger, a scan emits ONE more stderr line: the single most
surprising transitive reach — a benign-named function (settings/config/util/load/…) inheriting a
boundary effect from a few hops away — with a ready-to-run **`candor path <fn> <effect>`** that
re-derives the chain, so a find is never wrong. **`candor tour [N]`** (default 10) lists the top-N
such reaches on demand from an existing report — no re-scan — with `--json` and the §3.3.1 grammar;
wired into `cargo candor tour`. The ranking is **deterministic** (pure call-graph + name analysis, no
LLM) and lives in one shared place, so the scan-time note and `tour` can't drift (conformance
**PART 4f**). Two calibration rules keep the opener from over-promising: a **salience floor** —
Clock/Log/Rand reaches never surface as "surprising" (**PART 4j**) — and test code is excluded by
whole module segment. A repo whose reaches are all mundane gets a plain "nothing hidden" line, never a
manufactured find.

### 🔊 a corrupt report fails loud — never an empty all-clear

A report that is FOUND but unparseable — or that parses as JSON of the wrong shape, a bare junk array
with every entry dropped — now **fails loud with exit 2** and a disclosure on every loud-consuming
verb. Previously it degraded to an empty entry list: `tour` printed "nothing hidden" and a policy
map/gate over the empty report would PASS — the §4 cardinal sin over corrupt input. A well-formed
`functions: []` report (a genuinely effect-free crate) is **still valid** and still exits 0, and one
corrupt file among several still yields the others (disclosed). Pinned four-way by conformance
**PART 4k**.

### 🏷 tour header — the plural `packages` envelope

The `tour` header honours the plural `packages` envelope (SPEC §2, the JVM shape): one package
verbatim, several by their longest common dotted prefix (whole segments only), basename fallback when
none is shared (conformance 4g addendum). 0.11.0 is also the first tagged build carrying the coverage
ledger's plain-English marker — **`classifier doesn't cover`** (was the internal `κ`; SPEC §7 item 14,
PART 4c) — recorded in detail under the 0.10 entry below, where it landed post-tag.

## spec 0.10 — the canonical query grammar (2026-07-12)

candor-scan and candor-query now declare **spec `0.10`** (both at crate **0.10.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.10.0**). **0.10 is the current spec
floor** — the ratchet from 0.9. This is a **tier-2 (pinned-tool-surface) rung** (candor-spec §"Conformance
tiers"): no report-schema or verdict change — a 0.9 report and a 0.9 `--gate-json` verdict are byte-identical
under 0.10 — the bump promotes the new §3.3.1 query grammar into the pinned contract.

### ✨ §3.3.1 canonical query grammar — report discovery + explicit `--report` / `--json` / `--policy` flags

The query surface gains a single canonical grammar (candor-spec §3.3.1): a query auto-**discovers** the
report (the `.candor/report*.json` in scope) so the report path no longer has to be spelled out, and the
inputs are named by explicit flags — **`--report <path>`**, **`--json`** (machine output), and
**`--policy <path>`** — rather than by argument position. The **old positional forms are deprecated but
still accepted** (a deprecation note on stderr; they parse and run exactly as before), so existing scripts
and the conformance goldens keep working unchanged. Pinned by the cross-engine conformance suite as
**PART 17**. No behavioural change to classification, the report schema, or the gate verdict.

### 🔤 coverage-ledger wording — drop the internal `κ` from user- and agent-facing output

The coverage-ledger line and the `--help`/README wording that mentioned the internal classifier shorthand
`κ` now read in plain English — nobody outside the maintainers can decode the Greek letter, and it was the
first thing a cold user met in scan output. The ledger line is now
`candor-scan: candor's classifier doesn't cover N dependencies this code calls into — their effects are
INVISIBLE to the scan (absent from the report, NOT a claim they're pure): …`, and the **stable greppable
marker** every engine shares is **`classifier doesn't cover`** (was `κ doesn't know`). `κ` stays only as
internal maintainer vocabulary (code identifiers, these history entries). No behavioural or schema change.

## spec 0.9 — the remedial-loop rung (2026-07-11)

candor-scan and candor-query now declare **spec `0.9`** (both at crate **0.9.0**). The internal library
crates **candor-report** and **candor-classify** are **aligned to `0.9.0`** too (from 0.5.8 / 0.5.9): every
candor crate now shares the toolchain version, so a crates.io visitor doesn't read the schema/classifier
libs as lagging the engines. They carry no external-consumer contract, so the one-time 0.5→0.9 jump costs
nothing; from here they move lockstep with the spec. 0.9 is a **tier-2 (pinned-tool-surface) rung** (candor-spec §"Conformance tiers"):
no report-schema or verdict change — a 0.8 report and a 0.8 `--gate-json` verdict are byte-identical under
0.9 — but the remedial tool loop (`fix`/`fix-gate`, `unverified`, and the gate's provable-purity
auto-disclosure, all detailed below) is promoted into the pinned §3.1/§3.3 contract. `SPEC_VERSION` is
`"0.9"` in candor-report; the envelope and `--gate-json` verdict declare it.

## [candor-scan 0.9.0] — 2026-07-11

### ✨ Gate scans auto-disclose the provable-purity gap (no need to know to run `unverified`)

A `candor-scan --policy` run now emits the `unverified` disclosure automatically as a stderr note: after the
gate verdict, any function in a `pure`/`deny <E>` scope that PASSES but is `Unknown` (an unresolvable call — the
classic fn/closure-injected "port") is named, with the `deny <E> Unknown <scope>` upgrade that would make the
layer PROVABLY clean. This closes the discovery gap — an author learns their "pure" layer isn't *provably* pure
without having to know the `unverified` subcommand exists. **Advisory only**: it's a note, never a violation, so
the exit code, the gate verdict, and `--gate-json` are all untouched. New `gate::unverified_holes` helper;
emitted from `scan.rs` after `record_gate_violations`. Mirrors the port to candor-java/ts/swift (four-engine
parity). Existing gate tests unchanged (115 pass). The gate note and `candor unverified` share ONE predicate
(`candor_classify::policy::unverified_hole_rule` + `rule_and_upgrade`, candor-classify 0.9.0) — a single
definition of "what a hole is", so the scan-path and query-path disclosures cannot drift (PART 12d pins it).

## [candor-query 0.9.0] — 2026-07-11

### ✨ `unverified` — the provable-purity disclosure (policy guidance from the fix-loop investigation)

New subcommand `unverified <prefix> [policy] [--strict]`. A `pure`/`deny <E>` layer PASSES a function that has
no such effect — but if that function is `Unknown` (candor couldn't resolve one of its calls), the pass is
UNVERIFIED: the Unknown could hide the very effect the rule forbids. The classic case is a fn/closure-injected
"port" — the domain reads as Unknown, so `deny Net domain`/`pure domain` clear it though it may reach Net at
runtime (eval/fixloop/DISPATCH-NOTE.md). `unverified` names every such function in a governed layer, its
`unknownWhy`, and the `deny <E> Unknown <scope>` upgrade that makes the layer PROVABLY clean. Advisory (exit 0);
`--strict` → exit 1 so CI can require provable purity. Text or `--json {ok, unverified[]}`. The gate verdict is
untouched — this only discloses the gap. Wired into `cargo candor unverified` + the `candor_unverified` MCP
tool. 2 regression tests. Rust-only for now; a java/ts/swift port is a natural follow-on.

## [candor-query 0.8.9] — 2026-07-11

### `fix`: the no-clean-hoist advice names the port purity hierarchy (soundness investigation)

Following the fix-loop eval's finding that models reach for a TRAIT port (which candor's gate rejects — it
resolves the dispatch back to the effect-performing impl), an empirical investigation (eval/fixloop/DISPATCH-
NOTE.md) confirmed candor's behaviour is CORRECT (accepting a trait port would silently under-report the effect
the layer reaches at runtime — the cardinal sin), and pinned the three fix shapes' distinct classifications:
trait dispatch → the effect (resolved); fn/closure value → Unknown; plain data → pure. The no-clean-hoist
advice now names the hierarchy: (a) hoist + thread DATA = provably pure (recommended); (b) fn/closure injection
clears `deny E` but leaves an Unknown hole a `deny E Unknown` policy would flag; (c) a trait port doesn't clear
the gate. Text-only; no gate change (the resolution is sound). A candor-scan test guards the classification.

## [candor-query 0.8.8] — 2026-07-11

### `fix`: no-clean-hoist advice rewritten (eval-driven — the remedy was steering agents wrong)

The fix-loop eval (candor-rust/eval/fixloop) measured that on the no-clean-hoist case candor's remedy did NOT
help and HURT weaker models (fable 60% vs control 100%): agents followed the literal "introduce a PORT (a
trait)" advice and wrote a trait port, which candor's OWN gate then rejected — it resolves the trait dispatch
back to the effect-performing impl, so the layer still violates. And "NO CLEAN HOIST" was computed on the
existing graph, so it wrongly declared impossible the simplest valid fix (add a thin composition root above
the layer). The advice now (a) LEADS with the composition-root hoist, and (b) recommends fn/closure injection
with candor's trait-dispatch caveat ("a trait port whose impl performs the effect still trips the gate").
Text-only (the cut/JSON is unchanged; conformance PART 12b still MATCHES). Re-running the eval: the fixed
remedy recovers the treatment arm to 100% across all four models (fable 60% → 100%). See eval/fixloop/RESULTS.md.

## [candor-query 0.8.7] — 2026-07-11

### `fix`: the sandwiched-layer case is now handled (last correctness gap closed)

When an ALLOWED layer is CALLED BY a forbidden one (`D1 → A → D2 → site`, deny on the D layer), hoisting the
effect to the nearest allowed frontier `A` would leave `D1` still inheriting it. `cleanHoist` is now `false`
in that case (a forbidden fn calls into the frontier), with a message that names the sandwich and offers the
port/relax options — instead of a misleading "hoist to A". Detected in the same upward climb that gathers
`hoistHigher`; identical across all four engines, pinned four-way by conformance PART 12b's sandwiched
sub-check. Read-only; additive.

## [candor-query 0.8.6] — 2026-07-11

### `fix`: cross-engine parity fixes (from a high-effort /code-review)

- **Start resolution** now prefers a name match that PERFORMS the effect (so `fix save Net` resolves to the
  effectful `Repo.save`, not a pure `Cache.save` that sorts first) — matching candor-ts/swift. Previously
  Rust/Java could give a false "nothing to hoist" all-clear while the other engines emitted the real fix.
- **`byName`-absent caller** in the up-walk is now skipped (a pure callgraph-only node never routes the
  effect), instead of being classified into the span/hoist — matching candor-swift; avoids naming a pure
  node as a hoist target.
- **`fix-gate` determinism**: functions/effects are iterated in sorted order, so the collapsed remedy's `fn`
  representative is deterministic across engines (the remedy set + order were already dedup-key-sorted).
- **`cargo candor fix`** now resolves the policy the way the `policy` command does (`CANDOR_POLICY` →
  `.candor/config` → `.candor/policy`), so the MCP `candor_fix` tool works zero-config in a repo that checks
  its policy into `.candor/config` — where it previously failed "policy required" though `candor_gate` worked.

## [candor-query 0.8.5] — 2026-07-11

### `fix`/`fix-gate`: the higher-hoist trade-off (FIX-SPEC's last refinement)

Each remedy now carries `hoistHigher` alongside `hoistTo`: the allowed-layer transitive callers of the
minimal frontier that also route the effect — every place you could originate it *further up*. The text
surfaces the trade-off (hoisting higher keeps the frontier pure too, at the cost of threading the value
through more signatures). `hoistTo` (the minimal fix) is unchanged. All four engines compute it identically,
pinned by candor-spec conformance PART 12b (the leaf-normalized remedy tuple now includes it). Read-only,
additive JSON field.

## [candor-query 0.8.4] — 2026-07-11

### `fix`/`fix-gate`: the pure span is now site-anchored (root-independent)

The remedy's `deniedSpan` (the forbidden-layer functions that must become pure) was computed as the
caller-closure from the querying function, so in `fix-gate` — where many inheritors of one crossing collapse
to a single plan — which inheritor won the dedup changed the span (a higher inheritor's closure omitted the
functions *below* it). The cut now anchors on the direct site and walks UP through the denied layer, so the
span is the complete set of forbidden-layer functions between the site and the hoist frontier, identical
whichever function triggered it. Same fix applied to the candor-java port (0.8.8) — the two engines share the
algorithm byte-for-byte. Regression tests unchanged (they already asserted the complete span).

## [candor-query 0.8.3] — 2026-07-11

### ✨ `candor-query fix-gate` + the edit-time loop hands the agent the FIX, not just the finding

New subcommand `fix-gate <prefix> [policy] [0|1]`: a remedy for EVERY deny/`pure` (AS-EFF-006) boundary
crossing in a report, not just one function. It reuses `fix`'s cut (now extracted to `compute_remedy`) and
**collapses the inheritors of one root cause to a single plan** — a `deny Net domain` that trips five domain
functions yields one remedy (one site, one hoist target), keyed by `(effect, layer, site, hoist)`. Text
prints the plan block(s); `--json` emits `{ok, remedies:[…]}`.

`integrations/claude-code/candor-review-source.sh` now folds that plan into the block message: when the
edit-time gate fails (an `AS-EFF` finding) and a policy is set, the loop calls `candor-query fix-gate` and
appends the remedy under the finding — so the agent self-corrects toward the *right* architecture instead of
guessing (adding `allow Net domain`, shuffling the I/O one call up, or threading a client handle the wrong
way). Graceful no-op when `candor-query` is absent or can't read the engine's report shape (`CANDOR_QUERY`
overrides the binary; today the candor-scan report — ts/swift/java remedies are FIX-SPEC P3). This is P2 of
the fix capability. Three regression tests pin the collapse, the clean case, and the fail-loud contract.

## [candor-query 0.8.2] — 2026-07-11

### ✨ `candor-query fix` — the boundary fix (the remedial inverse of `whatif`)

New subcommand: `fix <prefix> <fn> <Effect> [policy] [0|1]`. When an edit makes a function perform an effect
its architecture layer forbids, `whatif` reports the violation; `fix` computes the *architectural remedy* —
where the effect belongs and the smallest refactor that puts it there. Deterministic graph-plus-policy cut
over what the report already holds (integrations/FIX-SPEC.md): the direct **site** (BFS through the effect-
carrying subgraph to the direct source), the **pure span** (the forbidden-layer functions that must thread
the value), and the **hoist frontier** (the nearest allowed-layer caller to perform the effect). Emits a
plan (text or `--json`: `{fn, effect, layer, site, deniedSpan, hoistTo, policyAlternative, cleanHoist}`) and
always offers the policy-relax alternative (`allow <Effect> <scope>`). No clean hoist (every caller up to the
entry is also forbidden) → the two honest options (introduce a port / relax the boundary), never an invented
target. Advisory only: candor names the structure, not the syntax; the gate re-scan stays the ground truth
(a bad suggestion blocks again). `denied_layer` mirrors `whatif`'s violation predicate exactly; fail-loud on
an unreadable/absent policy (exit 2). Six regression tests pin the worked example + every branch. This is P1
of the fix capability; P2 folds a `remedy` field into `--gate-json` + the agent-loop block message.

## [candor-scan 0.8.8] — 2026-07-11

### ⚠ candor-scan: a trait DEFAULT method via an empty impl now charges (soundness R30 — report-affecting)

A trait's provided (default) method reached through an EMPTY `impl Trait for T {}` read silent-pure —
`impl Logger for FileLogger {}` + `l.flush()` inheriting `Logger::flush`'s effect was dropped. The
caller-fallback that edges `t.m()` → the inherited `Trait::m` default body already existed, but a type whose
ONLY impl is an (empty or non-overriding) trait impl has no fn unit of its OWN, so it never entered
`local_types` — which made its typed call un-resolvable and GATED OUT the fallback. Fix: register every type
with a local trait impl as local. An OVERRIDE still wins (only the override's effect flows; the default is
not also charged — no fabrication); a pure default stays pure. Found by an autonomous cross-engine soundness
sweep; gated by `trait_default_method_via_empty_impl_charges_the_default_body`.

## [candor-scan 0.8.7] — 2026-07-10

### ⚠ candor-scan: a bounded-generic struct field now dispatches (soundness R31 — report-affecting)

A stored field typed as the STRUCT's own bounded generic param — `struct Pipe<T: Saver> { item: T }`
reaching `self.item.save()` — read silent-pure: field types were resolved with an EMPTY generic-bounds
map, so `T` never resolved to its `Saver` bound and `item.save()` never dispatched to the trait's
implementors. Now the struct's own `<T: Bound>` (and `where T: Bound`) seeds the field's trait leaves, so
the existing dispatch-typed-field CHA fires. An unconstrained-generic field read (no method call) stays
pure (no fabrication). Found by an autonomous cross-engine soundness sweep (the swift R27 analog); gated by
`generic_struct_field_resolves_to_its_trait_bound_dispatch`. (Known open: R30 — a trait DEFAULT method used
via an empty `impl Trait for T {}` still reads pure; tracked in candor-spec SOUNDNESS.md.)

## [candor-scan 0.8.6] — 2026-07-10

### ⚠ candor-scan: the `baseline` config key now ACTIVATES the AS-EFF-005 guard (spec §7 item 5)

The stable scanner implements the family-MUST baseline regression guard, with candor-java's
`checkBaseline` as the exact model: `CANDOR_BASELINE=<saved report path or --out prefix>` (or the
`.candor/config` `baseline` key — **previously recognized-but-inert with a loud warning; a repo that
already checked one in gets a LIVE ratchet on its next scan**). Semantics: an *existing* function
that gained an effect vs its baseline transitive set → one `[AS-EFF-005]` violation per fn + exit 1,
joined into the `--gate-json` verdict by the same accumulator as the policy gate; new fns exempt
(regressions in existing code only); baseline file absent → one stderr note ("guard not active;
record one: candor-scan <dir> --out <prefix>") + exit unchanged; baseline unparseable (incl. any
dropped entry), missing its provenance header, or produced by a **different scanner build** (envelope
`candor.version` vs this build) → exit 2 WITHOUT evaluating (the §2.1 stale-baseline posture — never
a silent skip, never a stale compare); a configured-but-EMPTY value → exit 2 (the bare-`policy`
posture); a guard over a crate with an unparseable source file → exit 2. Dependency scans under
`--deps` stay guard-free. Same advisory-floor caveat as the scan policy gate.

### Docs — reference-engine attribution, Path A `Unknown` claims, status refresh

README/AGENTS corrections (with a new in-repo grep gate + a behavioral pin holding them): candor-java
is the family's REFERENCE engine (this repo is the deep Rust engine — the README claimed "the
reference implementation"); Path A/candor-scan does emit `Unknown` for invoked fn-values, FFI
`extern` calls and untrusted chained reports (AGENTS.md and the scan docs claimed it "never emits
`Unknown`" — other misses remain silent by design, now stated precisely); the status section now
reflects the 35-crate calibration/fuzzer/oracle reality and spec 0.8 (it still said "Prototype,
validated on ebman"); the conformance blurb counts four code engines + the agents domain engine;
stale version examples became placeholders.

## [candor-scan 0.8.5 · candor-query 0.8.1] — 2026-07-10

### ⚠ `pure` no longer counts `Unknown` as a violation (family ruling — verdict-affecting)

An Unknown-only function no longer trips a `pure` rule in candor-scan: `Unknown` is the §4 trust
marker, not an effect — §6.2's `pure` forbids every EFFECT, AS-EFF-003 owns the uncertainty
residual, and **`deny Unknown <scope>`** is the explicit knob (it keeps firing, effects
`["Unknown"]`). The reference engine (candor-java) and the rust deep engine already read the
predicate this way; candor-scan (with candor-ts and candor-swift, fixed in their repos) was
counting the marker — a cross-engine verdict split on the same policy file. Pinned four-way by
conformance PART 16.

### ⚠ Deep-engine `deny <Effect>` no longer fires on `Unknown` (SEMANTICS §6 alignment, family ruling)

The nightly gate folded `Unknown` into EVERY `deny` projection: a function whose only marker was
`Unknown` (an unresolvable call) tripped `deny Net` — a **false-positive divergence** from the
family. The reference engine (candor-java), candor-scan and candor-ts all implement the SEMANTICS §6
predicate exactly: AS-EFF-006 fires iff `I(f) ∩ Forbidden(r) ≠ ∅` — an effect PROVABLY in the
transitive set. The deep engine now agrees: `deny X` fires only when `X` is provably in `I(f)`, and
`Unknown` never appears in a deny-X verdict's effects. **The strictness knob is explicit**: where a
boundary must also exclude uncertainty, pair it with **`deny Unknown <scope>`** (the §6.2 grammar's
denyable token — it keeps firing, with effects `["Unknown"]`). A `pure` rule likewise forbids every
*real* effect but not the `Unknown` visibility marker (AS-EFF-003's concern), matching the reference
engine. `candor-query whatif` mirrors the same projection, so the pre-edit verdict cannot diverge
from the gate.

### Internal — candor-query main.rs split into per-command-family modules (byte-identity gated)

The 2.9k-line `crates/candor-query/src/main.rs` is now 9 modules (load / matching / audit / show /
callers / policy / diff / containment / state + `tests.rs`), flat namespace preserved via
`pub(crate)` re-exports. Gated by a 37-run command battery against fixed deep+scan reports
(stdout/stderr/exit codes byte-identical).

### Internal — deep-engine mega-fns decomposed (no behavior change, byte-identity gated)

`check_expr` (~615 lines) and `check_crate_post` (~795 lines) in src/lib.rs each carried a dozen
concerns inline. Twelve per-concern helper methods extracted (value-reference/static-force/
thread_local/deref edge probes, callback flow both sides, unresolved-dispatch disclosure, resolved-
call recording, explain, layering, report emission, gate verdict) — bodies moved verbatim. Gated by
a deep-engine byte-identity battery (reports, sidecars, ledgers, violations sentinels, gate
verdicts, AS-EFF diagnostics — identical modulo the git build-id stamp).

### Internal — candor-scan main.rs split into modules (no behavior change, byte-identity gated)

The 7.9k-line `crates/candor-scan/src/main.rs` is now 12 modules along its natural seams
(model / lang / lazy / deps / collector / decls / cache / config / gate / propagate / scan + a
`tests.rs` file module). One flat crate namespace is preserved via `pub(crate) use` re-exports, so
no call site changed. Gated by a byte-identity battery (conformance fixtures incl. the gate/policy
runs, sample crates, a workspace root, minreq/fs_extra/xshell from crates.io — reports, sidecars,
gate verdicts, stdout/stderr and exit codes all identical) plus the 120-edit incremental-cache
equivalence run on tokio.

### Added — the coverage wave's pins (tests only, no behavior change)

- candor-scan: the `--deps` registry-tree mode is covered end-to-end for the first time (hermetic
  fake registry — discovery, the documented `.candor/deps/<name>@<version>/` layout, effect + literal
  surface crossing the chain, policy-free dep scans, lockless exit 2, cache reuse); the nested
  `cfg(all/any/not)` evaluator, `push_quoted`, `is_non_nominal_type` and tuple-destructured binding
  are unit-pinned (with an anti-fabrication rebind twin).
- candor-query: in-repo pins for the previously conformance-only arms — `callers --include-unknown`
  (hierarchy-gated frontier), `blindspots` (ranking + blast radius), `rewire` (exit contract),
  `locate` — so an engine-local regression fails this repo's own CI, not just the spec repo's.

## [candor-scan 0.8.4 · candor-query 0.8.0 · candor-report 0.5.8] — 2026-07-09

### ⚠ Fail-open paths that used to pass GREEN now exit 2 — intentional bug-fix semantics

If your CI went red on one of these after updating the clone, the gate was previously **not
running** and telling you it passed. Exit 2 always means "the gate could NOT evaluate", never a
violation (that's exit 1):

- **`cargo candor policy` on a run that couldn't complete.** A crate that failed to build under
  dylint (or the engine's own §6.2 unreadable-policy exit) was swallowed by `|| true` and printed
  "policy OK" with exit 0. Now: no report snapshot → exit 2 before enforcing (a snapshot-less
  enforce also silently dropped cross-crate `allow` resolution); a nonzero dylint exit → exit 2
  ("policy NOT evaluated").
- **`cargo candor guard` with no baseline at all** (never snapshotted / typo'd prefix) exits 2 with
  the snapshot incantation — the engine used to warn "guard NOT active" and exit 0. A **per-crate
  baseline gap** (a new workspace member) is disclosed by the engine as a `GUARD-UNAVAILABLE`
  sentinel and also exits 2. (Completes the fail-closed arc started by the stale-baseline and
  absent-provenance-sidecar fixes of 2026-07-08.)
- **A configured-but-EMPTY `policy`** (a bare `policy` line in `.candor/config`) exits 2 — never a
  silently skipped gate.

### Changed — `CANDOR_CONFIG` → `CANDOR_RULES` (deep engine, clean rename, NO fallback)

The lint's classifier-extension rules file is now pointed at by **`CANDOR_RULES`**. `CANDOR_CONFIG`
now means one thing family-wide: the spec-§3.4 config-file override path (which candor-scan already
implemented). One variable meaning two incompatible things was worse than a break.

### Added

- **`.candor/config` on the deep path** (spec §3.4): the wrapper discovers the checked-in config
  (walk up from the project; `$CANDOR_CONFIG` overrides), wires `policy`/`baseline`/`deps`, warns on
  unknown keys, exits 2 on a configured-but-unusable file, and DISCLOSES recognized-but-unwired keys.
  Relative path values anchor to the config's HOME directory (the one containing `.candor/`) in both
  the wrapper and candor-scan — never the process CWD.
- **`--gate-json` on the deep path** (spec §3.3): `cargo candor policy|guard --gate-json <path|->`
  emit the structured verdict `{ spec, ok, violations }` — same shape, field names and byte layout as
  candor-scan's (one shared `candor_report::GateViolation` + serializer, candor-report **0.5.8**),
  assembled from the engine's `CANDOR_GATE_JSON` NDJSON records by the new
  `candor-query gate-verdict`. Violation → exit 1; incomplete/unwritable verdict → exit 2, file
  removed, never a stale or silent verdict.
- **candor-scan 0.8.4**: the κ ledger honors the §2 rule-3 coverage exemption for CHAINED reports,
  keyed on the envelope `package`/`packages` field — an EMPTY chained report is a purity claim, not
  a "κ doesn't know" line; config keys the engine recognizes but does not implement
  (baseline/strict/no-ambient/closed-world/taint) warn loudly instead of reading as an active gate.
- **candor-query 0.8.0**: joins the spec-tracks-version convention; full crates.io metadata + a crate
  README; candor-report floor 0.5.8 (resolution can never produce a pre-0.8 spec declaration).

## [candor-scan 0.8.3] — 2026-07-02

- **`.candor/config`** (spec §3.4): the checked-in configuration file — target-anchored discovery
  (walk up from the scan target), `policy`/`deps` wired, unknown keys warn, configured-but-unusable
  exits 2. CI becomes "point at the repo".
- Allocation-free violation sort (identical order).

## [candor-scan 0.8.2] — 2026-07-02

- `--gate-json` takes a real path only (a flag-shaped/valueless value exits 2 — it used to swallow
  `--policy` and scan the wrong target, gateless); `--gate-json -` streams the verdict to stdout,
  which stays pure JSON (AS-EFF lines → stderr).

## [candor-scan 0.8.1] — 2026-07-02

- Workspace `--gate-json` accumulates across members (spec §3.3 MUST: the verdict agrees with the
  exit code — a clean last member no longer overwrites an earlier violator's verdict).

## [candor-scan 0.8.0 + candor-report 0.5.7] — 2026-07-01/02 — spec 0.8

- **`--gate-json <file>`** (spec §3.3 ⟨0.8⟩): the structured gate verdict
  `{ spec, ok, violations: [{rule, fn, effects, detail}] }`, written from the SAME check that sets
  the exit code — the machine analog of the AS-EFF console lines (feeds the PR-native SARIF
  reporter). candor-report 0.5.7 declares `SPEC_VERSION = "0.8"`; candor-scan 0.8.0 pins it as a
  floor so dependency resolution can't produce a pre-0.8 declaration.
- In the same window (in-tree, not a crates.io release): the nightly lint gained the
  CANDOR_VIOLATIONS machine-readable violation sentinel + a deep-engine dynamic-oracle lane;
  `cargo candor guard` turned fail-closed on a stale baseline and an absent provenance sidecar.

### (bridge) 0.4 → 0.7, briefly

The arc between this entry and 0.3.7 below: spec 0.5, **spec 0.6** (2026-06-19: the `blindspots`
query + `unknownWhy` required on direct Unknown sources — candor-report 0.5.5, candor-scan 0.5.19),
a long candor-scan 0.5.x soundness run (FFI/drop-glue seams, lazy-static deferred init,
iterator-forcing, masked-literal fail-closed, implicit conversion, 20-crate coverage calibration),
then **spec 0.7** (2026-06-19: engine versions aligned to the spec — candor-scan/candor-query
0.7.0, candor-report 0.5.6) and the 0.7.x review fixes. Detail: `git log` around those dates.

## [0.3.7] — 2026-06-12 (crates: candor-report / candor-classify / candor-scan, lockstep)

### Changed — spec 0.4 (conformance-breaking upgrade, wire-compatible)

- Reports now declare **spec `0.4`** (candor-report's `SPEC_VERSION`). 0.4 upgrades four SHOULDs
  to MUST — §2.1 version-trust at the chain join (missing version = unverifiable = Unknown), the
  §7.14 κ-coverage ledger, universal `hash` emission, and literal surfaces when `allow` rules are
  enforced. This engine already satisfies all four; the bump is the declaration.

### Added — the κ-coverage ledger + report chaining (the curation treadmill's exit; SPEC §7.14 / §2)

- **The κ-coverage ledger:** the receipt names every `Cargo.toml` dependency the code demonstrably
  calls that the classifier knows nothing about — `κ doesn't know N dependencies … effects through
  them are INVISIBLE (not Unknown)`. Per-scan evidence instead of a doc footnote. Exempt: the
  platform frontier, calibrated crates, and crates a chained report covers (an all-pure dep's EMPTY
  report registers as covered — its emptiness is the purity claim).
- **`CANDOR_DEPS` chaining:** reports now carry the §2 join key (`hash: crate#qual`), and an
  unclassified call into a crate a sibling report covers inherits its effects AND literal surfaces
  (unambiguous tail-first join; a report from a different scanner version downgrades to `Unknown`,
  §2.1).
- **`--deps`:** scan the whole Cargo.lock dependency tree (unbuilt registry sources, in-process —
  the self-gate's own `deny Exec` forbids the spawn-yourself shortcut) into `.candor/deps/`
  (one subdirectory per name@version, skip-if-already-scanned), then scan the root chained over
  it. Measured on a real 328-dep app: 75s one-time (~0.23s/dep, cached after), the ledger dropping
  12 unlisted deps → 1 (the path dep), 7 fns gaining real effects. The pre-release review closed
  the trap cluster: the root policy no longer leaks into dep scans via CANDOR_POLICY; `--out` is
  honoured; same-crate version pairs don't overwrite each other; the dep-dir + CANDOR_DEPS
  double-load no longer drops every join as ambiguous; joins use the crate-relative path
  (bare-leaf fallback removed — it fabricated); cargo_deps reads every workspace manifest and the
  `[dependencies.name]` header forms.

### Added — local-trait dispatch (syntactic CHA; SPEC §4 bounded-CHA discipline)

- A dispatch-typed receiver — `&dyn T` / `impl T` / generic `X: T` (inline or where-clause), through
  `Box`/`Rc`/`Arc`/`RefCell`/`Mutex`/`RwLock`, as a param, let, or STRUCT FIELD — resolves to the
  trait's local implementors when narrow (≤12, the cross-engine bound; both sides of the bound are
  unit-tested), so the DI pattern (`self.store.save()` → `PgStore::save`) carries its effects on
  the stable scanner. A local trait declaring the method but with no visible impl (or too many, or
  an ambiguous name) reads honest `Unknown` — the previous SILENT miss, closed.
- Resolution is gated to LOCALLY-DECLARED traits whose declaration carries the called METHOD
  (the pre-release review execution-verified the wider rule fabricating: `impl Iterator for
  RowIter` + `fn f(it: impl Iterator)` charged pure `f` with RowIter's `Db`; a same-named method
  on a non-dispatching bound was the same wrongness). External-trait dispatch stays a documented
  miss; `.clone()` on a bound param neither edges nor floods.

### Added — classifier (candor-classify)

- The **entropy tier**: `SaltString::generate` (argon2/scrypt/pbkdf2/password_hash), bcrypt's
  `hash`/`hash_with_result`, and `rand_core`'s OsRng surface → `Rand` (the TS engine's CTA lesson,
  found by the ledger's first probe).
- **Comma-list FROM extraction**: `SELECT a FROM t1, t2, t3` yields all three tables
  (comma-adjacent continuation; an alias breaks the chain — the fabrication guard for column
  lists). Pinned three-way by the conformance vector battery (20 vectors).

## [0.3.6] — 2026-06-11 (crates: candor-report / candor-classify / candor-scan, lockstep)

### Added — the Db literal surface (`tables` + `allow Db`)

- **`tables`** joins `hosts`/`cmds`/`paths` as the fourth literal-refinement surface (SPEC §2):
  table-position identifiers extracted from SQL string literals at `Db`-classified calls
  (`FROM`/`JOIN`/`INTO` anywhere; statement-leading `UPDATE`/`TRUNCATE`; `TABLE` — extraction is
  conservative in the fabrication direction: non-SQL strings and `FOR UPDATE` locking clauses yield
  nothing). Captured by the nightly lint AND candor-scan, propagated transitively, carried
  cross-crate, rendered by `show` as `Db(table,…)`.
- **`allow Db in <scope> <table>…`** (AS-EFF-008) gates it in both backends: case-insensitive
  qualified-name match, `schema.*` covers a schema, an unqualified allow does NOT cover a qualified
  reach. "Billing may only touch `ledger.*`" is now a deterministic CI rule. Both engines (the JVM
  engine shipped the same change in lockstep); the conformance grammar battery covers the new rule.
  This is also the zero-new-engine first step of the database-development transfer (BACKLOG P5).

## [0.3.5] — 2026-06-11 (candor-scan only)

### Fixed — resolution (under-reports recovered)

- **`-> Self` constructors now type their locals.** `let agent = Agent::new_with_defaults();`
  followed by `agent.run(..)` formed `Self::run` (no local def) and silently dropped the edge —
  found by the PROVE-IT dogfood on `ureq`, where 3 public API entry points were missing from a
  16-function blast radius. `Self` in an impl method's return position now resolves to the impl
  type (also un-defeats the ambiguity check: two same-named `-> Self` ctors on different types no
  longer collide as "Self" == "Self").
- **Tuple-struct fields index by position**, so a newtype-wrapped receiver (`self.0.run()`,
  chained `self.0.0`) resolves like a named field.
- README "Misses" updated to the measured blind-spot list (Deref-coercion receivers and
  generic-parameter fields are the ureq residual: 14/16 found, never fabricated); PROVE-IT.md
  prompt aligned (callgraph naming convention, `#[cfg(test)]` scope).

## [Unreleased] (nightly lint)

### Fixed — soundness

- **Method calls on a returned `impl Trait` (opaque) receiver were silently dropped** (bug #33, found
  by dogfooding the site's primary CTA on the `which` crate: `which()` reported `["Env"]` with
  `unresolved: false` while truly reaching `Fs` through
  `which_all(..).and_then(|mut i| i.next())`). Two halves: `devirtualize` now RETRIES resolution under
  a post-analysis typing env (opaques revealed, as codegen resolves) so the call pins to the concrete
  local impl — `which` now carries `Fs` via a real edge to `<WhichFindIterator as Iterator>::next`;
  and `is_dyn_receiver` reveals a local opaque whose hidden type is a `Box<dyn …>` so the dispatch is
  honestly `Unknown`. Teeth: soundness/gen.py `opaque_iter` + `opaque_dyn` forms.

## [0.3.3] — 2026-06-10 (crates: candor-report / candor-classify / candor-scan, lockstep)

Republish so the crates.io artifacts carry the fixes committed after 0.3.2 (the published 0.3.2 had
diverged from the 0.3.2 source tree). Surfaced by a maximum-effort multi-agent `/code-review`.

### Fixed — precision / correctness

- **`candor-classify`: IPv6-aware policy host matching** — `host_part` now keeps a bracketed
  `[::1]:8080` host and a bare `2001:db8::1` intact instead of truncating at the first `:` (which had
  mangled IPv6 endpoints in `allow Net in <scope> <host>` rules into a useless prefix).
- **`candor-scan`: single-codepoint type idents** — the CamelCase test in `type_from_value_path` uses
  `chars().count()`, not byte `len()`, so a one-character non-ASCII type ident (`struct É;`) still
  counts as a single character (a snake/SCREAMING const still yields `None` — honest under-report).

report is bumped in lockstep (unchanged content) to keep the three crates' shared version and their
inter-crate `version =` dependencies resolvable on crates.io.

## [0.3.4] — 2026-06-11 (candor-scan only)

### Added

- **`--policy <file>` / `CANDOR_POLICY` — the stable policy-gate floor.** The published scanner can now
  enforce a spec-§6.2 policy (`deny`/`pure`/`allow`/`forbid`; AS-EFF-006/008/009) over its own scan and
  fail the build, with zero extra install. Explicitly the **advisory floor**: the syntactic backend
  under-reports, so a clean run is necessary, never sufficient — the nightly engine remains the sound
  gate. Shares the §6.2 parser with the nightly and JVM gates (one grammar, everywhere); an unreadable
  policy file exits 2 loudly rather than silently not enforcing.

## [0.3.3] — 2026-06-10 (candor-scan only)

- Metadata release: repository URL moved to `tombaldwin/candor-rust` (the family umbrella now owns
  `tombaldwin/candor`); includes all 0.3.2-era scanner fixes.

## [0.3.2] — 2026-06-10 (crates: candor-report / candor-classify / candor-scan, lockstep)

The "validated everywhere" release: 18 product fixes found by systematic validation (blackout screens,
report-vs-source A/B audits, query property harnesses, fuzzer extensions) since 0.3.1.

### Fixed — soundness / recall (the dangerous direction)

- **`src/build.rs` modules are scanned** (only the crate-root Cargo build script is skipped) — git2's
  `RepoBuilder` module had vanished entirely, so `Repository::clone` reported no `Net`.
- **Struct-literal bindings infer their type** (`let s = S;` / `let s = S{..};` — previously only
  annotated lets), CamelCase-gated; `Enum::Variant` types as the enum.
- **Classifier tiers added:** libcurl FFI (`curl_easy_perform`/send/recv/upkeep + multi pumps → Net)
  + the `curl` consumer crate rule; libgit2 submodule clone/update → Net; `std::path::Path`/`PathBuf`
  stat family → Fs (gix-dir, a directory walker, had reported zero Fs); DB verb dialects — rusqlite's
  canonical API (`query_row`/`query_map`/`execute_batch`/`prepare_cached`/`open`…) had classified
  PURE for consumers, plus `tokio_postgres::query_typed`, diesel `first`/`load_iter`, sqlx `fetch_many`.
- **Report fields:** `spec` (the contract version — required by SPEC §2.1), `unknownWhy`, `entryPoint`
  now emitted by the report crate (published 0.3.0/0.3.1 artifacts predated them).

### Fixed — precision / correctness

- **Callgraph sidecar completeness (SPEC §2.2):** every analyzed function is a key (uncalled leaves
  were invisible to `whatif`/`callers`, conflating "no callers" with "no such function").
- **Name-query matching ladder:** exact > segment-suffix > substring — a precise partial name
  (`Pricing::quote`) no longer silently widens a blast radius to substring cousins (`quote_bulk`).
- **`map` buckets crate-root free functions into `(root)`** per SPEC §6.1 (was one pseudo-module per
  function on flat crates).
- **`diff` fails loud on a prefix matching no reports** (a typo'd current path previously showed zero
  gains — silently passing a gained-effect gate).
- **The shared `CANDOR_POLICY` parser** (SPEC §6.2) — one canonical implementation for the gate,
  `whatif`, and the new `parsepolicy` dump; `deny Unknown <scope>` now parses everywhere.

### Added

- `PROVE-IT.md` — a self-experiment prompt an adopter's agent runs on their own repo (this release is
  its minimum version: earlier published binaries exhibit the since-fixed resolution bugs above).

## [0.3.0] — 2026-06-08

The "enforce, soundly, at scale" release. candor goes from *describing* effects to **enforcing**
architecture-as-code across a whole workspace, makes "never silently under-reports" a set of
CI-enforced fuzzers instead of a hope, and ships a rigorously-measured demonstration that it changes
the code agents *ship*, not just what they report.

### Added — architecture-as-code policy (`cargo candor policy`)

- **Literal allowlists (AS-EFF-008).** `allow <Effect> [in <scope>] <value>…` constrains *which* values
  an effect may reach, checked against the **transitive** literal surface:
  - `allow Net … <host>` — network host allowlist ("billing may only talk to Stripe"), matched by hostname.
  - `allow Exec … <cmd>` — subprocess command allowlist ("build may only run git"), matched by basename.
  - `allow Fs … <path>` — filesystem path allowlist ("config may only read /etc/app"), matched by prefix.
  A model can't self-check these: the literal is buried in a deep, often cross-crate, callee.
- **Module-layering rules (AS-EFF-009).** `forbid <A> -> <B>` — a function in scope `A` must not
  transitively call into scope `B` (the dependency-direction boundary). Follows dependencies **across
  crates**, including ones laundered through a third crate (via per-crate `layerreach` sidecars written
  during the workspace enforce pass).
- **One-command workspace gate.** `cargo candor policy` now snapshots every crate then enforces with the
  siblings loaded, so cross-crate boundaries (effects, hosts, layering) hold in a single invocation.
  Gates on AS-EFF-006 / 008 / 009.
- **`CANDOR_REPORTS`** — a read-only cross-resolution prefix usable in enforcement modes.

### Added — effect detail in the report

- **`hosts` / `cmds` / `paths`** report fields: the statically-visible literal Net endpoints, subprocess
  commands, and filesystem paths a function reaches (the decidable subset; never a completeness claim).
  Propagated transitively and across crates.

### Added — soundness, now a gate

- **Adversarial soundness fuzzers**, all CI-enforced, all teeth-verified (reverting the relevant fix
  turns them red):
  - construction fuzzer — threads a known effect through every call form (closures, `dyn`, generic /
    boxed callbacks, `Arc<dyn>` arbitrary-self-type, macros);
  - cross-crate variant (lib→bin DefPathHash propagation);
  - dynamic oracle — runs each program under `strace` and asserts candor over-approximates the effects
    the kernel actually observed, plus a per-function attribution variant;
  - **drop fuzzer** — threads the effect through a `Guard`'s `Drop` wrapped in random container forms.
- **Implicit `Drop` edges.** candor now reads MIR `Drop` terminators and follows the dropped type's
  reachable local `Drop::drop` impls — including value-embedded fields **and** std owning containers
  (`Box`/`Vec`/`Rc`/`Arc`/`HashMap`/…). An effectful RAII guard (I/O on scope exit) is no longer
  silently dropped from the effect graph. (Found by the Bet 4 MIR spike; see `eval/bet4/FINDINGS.md`.)

### Added — evidence

- **Pre-registered outcome eval** (`eval/bet2/`): when a task tempts an agent to put I/O in a layer that
  must stay pure, candor took the **shipped** violation rate from ~80% to 0% (Fisher p<0.001). Two prior
  pre-registered nulls (floor effects) are reported honestly alongside.
- **Real-world validation** (`eval/realworld/`): the policy + detail features run on candor's own
  non-fixture code; literal extraction matches `build.rs`'s actual `git`/path I/O exactly.

### Changed

- **Policy scope matching now uses the crate-prefixed path** (`<crate>::<path>`), so a layering/allow/
  deny scope spelled as a **crate name** matches that crate's own functions instead of being a silent
  no-op. Module/type-name scopes are unaffected.
- Classifier: `tokio::process` → `Exec`; async runtimes; `time`/`fs_err`/`tempfile`/`glob`/`duct`/
  `dotenvy`; compiler diagnostic emission → `Log`; `rand` verb-gated.
- **crates.io-ready:** vendored `span_lint`, dropping the only git dependency (`clippy_utils`).
- Nightly pin is now auto-bumped weekly by `.github/workflows/nightly-bump.yml` (opens a reviewed PR).

### Fixed

- Soundness holes: `Box<dyn Fn>` called directly, non-local callbacks, and `dyn` behind a smart pointer
  (`Arc<dyn>`) are no longer reported pure; `parse_dph` ICE on a non-ASCII hash.
- Closure-flow: effects propagate through a named function passed as a callback.
- Tooling robustness: the source-freshness hash, the `settings.json` Stop-hook merge, and report
  discovery moved out of fragile (duplicated, drifted) shell into typed, unit-tested `candor-query`
  subcommands (`state`, `reports`, `merge-hook`). The guard no longer fails open; `install.sh` no longer
  risks clobbering a user's settings.

## [0.2.0]

The agent-facing baseline: per-function transitive effect inference, the v0.2 report envelope,
cross-crate propagation by `DefPathHash`, the `cargo candor` wrapper and `candor-query` CLI, the
CANDOR_STRICT / NO_AMBIENT / BASELINE / POLICY enforcement modes, and the Claude Code integration.
