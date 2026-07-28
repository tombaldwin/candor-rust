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
/// `Err(2)` on a locator that matches no report, or one that is found-but-corrupt: §3.1's found-but-
/// corrupt rule — a report that cannot be parsed is corrupt input, not an effect-free package, and a
/// policy gated over the resulting empty map would PASS. Never a silently-empty "no violations".
fn load_gate_report(prefix: &str) -> Result<GateReport, i32> {
    let paths = glob_reports(prefix);
    if paths.is_empty() {
        eprintln!(
            "candor-query gate: no report files at prefix `{prefix}` — nothing to gate \
             (scan first: candor-scan . --out {prefix})"
        );
        return Err(2);
    }
    let mut out = GateReport {
        entries: Vec::new(),
        analyzed_count: 0,
        unanalyzed: Vec::new(),
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
        eprintln!(
            "candor-query gate: refusing to gate over a report that did not load cleanly — \
             re-run the scan (a partial signature makes a green verdict meaningless)"
        );
        return Err(2);
    }
    Ok(out)
}

// ── the report route into the gate ──────────────────────────────────────────────────────────────

/// The owned accumulators a [`candor_classify::gate::GateInput`] borrows. Built from a written report
/// and nothing else; the counterpart of candor-scan's `policy_violations`, which builds the same struct
/// from the classifier's fixpoints. Both feed the one `candor_classify::gate::gate`.
struct ReportSignature {
    all: Vec<String>,
    inferred: HashMap<String, BTreeSet<String>>,
    calls: HashMap<String, BTreeSet<String>>,
    hosts: HashMap<String, BTreeSet<String>>,
    cmds: HashMap<String, BTreeSet<String>>,
    paths: HashMap<String, BTreeSet<String>>,
    tables: HashMap<String, BTreeSet<String>>,
    /// Deliberately EMPTY — see [`gate_input_from_report`]; every `allow` rule is refused upstream.
    surface_incomplete: HashMap<String, BTreeSet<String>>,
    reason_classes: HashMap<String, BTreeSet<String>>,
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
fn gate_input_from_report(rep: &GateReport) -> ReportSignature {
    let mut inferred: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut net: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut why_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut names: BTreeSet<String> = BTreeSet::new();

    for e in &rep.entries {
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
/// by `deny E Unknown[unresolved]` is ANSWERED and not refused: `gate_input_from_report` contributes
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
fn unanswerable_scoped_filters(
    p: &candor_classify::policy::ParsedPolicy,
    sig: &ReportSignature,
) -> Vec<String> {
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
                out.push(format!(
                    "`{}` narrows on the Net DESTINATION CLASS, but `{q}` carries Net with no `netClass` \
                     in this report — the field the filter reads is absent, so the narrowing would \
                     succeed for lack of evidence and drop a Net the bare `deny Net` catches. Refusing \
                     (exit 2) rather than passing: an absent optional field must not relax a fail-closed \
                     gate. Use the bare `deny Net`, or gate at scan time.",
                    r.raw.trim()
                ));
                break;
            }
            if !r.unknown_classes.is_empty()
                && has("Unknown")
                && sig.reason_classes.get(q).map(|c| c.is_empty()).unwrap_or(true)
            {
                out.push(format!(
                    "`{}` narrows on the Unknown REASON CLASS, but `{q}` carries Unknown with no reason \
                     reachable in this report — neither its own `unknownWhy` nor a `calls` edge to one. \
                     §6.2 resolves the class set TRANSITIVELY over the gate's reach; with the channel \
                     missing, every narrowed filter silently tolerates while only the bare `deny Unknown` \
                     fires. Refusing (exit 2). Use the bare `deny Unknown`, or gate at scan time.",
                    r.raw.trim()
                ));
                break;
            }
        }
    }
    out
}

// ── the CLI ─────────────────────────────────────────────────────────────────────────────────────

const GATE_USAGE: &str =
    "usage: candor-query gate --report <locator> --policy <file> [--json] [--gate-json <file>]";

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
pub(crate) fn cmd_gate(args: &[String]) -> i32 {
    let mut report_flag: Option<String> = None;
    let mut policy_flag: Option<String> = None;
    let mut gate_json: Option<String> = None;
    let mut want_json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => want_json = true,
            // candor-ts output-mode flags (#8): rust prose is the default, so accept + ignore.
            "--text" | "--human" => {}
            "--report" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("candor-query gate: --report requires a <locator> argument ({GATE_USAGE})");
                    return 2;
                };
                report_flag = Some(resolve_locator(v));
                i += 1;
            }
            "--policy" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("candor-query gate: --policy requires a <file> argument ({GATE_USAGE})");
                    return 2;
                };
                policy_flag = Some(v.clone());
                i += 1;
            }
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
                        eprintln!(
                            "candor-query gate: --gate-json requires a value (a path, or `-` for stdout)"
                        );
                        return 2;
                    }
                }
            }
            other => {
                // A stray positional is a USAGE error, never ignored: `gate` takes none, so a swallowed
                // token (a mistyped locator, say) would otherwise gate a DISCOVERED report and read green.
                if other.starts_with('-') && other.len() > 1 {
                    eprintln!("candor-query gate: unknown flag `{other}` ({GATE_USAGE})");
                } else {
                    eprintln!("candor-query gate: unexpected argument `{other}` ({GATE_USAGE})");
                }
                return 2;
            }
        }
        i += 1;
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
        eprintln!(
            "candor-query gate: a policy is required — pass `--policy <file>`, set CANDOR_POLICY, or add \
             a `policy` key to .candor/config. `gate` applies a policy to an existing report; with no \
             policy there is no verdict to give."
        );
        return 2;
    };
    let Ok(policy_text) = std::fs::read_to_string(&policy_path) else {
        eprintln!(
            "candor-query gate: policy file {policy_path} could not be read — failing (exit 2), policy \
             NOT evaluated"
        );
        return 2;
    };
    // ⟨0.19⟩ `unknown-alias` expansion for an `Unknown[<alias>]` filter, anchored to the POLICY file
    // exactly as `parsepolicy` anchors it — an alias is part of the policy's own vocabulary, not of the
    // report. The ⟨0.20⟩ `net-partner` list is deliberately NOT loaded: `netClass` is read verbatim from
    // the report, so re-classifying its hosts through THIS machine's config would be the re-derivation
    // §3.1 ⟨0.24⟩ forbids (and would make the verdict depend on the consumer's CWD).
    let aliases = candor_classify::policy::discover_config_text(Path::new(&policy_path))
        .map(|t| candor_classify::policy::parse_unknown_aliases(&t))
        .unwrap_or_default();
    let p = candor_classify::policy::parse_policy_with_aliases(&policy_text, &aliases);

    // THE POLICY-LEVEL REFUSALS. Whole-policy, not per-rule: enforcing the answerable half and exiting 0
    // is gateless-green — the user believes a rule is enforced that never ran.
    if !p.layer_rules.is_empty() {
        eprintln!(
            "candor-query gate: this policy has {} `forbid` rule(s), which `gate --report` cannot \
             evaluate — a report's `calls` graph is EFFECT-RELEVANT (only callees with a non-empty effect \
             set are written), so a crossing into a wholly PURE unit is invisible in it, while `forbid` \
             matches on NAME. The rule would read green over a crossing a scan fails on. Gate layering at \
             scan time: candor-scan . --policy {policy_path}",
            p.layer_rules.len()
        );
        return 2;
    }
    if !p.allow_rules.is_empty() {
        let effects: BTreeSet<&str> = p.allow_rules.iter().map(|r| r.effect).collect();
        eprintln!(
            "candor-query gate: this policy has `allow {}` rule(s), which `gate --report` cannot \
             evaluate — the AS-EFF-008 surface-completeness marker does not ride the report wire as a \
             gate-usable fact, so a benign visible literal beside a runtime-computed endpoint would be \
             CERTIFIED here and flagged by a scan. (`netClass: unknown-host` is NOT that marker — it also \
             names a merely unrecognised host.) Gate allowlists at scan time: candor-scan . --policy \
             {policy_path}",
            effects.into_iter().collect::<Vec<_>>().join("`/`")
        );
        return 2;
    }

    let Some(prefix) = report_flag.or_else(discover_report_prefix) else {
        eprintln!(
            "candor-query gate: no report — pass --report <locator> or run from a repo with a .candor/ \
             dir (scan: candor-scan . --out .candor/report)"
        );
        return 2;
    };
    let rep = match load_gate_report(&prefix) {
        Ok(r) => r,
        Err(code) => return code,
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
    let sig = gate_input_from_report(&rep);

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
    let refused = unanswerable_scoped_filters(&p, &sig);

    let mut violations = candor_classify::gate::gate(&p, &sig.as_input());
    if violations.is_empty() && !refused.is_empty() {
        // SOLE refusal: nothing certain to report, so the gate genuinely could not be evaluated.
        for why in &refused {
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
        return 2;
    }
    if !refused.is_empty() {
        eprintln!(
            "candor-query gate: NOTE — {} policy rule(s) could not be evaluated over this report and are \
             NOT answered by the verdict below. The verdict stands anyway: a rule FIRED on evidence this \
             report carries, and no resolution of an unanswered rule can un-reject a rejected policy \
             (SPEC §3.1, PAPER3 Lemma 2). Unanswered:",
            refused.len()
        );
        for why in &refused {
            eprintln!("    {why}");
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
    } else {
        eprintln!("candor-query gate: policy ✓ (the report's own signature — no re-scan, no re-derivation)");
        0
    }
}

/// Write the §3.3 verdict through the SHARED serializer candor-scan's `write_gate_json` uses — the same
/// document, the same field order, the same violation sort. `--json` is `--gate-json -`, so both may fire.
/// Returns false when a write failed (the caller exits 2 — a verdict a consumer never receives must not
/// pass for one it did).
fn write_verdict(
    violations: &mut [candor_report::GateViolation],
    coverage: Option<&candor_report::GateCoverage>,
    analyzed_count: usize,
    unanalyzed: &[candor_report::UnanalyzedUnit],
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
    let json = match candor_report::gate_verdict_json_full(violations, coverage, analyzed_count, unanalyzed) {
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
