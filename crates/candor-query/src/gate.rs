//! ⟨0.24⟩ `candor-query gate --report <locator> --policy <file>` (SPEC §3.1) — apply a policy to an
//! EXISTING report, with NO scan.
//!
//! **WHY IT IS A MUST AND NOT A CONVENIENCE.** `candor-scan … --policy` recomputes `S` from source, so
//! the classifier is always in the loop; `whatif` reports only what a hypothetical INTRODUCES (a report
//! already carrying `Net` under `deny Net` answers `ok: true`, by design). The gate was therefore never
//! reachable as a function of a GIVEN signature, and a defect in the gate was indistinguishable from a
//! defect in the classifier by any test that could be written here. With this verb, conformance can hand
//! the engine a signature `candor-spec/reference/policy_model.py` has already judged and compare verdicts
//! directly. It is also the supply-chain verb: gating a dependency's PUBLISHED report is the operation an
//! adopter wants and could not previously express without re-analysing code they do not have.
//!
//! **THE MATCHING LIVES IN `candor_classify::gate`.** This module only builds a `GateInput` and owns the
//! CLI/exit choreography. There is deliberately no second copy of the §6.2 rules on this side — the
//! clause that mandates the verb was written about exactly that mistake.

use crate::*;
use candor_classify::policy::ReasonClass;

// ── the reader ──────────────────────────────────────────────────────────────────────────────────
//
// A NEW READER WAS NEEDED, and this is where it lives. The loaders beside it are built to ENRICH —
// `load_callgraph` merges the §2.2 sidecar, `load_hierarchy` merges the type hierarchy, `fix`/`tour`
// walk both — and this verb must read strictly LESS than any of them, which is not a subset reachable
// by passing a flag. `load_entries` alone would also do (it opens no sidecar), but it returns only the
// `functions` array and drops the §2 ENVELOPE the verdict is written from: the ⟨0.21⟩ completeness
// manifest and the ⟨0.15⟩ κ-coverage ledger. So one pass, one file set, three facts.

/// A report as far as the ⟨0.24⟩ gate is concerned: the entries, plus the envelope facts the VERDICT
/// carries. Every field is read VERBATIM off the wire — this is `S` and `D` as the producer wrote them.
struct GateReport {
    entries: Vec<ReportEntry>,
    /// ⟨0.21⟩ `analyzed.count`, summed across the reports under the locator exactly as a workspace
    /// scan sums it across members (`record_gate_analyzed`).
    analyzed_count: usize,
    /// ⟨0.21⟩ the target source the producing scan could NOT analyze. Non-empty ⇒ the gate cannot be
    /// green over it, the same verdict the scan reached from the same fact.
    unanalyzed: Vec<candor_report::UnanalyzedUnit>,
    /// ⟨0.30⟩ the peek's findings, carried BY the report — this route cannot peek (it has no target,
    /// only a document), which is exactly why the field rides the report and why §3.1's byte-equality
    /// holds here by construction rather than by two authors agreeing.
    out_of_scope: Vec<candor_report::OutOfScopeFinding>,
    /// ⟨0.31⟩ one record per report that carried the key — a prefix can match several.
    net_partners: Vec<candor_report::NetPartners>,
    /// ⟨0.15⟩ the κ ledger's package NAMES, unioned across reports — the verdict's advisory note.
    coverage_packages: BTreeSet<String>,
    /// ⟨0.24⟩ The PACKAGES under this locator whose report says it JUDGED NOTHING (SPEC §2's
    /// `analyzed.count == 0` rule — [`candor_report::report_judged_nothing`]). SPEC §3.1 ⟨0.24⟩ binds the
    /// same rule to this verb: such a report *"has judged nothing, so it licenses no purity claim and
    /// **the verb MUST SAY SO**. The obligation is on the reading, not on the route the report arrived
    /// by."*
    ///
    /// **A DISCLOSURE, NOT AN EXIT CODE — and this field is a LIST because of it.** ⟨0.24⟩'s corrected
    /// clause is explicit that *"the exit code and the verdict document are UNCHANGED"*: refusing here
    /// contradicted §3.1's own byte-equality MUST, since `candor-scan` over a facade package exits 0 with
    /// a clean verdict and this verb must match it. It also would have minted a third exit-2 cause — §3.3
    /// enumerates exactly two, a broken gate CONFIG and an INCOMPLETE analysis of the target's own code —
    /// which is the definition of splitting the verb.
    ///
    /// Judged-nothing is decided PER FILE, never on the summed count, and the difference is SPEC §2's
    /// third row: a pre-⟨0.21⟩ report carrying entries and no manifest contributes 0 to the sum while
    /// having plainly judged something. `analyzed` absent ⇒ judged-nothing only when that file also lists
    /// no entries. Since the treatment is now a line of prose rather than a refusal, a locator naming
    /// several members discloses EACH silent one by name instead of only the all-silent case — the
    /// conjunction only existed because one refusal had to answer for the whole file set.
    judged_nothing_pkgs: Vec<String>,
}

/// Load the report(s) at `prefix` AND NOTHING ELSE.
///
/// **THE MUST NOT LIVES HERE.** SPEC §3.1 ⟨0.24⟩: *"An engine MUST NOT re-derive, widen, or re-classify
/// anything while serving this verb … In particular a report entry that is ABSENT is absent — the ⟨0.21⟩
/// purity claim — and MUST NOT be back-filled from a callgraph sidecar or a chained dep."* Each of the
/// following is something this codebase's other loaders do and this function does not:
///
///   - **no `.callgraph.json` sidecar.** `load_callgraph` (callers/tour/whatif/fix/rewire) merges it, so
///     a fn absent from `functions` still gets edges there. Here it is not opened, and an fn absent from
///     `functions` has no entry at all — which is precisely the ⟨0.21⟩ purity claim, taken as given;
///   - **no `.hierarchy.json`**, no CHA, no frontier widening;
///   - **no dep chaining.** `CANDOR_DEPS` / the `.candor/config` `deps` key join a dependency's effects
///     into the sets during a SCAN (candor-scan's `load_dep_reports`). Nothing on this path reads either,
///     so a chained dep cannot give an absent entry its effects. This verb opens exactly one config, for
///     the ⟨0.19⟩ `unknown-alias` vocabulary, anchored to the POLICY file — and reads only that key;
///   - **no re-classification.** `hosts`/`cmds`/`paths`/`tables`/`netClass` are taken verbatim. They are
///     already TRANSITIVE on the wire (candor-scan writes the fixpointed accumulators), so no literal is
///     re-matched and no host is re-mapped through THIS machine's `net-partner` config — which would
///     also make the verdict depend on the consumer's CWD.
///
/// `Err(<reason>)` on a locator that matches no report, or one that is found-but-corrupt: §3.1's found-but-
/// corrupt rule — a report that cannot be parsed is corrupt input, not an effect-free package, and a
/// policy gated over the resulting empty map would PASS. Never a silently-empty "no violations".
fn load_gate_report(prefix: &str) -> Result<GateReport, String> {
    let paths = glob_reports(prefix);
    if paths.is_empty() {
        let why = format!(
            "no report files at prefix `{prefix}` — nothing to gate (scan first: candor-scan . --out \
             {prefix})"
        );
        eprintln!("candor-query gate: {why}");
        return Err(why);
    }
    let mut out = GateReport {
        entries: Vec::new(),
        analyzed_count: 0,
        unanalyzed: Vec::new(),
        out_of_scope: Vec::new(),
        net_partners: Vec::new(),
        coverage_packages: BTreeSet::new(),
        judged_nothing_pkgs: Vec::new(),
    };
    let mut hard_fail = false;
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("candor-query gate: report {} could not be read", path.display());
            hard_fail = true;
            continue;
        };
        match candor_report::report_entries_counted(&text) {
            Some((es, dropped)) => {
                if dropped > 0 {
                    // A dropped entry is a function that VANISHES from the signature and therefore reads
                    // as pure to the gate — the under-report the gate exists to prevent. Disclose it, and
                    // treat an all-junk file as corrupt.
                    eprintln!(
                        "candor-query gate: report {} — {dropped} function entr{} could not be parsed; \
                         a dropped entry reads as PURE to the gate, so this verdict would under-report",
                        path.display(),
                        if dropped == 1 { "y" } else { "ies" }
                    );
                    hard_fail = true;
                }
                out.entries.extend(es);
            }
            None => {
                eprintln!(
                    "candor-query gate: report {} failed to parse — corrupt input, not an effect-free \
                     package (a gate over the empty map would PASS)",
                    path.display()
                );
                hard_fail = true;
                continue;
            }
        }
        // ⟨0.24⟩ EVERY §2 ENVELOPE KEY THE VERDICT READS IS READ STRICTLY HERE: ABSENT may take its
        // documented default, PRESENT-BUT-UNPARSEABLE is a refusal that NAMES THE KEY. SPEC §2: *"That
        // default is always the permissive value — `0`, `[]`, absent — so the coercion converts corrupt
        // input into a claim, and on every one of these keys the claim is the safe-looking one."*
        //
        // MEASURED (2026-07-28) on `unanalyzed: [{"unit":…,"why":…}]` — the right shape with the wrong
        // field names, exactly what a hand-built or foreign-produced report yields: the old
        // `from_value(u).ok().unwrap_or_default()` in `report_unanalyzed` returned `[]`, and since
        // `unanalyzed` NON-EMPTINESS *is* the fail-closed trigger, candor-rust exited 0 `policy ✓` where
        // ts, java and swift all exited 2. Not a lost hedge — an inverted verdict.
        //
        // The refusal is per-key and names it, because "this report did not load" sends the user back to
        // a scan they may not own, while "your `unanalyzed` key is not `[{path, reason}]`" is actionable.
        macro_rules! strict {
            ($read:expr, $key:literal, $shape:literal, $permissive:literal, $absent:expr) => {
                match $read {
                    candor_report::KeyRead::Absent => $absent,
                    candor_report::KeyRead::Present(v) => v,
                    candor_report::KeyRead::Corrupt => {
                        eprintln!(
                            "candor-query gate: report {} — the `{}` key is PRESENT but is not {} (SPEC §2). \
                             A key that cannot be READ is corrupt input, never its empty value: coerced to \
                             the default it would become a claim, and here that default is {}. Fix the key, \
                             or re-run the scan that wrote it.",
                            path.display(),
                            $key,
                            $shape,
                            $permissive,
                        );
                        hard_fail = true;
                        continue;
                    }
                }
            };
        }
        out.analyzed_count += strict!(
            candor_report::report_analyzed(&text),
            "analyzed",
            "`{ count: <integer>, digest: <hex> }`",
            "`count: 0`, which understates the judged universe every downstream number is scaled against",
            Default::default()
        )
        .count;
        // ⟨0.24⟩ …and separately from the SUM, whether this file judged anything at all. Read from the
        // one shared predicate candor-scan's chained join uses, so the two routes ⟨0.24⟩ binds cannot
        // drift: the rule is about the READING, not the route the report arrived by.
        if candor_report::report_judged_nothing(&text) {
            // Named by its `package`, which is what an adopter recognises; the file only when the
            // envelope carries no name (a pre-⟨0.4⟩ report, or a bare v0.1 array).
            let pkg = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.get("package").and_then(|p| p.as_str()).map(str::to_owned))
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| path.display().to_string());
            out.judged_nothing_pkgs.push(pkg);
        }
        out.unanalyzed.extend(strict!(
            candor_report::report_unanalyzed(&text),
            "unanalyzed",
            "a list of `{ path, reason }`",
            "the EMPTY list — and `unanalyzed` non-emptiness IS the fail-closed trigger, so that default \
             turns this verb's exit 2 into `policy ✓`",
            Vec::new()
        ));
        // ⟨0.30⟩ read STRICTLY for `unanalyzed`'s reason — the empty default is the claim "I looked and
        // nothing was there", which is the safe-LOOKING value. ABSENT stays absent (⟨0.26⟩ cannot-answer):
        // a report produced with no policy was never asked, and must not become exit 2 on contact.
        // ⟨0.31⟩ the producer's ambient-partner provenance, carried through verbatim. ABSENT is the
        // ordinary case (no partner participated) and must stay absent — this key is additive, so a
        // pre-rung report reads exactly as it did before.
        if let candor_report::KeyRead::Present(np) = candor_report::report_net_partners(&text) {
            if !out.net_partners.iter().any(|e| *e == np) {
                out.net_partners.push(np);
            }
        }
        out.out_of_scope.extend(strict!(
            candor_report::report_out_of_scope(&text),
            "outOfScope",
            "a list of `{ fn, path, effects, class, reason }`",
            "the EMPTY list — and ⟨0.30⟩ makes `outOfScope` non-emptiness a fail-closed trigger, so that \
             default turns this verb's exit 2 into `policy ✓`",
            Vec::new()
        ));
        let cov: candor_report::Coverage = strict!(
            candor_report::report_coverage_strict(&text),
            "coverage",
            "`{ uncovered: [{ name, calls }] }`",
            "an EMPTY κ ledger, which deletes the coverage hedge from the verdict a machine reads",
            Default::default()
        );
        out.coverage_packages.extend(cov.uncovered.into_iter().map(|e| e.name));
    }
    if hard_fail {
        let why = "refusing to gate over a report that did not load cleanly — re-run the scan (a \
                   partial signature makes a green verdict meaningless); the specific key or file is \
                   named on stderr above"
            .to_string();
        eprintln!("candor-query gate: {why}");
        return Err(why);
    }
    Ok(out)
}

/// ⟨0.28⟩ SPEC §3.3.1 (3) — the FILES a `gate --report` locator names, by the SAME resolution the run
/// applies (`resolve_locator` → `glob_reports`, or the discovered prefix when no `--report` was given —
/// exactly the `report_flag.or_else(discover_report_prefix)` line in `cmd_gate`). Exists for the
/// input-collision guard in the pre-pass there: the guard compared the sink against the raw LOCATOR
/// token, and a locator is a PREFIX — so `--gate-json <one of the expanded siblings>` armed the refusal
/// OVER the very report the gate was asked to judge. MEASURED on this engine 2026-08-12:
///
///   gate --report r --policy P --gate-json r.gatedemo.scan.json
///       → the armed refusal replaced the operator's report, the load then failed on the wreckage
///         ("failed to parse — corrupt input"), and the exit-2 refusal document was written over it
///         AGAIN. The report is gone, at an exit code indistinguishable from the refusal that should
///         have happened. The discovery spelling (no `--report`, sink = the discovered
///         `.candor/report.<crate>.scan.json`) destroyed it identically.
///
/// Kept ADJACENT to `load_gate_report` so the guard and the loader cannot drift about what this verb
/// reads — the one-list discipline candor-scan's `run_inputs` keeps, and the reason a hand-written
/// second expansion is forbidden by the clause itself.
///
/// THE §2.2 SIDECARS EXPAND TOO (same clause). The gate opens no sidecar — that MUST NOT is
/// `load_gate_report`'s — but the locator NAMES the pair, and a sink on the pair's other half is worse
/// than the report case: the report loads fine, the gate runs green, and a REAL verdict lands on the
/// callgraph at exit 1 (measured) — `callers`/`whatif` then read a verdict document where the graph
/// belongs. Two exclusions from `SIDECAR_KINDS`, each because refusing it would break a legitimate
/// spelling, not because destroying it is fine: `gate` (`<stem>.gate.json` is a verdict sink by
/// designation — the beside-the-report layout this flag exists for), and `encountered-*` is absent from
/// that const anyway (engine-local scan bookkeeping no query reads).
fn gate_report_input_files(report_flag: Option<&str>) -> Vec<String> {
    let Some(prefix) = report_flag.map(resolve_locator).or_else(discover_report_prefix) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in glob_reports(&prefix) {
        let r = r.display().to_string();
        if let Some(stem) = r.strip_suffix(".json") {
            for kind in candor_report::SIDECAR_KINDS {
                if kind == "gate" {
                    continue;
                }
                let side = format!("{stem}.{kind}.json");
                if Path::new(&side).is_file() {
                    out.push(side);
                }
            }
        }
        out.push(r);
    }
    out
}

// ── the report route into the gate ──────────────────────────────────────────────────────────────

/// The owned accumulators a [`candor_classify::gate::GateInput`] borrows. Built from a written report
/// and nothing else; the counterpart of candor-scan's `policy_violations`, which builds the same struct
/// from the classifier's fixpoints. Both feed the one `candor_classify::gate::gate`.
pub(crate) struct ReportSignature {
    all: Vec<String>,
    inferred: HashMap<String, BTreeSet<String>>,
    calls: HashMap<String, BTreeSet<String>>,
    hosts: HashMap<String, BTreeSet<String>>,
    cmds: HashMap<String, BTreeSet<String>>,
    paths: HashMap<String, BTreeSet<String>>,
    tables: HashMap<String, BTreeSet<String>>,
    /// Deliberately EMPTY — see [`report_signature`]; every `allow` rule is refused upstream.
    surface_incomplete: HashMap<String, BTreeSet<String>>,
    /// The TRANSITIVE reason classes — §6.2's `D`. `pub(crate)` because the advisory verbs read it from
    /// HERE rather than running a second fixpoint of their own: `unverified` and `fix-gate` select over
    /// exactly the set the gate scopes over, and a private copy is how they came to disagree with it.
    pub(crate) reason_classes: HashMap<String, BTreeSet<String>>,
    net_classes: HashMap<String, Vec<String>>,
}

impl ReportSignature {
    fn as_input(&self) -> candor_classify::gate::GateInput<'_, String> {
        candor_classify::gate::GateInput {
            all: &self.all,
            inferred: &self.inferred,
            calls: &self.calls,
            hosts: &self.hosts,
            cmds: &self.cmds,
            paths: &self.paths,
            tables: &self.tables,
            surface_incomplete: &self.surface_incomplete,
            reason_classes: &self.reason_classes,
            net_classes: &self.net_classes,
        }
    }
}

/// ⟨0.24⟩ THE REPORT ROUTE INTO THE GATE — a signature read from a written report, with no scan and no
/// classifier.
///
/// `surface_incomplete` is left EMPTY, and that is why the caller REFUSES every AS-EFF-008 `allow` rule.
/// Reconstructing it from `netClass ∋ unknown-host` is NOT available: `net_dest_class` returns that token
/// for any host it does not RECOGNISE, so it also names a merely unrecognised, fully-visible host — the
/// reference engine's equivalence test refuted that reconstruction in one run, where it flagged two
/// functions the scan passes. (This engine's wire DOES carry a per-entry `incomplete` field, so the
/// marker is not structurally out of reach here; it is refused anyway, because §3.1 makes the refusal a
/// MUST and an engine that answers a question its three siblings refuse has split the verb.)
///
/// The one thing COMPUTED here is the transitive closure of the reason classes, because `unknownWhy` is
/// direct-only by contract (SPEC §4) while §6.2 resolves the class set over the gate's own reach. It runs
/// the SHARED `propagate_str` over the REPORT's own `calls` edges: report data in, report data out, and
/// the same fixpoint the scan route uses, so the two cannot drift.
///
/// ⟨0.24⟩ **TAKES THE ENTRIES, NOT THE `GateReport`, BECAUSE THE ADVISORY VERBS BUILD IT TOO** (SPEC
/// §3.2, candor-spec `4fd140c`: *an advisory verb may be LESS certain than the gate, never more*).
/// `unverified` and `fix-gate` used to carry their own reason-class fixpoint (`reason_class_acc`) — the
/// same arithmetic, written twice, which is one of the three shapes that law was written about. They now
/// read the gate's own signature, so "which classes does this function have" has one answer on this side
/// of the report boundary. The `GateReport` envelope (the ⟨0.21⟩ manifest, the κ ledger) is the
/// VERDICT's input and stays with the gate.
pub(crate) fn report_signature(entries: &[ReportEntry]) -> ReportSignature {
    let mut inferred: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut net: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut why_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut names: BTreeSet<String> = BTreeSet::new();

    for e in entries {
        let fn_ = e.func.clone();
        names.insert(fn_.clone());
        // UNION on a repeated `fn` rather than overwrite: a duplicate key is malformed input, and the
        // union is the direction that cannot turn a violation into a pass.
        inferred.entry(fn_.clone()).or_default().extend(e.inferred.iter().cloned());
        calls.entry(fn_.clone()).or_default().extend(e.calls.iter().cloned());
        if !e.hosts.is_empty() {
            hosts.entry(fn_.clone()).or_default().extend(e.hosts.iter().cloned());
        }
        if !e.cmds.is_empty() {
            cmds.entry(fn_.clone()).or_default().extend(e.cmds.iter().cloned());
        }
        if !e.paths.is_empty() {
            paths.entry(fn_.clone()).or_default().extend(e.paths.iter().cloned());
        }
        if !e.tables.is_empty() {
            tables.entry(fn_.clone()).or_default().extend(e.tables.iter().cloned());
        }
        if !e.net_class.is_empty() {
            net.entry(fn_.clone()).or_default().extend(e.net_class.iter().cloned());
        }
        for why in &e.unknown_why {
            why_direct.entry(fn_.clone()).or_default().insert(ReasonClass::classify(why).token().to_string());
        }
        // ⟨0.24⟩ SPEC §6.2 requirement (3), THE CONTRIBUTION, on the one route where the producer-side
        // repair cannot reach: a report is DATA, so candor-scan's `reason_class_direct` contribution
        // (which makes this state unreachable in a report THIS engine writes — the §4 invariant beside
        // the writer asserts it) says nothing about a hand-authored or foreign one. An entry that raises
        // `Unknown` DIRECTLY and names no reason for it CONTRIBUTES `unresolved` here, at the ENTRY,
        // BEFORE the fixpoint — which is what makes it compose: a caller of one reasonless entry and one
        // `dispatch:` entry accumulates {unresolved, dispatch} and is caught by BOTH filters. Contributing
        // at the join instead (an empty-set default) cannot do that: by then the two sets have been
        // unioned and the caller of both is byte-identical to the caller of the reasoned one alone — the
        // §6.2 counterexample in which ADDING a call turned a red verdict green.
        //
        // GATED ON A DIRECT `Unknown` IT DID NOT NAME, never on the reason set being absent, because
        // absence is ALSO what an INHERITED `Unknown` looks like and marking those is the mirror
        // fabrication (measured elsewhere at 435 functions where the legitimate count is 0). An ABSENT
        // `direct` key reads as an empty set and contributes NOTHING: that is a report which did not carry
        // the channel, not a claim of a direct `Unknown`.
        if e.direct.iter().any(|d| d == "Unknown") && e.unknown_why.is_empty() {
            why_direct.entry(fn_).or_default().insert(ReasonClass::Unresolved.token().to_string());
        }
    }
    let all: Vec<String> = names.into_iter().collect();
    let reason_classes = candor_classify::propagate::propagate_str(&why_direct, &calls, &all);
    ReportSignature {
        net_classes: net.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect(),
        all,
        inferred,
        calls,
        hosts,
        cmds,
        paths,
        tables,
        surface_incomplete: HashMap::new(),
        reason_classes,
    }
}

// ── answerability (SPEC §3.1 ⟨0.24⟩) ────────────────────────────────────────────────────────────

/// THE THIRD ANSWERABILITY CASE — a class-scoped `deny` filter over a report that cannot answer it.
/// Returns the refusal message, or `None` when every scoped filter is answerable.
///
/// A bare `deny Net` / `deny Unknown` asks a question the effect set alone answers. A SCOPED one —
/// `deny Net[unknown-host]`, `deny Unknown[dispatch]` — asks a second question ("…and is the destination
/// / the reason class one of THESE?") and NARROWS the gate on the answer. When the report does not carry
/// the evidence for that second question the fields it reads are simply absent, the matcher sees an empty
/// set, nothing matches, and the effect is DROPPED from the violation. **The narrowing succeeds because
/// the evidence is missing** — an absence-keyed relaxation of a fail-closed security gate, and a silent
/// one, because the scoped rule is exactly the one a hardening team reaches for. Measured on the
/// reference engine, one function per hand-built report:
///
/// ```text
///   report                                     deny Net[unknown-host]   deny Net
///   Net-bearing entry, netClass ABSENT         exit 0  ← green          exit 1
///   inherited Unknown, `calls` ABSENT          deny Unknown[dispatch]   deny Unknown
///                                              exit 0  ← green          exit 1
/// ```
///
/// **THE REFUSAL IS MINIMAL, and monotone denial is what makes that safe** (§3.1 ⟨0.24⟩). A class-scoped
/// `deny` is not unanswerable merely because some evidence is missing: the class set only ever GROWS
/// (§6.2 — a reason is CONTRIBUTED, never retracted) and `Reject` is upward-closed in it (PAPER3 Lemma
/// 2). So when the classes determinable FROM THE ENTRY ALONE are non-empty, whatever the missing data
/// would have added could only have added matches, and the rule is answered — it fires or it does not,
/// and no further evidence can change which. Only an EMPTY determinable set leaves the question open,
/// and that is the only state refused here. Concretely this is why a reasonless DIRECT `Unknown` gated
/// by `deny E Unknown[unresolved]` is ANSWERED and not refused: `report_signature` contributes
/// `unresolved` from the entry itself, with no transitive step, so the set is not empty. (candor-swift's
/// original refusal of that case is recorded in the SPEC as over-broad.)
///
/// Refusing costs nothing on a report THIS engine wrote, which is what keeps the equivalence obligation
/// satisfiable: `netClass` is emitted for every `Net`-bearing entry and is floored at `unknown-host`
/// (`net_classes_of` inserts it whenever no host is visible), so an empty set on a `Net` entry means
/// "this producer did not carry the field", never "this function reaches nothing"; and an `Unknown` that
/// is INHERITED comes from a callee carrying `Unknown`, which is therefore effectful and present in
/// `calls` by construction, while a DIRECT `Unknown` records its `unknownWhy` at the site.
///
/// Per (rule, function), NOT per policy: a scoped rule whose matched functions all carry their evidence
/// evaluates normally, and only the rule that would have been silently narrowed is refused.
///
/// ⟨0.24⟩ RETURNS EVERY UNANSWERABLE RULE, not the first. SPEC §3.1: *"The refusal message MUST still
/// disclose which rules could not be evaluated — exit 1 reports the violation it is sure of, it does not
/// conceal the part it could not read."* Since a refusal can now be OVERRULED by a certain violation
/// (the precedence correction), the list is a DISCLOSURE that has to travel alongside a verdict, and one
/// rule out of three is a partial one. At most one message per RULE — the first function that defeats it
/// is the example; naming all of them would bury the rule.
///
/// ⟨0.24⟩ **THE REASON ONLY, WITH NO DISPOSITION.** These strings used to end *"Refusing (exit 2)."* —
/// written when the only thing that could follow an unanswerable rule WAS exit 2. Since the precedence
/// correction a firing rule dominates, so the identical sentence was printed verbatim on a run that
/// exited **1**: a message asserting an exit code that had already been overruled. Whether this refusal
/// decides the exit is the CALLER's fact, not this function's, so the caller says it. What is left here
/// is the part that is true either way — which rule, which function, and why the evidence is missing.
///
/// ⟨0.24⟩ **RETURNS `(rule, why)` PAIRS, NOT PROSE** (SPEC §3.1 `fc4b5f6`). The disclosure is a MACHINE
/// field now, so the raw policy line is a datum rather than something spliced into a sentence — and the
/// stderr line is DERIVED from the pair, so the human channel and `unevaluated` cannot disagree about
/// which rule went unanswered.
///
/// ⟨0.24⟩ **IT IS A THIN WRAPPER NOW — SPEC §3.2 (candor-spec `4fd140c`).** The pairs themselves are
/// [`unanswerable_pairs`], because the ADVISORY verbs need them per FUNCTION and this verb needs them per
/// RULE, and computing that twice is exactly the shape *"an advisory verb may be less certain than the
/// gate, never more"* was written about. MEASURED before the split, conformance R11 over a report with
/// `hosts` and no `netClass` under `deny Net[unknown-host] app`: `gate --report` exited 2 refusing to
/// judge `app.noClass`, while `unverified` cleared it and named a different hole, exit 0. Below, this
/// function keeps its OLD output byte-for-byte — one entry per rule, the first function that defeats it
/// as the example — so the gate's own disclosure is unchanged by the sharing.
fn unanswerable_scoped_filters(
    p: &candor_classify::policy::ParsedPolicy,
    sig: &ReportSignature,
) -> Vec<candor_report::Unevaluated> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    unanswerable_pairs(p, sig)
        .into_iter()
        // At most one message per RULE — the first function that defeats it is the example; naming all
        // of them would bury the rule. `unanswerable_pairs` yields rules in policy order and functions
        // in `sig.all` order, so "first" is the same function this reported before it was extracted.
        .filter(|u| seen.insert(u.rule.clone()))
        .map(|u| candor_report::Unevaluated { rule: u.rule, why: u.why })
        .collect()
}

/// ⟨0.24⟩ ONE `(rule, function)` THE GATE CANNOT EVALUATE OVER THIS REPORT, and why its evidence is
/// missing — the unit [`unanswerable_scoped_filters`] aggregates and the unit the ADVISORY verbs need.
pub(crate) struct Unanswerable {
    /// The rule's source line, verbatim.
    pub(crate) rule: String,
    /// The function the rule could not be evaluated ON. The same rule may be answerable on another.
    pub(crate) func: String,
    /// WHICH EVIDENCE IS MISSING — never the class a derivation would have supplied. SPEC §3.2: *"the
    /// reason recorded is the MISSING EVIDENCE, never the derived class"*; recording a derivation would
    /// restate the defect as a disclosure.
    pub(crate) why: String,
}

/// ⟨0.24⟩ EVERY `(rule, function)` PAIR THE GATE WOULD REFUSE — the answerability question asked once,
/// for every consumer of the answer (SPEC §3.1 for the refusal, §3.2 `4fd140c` for why it is shared).
///
/// The gate reports these per RULE and refuses; `unverified` must NAME each `func` as a hole, because a
/// function the gate COULD NOT JUDGE is an unverified hole in the strongest sense that verb has; and
/// `fix-gate` must not plan a hoist across a boundary this names. Three consumers, one predicate: the
/// law being enforced is a COMPARISON between verbs, so it is checked by construction rather than by
/// three authors agreeing.
///
/// ⟨0.29⟩ THE TWO WHOLE-POLICY UNANSWERABLE KINDS, as a function so every report route shares one.
///
/// `forbid` and `allow` cannot be answered from a §2 report (SPEC §3.1 ⟨0.24⟩ ANSWERABILITY). This lived
/// INLINE in `gate --report` and only there, so the advisory verbs that read the same report never saw it:
/// MEASURED, `candor-query fix-gate --report <r> --policy <forbid-only>` printed
/// `no deny/pure boundary crossings in this report ✓` at exit 0, and `unverified` printed
/// `every function in a pure/deny layer is PROVABLY clean ✓` — a green over a policy whose only rule had
/// been evaluated by nothing. candor-java, the reference engine, disclosed and withheld `ok` on both;
/// rust, ts and swift did not. Extracted rather than copied so the fourth caller inherits it.
pub(crate) fn whole_policy_refusals(
    p: &candor_classify::policy::ParsedPolicy,
    policy_path: &str,
) -> Vec<candor_report::Unevaluated> {
    let mut policy_refusals: Vec<candor_report::Unevaluated> = Vec::new();
    if !p.layer_rules.is_empty() {
        let why = format!(
            "`gate --report` cannot evaluate a `forbid` rule — a report's `calls` graph is \
             EFFECT-RELEVANT (only callees with a non-empty effect set are written), so a crossing into a \
             wholly PURE unit is invisible in it, while `forbid` matches on NAME. The rule would read \
             green over a crossing a scan fails on. Gate layering at scan time: candor-scan . --policy \
             {policy_path}"
        );
        policy_refusals.extend(p.layer_rules.iter().map(|r| candor_report::Unevaluated {
            rule: r.raw.trim().to_string(),
            why: why.clone(),
        }));
    }
    // ⟨0.29⟩ `only` IS AS UNANSWERABLE AS `forbid`, and for a STRICTER reason. Both match on NAME, which
    // a report's effect-relevant wire cannot settle — but `forbid` asks whether one named crossing is
    // present, while `only` asks whether EVERY reached scope is on a list. A report that omits a crossing
    // makes `forbid` read green; it makes `only` read green too, and there the green is a claim of
    // COMPLETENESS. Refusing both from one place is what stops the next route inheriting only half.
    if !p.only_rules.is_empty() {
        let why = format!(
            "`gate --report` cannot evaluate an `only` rule — it asks whether EVERYTHING a scope reaches \
             is on a list, and a report carries an effect-relevant call surface rather than the complete \
             dependency graph a NAME-matching rule needs. Answering it here would certify completeness \
             from evidence that is not complete. Gate permissions at scan time: candor-scan . --policy \
             {policy_path}"
        );
        policy_refusals.extend(p.only_rules.iter().map(|r| candor_report::Unevaluated {
            rule: r.raw.trim().to_string(),
            why: why.clone(),
        }));
    }
    if !p.allow_rules.is_empty() {
        let effects: BTreeSet<&str> = p.allow_rules.iter().map(|r| r.effect).collect();
        let why = format!(
            "`gate --report` cannot evaluate an `allow {}` rule — the AS-EFF-008 surface-completeness \
             marker WAS said not to ride the report wire; ⟨0.29⟩ made it ride, but only when the \
             producing report declares `incomplete` in `resolves`. This verb refuses UNIFORMLY \
             rather than answering per-report, because an engine that evaluated where its \
             siblings refuse would SPLIT THE VERB — a benign visible literal beside a \
             runtime-computed endpoint would be CERTIFIED here and flagged by a scan. \
             (`netClass: unknown-host` is NOT that marker — it also names a merely \
             unrecognised host.) \
             Gate allowlists at scan time: candor-scan . --policy {policy_path}",
            effects.into_iter().collect::<Vec<_>>().join("`/`")
        );
        policy_refusals.extend(p.allow_rules.iter().map(|r| candor_report::Unevaluated {
            rule: r.raw.trim().to_string(),
            why: why.clone(),
        }));
    }
    policy_refusals
}

/// See [`unanswerable_scoped_filters`] above for the whole argument about WHEN a scoped filter is
/// unanswerable and why the refusal is minimal.
pub(crate) fn unanswerable_pairs(
    p: &candor_classify::policy::ParsedPolicy,
    sig: &ReportSignature,
) -> Vec<Unanswerable> {
    let mut out = Vec::new();
    for r in &p.rules {
        for q in &sig.all {
            if let Some(s) = &r.scope
                && !candor_classify::policy::scope_matches(q, s)
            {
                continue;
            }
            let inf = sig.inferred.get(q);
            let has = |e: &str| inf.is_some_and(|s| s.iter().any(|x| x == e));
            if !r.net_classes.is_empty()
                && has("Net")
                && sig.net_classes.get(q).map(|c| c.is_empty()).unwrap_or(true)
            {
                out.push(Unanswerable {
                    rule: r.raw.trim().to_string(),
                    func: q.clone(),
                    why: format!(
                        "it narrows on the Net DESTINATION CLASS, but `{q}` carries Net with no \
                         `netClass` in this report — the field the filter reads is absent, so the \
                         narrowing would succeed for lack of evidence and drop a Net the bare `deny Net` \
                         catches. The rule is WITHHELD on `{q}` rather than tolerated there: an absent \
                         optional field must not relax a fail-closed gate. Use the bare `deny Net`, or \
                         gate at scan time."
                    ),
                });
                // NO `break`. The gate names one function per rule as the example, and that aggregation
                // now lives in [`unanswerable_scoped_filters`] — stopping here would ALSO stop the
                // advisory verbs at one function per rule, which is the defect in miniature: a second
                // function the gate could not judge, cleared in silence because a first one was named.
                continue;
            }
            if !r.unknown_classes.is_empty()
                && has("Unknown")
                && sig.reason_classes.get(q).map(|c| c.is_empty()).unwrap_or(true)
            {
                out.push(Unanswerable {
                    rule: r.raw.trim().to_string(),
                    func: q.clone(),
                    why: format!(
                        "it narrows on the Unknown REASON CLASS, but `{q}` carries Unknown with no reason \
                         reachable in this report — neither its own `unknownWhy` nor a `calls` edge to \
                         one. §6.2 resolves the class set TRANSITIVELY over the gate's reach; with the \
                         channel missing there is nothing for the filter to read, so the rule is WITHHELD \
                         on `{q}` — neither charged (which would assert a reason nobody recorded) nor \
                         tolerated (which would relax the gate for lack of evidence). Use the bare `deny \
                         Unknown`, or gate at scan time."
                    ),
                });
            }
        }
    }
    out
}

// ── the CLI ─────────────────────────────────────────────────────────────────────────────────────

const GATE_USAGE: &str =
    "usage: candor-query gate --report <locator> --policy <file> [--json] [--gate-json <file>]";

/// ⟨0.24⟩ REFUSE: exit 2, AND write the refusal document `--gate-json` was promised (SPEC §3.1).
///
/// **THE STALE-VERDICT HAZARD.** A refusal used to return 2 having written nothing at all, so a CI
/// wrapper of the shape `candor-query gate … --gate-json v.json || true; jq .ok v.json` re-read **the
/// PREVIOUS run's document as current** — a green file from yesterday's clean run, still on disk, is how
/// a refusal becomes an all-clear. Deleting the path is not the fix either: a consumer that treats a
/// missing file as "nothing to report" fails open by a different route. The only safe answer is a
/// document whose NAIVE read is the fail-closed one, which is what [`candor_report::gate_refusal_json`]
/// is (`ok:false`, `refused:true`, the reason, and NO `violations` key).
///
/// **EVERY EXIT-2 CAUSE, WITH NO EXEMPTIONS** (candor-spec `1503368` (b)). This used to be scoped to the
/// ANSWERABILITY refusals — the policy LOADED and the gate could not answer it over THIS report — while a
/// gate CONFIG or a report that never loaded AS one wrote nothing, per §3.3's cause (a). That carve-out
/// was recorded here as being in tension with the clause above it, and it was: **the stale-path hazard is
/// identical for both buckets, and a stale green does not care why this run declined to overwrite it.**
///
/// The objection the carve-out rested on — "even `refused: true` would be attributing a refusal to a
/// policy nobody could parse" — is answered by the document's own shape. It carries **no `violations`
/// key**, so it attributes nothing: it says the run refused and names why, which is exactly what is true
/// when the policy could not be read at all. Two tests pinned the old rule and both now assert the
/// document (`a_present_but_unparseable_section2_key_refuses_and_an_absent_one_does_not`, and the
/// broken-gate-CONFIG row of candor-scan's `a_violation_survives_an_incomplete_scan…`).
///
/// That includes the USAGE errors, and the argument scan runs to completion after the first one purely so
/// the path is known wherever on the line it sits — a document that appears only when the mistake happens
/// to come after `--gate-json` is a stale-verdict hazard keyed on argument ORDER.
///
/// Returns the exit code (always 2) so call sites read `return refuse(…)`. A failure to WRITE is not
/// escalated: the exit is already 2 and already fail-closed, and a second exit code would be a lie
/// about which refusal happened.
fn refuse(reason: &str, want_json: bool, gate_json: Option<&str>) -> i32 {
    refuse_disclosing(reason, &[], want_json, gate_json)
}

/// ⟨0.24⟩ [`refuse`] carrying the `unevaluated` list (SPEC §3.1 `fc4b5f6`). A SOLE refusal is where the
/// disclosure matters most: nothing fired, so `reason` is the whole document, and a prose reason is not a
/// list of rules a consumer can iterate.
fn refuse_disclosing(
    reason: &str,
    unevaluated: &[candor_report::Unevaluated],
    want_json: bool,
    gate_json: Option<&str>,
) -> i32 {
    let mut targets: Vec<&str> = Vec::new();
    if want_json {
        targets.push("-");
    }
    if let Some(p) = gate_json
        && !(want_json && p == "-")
    {
        targets.push(p);
    }
    if !targets.is_empty() {
        match candor_report::gate_refusal_json_v24(reason, unevaluated) {
            Ok(json) => {
                for t in targets {
                    if t == "-" {
                        println!("{json}");
                    } else if let Err(e) =
                        candor_report::write_atomic(Path::new(t), format!("{json}\n").as_bytes())
                    {
                        eprintln!(
                            "candor-query gate: could not write the refusal document to --gate-json {t} \
                             ({e}) — a consumer reading that path will see the PREVIOUS run's verdict, \
                             which is stale. Delete it, or treat exit 2 as a failure."
                        );
                    }
                }
            }
            Err(e) => eprintln!("candor-query gate: could not serialize the refusal document ({e})"),
        }
    }
    2
}

/// `candor-query gate --report <locator> --policy <file> [--json] [--gate-json <file>]`
///
/// A QUERY verb, not a scan flag, so it inherits §3.3.1's grammar unchanged: the same `--report` locator
/// rules and discovery fallback, the same `--policy` fallback (`CANDOR_POLICY`, then the config `policy`
/// key), the same loud exit 2 on an unreadable policy, and NO positionals — `gate` has no argument of its
/// own, and a swallowed token is how a gate runs green. Exit codes are `candor-scan --policy`'s exactly:
/// 0 / 1 / 2.
///
/// `--json` IS `--gate-json -`, deliberately: on a scan `--json <file>` writes the REPORT, and there is
/// no report to write here, so the verb's machine output is the verdict. A second meaning for `--json`
/// would be the one place a consumer could tell the two routes apart.
/// SPEC §3.3.1 ⟨0.27⟩ — is this one artifact under two names? Not a string comparison: `--policy /w/P
/// --gate-json ./P` run from `/w` names one file twice.
fn same_artifact(a: &str, b: &str) -> bool {
    if a == "-" || b == "-" {
        return false;
    }
    fn resolve(p: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(p);
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        let parent = p.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        Some(parent.canonicalize().ok()?.join(p.file_name()?))
    }
    matches!((resolve(a), resolve(b)), (Some(x), Some(y)) if x == y)
}

/// `.candor/config` is never a verdict sink, wherever it is.
fn is_candor_config(p: &str) -> bool {
    let path = std::path::Path::new(p);
    path.file_name().is_some_and(|n| n == "config")
        && path
            .parent()
            .map(|d| if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d })
            .and_then(|d| d.canonicalize().ok().or_else(|| Some(d.to_path_buf())))
            .and_then(|d| d.file_name().map(|n| n == ".candor"))
            .unwrap_or(false)
}

/// The `--gate-json` path this run was given, so the SHARED config loader in candor-classify can refuse
/// through it. `set_refusal_sink` takes a plain `fn` pointer and therefore cannot capture the path.
static QUERY_GATE_JSON: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// SPEC §3.3.1 ⟨0.28⟩ — every `--gate-json` this argv names. THE RUNG BINDS EVERY ROUTE, and this one
/// went without it for a release: the scan CLI refused a duplicate while `gate --report` kept last-wins,
/// so a gate that FIRED wrote red to the last sink and left the first holding a previous run's
/// `{"ok": true}`. A route is not covered by its sibling.
fn all_gate_sinks(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        // `filter` rather than nested ifs: clippy's `collapsible_if` rejects the nesting and its
        // suggested fix is a LET-CHAIN, which this crate's MSRV cannot use — the same reason the
        // pre-pass below is written this way.
        if let Some(v) = args
            .get(i + 1)
            .filter(|_| args[i] == "--gate-json")
            .filter(|v| v.as_str() == "-" || !v.starts_with('-'))
        {
            out.push(v.clone());
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

/// Two spellings of one path are ONE sink (§3.3.1's artifact rule); two artifacts are the ambiguity.
fn distinct_gate_sinks(all: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in all {
        if !out.iter().any(|k| k == s || (k != "-" && s != "-" && same_artifact(k, s))) {
            out.push(s.clone());
        }
    }
    out
}

fn refuse_via_registered_sink(reason: &str) -> ! {
    refuse(reason, false, QUERY_GATE_JSON.get().map(String::as_str));
    std::process::exit(2)
}

pub(crate) fn cmd_gate(args: &[String]) -> i32 {
    // ── SPEC §3.3.1 ⟨0.27⟩ ARM FIRST, AND NEVER OVER AN INPUT.
    //
    // Every ENUMERATED exit below already writes a refusal through `refuse`, and the usage-error
    // collection above is careful to learn the path first — that part was right. What it cannot cover is
    // the run that never reaches an exit: a panic, an OOM, a CI timeout, a `kill -9` all leave the
    // PREVIOUS run's green document on disk. Enumerating exits is the approach that keeps missing one;
    // writing the refusal at the start and letting the verdict replace it does not.
    //
    // And arming WRITES, so a sink naming the policy destroys it — measured across four engines as a red
    // gate turning green with `"ok": true`.
    {
        let (mut gate, mut policy, mut report) = (None::<&str>, None::<&str>, None::<&str>);
        let mut i = 0;
        while i < args.len() {
            // Flattened rather than nested: clippy's `collapsible_if` fires on the nested form from
            // 1.97, and its suggested fix is a LET-CHAIN, which this crate's MSRV cannot use. `filter`
            // says the same thing on every toolchain.
            let takes = args[i] == "--gate-json" || args[i] == "--policy" || args[i] == "--report";
            if let Some(v) = args
                .get(i + 1)
                .filter(|_| takes)
                .filter(|v| v.as_str() == "-" || !v.starts_with('-'))
            {
                match args[i].as_str() {
                    "--gate-json" => gate = Some(v),
                    "--policy" => policy = Some(v),
                    _ => report = Some(v),
                }
                i += 1;
            }
            i += 1;
        }
        // ⟨0.28⟩ EVERY named sink is checked, not just the one the parse honours. `gate` gets the same
        // treatment the scan route has, and the input set includes CANDOR_CONFIG by PATH — omitting it
        // is how `set_refusal_sink` came to overwrite an operator's config file on this route.
        let named_sinks = distinct_gate_sinks(&all_gate_sinks(args));
        if let Some(gp) = gate {
            let env_policy = std::env::var("CANDOR_POLICY").ok();
            let env_config = std::env::var("CANDOR_CONFIG").ok();
            // ⟨0.28⟩ …AND THE FILES THE LOCATOR EXPANDS TO, because the raw flag value below is not
            // what this verb READS. A `--report` value is a PREFIX (or a discovery, when absent), and
            // `load_gate_report` reads its expansion — so a sink naming one of the expanded files named
            // an input by any honest reading, and the token comparison could not see it. Enumerated by
            // `gate_report_input_files`, the loader-adjacent list; see the measurement there.
            let report_set = gate_report_input_files(report);
            // §3.3.1 names "a report being read (`gate --report`)" as an input. Writing the verdict
            // there destroys the very report the gate was asked to judge — and the diagnostic then
            // blames the report ("no `functions` array") rather than the collision, so the operator is
            // told their report is corrupt by the run that corrupted it.
            // Checked for EVERY named sink, so a duplicate cannot smuggle an input past a guard that
            // only ever looked at the last one.
            for s_named in named_sinks.iter().filter(|s| s.as_str() != "-") {
                for (other, flag) in [(policy, "--policy"), (report, "--report"),
                                      (env_policy.as_deref(), "CANDOR_POLICY"),
                                      (env_config.as_deref(), "CANDOR_CONFIG")] {
                    if let Some(other) = other.filter(|o| same_artifact(s_named, o)) {
                        eprintln!("candor-query gate: --gate-json {s_named} names the SAME FILE as {flag} {other} — refusing (exit 2).");
                        eprintln!("        Nothing was written; give the verdict its own path.");
                        return 2;
                    }
                }
                // ⟨0.28⟩ the expanded report set (and its §2.2 sidecars), exactly as the single-sink
                // path asks it below — a duplicate must not smuggle an expanded input past the guard.
                for f in &report_set {
                    if same_artifact(s_named, f) {
                        eprintln!("candor-query gate: --gate-json {s_named} names a file this gate reads — {f} — refusing (exit 2).");
                        eprintln!("        Nothing was written; give the verdict its own path.");
                        return 2;
                    }
                }
                if is_candor_config(s_named) {
                    eprintln!("candor-query gate: --gate-json {s_named} is a .candor/config — refusing (exit 2). Nothing was written.");
                    return 2;
                }
            }
            for (other, flag) in [(policy, "--policy"), (report, "--report"),
                                  (env_policy.as_deref(), "CANDOR_POLICY"),
                                  (env_config.as_deref(), "CANDOR_CONFIG")] {
                if let Some(other) = other.filter(|o| same_artifact(gp, o)) {
                    eprintln!("candor-query gate: --gate-json {gp} names the SAME FILE as {flag} {other} — refusing (exit 2).");
                    eprintln!("        The verdict is armed before the policy is read, so this would overwrite your");
                    eprintln!("        policy and then gate on the wreckage. Nothing was written.");
                    return 2;
                }
            }
            // ⟨0.28⟩ the expanded report set: the raw `--report` comparison above is about the TOKEN
            // the operator typed; this one is about the FILES the run will read (and their §2.2
            // sidecars). MEASURED before this loop existed: the prefix spelling destroyed the
            // operator's report at exit 2, and the callgraph spelling wrote a REAL verdict over the
            // pair's other half at exit 1 — see `gate_report_input_files`.
            for f in &report_set {
                if same_artifact(gp, f) {
                    eprintln!("candor-query gate: --gate-json {gp} names a file this gate reads — {f} — refusing (exit 2).");
                    eprintln!("        The verdict is armed before the run reads its inputs, so this would overwrite");
                    eprintln!("        that input and then gate on the wreckage. Nothing was written; give the verdict");
                    eprintln!("        its own path.");
                    return 2;
                }
            }
            // THE CONFIG-DECLARED POLICY. This verb's policy ladder falls back to the `policy` key of
            // the config discovered from the CWD, and the guard checked only the flags — so the
            // checked-in form, which is the one a CI job has, was destroyed at exit 0 while the flag
            // form refused. The same hole the scan route closed, one route across.
            // THE SHAPE CHECK COMES FIRST, because everything below it can WRITE. It used to sit after the
            // config was read, which was fine while an unreadable config exited without writing — the
            // registration below changes that, so a sink that IS the unreadable config would have been
            // overwritten by the refusal added to protect it. §3.3.1's "never arm over an input" has to
            // outrank every writer, including a new one.
            if is_candor_config(gp) {
                eprintln!("candor-query gate: --gate-json {gp} is a .candor/config — refusing (exit 2). This would");
                eprintln!("        destroy the config that configures this run. Nothing was written.");
                return 2;
            }
            // ⟨0.28⟩ …and only now, with every named sink known NOT to be an input, may the duplicate
            // refusal write.
            if named_sinks.len() > 1 {
                let list = named_sinks.join(", ");
                eprintln!("candor-query gate: --gate-json given more than once ({list}) — refusing (exit 2).");
                eprintln!("        A gate publishes ONE verdict. Naming two sinks says where it goes twice, and the");
                eprintln!("        reader of the path that loses cannot tell it lost. Name one, or run the gate twice.");
                let doc = candor_report::gate_refusal_json(&format!(
                    "--gate-json was given more than once ({list}) — a run publishes one verdict to one sink"
                ))
                .unwrap_or_else(|_| "{\"ok\":false,\"refused\":true}".to_string());
                for t in &named_sinks {
                    if t == "-" {
                        println!("{doc}");
                    } else if let Err(e) = std::fs::write(t, format!("{doc}\n")) {
                        eprintln!("candor-query gate: could not write the refusal to --gate-json {t} ({e})");
                    }
                }
                return 2;
            }
            // The shared config loader exits 2 on an unreadable config and cannot see this verb's sink,
            // so register the writer before calling it. Without this the STREAM sink was left empty on
            // exactly this cause — the earliest exit-2 cause there is, and the one the sink is least
            // likely to be armed for (the same reason PART 36 row (b11) exists for the scan route).
            let _ = QUERY_GATE_JSON.set(gp.to_string());
            candor_classify::policy::set_refusal_sink(refuse_via_registered_sink);
            if let Some((cfg_path, text)) = candor_classify::policy::discover_config(std::path::Path::new(".")) {
                let home = {
                    let parent = cfg_path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                    if parent.file_name().and_then(|n| n.to_str()) == Some(".candor") {
                        parent.parent().map(std::path::Path::to_path_buf).unwrap_or(parent)
                    } else {
                        parent
                    }
                };
                if same_artifact(gp, &cfg_path.display().to_string()) {
                    eprintln!("candor-query gate: --gate-json {gp} is the .candor/config this run reads — refusing (exit 2).");
                    return 2;
                }
                for raw in text.lines() {
                    let line = raw.split('#').next().unwrap_or("").trim();
                    let mut it = line.splitn(2, char::is_whitespace);
                    if it.next().map(str::to_ascii_lowercase).as_deref() != Some("policy") {
                        continue;
                    }
                    let Some(v) = it.next().map(str::trim).filter(|v| !v.is_empty()) else { continue };
                    let abs = if std::path::Path::new(v).is_absolute() {
                        std::path::PathBuf::from(v)
                    } else {
                        home.join(v)
                    };
                    if same_artifact(gp, &abs.display().to_string()) {
                        eprintln!("candor-query gate: --gate-json {gp} names the policy this run reads via");
                        eprintln!("        {}'s `policy` key — refusing (exit 2). Nothing was written.", cfg_path.display());
                        return 2;
                    }
                }
            }
            if gp != "-" {
                let armed = format!(
                    "{{\n  \"spec\": \"{}\",\n  \"ok\": false,\n  \"refused\": true,\n  \"reason\": \"the gate did not complete — this document was written when the run STARTED and was never replaced by a verdict, so the run failed, crashed or was killed before it could decide. It is NOT a verdict about the code; see the run's stderr for the cause.\"\n}}\n",
                    candor_report::SPEC_VERSION
                );
                if let Err(e) = std::fs::write(gp, armed) {
                    eprintln!("candor-query: could not arm --gate-json {gp} fail-closed ({e}) — if this run does not complete, that path may still hold a PREVIOUS run's verdict");
                }
            }
        }
    }
    let mut report_flag: Option<String> = None;
    let mut policy_flag: Option<String> = None;
    let mut gate_json: Option<String> = None;
    let mut want_json = false;
    // ⟨0.24⟩ THE USAGE ERROR IS COLLECTED, NOT RETURNED (SPEC §3.1 `1503368` (b)). Returning on the spot
    // made the DOCUMENT depend on where in the command line the mistake sat: `--gate-json v.json --bogus`
    // knew the path and `--bogus --gate-json v.json` did not, so one of them wrote the fail-closed
    // document and the other left yesterday's green on disk — a stale-verdict hazard keyed on argument
    // ORDER. So the FIRST error is recorded and the scan of the arguments RUNS ON, purely to learn where
    // the verdict was supposed to go. Nothing else is done with what it finds: the run is already exit 2.
    let mut usage_error: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => want_json = true,
            // candor-ts output-mode flags (#8): rust prose is the default, so accept + ignore.
            "--text" | "--human" => {}
            // SPEC §3.2 ⟨0.28⟩: "given no value" MEANS the next token is flag-shaped, or the clause is
            // unimplementable — consuming the token as a locator made this very diagnostic unreachable,
            // and `--policy --gate-json -` read the operator's verdict sink as an unreadable policy
            // path. Measured on this verb: exit 2 with NOTHING on the stream where the refusal document
            // belongs (the gate sibling of conformance §3.1 (b13); the scan route was fixed an hour
            // before this one — the sibling-route habit). The flag-shaped token is NOT consumed: the
            // run has a broken command line, not a redefined one, so the scan of the arguments runs on
            // (the ⟨0.24⟩ posture above) and a sink the live token names is still a sink.
            "--report" => match args.get(i + 1) {
                Some(v) if v == "-" || !v.starts_with('-') => {
                    report_flag = Some(resolve_locator(v));
                    i += 1;
                }
                Some(v) => {
                    usage_error.get_or_insert_with(|| {
                        format!("--report was given no value — the next token `{v}` is a flag, not a locator (a path really named that is spelled ./{v})")
                    });
                }
                None => {
                    usage_error
                        .get_or_insert_with(|| format!("--report requires a <locator> argument ({GATE_USAGE})"));
                    break;
                }
            },
            // Same rule; `-` is accepted here and fails loud as an unreadable policy file a moment
            // later — strictly narrower than refusing it in the grammar (the scan route's reasoning).
            "--policy" => match args.get(i + 1) {
                Some(v) if v == "-" || !v.starts_with('-') => {
                    policy_flag = Some(v.clone());
                    i += 1;
                }
                Some(v) => {
                    usage_error.get_or_insert_with(|| {
                        format!("--policy was given no value — the next token `{v}` is a flag, not a path (a file really named that is spelled ./{v})")
                    });
                }
                None => {
                    usage_error
                        .get_or_insert_with(|| format!("--policy requires a <file> argument ({GATE_USAGE})"));
                    break;
                }
            },
            "--gate-json" => {
                // The scan path's own dash-check, so `--gate-json --policy p` cannot swallow `--policy`
                // and run gateless-green. `-` (stream the verdict to stdout) is the one dash-shaped
                // value allowed.
                match args.get(i + 1) {
                    Some(v) if v == "-" || !v.starts_with('-') => {
                        gate_json = Some(v.clone());
                        i += 1;
                    }
                    _ => {
                        usage_error.get_or_insert_with(|| {
                            "--gate-json requires a value (a path, or `-` for stdout)".to_string()
                        });
                    }
                }
            }
            other => {
                // A stray positional is a USAGE error, never ignored: `gate` takes none, so a swallowed
                // token (a mistyped locator, say) would otherwise gate a DISCOVERED report and read green.
                usage_error.get_or_insert_with(|| {
                    if other.starts_with('-') && other.len() > 1 {
                        format!("unknown flag `{other}` ({GATE_USAGE})")
                    } else {
                        format!("unexpected argument `{other}` ({GATE_USAGE})")
                    }
                });
            }
        }
        i += 1;
    }
    if let Some(why) = usage_error {
        eprintln!("candor-query gate: {why}");
        return refuse(&why, want_json, gate_json.as_deref());
    }

    // The policy: flag, then CANDOR_POLICY, then the config `policy` key — §3.3.1's fallback, the same
    // ladder candor-scan resolves. No policy is a usage error, never a green verdict.
    let policy_path = policy_flag
        .or_else(|| std::env::var("CANDOR_POLICY").ok().filter(|s| !s.is_empty()))
        .or_else(|| {
            candor_classify::policy::discover_config_text(Path::new("."))
                .and_then(|t| config_value(&t, "policy"))
        });
    let Some(policy_path) = policy_path else {
        let why = "a policy is required — pass `--policy <file>`, set CANDOR_POLICY, or add a `policy` \
                   key to .candor/config. `gate` applies a policy to an existing report; with no policy \
                   there is no verdict to give."
            .to_string();
        eprintln!("candor-query gate: {why}");
        return refuse(&why, want_json, gate_json.as_deref());
    };
    let Ok(policy_text) = std::fs::read_to_string(&policy_path) else {
        let why = format!(
            "policy file {policy_path} could not be read — failing (exit 2), policy NOT evaluated"
        );
        eprintln!("candor-query gate: {why}");
        return refuse(&why, want_json, gate_json.as_deref());
    };
    // ⟨0.19⟩ `unknown-alias` expansion for an `Unknown[<alias>]` filter, anchored to the POLICY file
    // exactly as `parsepolicy` anchors it — an alias is part of the policy's own vocabulary, not of the
    // report. The ⟨0.20⟩ `net-partner` list is deliberately NOT loaded: `netClass` is read verbatim from
    // the report, so re-classifying its hosts through THIS machine's config would be the re-derivation
    // §3.1 ⟨0.24⟩ forbids (and would make the verdict depend on the consumer's CWD).
    //
    // ⟨0.24⟩ THE PATH TRAVELS OUT with the text now: a config that supplies vocabulary the verdict used
    // must be NAMED on the verdict (SPEC §3.1), and discovery walks parent directories, so the file that
    // moved the answer can be several levels above the one the operator was looking at.
    let cfg = candor_classify::policy::discover_config(Path::new(&policy_path));
    let aliases =
        cfg.as_ref().map(|(_, t)| candor_classify::policy::parse_unknown_aliases(t)).unwrap_or_default();
    let p = candor_classify::policy::parse_policy_with_aliases(&policy_text, &aliases);
    let vocabulary = (!p.used_aliases.is_empty())
        .then(|| {
            cfg.as_ref().map(|(path, _)| candor_report::GateVocabulary {
                config: path.display().to_string(),
                // ⟨0.24⟩ name → the classes it expanded to, verbatim from the parse (SPEC §3.1
                // `7f5b5ba`) — the same map candor-scan's route accumulates, so §3.1's byte-equality
                // MUST holds one level down too.
                aliases: p.used_aliases.clone(),
            })
        })
        .flatten();

    // ⟨0.24⟩ THE POLICY COULD NOT BE HONOURED AS WRITTEN (SPEC §6.2) — the UNREADABLE-POLICY posture,
    // exactly like the unreadable-file branch above and byte-identically to candor-scan's route on the
    // same policy. See `ParsedPolicy::errors` for why this stopped being a warning: dropping an
    // unrecognised class token rewrites the rule, and the direction that matters NARROWS it (`deny
    // Unknown[dispatch,nativ]` → `[dispatch]`), so the gate silently stops covering native-caused holes
    // while the operator reads a gate that looks armed.
    // ⟨0.24⟩ FATAL errors only — see `ParsedPolicy::fatal_messages`.
    let fatal = p.fatal_messages();
    if !fatal.is_empty() {
        for e in &fatal {
            eprintln!("candor-query gate: policy error — {e}");
        }
        let why = format!(
            "refusing to evaluate a policy that cannot be honoured AS WRITTEN (exit 2, policy NOT \
             evaluated). Fix the token, or define it as an `unknown-alias` in the `.candor/config` \
             beside {policy_path}. Policy error(s): {}",
            fatal.join("  ·  ")
        );
        eprintln!("candor-query gate: {why}");
        return refuse(&why, want_json, gate_json.as_deref());
    }

    // ⟨0.28⟩ SPEC §6.2: THE LINES THE PARSE DROPPED ride the verdict document as `ignored` —
    // `[{line, text, reason}]`, distinct from `unevaluated` (rules that PARSED and could not be
    // answered). Survivable errors only: a fatal error refused just above, and the zero-rule arm below
    // refuses too, so this list reaches only VERDICT documents. Omitted when empty (byte-identity on a
    // clean policy). Same reader as candor-scan's route (`ParsedPolicy::errors`, `fatal == false`), so
    // §3.1's byte-equality MUST holds for this key as well.
    let ignored: Vec<candor_report::IgnoredLine> = p
        .errors
        .iter()
        .filter(|e| !e.fatal)
        .map(|e| candor_report::IgnoredLine {
            line: e.line,
            text: e.text.clone(),
            reason: e.message.clone(),
        })
        .collect();

    // ⟨0.28⟩ A CONFIGURED POLICY THAT YIELDED ZERO RULES (SPEC §6.2) — the same refusal posture as the
    // branch directly above, and THE SIBLING OF candor-scan's. §6.2 states the defect was measured on
    // this verb too, and "a route is not covered by its sibling": the scan CLI got the rung first and
    // this route kept exiting 0 with `{"ok":true,"violations":[]}` over a README, which on the
    // SUPPLY-CHAIN surface — a consumer pointing the gate at a report someone else produced — is the
    // reading this rung exists to make impossible.
    //
    // All three rule vectors, for the reason candor-scan's commit records: keying on `rules` alone makes
    // an allow-only policy refuse as if it had none. A `forbid`-only policy refuses on this route anyway,
    // one branch below, for an unrelated and specific reason (a report's `calls` graph is
    // effect-relevant), and that refusal names its own cause rather than this one.
    if p.rules.is_empty() && p.allow_rules.is_empty() && p.layer_rules.is_empty() && p.only_rules.is_empty() {
        let why = format!(
            "the policy at {policy_path} yielded NO RULES — refusing (exit 2, policy NOT evaluated). \
             Every line was ignored, the file is empty, or it holds only comments. A gate with no rules \
             cannot have caught anything, and reporting `ok: true` here would be indistinguishable from \
             a gate that ran and found nothing. If you did not mean to gate, do not configure a policy."
        );
        eprintln!("candor-query gate: {why}");
        // ⟨0.28⟩ SPEC §6.2: "The `unevaluated` list carries one entry naming the whole policy" — the
        // shape §3.1 already pins for a policy with no lines to name. This branch used to write the
        // refusal with NO `unevaluated` at all, while candor-scan's zero-rule refusal carried exactly
        // this entry — the two gate routes disagreeing about one document (§3.1's byte-equality MUST).
        // Same strings as the scan route's, so the routes cannot drift.
        return refuse_disclosing(
            &why,
            &[candor_report::Unevaluated {
                rule: format!("(entire policy {policy_path} — no rules parsed)"),
                why: "the configured policy yielded zero rules, so nothing was evaluated and no rule \
                      can have passed"
                    .to_string(),
            }],
            want_json,
            gate_json.as_deref(),
        );
    }

    // THE POLICY-LEVEL REFUSALS. Whole-policy, not per-rule: enforcing the answerable half and exiting 0
    // is gateless-green — the user believes a rule is enforced that never ran.
    //
    // ⟨0.24⟩ **COMPUTED HERE, ACTED ON AFTER THE GATE — the precedence has no carve-out for a refusal's
    // KIND** (SPEC §3.1 `1503368`). These two used to `return refuse(…)` on the spot, so MEASURED
    // 2026-07-28: `deny Fs app.writes` beside `forbid app -> dep` exited 2 with the certain `Fs`
    // violation absent from the document — the identical harm `8b97e5c` had just fixed for the
    // per-(rule, function) refusal, surviving one branch higher up because that was not where the
    // measurement had been taken.
    //
    // **Lemma 2 does not care which KIND of refusal stands beside the firing rule.** `Reject` is
    // upward-closed, so a rule that already fired on carried evidence stays fired however the refused
    // rule would have resolved. The WHOLE-POLICY granularity of these two governs *which rules go
    // unevaluated*; it was never a licence to suppress a violation that was evaluated and certain.
    //
    // THE EXCEPTION IS PRINCIPLED, NOT AN OPTIMISATION: with no `deny`/`pure` rule in the policy there is
    // nothing that COULD dominate, so the refusal is the whole verdict and is taken immediately — which
    // also means a `forbid`-only policy still refuses without needing a report to exist. Deferring is
    // only ever right when something might overrule.
    //
    // ⟨0.24⟩ **ONE ENTRY PER RULE, NOT PER KIND** (SPEC §3.1 `fc4b5f6`). These two used to be one string
    // each — `"this policy has 2 forbid rule(s)…"` — which is the KIND AGGREGATE the clause rejects: it
    // answers *how many* when the operator's question is *which*. The reason genuinely IS a property of
    // the kind, so the same `why` repeats across that kind's entries; what must not repeat is the `rule`,
    // and that is the field a consumer joins on.
    let policy_refusals = whole_policy_refusals(&p, &policy_path);
    // The human line is DERIVED from the same pair the document carries, so the two channels cannot
    // disagree about which rule went unanswered — the split that produced the false disposition claim
    // `8b97e5c` removed.
    let say = |u: &candor_report::Unevaluated| format!("`{}` — {}", u.rule, u.why);
    if !policy_refusals.is_empty() && p.rules.is_empty() {
        for u in &policy_refusals {
            eprintln!("candor-query gate: {}", say(u));
        }
        return refuse_disclosing(
            &policy_refusals.iter().map(&say).collect::<Vec<_>>().join("  ·  "),
            &policy_refusals,
            want_json,
            gate_json.as_deref(),
        );
    }
    // **AND A REFUSED RULE IS UNEVALUATED — THAT IS WHAT REFUSED MEANS.** Dropping them from the policy
    // handed to `gate()` is not tidying: the refusal used to `return` before `gate()` ran, so deferring
    // it silently started EVALUATING them. Caught by this file's own test on the first run — `deny Net`
    // beside `allow Net other.example.com` produced an AS-EFF-008 record in the document, derived from a
    // `surface_incomplete` map `report_signature` leaves EMPTY on purpose. That is precisely the
    // unsound verdict the `allow` refusal exists to prevent, now shipped INSIDE the document as if it
    // were certain, and the same argument applies to `forbid` over an effect-relevant `calls` graph.
    //
    // A neat illustration of the general hazard: the question to ask of any short-circuit removal is what
    // code now runs that never ran, and what it assumed about who would call it. Same question, same
    // session, second answer.
    let mut p = p;
    p.allow_rules.clear();
    p.layer_rules.clear();
    // ⟨0.29⟩ …AND THE PERMISSION FORM. This line was MISSING for one build: `only` was disclosed as
    // unanswerable by `whole_policy_refusals` above and then evaluated anyway, so the gate printed
    // `[AS-EFF-009] model::leaks reaches infra::db_read` beside its own statement that the rule could not
    // be evaluated — a rule evaluated from a report, which is the §3.1 MUST this removal exists to
    // enforce. The comment above asks "what code now runs that never ran"; the mirror question is what
    // code STOPS running when a kind is added, and the answer here was "nothing, because nobody told it".
    //
    // Found by the conformance row, and only after that row was made non-vacuous twice: with an
    // `only`-only policy every engine refuses before evaluating anything, and over a wholly PURE fixture
    // the report carries no call graph to walk. It took an answerable rule beside it AND an effect in the
    // tree before the leak could show itself. The same defect shipped in the java arm for one build.
    p.only_rules.clear();
    let p = p;

    let Some(prefix) = report_flag.or_else(discover_report_prefix) else {
        let why = "no report — pass --report <locator> or run from a repo with a .candor/ dir (scan: \
                   candor-scan . --out .candor/report)"
            .to_string();
        eprintln!("candor-query gate: {why}");
        return refuse(&why, want_json, gate_json.as_deref());
    };
    let rep = match load_gate_report(&prefix) {
        Ok(r) => r,
        // ⟨0.24⟩ The reason travels now (SPEC §3.1 `1503368` (b)): a report that did not load is exactly
        // the case where a consumer reading the `--gate-json` path unconditionally most needs to be told
        // something other than yesterday's answer.
        Err(why) => {
            return refuse(&why, want_json, gate_json.as_deref());
        }
    };
    // ⟨0.24⟩ THE REPORT JUDGED NOTHING (SPEC §3.1) — DISCLOSED, NOT REFUSED.
    //
    // This verb's whole contract is that the report IS the signature: nothing is re-derived, and an entry
    // ABSENT from it is the ⟨0.21⟩ purity claim, taken as given. A report whose `analyzed.count` is 0 has
    // made no judgment for any unit, so every rule below is answered by silence — this is not an
    // effect-free package, it is a package nothing was said about, and the verb MUST say so.
    //
    // IT SAYS SO ON STDERR, AND CHANGES NOTHING ELSE. This branch used to `return 2` and write no verdict
    // document, and that was WRONG in a way three of these lines argued for at length. §3.1's own
    // byte-equality MUST says this verb's `--gate-json` must match `candor-scan --policy`'s, and a scan
    // of an empty facade crate exits 0 with a clean `{ok:true, analyzed:{count:0}, violations:[]}`.
    // Measured 2026-07-28 on a real crate this engine's own scan judges as count-0: the scan wrote that
    // document and exited 0 while this route exited 2 and wrote NOTHING — the strongest possible failure
    // of the byte-equality MUST, on a report the scan itself had just produced, and on a measured 7–10%
    // of real dependency reports. Refusing also minted a THIRD exit-2 cause: §3.3 enumerates exactly two
    // (a broken gate CONFIG; an INCOMPLETE analysis of the target's OWN code) and a judged-nothing
    // DEPENDENCY is neither. Tom corrected the spec clause rather than the engines (candor-spec
    // `0744d29`) — candor-ts had already taken this reading, and the harm here is the DELETED DISCLOSURE,
    // not the verdict: restoring a verdict would assert an effect the consumer has no evidence for.
    //
    // Keyed on the integer, never on `functions` being empty — a legitimately all-pure `count: n>0,
    // functions: []` report is a CLAIM §2 rule 3 requires this verb to BELIEVE, and a predicate keyed on
    // emptiness would withdraw it. Measured over 1997 JVM dependency jars, that plausible-but-wrong fix
    // would have withdrawn 104 real claims to hedge 6.
    for pkg in &rep.judged_nothing_pkgs {
        eprintln!(
            "candor-query gate: NOTE — `{pkg}` says it JUDGED NOTHING (⟨0.24⟩ `analyzed.count` is 0, or \
             absent with no entries), so the verdict below is about no unit at all: an absent entry is \
             candor's purity claim only where something was judged. This is usually a facade or \
             re-export-only package — gate what it re-exports, or scan its source \
             (candor-scan <dir> --policy {policy_path}). The verdict and exit code are unchanged: this \
             report makes no claim, and inventing one for it would be the opposite defect."
        );
    }
    let sig = report_signature(&rep.entries);

    // ⟨0.24⟩ THE PRECEDENCE: **violation (1) > refusal (2) > incomplete (2)**, and the first rung is
    // FORCED by Lemma 2 rather than chosen (SPEC §3.1).
    //
    // The third refusal — the only one that depends on the REPORT rather than on the policy alone — is
    // COMPUTED here and ACTED ON below, after the gate has run. It used to `return 2` on this line,
    // before `gate()` was ever called, so a policy carrying a firing `deny Fs` PLUS one unanswerable
    // scoped rule exited 2 and wrote NO document: **the certain violation was deleted from the
    // machine-consumer channel by the rule it had nothing to do with.** Measured 2026-07-28 on
    // `deny Fs app.fsUnit` + `deny Net[unknown-host] app.netNoClass` — exit 2, no `--gate-json` file,
    // the `Fs` finding gone. Byte-identical in harm to the incomplete-analysis path `ff34070` fixed one
    // rung down, and the same fix: compute the verdict FIRST, decide the exit FROM it.
    //
    // WHY THE VIOLATION IS SAFE TO REPORT even though a rule went unanswered: if one rule FIRES on
    // evidence the report carries, the policy is REJECTED, and `Reject` is upward-closed (PAPER3 Lemma
    // 2) — however the unanswerable rule would have resolved cannot un-reject it. Exit 1 is therefore
    // not merely fail-closed here, it is CERTAIN, and it is strictly more informative than exit 2
    // because it names the violation. (All four engines had this backwards; the spec clause pinning
    // "refusal > violation" was corrected within the hour of being written — candor-spec `7271c69`.)
    //
    // The refusal is NOT swallowed: when a violation dominates, every unanswerable rule is still
    // disclosed on stderr below. Exit 1 reports the violation it is sure of; it does not conceal the
    // part it could not read.
    //
    // ⟨0.24⟩ ONE LIST, BOTH KINDS. The whole-policy `forbid`/`allow` refusals join the per-(rule,
    // function) ones here rather than having taken their own exit above, because the precedence is
    // general (SPEC §3.1 `1503368`) and because two refusal channels would mean two chances to write the
    // disclosure and two chances to forget it. Whole-policy first: it is the bigger claim about what the
    // verdict below does not cover.
    let mut refused = policy_refusals;
    refused.extend(unanswerable_scoped_filters(&p, &sig));

    // ⟨0.24⟩ `gate()` now WITHHOLDS the `(rule, function)` pairs whose narrowing filter the signature
    // cannot answer, instead of charging them off the matcher's `unresolved` floor (SPEC §3.1). On THIS
    // route the withheld set is by construction a subset of what `unanswerable_scoped_filters` already
    // refuses — both key on "the rule narrows, the fn carries the effect, the determinable class set is
    // empty" — so the disclosure below covers it and the two cannot disagree about a function. The
    // ASSERTION is what keeps that true if either predicate is edited: a pair withheld by the gate and
    // NOT named by a refusal would be a rule dropped in silence.
    let outcome = candor_classify::gate::gate(&p, &sig.as_input());
    debug_assert!(
        outcome.withheld.is_empty() || !refused.is_empty(),
        "gate() withheld {:?} but no rule was refused — a withheld rule must always be disclosed",
        outcome.withheld
    );
    // ⟨0.27⟩ SPEC §4 — THE ZERO-MATCH DISCLOSURE BELONGS ON THIS ROUTE TOO, and its absence here was
    // found by a cross-engine differential: java and swift disclosed on `gate --report`, rust and ts did
    // not, so the same typo'd policy was reported by two engines and silently scored as satisfied by two.
    // §4's MUST carries no route qualifier, and this is the SUPPLY-CHAIN gate — the surface a consumer
    // points at a report someone else produced, and the one ⟨0.24⟩ called the enforcement surface. The
    // exit code is untouched here exactly as it is on the scan route.
    for raw in &outcome.zero_match {
        eprintln!(
            "candor: policy rule matched NO function — `{raw}`. It was evaluated and bound nothing, \
             so it cannot have caught anything. Legitimate when one policy is shared across repos; \
             a typo'd layer name otherwise."
        );
    }
    // ⟨0.27⟩ …and the SAME list rides the verdict document as `zeroMatch` (SPEC §4): stderr is not the
    // machine channel, and §3.1's byte-equality MUST means this route and the scan route must carry it
    // identically — the shared `gate()` computes it once, both writers serialize it.
    let zero_match = outcome.zero_match;
    let mut violations = outcome.violations;
    if violations.is_empty() && !refused.is_empty() {
        // SOLE refusal: nothing certain to report, so the gate genuinely could not be evaluated. THIS is
        // the branch that gets to say "Refusing (exit 2)", and it says it because it is about to do it.
        //
        // BUILT ONCE AND USED FOR BOTH CHANNELS, so the human line and the `--gate-json` document's
        // `reason` cannot disagree about the disposition — which is precisely the split that produced the
        // false claim this commit removes.
        let sole: Vec<String> = refused
            .iter()
            .map(|u| {
                format!(
                    "{} Refusing (exit 2) — no rule fired on evidence this report carries, so there \
                     is no verdict to stand beside this.",
                    say(u)
                )
            })
            .collect();
        for why in &sole {
            eprintln!("candor-query gate: {why}");
        }
        if !rep.unanalyzed.is_empty() {
            // Refusal (2) outranks incomplete (2) — same exit, and the refusal is the reason the
            // verdict does not exist. The manifest still gets said, on the human channel.
            eprintln!(
                "candor-query gate: (the report ALSO declares {} unanalyzed unit(s) — that alone would \
                 have been exit 2)",
                rep.unanalyzed.len()
            );
        }
        // ⟨0.24⟩ …and the REFUSAL DOCUMENT carries the list too (SPEC §3.1 `fc4b5f6`). A prose `reason`
        // is not a list of rules: this is the case where nothing fired, so `reason` is all a consumer
        // would otherwise have, and it is exactly the case the operator most needs to iterate.
        return refuse_disclosing(&sole.join("  ·  "), &refused, want_json, gate_json.as_deref());
    }
    // ⟨0.24⟩ THE DOMINATED DISCLOSURE — and the claim in it is now COUNTED, not asserted.
    //
    // MEASURED (2026-07-28, `deny Unknown[unresolved] app.opaque` as the SOLE rule in the policy): this
    // note printed *"The verdict stands anyway: a rule FIRED on evidence this report carries"* when NO
    // rule had fired on carried evidence — the only rule in the policy was the unanswerable one, and the
    // "violation" it was standing on was the fabrication the commit before this one removed. The sentence
    // was attached UNCONDITIONALLY to the refusal path rather than conditioned on a violation having been
    // recorded, so it could not have been true in the sole-refusal case and was never going to be.
    //
    // A FALSE DISCLOSURE IS WORSE THAN A MISSING ONE — this family already has the precedent (`net-partner`
    // reported as an "ignoring unknown config key" WHILE BEING HONOURED, conformance PART 13b). So the
    // claim is made from the fact rather than beside it: the branch is guarded on BOTH lists explicitly
    // rather than on falling past the `return` above, and the sentence NAMES the number of violations it
    // is standing on. A count cannot be printed truthfully at zero, which is the point — the shape of the
    // message makes the false version unwritable instead of merely unwritten.
    if !refused.is_empty() && !violations.is_empty() {
        eprintln!(
            "candor-query gate: NOTE — {} policy rule(s) could not be evaluated over this report and are \
             NOT answered by the verdict below. The verdict stands anyway: the {} violation(s) reported \
             below FIRED on evidence this report carries, and no resolution of an unanswered rule can \
             un-reject an already-rejected policy (SPEC §3.1, PAPER3 Lemma 2). The exit code below \
             answers those, and NOT these:",
            refused.len(),
            violations.len(),
        );
        for u in &refused {
            eprintln!("    {}", say(u));
        }
    }
    // Human output goes to STDERR whenever stdout carries the verdict document, exactly as the scan
    // routes it, so `candor-query gate … --json | jq` sees pure JSON.
    let stdout_is_json = want_json || gate_json.as_deref() == Some("-");
    for gv in &violations {
        let line = format!("[{}] {}", gv.rule, gv.detail);
        if stdout_is_json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }

    // ⟨0.15⟩ the advisory κ note — the same names, from the same ledger, that the scan's verdict carries.
    let coverage = (!rep.coverage_packages.is_empty()).then(|| candor_report::GateCoverage {
        uncovered: rep.coverage_packages.len(),
        packages: rep.coverage_packages.iter().cloned().collect(),
    });
    // ⟨0.21⟩ COMPLETENESS MANIFEST — ON THE SAME DOCUMENT AS THE VIOLATIONS, never instead of them.
    //
    // THE DEFECT THIS ORDERING FIXES (measured 2026-07-28 on a `deny Net` over a report carrying two Net
    // units AND a one-entry `unanalyzed`): the manifest branch used to run FIRST and write
    // `write_verdict(&mut [], …)` — an EMPTY violation list — so a CI consumer read `ok:false`,
    // `incomplete:true` and NO violations, and the two findings that had just been printed to stderr
    // never reached the PR. Incompleteness and a violation are not alternatives: SPEC §3.3 says a
    // configured gate over incompletely-analyzed code MUST fail closed (exit ≠ 0) and "a real violation
    // (exit 1) still dominates". Dominating the EXIT CODE while deleting the finding from the DOCUMENT
    // is the worse half of the same sin — the exit code is one bit, the document is the evidence.
    //
    // So: ONE verdict, always, carrying violations + `analyzed` + (when non-empty) `incomplete`/
    // `unanalyzed` + the κ note — `gate_verdict_json_full` already computes `ok = no violations AND not
    // incomplete`. Then the exit code is decided FROM it: 1 if anything was violated, else 2 if anything
    // was unanalyzed, else 0. candor-scan's route was rewritten to the same shape in the same commit, so
    // §3.1's byte-equality MUST still holds on this path (it did not before — the two routes agreed only
    // because they dropped the same violations).
    if !write_verdict(
        &mut violations,
        coverage.as_ref(),
        rep.analyzed_count,
        &rep.unanalyzed,
        vocabulary.as_ref(),
        // ⟨0.24⟩ THE UNANSWERED RULES RIDE THE SAME DOCUMENT AS THE VIOLATIONS (SPEC §3.1 `fc4b5f6`).
        // Until now this list existed, was correct, and went to stderr ONLY — so a machine consumer of
        // an exit-1 verdict could not see that any rule had gone unanswered, which is a finding that
        // never reaches the consumer arriving through the disclosure this rung added to stop that. It is
        // BESIDE the violations and not instead of them: Lemma 2 makes a firing rule certain however
        // these would have resolved.
        &refused,
        // ⟨0.27⟩ the zero-match disclosure, same bytes as the scan route's (SPEC §4 `zeroMatch`).
        &zero_match,
        // ⟨0.28⟩ the dropped-line disclosure, same bytes as the scan route's (SPEC §6.2 `ignored`).
        &ignored,
        // ⟨0.30⟩ the peek's findings, off the REPORT — same bytes as the scan route's, which is what
        // makes §3.1 byte-equality hold on a route that cannot peek for itself.
        &rep.out_of_scope,
        &rep.net_partners,
        want_json,
        gate_json.as_deref(),
    ) {
        return 2;
    }
    if !violations.is_empty() {
        eprintln!("candor-query gate: {} policy violation(s)", violations.len());
        eprintln!("→ candor-query fix-gate names the remedy for each (or `candor fix <fn> <Effect>` for one)");
        1
    } else if !rep.unanalyzed.is_empty() {
        eprintln!(
            "candor-query gate: NOT certified — the report declares {} unit(s) candor could not analyze; \
             a gate cannot be green over unanalyzed code",
            rep.unanalyzed.len()
        );
        2
    } else if !rep.out_of_scope.is_empty() {
        // ⟨0.30⟩ the SCOPE half of the same posture, and the same exit. Named separately from the
        // `unanalyzed` arm above because the repairs differ: that one wants a scan that can read a file,
        // this one wants a scan whose selector reaches the code the policy is about.
        eprintln!(
            "candor-query gate: NOT certified — the report names {} function(s) OUTSIDE the scan's scope \
             performing an effect this policy denies; the gate did not judge them, so the verdict is \
             incomplete rather than a pass",
            rep.out_of_scope.len()
        );
        2
    } else {
        eprintln!("candor-query gate: policy ✓ (the report's own signature — no re-scan, no re-derivation)");
        0
    }
}

/// Write the §3.3 verdict through the SHARED serializer candor-scan's `write_gate_json` uses — the same
/// document, the same field order, the same violation sort. `--json` is `--gate-json -`, so both may fire.
/// Returns false when a write failed (the caller exits 2 — a verdict a consumer never receives must not
/// pass for one it did).
#[allow(clippy::too_many_arguments)]
fn write_verdict(
    violations: &mut [candor_report::GateViolation],
    coverage: Option<&candor_report::GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[candor_report::UnanalyzedUnit],
    vocabulary: Option<&candor_report::GateVocabulary>,
    unevaluated: &[candor_report::Unevaluated],
    zero_match: &[String],
    ignored: &[candor_report::IgnoredLine],
    out_of_scope: &[candor_report::OutOfScopeFinding],
    // ⟨0.31⟩ the producer's ambient-partner provenance, copied — never recomputed here.
    net_partners: &[candor_report::NetPartners],
    want_json: bool,
    gate_json: Option<&str>,
) -> bool {
    let mut targets: Vec<&str> = Vec::new();
    if want_json {
        targets.push("-");
    }
    if let Some(p) = gate_json
        && !(want_json && p == "-")
    {
        targets.push(p);
    }
    if targets.is_empty() {
        return true;
    }
    let json = match candor_report::gate_verdict_json_v31(
        violations,
        coverage,
        analyzed_count,
        unanalyzed,
        vocabulary,
        unevaluated,
        zero_match,
        ignored,
        out_of_scope,
        net_partners,
    ) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("candor-query gate: could not serialize the gate verdict ({e})");
            return false;
        }
    };
    for t in targets {
        if t == "-" {
            println!("{json}");
        } else if let Err(e) = candor_report::write_atomic(Path::new(t), format!("{json}\n").as_bytes()) {
            eprintln!("candor-query gate: could not write --gate-json {t} ({e})");
            return false;
        }
    }
    true
}

/// Read a single-value `.candor/config` key (`<key> <value>`), case-insensitive on the key like every
/// other config reader in the family. Only ever asked for `policy` here — the `deps` key this file
/// deliberately never consults is what the §3.1 MUST NOT is about.
fn config_value(text: &str, key: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.splitn(2, char::is_whitespace);
        if it.next().is_some_and(|k| k.eq_ignore_ascii_case(key)) {
            let v = it.next().unwrap_or("").trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}
