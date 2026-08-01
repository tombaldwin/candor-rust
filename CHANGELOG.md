# Changelog

All notable changes to candor are recorded here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); candor is pre-1.0, so minor versions may include
behavioural changes (always in the soundness-increasing direction — see the §4 trust contract).

## Unreleased

### ⟨0.24⟩ "PROVABLY clean" over a report declaring source candor could not read

SPEC §3.2 `ec1a441`/`93cef40`/`142740a`. `0075987` ruled the omit-`ok` shape for `whatif` and this engine
implemented it **for `whatif`, in `whatif`'s own file** — `unverified.rs` and `fix.rs` contained zero
occurrences of `incomplete`. MEASURED on a report declaring one `unanalyzed` unit, **no `Unknown` holes at
all**, and a `deny Net app` nothing violates:

```text
  gate --report        exit 2   ok:false, incomplete:true + manifest   ← correct
  unverified --strict  exit 0   {"ok": true, "unverified": []}
                       stdout   "every function in a pure/deny layer is PROVABLY clean … ✓"
  fix-gate  --strict   exit 0   {"ok": true, "remedies": []}
                       stdout   "no deny/pure boundary crossings in this report ✓"
```

`unverified`, `fix-gate` and `fix` now read the ⟨0.21⟩ manifest through the SAME file set and the SAME
reader `gate --report` uses, emit `incomplete: true` + the `unanalyzed` manifest, **OMIT `ok`** (never
`ok: false` — on an advisory verb that asserts a finding the analysis did not make), and exit 2 under
`--strict`. **On every channel**: the prose `✓` is the prose `ok: true`, and `unverified` had a second
sentence of the same kind — *"The gate still PASSES"* — which is not merely unhedged but false, since the
gate exits 2 over those bytes. `fix` carries the disclosure with its exit code unchanged (it answers no
`ok`, matching its own gate-refusal branch and candor-ts).

`142740a` also folds in the WITHHELD-RULE trigger, where this engine emitted `ok: false`: where a rule was
withheld no hole was FOUND, the question was declined, so `false` asserts the finding that did not happen
— the same shape and the same answer as the incompleteness trigger, ruled a day apart. And "the same
bytes" means the same report SET: MEASURED here as a null result, since both routes go through one
`glob_reports`, but pinned by a row so a later split cannot reintroduce candor-java's defect silently.

The findings still ship in every case — a partial answer that says it is partial beats a refusal.

13-mutant audit, all killed. The one that mattered: **keeping the `✓` on `unverified`'s all-clear branch
SURVIVED** the first draft of the row, because every existing fixture pairs incompleteness with something
else to find and the verb never reached its own all-clear line. A/B on 40 real report corpora × 7 policies
× 8 invocations (2240 runs): **one** difference, and it is the intended `142740a` change on a legacy
`hosts`-only report. Those 40 corpora declare zero `unanalyzed` units between them, so they prove no loss
and nothing else — the incompleteness path is exercised by hand-written fixtures.

### ⟨0.24⟩ a chained dependency report that judged NOTHING must not read as full coverage

A report carrying `functions: []` and `analyzed.count: 0` bought a consumer **more confidence than not
chaining the package at all**. Its caller dropped out of `functions` — under ⟨0.21⟩ a positive **purity
claim** — with no `invisible` on the entry, no `coverage.uncovered` in the envelope, no verdict caveat and
no line on stderr, while the same scan with `CANDOR_DEPS` unset disclosed all four. Conformance PART 26
found the same door in all four engines and printed `rust INDISTINGUISHABLE — the engine is not reading
analyzed.count`.

**The harm stated precisely, because the loose form sends you after the wrong symptom.** The empty report
carries no effects, so the count-0 arm cannot itself *trip* a gate — it and the unchained arm both exit 0
on `deny Fs`. What it DELETES is the **disclosure**; the gate flip exists only against the *trusted* arm.
So this fix restores the disclosure channel and deliberately does not manufacture a verdict: asserting an
effect the consumer has no evidence for is the mirror sin.

    unchained   entry -> invisible: ['deplib'], coverage.uncovered: [deplib]   exit 0
    trusted     entry -> ['Fs']                                                exit 1
    count: 0    pre   entry ABSENT from `functions`, no coverage, no advisory  exit 0
    count: 0    post  entry -> invisible: ['deplib'], coverage.uncovered       exit 0  + a named stderr line

**WHERE THE RULE LIVES.** A third conjunct on the COVERED set — `deps.rs`'s one `cover` closure and the κ
ledger's one `covered` predicate — beside the §2.1 staleness gate and the ⟨0.21⟩ incompleteness one.
Coverage is the single mechanism that turns a report's SILENCE into a purity claim, and this rung is the
third answer to *"may this silence speak?"*, after staleness and incompleteness. Not the gate: a gate
reads its verdict off a coverage decision already made, so the rule there would have to be repeated per
verb and would leave the report, the κ ledger and every other consumer of the same silence untouched.
Same placement candor-java (`Loader.loadCrossDeps`) and candor-swift (`Deps.swift`) reached independently.
It needed no new plumbing: withhold coverage and the existing `invisible` / `coverage.uncovered` /
`--gate-json` block all fall out.

**Coverage is anchored FOUR times here** — the envelope `package`, the JVM-shape `packages[]`, the
filename fallback and each entry's `hash` prefix — and all four funnel through one closure, so there is
one place to gate. That matters more for this rung than the last: a count-0 report reaches the entry loop
with **no entries**, so its `hash` anchor never fires and gating that one alone would have been exactly
the no-op java measured. `coverage_has_exactly_one_anchor_and_exactly_one_consumer` now enumerates four
writes and three consumers out of the source.

**THE SECOND ROW OF SPEC §2's TABLE IS A CONTROL, NOT A FOOTNOTE, and it is why this is not a one-liner.**
`functions: []` is equally the shape of a legitimately all-pure dependency, whose empty report §2 chaining
rule 3 requires a consumer to BELIEVE. `analyzed.count` is the only thing on the wire that separates them,
so the predicate is keyed on **that integer** and never on the emptiness of `functions` — `functions`
enters for exactly one row, the manifest-less one. Measured over 1997 JVM dependency jars: a fix keyed on
emptiness would have withdrawn **104 real claims to catch 6**.

**BOTH CONTROLS, MUTATION-VERIFIED IN THREE DIRECTIONS.**

- `count: 0` → not covered, and the consumer's answer is asserted **EQUAL to the unchained arm's** report
  rather than against a literal, because "exactly as if it had not been chained" is what §2 states.
  Reverting the predicate fails 5 tests, every one on a FLOOR assertion.
- `count: n>0` → **UNCHANGED**: covered, believed all-pure, exit 0, no hedge, no advisory. Keying on
  emptiness instead fails 4 tests, every one on a CONTROL assertion — **while the count-0 rows stay
  green**, which is what a fix that had "worked" looks like from the floor arm alone.
- Removing the anchor branch, or the ledger conjunct, each fails 4 — including the structural test, which
  is the one that would otherwise let the two halves drift apart.

**SPEC §2's THIRD ROW retires a pre-⟨0.21⟩ affordance, recorded rather than smuggled.** A manifest-less
empty report DID buy coverage, and `kappa_ledger_honors_an_empty_chained_report_as_coverage` pinned exactly
that. Its subject — the envelope `package` field carrying coverage independent of the filename — is
unchanged; it is RE-POINTED at a manifest-bearing fixture, with the count-0 and manifest-less forms added
beside it as their own rows, which is what the spec clause asks for.

**CONSERVATIVE ON CONFLICT, and a deliberate divergence from candor-swift.** A crate chained once as judged
and once as judged-nothing keeps the hedge here; swift subtracts so the real report wins. Swift's argument
is good (a count-0 report makes no claim in either direction), but rust's index DROPS a key two dep entries
disagree under, so granting coverage on one report's authority can make the very key that mattered resolve
to nothing and read confidently pure — the same reasoning `63bbe87` recorded for fresh-vs-stale. The cost
of being wrong this way is one extra hedge; the other way it is a false all-clear.

**⟨0.24⟩ THE SAME RULE BINDS `gate --report` (SPEC §3.1)** — "the obligation is on the reading, not on the
route by which the report arrived". Measured: the verb printed `policy ✓` and exited 0 over a count-0
report, byte-identical to the legitimately-all-pure case. It now **REFUSES (exit 2) and writes no verdict**
— §3.3's exit-2 cause (a), the gate could not be evaluated at all, so an `ok` either way would be a guess.
(Cause (b)'s machine-legible `incomplete` verdict is keyed to `unanalyzed`, a NAMED list of source the
producer could not read; a count-0 report names nothing.) It lands beside the verb's three existing
refusals, which refuse for the same reason. The predicate itself lives in `candor-report`, the one crate
both routes depend on — a rule written twice is a rule that can drift between its routes.

**BLAST RADIUS, real chained trees.** 17 of candor-rust's own 173 dep reports (9.8%) say `count: 0`, 27 of
ebman's 409 (6.6%), 20 of pgman's 270 (7.4%) — every one a macro-only, platform-link-stub, data-blob or
re-export-only crate (`cfg_if`, `windows_*_msvc`, `icu_*_data`, `stable_deref_trait`, `pin_utils`,
`static_assertions`). None is manifest-less. Against them stand 16 / 49 / 39 legitimately all-pure reports
(9.2% / 12.0% / 14.4%) that an emptiness-keyed fix would have hedged. On candor-rust's own chained scan the
REPORTS are **identical before and after** — none of the 17 is a crate this code demonstrably calls — so
the whole live effect is one new stderr advisory naming all 17. A rare-facade catch, not a rule that turns
a dep tree uncovered.

**PART 26**, `rust/empty_zero`: 64 live cells move from `A` (ABSENT — a silent purity claim) to `h`
(HEDGED_LOSS, the correct §2.1 shape), leaving **0 failing cells**, and

    rust     SEPARATED on 64/80 cells — the engine distinguishes them

where it read `INDISTINGUISHABLE — the engine is not reading analyzed.count`. The `rust/empty_zero` waiver
in candor-spec's ratchet baseline is now fully STALE (the harness says so: "every cell now passes") and
should be DELETED rather than narrowed — that is a candor-spec edit and is left alone here.

### ⟨0.24⟩ `candor-query gate --report <locator> --policy <file>` — apply a policy to an EXISTING report

SPEC §3.1 ⟨0.24⟩ makes this a MUST and rust did not have it: conformance PART 27's R6 row printed
`NOSURF` for this engine, `NOSURF` does not fail the run, and the 0.24 CHANGELOG entry in candor-spec had
to be publicly corrected to "pinned 2-of-4" because of it. It is the supply-chain verb — gating a
dependency's PUBLISHED report is the operation an adopter actually wants and could not previously express
without re-analysing code they do not have.

**The reason it is a MUST rather than a convenience is that it makes the code-implements-spec direction
testable at all.** `candor-scan --policy` recomputes the effect set from source, so the classifier is
always in the loop; `whatif` reports only what a hypothetical INTRODUCES. So the gate had never been
reachable as a function of a GIVEN signature, and a defect in the GATE and a defect in the CLASSIFIER were
indistinguishable from any test that could be written here.

**THE SEAM.** The §6.2 matching moved into `candor_classify::gate::gate(&ParsedPolicy, &GateInput)` — one
copy, two routes in. `candor-scan --policy`'s `policy_violations` is now a thin adapter that builds a
`GateInput` from the classifier's fixpoints; `candor-query gate --report` builds one from a written report
and nothing else. `net_classes_of` moved with it, because the report FIELD and the gate FILTER have to be
the same set.

**A NEW READER WAS NEEDED, and that is not an accident of this codebase.** Every existing loader is built
to ENRICH — the `.callgraph.json` sidecar, the type hierarchy, chained deps — and this verb has to read
strictly LESS than any of them, which is not a subset reachable by passing a flag. (candor-swift reported
the same thing from its own tree.) `load_entries` alone would not do either: it returns the `functions`
array and drops the §2 envelope the verdict is written from.

**THE MUST NOT: an ABSENT entry is absent.** No callgraph sidecar, no chained dep, no `.candor/config`
`deps` key, no re-classification of `hosts`/`netClass` through this machine's `net-partner` list. Proved
with all three back-fill channels open at once over an entry that is not in the report — `deny Fs` exits
0 — beside the negative control that exits 1 when the same effect is written INTO the report. Both arms
mutation-verified; without the control, "did not back-fill" and "never evaluated" are the same green.

**ANSWERABILITY: a rule whose evidence the wire does not carry is REFUSED (exit 2), never evaluated** —
each of the three is fail-OPEN if approximated. `forbid A -> B` and `allow <E> …` are refused
whole-policy (enforcing the answerable half and exiting 0 is gateless-green); a class-scoped `deny` whose
scoping datum is an absent optional field is refused per (rule, function), so a scoped rule whose own
matches carry their evidence still evaluates. The refusal is MINIMAL: because the class set only GROWS and
`Reject` is upward-closed, a scoped rule whose determinable classes are already non-empty is ANSWERED —
including the ⟨0.24⟩ CONTRIBUTES counterexample (a reasonless DIRECT `Unknown` under
`deny E Unknown[unresolved]`), which contributes its class from the entry alone and therefore fires.

**EQUIVALENCE IS THE ACCEPTANCE TEST AND IT IS BYTE-LEVEL** — `analyzed.count`, `reasonClass`, `netClass`
and the coverage advisory included. Measured over **90 rows**: 30 policies × three corpora (ebman, pgman,
and this workspace's own five members), 55 of them with violations. `ci/gate-equivalence.sh` keeps 48 of
those rows as a standing CI gate plus a 49th for the arm no in-tree crate can reach — a scan whose own
analysis was INCOMPLETE, where both routes must exit 2 and write the same ⟨0.21⟩ `incomplete:true`
verdict — and it FAILS when no policy in its matrix fires: byte-equal empty verdicts prove nothing.

### fix — the scan gate double-counted a violation on two `#[cfg]`-gated units sharing one name

FOUND BY the equivalence obligation above, on 15 of the first 90 rows. `#[cfg(unix)] fn f` beside
`#[cfg(not(unix))] fn f` puts the qualified name in the gate's function list TWICE while `inferred` holds
ONE merged signature — so the gate emitted two byte-identical `GateViolation` records, an inflated
`N policy violation(s)` count, and a `--gate-json` document a consumer reads as two findings. The report
route cannot reproduce it (a report is keyed by name), which is exactly how it surfaced. The gate now
answers once per (rule, function); the report is unchanged and still lists both units.

### model cross-check — the engine now agrees with `reference/policy_model.py` directly

The verb's other purpose. 2,949,120 rows: 30 policies (`deny e`, `deny e Unknown[C]`, `pure`) over all
**98,304 REACHABLE** signatures of the (S, D) lattice, fed to the shipped binary as one report and
compared against the model's `Reject`. **Zero disagreements.** The domain is `reachable_lattice()`, not
`full_lattice()`: every engine co-emits `Llm` with `Net`, so `Llm ∈ S ∧ Net ∉ S` names 32,768 points none
can produce, and the unrestricted run reports them as 40 phantom `deny Net` families — which is the
negative control proving the differential discriminates at all.

### usage ⟨0.24⟩ — a `--class` value that cannot be honoured is now REFUSED, not quietly narrowed

SPEC §6.2's value grammar, which conformance PART 27 found unimplemented in **all four** engines rather
than divergent between them — the suite's only `engine: "*"` waiver, and the last thing holding the floor
below 0.24. `--class <c>[,<c>…]` takes ONE comma-separated list of the six reason classes plus the two
aliases `dynamic` and `*`. Two things that used to exit 0 now exit 2, on **both** verbs that take the
flag (`unverified --class` and `blindspots --class`):

- **an unrecognised token.** `--class dyanmic` used to print `--class ignores unknown reason-class …`
  and carry on with whatever was left of the list — for a single-token list, an EMPTY filter. It now
  names the token and lists the accepted set.
- **a repeated `--class`.** It was last-wins, silently discarding the first list. A second occurrence is
  a usage error, not a union — and not last-wins either. Both silent readings answer a different question
  than the line on screen.

**Why this is not the policy side's drop-with-a-warning, since the asymmetry looks like an inconsistency
until you write it down.** A token dropped out of `deny E Unknown[reflect,dyanmic]` leaves the WIDER rule
standing: the mistake surfaces as a gate that over-fires, and someone comes to look. The same token
dropped out of `--class` leaves a NARROWER filter, and a narrower filter on `unverified` comes back as a
SMALLER NUMBER — which is indistinguishable from a real all-clear, in the one verb whose entire job is to
say "green, but not provably so". That is the fail-open the surrounding §6.2 clause exists to close, and
it is why the query side refuses what the policy side approximates.

The token rule lives in ONE place (`parse_class_filter`, now `Result`-returning) so the two verbs cannot
drift, and `*` is evaluated after the whole list is walked — `--class *,dyanmic` still reports the typo
rather than short-circuiting past it. The refusal emits **no answer document at all**; a narrower result
one exit code away from a refusal is the same fail-open wearing a different hat. Nothing changes for
well-formed input: the tests pin the unfiltered baseline and each filter's exact selection, on both verbs.

### soundness — three silent under-reports, all three found by ONE SPELLING never being tried

Found by conformance PART 24 (split-invariance) on its first run, then each re-derived from a
hand-written fixture so none rests on the generator. The common shape: **the answer was already under
the right key and the consumer's join had a branch that did not fire.** No report-format change was
needed for any of the three — the third time in a row this vein has come out that way.

1. **A chained dependency's lazy static was charged only through a PATH-QUALIFIED read.**
   `deplib::CFG.len()` → `['Env']`; `use deplib::CFG; CFG.len()` → silent-pure. Deref vs method call made
   no difference — the `use` did, because the import leaves a ONE-segment path behind and the branch
   required two. Conformance PART 19's rust fixture uses the qualified spelling. Fixed by consulting the
   file- **and body-level** `use` maps, and by asking for the dependency's own MODULE-qualified key
   (`<lazy>::cfg::MODC`) as well as the crate-root one — a dep static declared inside a module was
   unreachable under *either* spelling.
2. **A chained dependency FACTORY call with no intermediate binding read silent-pure.**
   `let c = deplib::build(); c.fetch()` resolved; `deplib::build().fetch()` was silent, because the
   provenance that drives the disclosure is only ever written by a `let`. A hole in a shipped guard, not
   an un-attempted precision gap: it broke that guard's own ruling that a key which could not be formed
   must never read pure. The unbound receiver now takes the same route, including through `?`, `.await`
   and `&` — and `visit_local` peels those too, so eliding the binding cannot change the answer either way.
3. **A lazy static read from OUTSIDE its module was not charged at all — single tree, no boundary.**
   `m::inside()` was charged; `fn outside() { let _ = *m::INNER; }` read pure.

**(3) is a regression introduced by `5447eba`, and the measurement is unambiguous.** That commit moved
the module path INSIDE the `<lazy>::` prefix so two same-named statics stop merging — which made the
WRITER module-qualified while the READER still built `<lazy>::<its own module>::NAME`. Three-way on one
fixture (a crate-root lazy static read from a submodule, the ordinary `use crate::ROOT_CFG;` shape):

| | `root_read` (same module) | 4 cross-module spellings |
|---|---|---|
| before `5447eba` (`c0a142c`) | `Fs` | **`Fs`** |
| at `5df4af1` | `Fs` | **PURE** |
| after this change | `Fs` | `Fs` |

A fabrication fix that introduced a cardinal sin, and much wider than the fixture that caught it: at
`5df4af1` *any* crate-root lazy static read from *any* submodule read pure. The identity property
`5447eba` bought is kept — its own test still passes, and the control that two modules' same-named
`CFG`s do not cross still holds — because the reader now takes the module the SPELLING names and keeps
its own-module key beside it, both filtered by `resolve_target`'s uniqueness rule.

**The mirror control caught a live fabrication in the first cut of fix (1)**, which is the reason it was
written: the five typed side-tables that answer "is this name shadowed by a local?" only hold bindings
whose type was RECOVERABLE, so `use deplib::C; … let C = "aa"; C.len()` charged the dependency's `Env`
to a local string. Harmless while only a qualified path could force (a shadow is spelled bare); live the
moment the bare spelling was added. `bound_idents` now collects every ident in a binding position,
typed or not. Mutation-verified in both directions, along with the module-discrimination control (pin
the derived module and the reader of the pure `b::CFG` picks up `a::CFG`'s `Fs`), the local-module
control (held independently by TWO guards — measured by removing each), and the per-static keying
control (make the dependency's pure lazy static effectful and its reader lights up).

**A/B, before/after, each binary producing its own dependency tree:**

| target | fns | gains, `Unknown` only | gains, CONCRETE effect | losses |
|---|---|---|---|---|
| ebman `--deps` | 546 → 550 | 95 | **0** | **0** |
| pgman `--deps` | 200 → 205 | 21 | **0** | **0** |
| candor-rust workspace `--deps` (5 members) | 223 → 230 | 32 | **0** | **0** |
| tb-tui-common | 9 → 9 | 0 | **0** | **0** |

Every gain on real code is (2)'s disclosure, never a concrete effect: functions carrying a direct
`dispatch:untyped cross-package receiver` go 18 → 52 on ebman, 2 → 18 on pgman, 0 → 31 on candor-rust.
**That is a large number and it is stated as one** — 95 of 550 ebman functions newly carry `Unknown`,
about 2.9× the shipped bound-spelling arm's own footprint.

**Every one of them is on a CHAINED dependency, and that was measured rather than argued.** The rung's
THIRD conjunct — the dependency must be chained, because for an unchained one the κ ledger already says
`invisible: [pkg]` and a second hedge is pure false uncertainty — lives on the shared CONSUMPTION path
in `scan.rs`, and this change is emission-side only, so the new arm inherits it. Instrumented at the
marker, *before* the gate:

| | markers | CHAINED | UNCHAINED (suppressed) |
|---|---|---|---|
| ebman alone, chained over its 410-crate dep tree | 53 → **141** | 32 → 73 | 21 → **68** |
| the whole dep-tree walk | 53 → **22,131** | 32 → 73 | 21 → **22,058** |

The conjunct suppresses **99.7%** of the new arm's markers, and the crate heads it suppresses are
exactly what it was written for: `std` (52), `String` (7), and local modules (`app`, `crate`, `eb_cli`,
`project`, `cost_cache`, …) — not one of them a real dependency. End-to-end attribution of the 95:
newly-direct functions with no chained marker **0**; backed only by an unchained marker **0**; of the 28
functions whose only untyped marker was unchained, **0** gained anything. The responsible crates are
chrono 21, serde_yml 7, futures 2, tokio 2, toml 1, tracing_subscriber 1, and all eight crates carrying
a chained marker have substantial reports (chrono 191 fns / 45 published return types; serde_yml 337/24;
tokio 818/114). The number is real and the disclosures are honest.

**The lever on it is DETERMINATION, not suppression** — the ⟨0.24⟩ ordering, and it applies to both
spellings identically so it cannot disturb the parity above. Of ebman's 73 chained markers, 10 resolve
today, 6 miss only because the dependency publishes its key MODULE-qualified where the consumer forms it
from the written path, and 57 are genuinely absent from the published surface. **37 of those 57 are
chrono's `Utc::now`, and the reason it is unpublished is a spurious collision**: chrono declares
`pub fn now() -> DateTime<Utc>` TWICE under mutually exclusive `#[cfg]`s (native and wasm32), the scan
walks both cfg arms by design, and the return index's never-guess-between-two-same-named-defs rule drops
the entry — even though **both candidates name the same return type**, so there is nothing to guess
between. Publishing a collision whose candidates AGREE would recover a little over half of ebman's
chained untyped markers. Recorded as a lead, not taken here.

The lazy halves gained nothing on real code, which the diff alone cannot distinguish from never firing,
so the precondition was instrumented instead: the `use`-spelling dependency marker is emitted 28,938 /
19,579 / 6,955 times across the three dependency trees, and the cross-module read fires 16 times on
ebman's (aws-config's `DEFAULT_PARTITION_RESOLVER`, declared in `endpoint_lib` and read from
`config::endpoint`) and once on pgman's (tracing-subscriber's `FILTERING`). Real shapes; those
particular initializers are pure, which is per-static keying working rather than the fix not landing.

Conformance PART 24's ratchet baseline is now EMPTY for rust (both entries deleted — a waiver that
outlives its defect masks the defect's return), and PARTs 18–24 are green four-way.

### soundness — `unverified --class` under-reported holes, and under-reported MORE the more you narrowed

`unverified` names the functions a `pure` / `deny E` layer PASSES without proving anything — the verb
whose whole job is "this gate is green but not provably so". Its `--class <c,…>` filter dropped holes it
was built to surface. Two independent faults, both live, both fixed here (fixing either alone is worse
than fixing neither — see below):

1. **It read the DIRECT-only field.** `unknownWhy` is direct-only by design (SPEC §4: a reason names an
   unresolvable site in the function's *own* body), so a function whose `Unknown` is purely INHERITED
   from a callee carries no reason of its own. The predicate `unknownWhy ∩ filter ≠ ∅` therefore excluded
   every inherited hole from *every* filter, including one naming the class the callee recorded. The
   `deny E Unknown[class]` gate has always resolved this transitively (`reason_class_acc`); the
   disclosure explaining that gate did not.
2. **It failed OPEN under absence.** An entry with an empty reason set matched no filter at all — not
   even `unresolved`. §6.2 says such a function CONTRIBUTES `unresolved`; it now does, per entry, into
   the direct map so the class propagates to callers like any other.

Measured with the §6.2 diagnostic — `--class dynamic` is an alias naming every genuine class, so it must
exclude NOTHING; a filtered count below the unfiltered one is the defect and the gap is its size:

| target | policy | before | after |
|---|---|---|---|
| candor-rust (chained `--deps`) | `deny Exec` | 54 → **26** (−52%) | 54 → 54 |
| candor-rust (chained `--deps`) | self-gate `deny Net Db Exec Ipc` | 54 → **26** (−52%) | 54 → 54 |
| candor-scan (own sources) | `deny Exec` | 7 → **1** (−86%) | 7 → 7 |
| candor-scan (own sources) | `deny Net Db Exec Ipc` | 7 → **1** (−86%) | 7 → 7 |
| ebman | `deny Exec` | 94 → **23** (−76%) | 94 → 94 |
| ebman | `deny Net Db Exec Ipc` | 74 → **22** (−70%) | 74 → 74 |
| pgman | `deny Exec` | 43 → **21** (−51%) | 43 → 43 |
| pgman | `deny Net Db Exec Ipc` | 36 → **19** (−47%) | 36 → 36 |

All eight target × policy rows converge exactly after the fix, matching the shape candor-swift measured
(387 → 230 and 51 → 16 before; both exact after).

Fault 1 was the bulk of it: 101 of 124 `Unknown`-bearing entries on ebman and 37 of 60 on pgman carry no
direct reason at all.

**The filter still DISCRIMINATES** — the control a blanket "keep everything" would fail. Post-fix on the
same rows, `--class unresolved` selects 0, `--class native` 0, `--class reflect` 0, while
`indirect`/`dispatch` partition the totals (ebman 64/37 of 94).

**Fixing only fault 2 would have been worse than fixing neither.** Contributing `unresolved` on the
absence of a reason set fabricates a class for an inherited `Unknown` that the callee classified
perfectly well — a fail-open traded for its mirror. The contribution is therefore gated on `direct ∋
Unknown` with nothing named (the unit INTRODUCED the hole and did not say why), which rust's reports
carry verbatim (§2 `direct`). Both directions are pinned by mutation: dropping the gate turns exactly one
control red ("an inherited, CLASSIFIED hole is not `unresolved`") and nothing else.

`blindspots --class` is deliberately UNCHANGED. It is the *source* view (§3.1) and excludes a unit whose
`Unknown` is purely inherited, so every entry it filters carries a direct reason by construction and the
direct-only read is CORRECT there — resolving transitively would pull in exactly the units the verb is
defined to exclude. Measured to confirm rather than assumed: unfiltered vs `--class dynamic` is 0→0,
1→1, 23→23, 23→23, 26→26 across the five targets. A shared code path is not a shared defect. Checked
too, since §4 forbids it: no rust report can put a reasonless entry into `sources` (the loop skips
`unknownWhy`-empty entries outright), and across 17,306 entries in 173 dependency reports plus this
repo's own five, **zero** entries carry a direct `Unknown` with no reason recorded beside it.

The `propagate_str` fixpoint moved to `candor-classify` and the §6.2 match rule became
`policy::reason_class_matches`, so the gate and the disclosure that explains it now resolve over one
implementation of the reach — two fixpoints that can drift apart is its own defect. Dependency reports
re-scan byte-identical.

### disclosure — the report glob claimed a SIDECAR, then reported its own mistake as data loss

Every query run against a prefix with a `<prefix>.<pkg>.hierarchy.json` sidecar beside it printed:

```
candor: report …/r.app.hierarchy.json failed to parse — its functions are OMITTED from this
query (corrupt or mid-write); re-run the scan
```

The sidecar was not corrupt, nothing was omitted, the results were correct, and the scan the user was
told to re-run was fine. `report_files` discriminated reports from sidecars by SEGMENT COUNT alone —
`<base>.<crate>.<type>.json`, exactly two segments — which excludes this engine's own sidecars (all
three-segment: `<base>.<crate>.<type>.callgraph.json`) but not a two-segment one from another
producer. SPEC §2.2 lets each engine pair a sidecar to its OWN report stem, so `<base>.<pkg>.hierarchy.json`
is a legitimate name that lands exactly on the `<crate>.<type>` shape. Two globs inside one binary
disagreed about the same file: `load_hierarchy` read it as a sidecar while `report_files` claimed it as
a report.

Measured blast radius — 10 verbs (`show`, `where`, `map`, `path`, `impact`, `reachable`, `blindspots`,
`tour`, `containment`, `callers --include-unknown`) plus `audit`, `receipt`, `diff`, `gains`, and the
lint's own cross-crate sibling loader. Both `.hierarchy.json` and `.callgraph.json` trigger it.

**It was not purely cosmetic.** Three consequences, each measured against a control run with the
sidecar removed:

- an **effect-free crate was refused outright**. The bogus parse failure set the `hard_fail` bit that
  `load_entries_loud` uses to tell "this crate genuinely has no effects" from "every report was
  corrupt", so a well-formed `"functions": []` report standing beside a sidecar exited **2** with
  *"refusing to report an empty (all-clear) answer over a corrupt report"*. The query answered nothing.
- **provenance was lost**: `report_build_version` reads the FIRST report by sorted path, and
  `r.app.hierarchy.json` sorts before `r.demo.scan.json`, so `gains --json` reported
  `baseline_version: ""` / `engine_version: ""` where the control gave the real build id — which also
  silences the §2.1 producing-build mismatch disclosure.
- `reports <prefix>` — the canonical "what counts as a report" oracle the wrapper's `--exists` check
  joins on — **listed the sidecars as reports**.

Fixed at the GLOB, not at the parse: `candor_report::SIDECAR_KINDS` names the reserved trailing
segments (`callgraph`, `hierarchy`, `calibrated`, `layerreach`, `locs`, `gate`) and `report_files`
excludes a `<type>` that is one of them, so a sidecar never enters the candidate set and there is
nothing left to diagnose. Suppressing the *message* would have left the file in the set, left all three
consequences above live, and been one refactor from returning.

This is a **denylist over `<type>`, deliberately**. The accept rule still takes any
`<base>.<a>.<b>.json`; only names that are provably not crate types are carved out (`<type>` is a
rustc `CrateType` or another engine's `Swift`/`jar`, never an artifact name). The allowlist inversion —
accept only known types — would make any report whose type segment we failed to anticipate silently
invisible to every query, a false all-clear. A denylist can only be *incomplete*, and incompleteness
here is LOUD: an unregistered sidecar suffix falls back into the candidate set and prints the
disclosure on every query. Noise, never a swallowed report. A crate legitimately NAMED `hierarchy` is
untouched — it sits in the `<crate>` position — and that is pinned as a test row.

The control that makes the fix meaningful: with the same sidecars present, a genuinely corrupt REPORT
must still be disclosed, must name the report and not a sidecar, and must still exit 2. Verified to
discriminate — with the exclusion disabled both tests go red; with the *wrong* fix (the `eprintln!`
deleted) the quiet test passes and only the control fails.

Engine-local. candor-ts (`query-core.mjs` `isReport`), candor-java (`Query.java`) and candor-swift
(`FixCLI.swift`) already exclude these suffixes by name; candor-rust was the one engine discriminating
by segment count.

### soundness ⟨0.24⟩ — the dispatch frontier silently dropped this engine's own dominant reason

`callers --include-unknown` built `possibleViaUnknownDispatch` by splitting each `dispatch:` detail into
an owner and a member on the LAST DOT, then asking condition (3): is some confirmed reacher an override
of `OWNER.M`? candor-scan emits `dispatch:untyped cross-package receiver` when it cannot type the
receiver of a call into a chained dependency — a DOT-FREE detail, and in a 1062-report census EVERY
dispatch reason on this engine was that form. With no dot, `simple_method`/`declaring_type` both fall
back to the whole string, so the lookup could never hit and the entry was **dropped with no diagnostic**.

Measured: a report where `mod.Caller.run` carries `dispatch:mod.Base.handle` and `mod.Dotfree.run`
carries `dispatch:untyped cross-package receiver` produced a frontier containing only `mod.Caller.run`,
in **both** the hierarchy arm and the no-hierarchy fallback arm. That omission is a false all-clear: a
consumer reads absence from `possibleViaUnknownDispatch` as "no function may reach the target through an
unresolved dispatch", which is exactly the claim the engine is not entitled to make.

A dot-free detail names no owner and no member, so condition (3) is **unanswerable — and an unanswerable
condition must not be scored as a failed one**. Such an entry is now disclosed with `viaDispatchOn` set
to the raw detail verbatim. This is the direction the no-hierarchy fallback already takes one rung up
(no sidecar → the subtype test is unanswerable → over-list rather than drop); the frontier over-lists by
construction and asserts nothing into `transitive`, so a spurious entry costs precision while a dropped
one costs the answer. Detected structurally (the detail contains no `.`), never by matching the
scanner's wording — an allowlist of known reason strings silently drops the ones it forgets.

The same whole-string fallback also produced the mirror defect, now closed: a dot-free detail that
happened to equal a confirmed reacher's dot-free Rust qual (`dispatch:app::Sub::handle`) MATCHED, with
the subtype check passing only by reflexivity over a string that is not a type name; and
`dispatch:handle` was disclosed with no hierarchy but dropped with one. Every dot-free detail now takes
the same branch, before the reacher index is consulted at all, so the answer no longer depends on
whether a sidecar happens to exist.

SPEC §2 chaining rule 3 turns a report's SILENCE into a purity claim, and registering its crate as
COVERED is exactly what silences the κ ledger's `invisible` hedge so the silence can be read that
way. A chained report carrying a non-empty ⟨0.21⟩ `unanalyzed` has just said it never read some of
its own source — and candor-scan granted it full coverage anyway, so chaining it was strictly WORSE
than not chaining it: the dependency's own gate refuses to certify itself over unanalyzed code
(`--gate-json` exits 2 for precisely this) and the consumer certified one on its behalf.

Live on crates.io code: `signal-hook-registry` 1.4.8's whole `src/lib.rs` fails to parse, so its
report carries **2 functions and an `unanalyzed` manifest naming the library itself**. Chained,
`signal-hook`'s `PendingSignals::add_signal` — which calls `signal_hook_registry::register_sigaction`,
i.e. installs a signal handler — read as a confident purity claim about that crate. It now carries
`invisible: ['signal_hook_registry']`, and the crate appears in the coverage ledger.

**The treatment differs from staleness, and the difference is the whole point.** A stale report's
entries are assertions from a build this engine will not repeat, so they are downgraded to `Unknown`.
An incomplete report's entries were derived from source it DID read and are true, so they are kept
exactly as they are — effects, literal surfaces, reason classes and all — and only COVERAGE is
withheld. An answered key still answers; only an unanswered one falls back to the hedge. Absent or
explicitly empty `unanalyzed` means COMPLETE (the writer omits the key when the manifest is empty);
anything else, malformed included, fails closed. Announced on stderr, and in `--help`.

Ported from candor-ts `21277eb` (java `d1d3045`, swift `74cd8f1`) — rust was the last engine gating
coverage on staleness alone.

### soundness — the IMPLICIT-STRINGIFICATION vein closed in BOTH backends (cardinal sin)

A formatting site runs the formatted value's `Display`/`Debug` impl. candor analysed those impls
correctly but never edged to them **when the value's type was not a concrete local ADT** — so
`fn describe<T: Display>(e: T) -> String { format!("{e}") }` read **silent-pure** even though it runs
`<T as Display>::fmt`, and in the deep engine even a fully concrete caller (`describe(Loud)`) was a
**false all-clear**. This is the four-way common-mode vein recorded in
`candor-spec/SOUNDNESS-VEIN-implicit-stringify.md` (found on HikariCP by the RQ1 runtime oracle).

Closed on both backends by extending the existing implicit-coercion machinery — no parallel mechanism:

- **candor-scan**: `charge_coercion` now falls through to bounded CHA over the BOUND, for the stringify
  family only (`Display`/`Debug`, plus `ToString` as Display's blanket alias) — a `T: Display` /
  `impl Display` / `&dyn Display` operand, or a LOCAL trait that inherits the formatter as a supertrait
  (`trait Entry: Display`, the narrow precise case). ≤12 local implementors → edges to each `Ty::fmt`;
  wider → honest `Unknown`; **no local implementor → nothing** (no edge, no flood). Every other
  coercion (operators, `Deref`, `Index`, the `write!` writer side) is untouched.
- **candor-scan**: NAMED and INLINE-CAPTURED format holes (`format!("{val}")`, `format!("{v}", v = x)`)
  were skipped outright — they now charge the same coercion. This alone recovered a genuine miss on
  third-party code: `cargo-llvm-cov`'s `ProcessBuilder::run`/`read`/`run_with_output` reported `[Exec]`
  while `format!("… {self}")` runs a `Display` impl that reads the environment → now `[Env, Exec]`.
- **deep engine**: `fmt_argument_local_edge` resolved only a local ADT; a `Param`/`dyn` formatted value
  now takes the same bounded CHA (`fmt_trait_local_cha`), and the generic `.to_string()` spelling is
  covered by `generic_to_string_edge`. Beyond the bound it discloses `Unknown` under a `dispatch:`
  reason class (spec 0.19).

**A/B, zero fabrication.** candor-scan over the whole local registry — **962 crates / 470,971 analyzed
functions**: **0 functions gained a concrete effect** other than the 4 genuine `cargo-llvm-cov`
recoveries, **0 functions lost anything**, 93 gained an honest `Unknown` (0.020%, all at real generic
format sites in formatting-heavy crates: tracing-subscriber, chrono, color-eyre, clap, tokio, winnow,
rustls…). Deep engine over 6 real crates: 0 concrete gains, 0 losses, 6 `Unknown` (all chrono).

## 0.23.1 (spec 0.23) — 2026-07-20

### performance — O(V²) propagation fixpoint replaced with a worklist (no output change)

The transitive effect-propagation fixpoint (`propagate`/`propagate_str`) used a naive
`while changed { for f in all }` sweep whose pass count equals the longest back-to-front call chain — up to
V for a single deep `f0→f1→…→fN` chain, so **O(V²)** on pathological long chains (real crates converge in
2–4 passes, so it never bit realistic code — `wide-4000` is unchanged). Replaced with a worklist over a
callee→callers reverse index: a function is reprocessed only when a callee actually gained an effect. Same
monotone set-union least fixed point → order-independent → **output byte-for-byte identical** (verified via
`--json` + full stdout/stderr across synthetic + real crates; `cargo test --workspace` 334 green, clippy
clean). ~3× on deep-chain corpora; realistic corpora unchanged.

## spec 0.19 — reason-scoped Unknown (2026-07-17) — current floor

Reason-scoped `Unknown` policies (SPEC §6.2): `deny E Unknown[reflect,dispatch,indirect,native,unresolved,setup]`
narrows the `Unknown` part of a deny to a fixed reason-class vocabulary projecting the §4 `unknownWhy` reasons,
with the built-in `dynamic`/`*` aliases and config-defined `.candor/config` `unknown-alias <name> = <class…>`
names. Bare `deny E Unknown` is unchanged (`Unknown[*]`, fires on any); an unrecognized reason maps to
`unresolved` (conservative); the reason class propagates **transitively** along the call graph like the effect.
An AS-EFF-006 `--gate-json` verdict whose `effects` include `Unknown` now carries a **`reasonClass`** array (all
classes on the fn). Report bytes unchanged. Conformance PART 4 (parse + `unknownClasses` + config alias) and
PART 12 (`reasonClass` invariant) pin it four-way.

## spec 0.18 — the trust-trio (2026-07-16)

candor-scan and candor-query now declare **spec `0.18`** (both at crate **0.18.0**). A pinned-tool-surface
rung — no report-schema or verdict change — closing three ways the tool could quietly mislead, each pinned
four-way in the conformance suite:

- **`--strict` advisory-verb CI gate** (§3.3.1): `fix-gate`, `gains`, and `unverified` are advisory (exit 0);
  `--strict` makes each a CI gate (exit 1 while a finding remains). A typo'd flag is rejected loud (exit 2),
  never swallowed into a disarmed gate; `gains` has no `--policy` (a passed one is an exit-2 error naming the
  scan-time `deny <E> gained` gate, `AS-EFF-005`).
- **mostly-Unknown disclosure**: the scan opener and `tour` never print "nothing hidden" over a ≥⅓-Unknown
  graph — they qualify + point at `blindspots`; `tour --json` carries an additive `unknown: {count, total}`.
- **hardening from a Fable-model code review**: gains rejects single-dash typos + tolerates cross-engine
  `--text`; a valueless `--policy` exits 2.

## spec 0.17 — the callgraph-aware baseline guard (2026-07-16)

candor-scan and candor-query now declare **spec `0.16`** (both at crate **0.16.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.16.0**). **0.16 is the current spec
floor** — the ratchet from 0.15. It sharpens the scan-time baseline ratchet so the hardest
supply-chain shape can no longer slip through as "new code".

### 🕸️ callgraph-aware baseline guard ⟨0.16⟩ — pure→effectful is caught

The scan-time ratchet (`candor-scan --gate --baseline`) now keys function EXISTENCE on the baseline
**callgraph sidecar** when present (the resolved report path with `.json` swapped for
`.callgraph.json`; SPEC §2.2 records every analyzed fn, including PURE leaves that reports omit). A fn
that is a baseline callgraph node — even a **baseline-pure leaf** with an empty effect set — that now
performs ANY effect is a **GAIN violation** (exit 1). This closes the report-only blind spot where a
**formerly-pure function turning effectful** was absent from the baseline report and so read as exempt
"new code". A fn genuinely absent from the callgraph stays exempt (real new code). This is the `gains`
`origin` existence rule (§3.1 ⟨0.12⟩) applied to the scan ratchet. When the sidecar is **absent** the
guard degrades to the pre-0.16 report-only existence (with a one-time stderr note that it is weaker);
a **corrupt** sidecar is `Invalid` (exit 2), never a silent narrowing.

### ⚠️ Unknown-only gain is advisory, not exit 1 ⟨0.16⟩

A baseline→current gain consisting **only of `Unknown`** (no real §6 boundary effect gained) is now an
**advisory**, not a hard violation: the ratchet fires only on gaining a REAL boundary effect. An
Unknown-only widening surfaces as a disclosed advisory (verdict-preserving), keeping the gate focused
on genuine supply-chain effect gains rather than resolution noise.

## spec 0.15 — the coverage envelope (2026-07-15)

candor-scan and candor-query now declare **spec `0.15`** (both at crate **0.15.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.15.0**). **0.15 is the current spec
floor** — the ratchet from 0.14, and unlike 0.14 it is a **feature rung for this engine**: the
coverage envelope, two host-resolution recall upgrades, and three soundness fixes from real-world
corpus testing.

### 📦 the coverage envelope ⟨0.15⟩ — the κ ledger travels WITH the report

The κ-coverage ledger is no longer stderr-only: a scan with an uncovered dependency now emits the
§2 **`coverage` envelope field** — `{"uncovered":[{"name","calls"}]}` — **omitted when empty**, so a
fully-covered scan's report stays **byte-identical** with a ⟨0.14⟩ one (the wire-compatibility rule).
One shared ledger computation feeds all three surfaces: the stderr line (bytes unchanged), the
envelope, and the **`--gate-json` coverage advisory** (`{"uncovered":N,"packages":[…]}`) — which is
**verdict-preserving**: `ok` / `violations` / the exit code are untouched (the ⟨0.9⟩ advisory
precedent). `gains --json` re-discloses the current ledger and adds **`coverageDelta`**
(`{nowUncovered, noLongerUncovered}`, names-only — the java reference shape); the TSV output is
untouched. `candor-query gate-verdict` gains an optional `--report` as the advisory source. Pinned by
conformance PART 4s.

### 🔎 host-resolution recall — statically-known hosts

Two upgrades let a **statically-known host** run the §1 Llm/Db/Net host refinement exactly like an
inline literal (previously bare `Net`):

- **const-string propagation** — a URL/host built from a literal-valued `const`/`static`
  (`const API_BASE = "…"; format!("{}/x", API_BASE)` — interpolation, bare ref, or const-left
  concat), matching candor-java's inlined static-final String. The const index is folded into the
  decl-cache digest, so incremental scans stay correct. PART 4q.
- **literal-head extraction** — a `format!` whose literal completes the authority before the first
  placeholder (`format!("https://api.openai.com/v1/{}", p)`).

Both keep the conservative boundary: a split authority, whole-host placeholder, runtime/config host,
or plain variable stays bare `Net` — no fabrication. PARTs 4q/4r.

### 🧯 soundness fixes — real-world corpus testing finds

Three silent-drop classes found by real-world corpus testing, each recovering real effects with
**zero fabrication** (the 1337-crate realworld-oracle stays green; clap/ripgrep byte-identical):

- **glob-reexport / `use crate::` rebind** — a cross-crate effectful call reached via
  `use x::prelude::*` or a `use crate::name` submodule rebind read pure and undisclosed, even under
  `--deps` chaining (real hit: sqlx-postgres's TCP dial to Postgres — `PgListener::connect` now
  discloses `Net`, `begin` discloses `Db`, and both chain).
- **`cfg_if!` macro arms** — effects inside a `cfg_if::cfg_if!` arm were dropped as a misleading
  "uncovered" ledger entry; the arm grammar is now parsed and every arm walked (all-arm
  over-approximation, like the existing cfg-branch handling). Recovers sqlx-core's `connect_tcp` →
  `Net`; a non-conforming shape falls back to the opaque path, never panics.
- **block-nested `use` bindings** — a `use path::X` inside a nested block (if/else arm, loop body)
  was not collected, so a call through it resolved to nothing → pure. The whole body tree is now
  walked, with a scope guard so an inner fn's imports don't leak. Recovers fd's gls-check → `Exec`.

## spec 0.14 — floor alignment (2026-07-14)

candor-scan and candor-query now declare **spec `0.14`** (both at crate **0.14.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.14.0**). **0.14 is the current spec
floor** — the ratchet from 0.13. This is a **declared-version alignment only**: reports and
`--gate-json` verdicts are **byte-identical** with 0.13 — there is no engine-local behaviour change.

The ⟨0.14⟩ rung is a **top-level-initializer fix in candor-ts / candor-swift**: those engines were
dropping a module's top-level effects as false-pure (the `<module>` / `<main>` initializer unit went
unattributed). **Rust has no top-level executable code** — a `const` / `static` must be
const-evaluable, so nothing runs at module load to attribute — so the rung is **N/A** for this engine.
candor-scan declares 0.14 purely to keep the family floor uniform; see the candor-spec 0.14 entry for
the contract change.

## spec 0.13 — the Llm effect (2026-07-14)

candor-scan and candor-query now declare **spec `0.13`** (both at crate **0.13.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.13.0**). **0.13 is the current spec
floor** — the ratchet from 0.12. The report schema is unchanged (`Llm` is a value in the existing
effect set), but a report that previously read `Net` on a model-provider call now reads `Llm` — the
new precision the bump pins into the contract.

### 🤖 the `Llm` effect — a model-provider call, refining Net

A call to a machine-learning model provider is now its own boundary effect, **`Llm`**, refining the
broad `Net` the same way `Db` does (both engines — the stable scanner and the deep dylint). It fires
on two signals: a **known model-host literal** (the `api.anthropic.com`-class hosts in the shared
`MODEL_HOSTS` table, verbatim from the java reference — a loopback Ollama `:11434` counts, an
arbitrary host on that port does not), OR a **model-SDK crate** (a curated list — `async-openai`,
`aws-sdk-bedrockruntime`, `ollama-rs`, …). A model call always carries `Net` alongside `Llm`, so it
never evades a `Net` gate; `Llm` sits in the boundary / ambient / injection / salience sets and takes
its own `deny`/`allow` gate, with a masked model host failing closed. The host classifier is tightened
to match the reference across engines (Bedrock keys off the first-label service, not a bare `bedrock`
substring that caught an S3 bucket).

### 🔗 the reqwest builder-chain Net gap, fixed alongside

The `Client::builder()…post(url).send()` builder chain was not being read as `Net`, so a real
model call made through it was seen as `Env`-only — a claimed-covered crate dropping its dominant
idiom. The chain is now classified `Net` with the host captured, so `Llm` fires on the model hosts
reached through it (verified on a real `api.anthropic.com` call).

## spec 0.12 — the gains origin field (2026-07-14)

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

## [0.23.0] — 2026-07-20 (crates: candor-report / candor-classify / candor-scan / candor-query, lockstep at the spec floor)

Spec floor → **0.23**. Soundness-increasing, report-shape-neutral:
- **cross-package interface dispatch** (interfaceUnion, the 0.23 rung): a chained consumer's trait-object
  dispatch resolves to the impl's effect (gated behind `CANDOR_WORKSPACE_CHAIN`; a default report is
  byte-identical). PART 18 conformance.
- **⚠ opaque callable → synchronous invoker** (`Iterator::for_each(cb)` direct-pass, Option/Result
  combinators) discloses `Unknown` — the four-way sync-callback rung (PART 1 `sync_callback_opaque`).
- trait-union emission guarded against same-leaf-trait name collision (never fabricate an unrelated impl's
  effect on a colliding trait tail).

## [0.22.0] — 2026-07-18 (crates: candor-report / candor-classify / candor-scan / candor-query, lockstep at the spec floor)

Spec floor → **0.22** (the `verify` oracle rung, shipped on the java/ts arms). candor-scan / candor-query declare
`0.22`; the report and verdict schema are unchanged from 0.21, so this engine's output is byte-identical across
the bump. No functional change to the Rust engine.

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
