//! The stable policy gate (spec §6.2): deny/pure (AS-EFF-006), allowlists (AS-EFF-008),
//! layering (AS-EFF-009), and the `--gate-json` structured verdict (spec §3.3).

use crate::*;

/// One structured gate violation (candor-spec §3.3 ⟨0.8⟩) — the SHARED `candor_report::GateViolation`
/// (one definition across the stable scanner, the deep engine, and `candor-query gate-verdict`, so the
/// verdict shape can never drift): `effects` is the specific effect set the violation concerns — the
/// denied set (006), the allowed effect (008), or [] (009 layer-flow, no single effect); `detail` is
/// the message BODY (no `[AS-EFF-00x]` prefix — the rule carries the code). The console gate prints
/// `[{rule}] {detail}`; --gate-json serializes these records verbatim.
pub(crate) use candor_report::GateViolation;

/// ⟨0.20⟩ The `Net` destination classes an fn reaches (transitive). MOVED to `candor_classify::gate`
/// at ⟨0.24⟩ and re-exported here so scan.rs's `netClass` writer keeps its call site: the report FIELD
/// and the gate FILTER have to be the same set, and §3.1's byte-equivalence obligation is exactly the
/// claim that they are — so the derivation now sits next to the gate that reads it off the wire.
pub(crate) use candor_classify::gate::net_classes_of;

/// Evaluate a CANDOR_POLICY (parsed by the SHARED §6.2 parser in candor-classify, so this gate can
/// never disagree with the nightly/JVM gates on grammar) over a finished scan. Returns one line per
/// violation: deny/pure (AS-EFF-006) against the transitive `inferred` sets, literal allowlists
/// (AS-EFF-008) against the transitive hosts/cmds/paths/tables surfaces, layering `forbid A -> B`
/// (AS-EFF-009) by reachability over the local call graph.
///
/// ⟨0.24⟩ THE SCAN ROUTE INTO THE GATE, and now only that: the matching itself moved to
/// `candor_classify::gate::gate`, which `candor-query gate --report` (SPEC §3.1) also calls with a
/// signature read from a written report. This function's remaining job is to build the [`GateInput`]
/// from the classifier's accumulators — including materializing the ⟨0.20⟩ destination classes for
/// every `Net`-bearing fn, exactly the set the lazy call used to compute on demand.
/// ⟨0.24⟩ What the caller must know about `policy_text` BEFORE any verdict is derived from it: its §6.2
/// POLICY ERRORS (empty ⇒ the policy can be honoured AS WRITTEN), and the `unknown-alias` names it
/// actually resolved through (non-empty ⇒ a config supplied vocabulary the verdict used, and §3.1 makes
/// the document NAME that file).
///
/// SILENT (no warnings): the caller prints the errors itself, and this must not double-print the
/// ordinary parse warnings [`policy_violations`] emits on the same text a line later.
///
/// A SEPARATE PASS rather than a second return value from `policy_violations`, because the refusal has
/// to happen BEFORE any of the classifier's accumulators are consulted — the point is that no verdict is
/// produced from a rewritten policy, not that one is produced and then discarded.
/// ⟨0.27⟩ Returns each fatal error as `(raw rule line, message)` rather than the bare message: the
/// composed-document clause (SPEC §3.1) makes the refused policy's rules travel as `unevaluated` entries,
/// whose `rule` field is the RAW line verbatim — so the caller needs the pair, not prose it would have to
/// re-parse a line out of.
pub(crate) fn policy_precheck(
    policy_text: &str,
    unknown_aliases: &std::collections::BTreeMap<String, BTreeSet<candor_classify::policy::ReasonClass>>,
) -> (Vec<(String, String)>, UsedAliases, Vec<candor_report::IgnoredLine>) {
    let p = candor_classify::policy::parse_policy_silent(policy_text, unknown_aliases);
    // ⟨0.24⟩ FATAL errors only. `ParsedPolicy::errors` now also carries the lines the parser DROPPED but
    // could survive (a malformed `forbid`, an unknown rule kind) — those are `parsepolicy`'s to report,
    // and refusing a build on them would be the opposite defect.
    let fatal = p
        .errors
        .iter()
        .filter(|e| e.fatal)
        .map(|e| (e.rule.clone(), e.message.clone()))
        .collect();
    // ⟨0.28⟩ …and the SURVIVABLE dropped lines, third: SPEC §6.2 puts them on the verdict document as
    // `ignored` (they are not fatal — the rest of the policy means what it says — but every one of
    // them is a gate that asked nothing, and only the verdict can tell its consumer so).
    let ignored = p
        .errors
        .iter()
        .filter(|e| !e.fatal)
        .map(|e| candor_report::IgnoredLine {
            line: e.line,
            text: e.text.clone(),
            reason: e.message.clone(),
        })
        .collect();
    (fatal, p.used_aliases, ignored)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn policy_violations(
    policy_text: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    hostsacc: &HashMap<String, BTreeSet<String>>,
    cmdsacc: &HashMap<String, BTreeSet<String>>,
    pathsacc: &HashMap<String, BTreeSet<String>>,
    tablesacc: &HashMap<String, BTreeSet<String>>,
    incompleteacc: &HashMap<String, BTreeSet<&'static str>>,
    // Transitive reason-CLASS tokens per fn (reflect/dispatch/…): the Unknown EFFECT propagates along the
    // call graph, so its REASON must too — else a `deny E Unknown[reflect]` at a caller inheriting Unknown
    // from a reflect-caused callee would see no class and NOT fire (under-gating). See the java reference.
    reasonclassacc: &HashMap<String, BTreeSet<String>>,
    // ⟨0.19⟩ `.candor/config` `unknown-alias` definitions, so `Unknown[<alias>]` resolves (SPEC §6.2).
    unknown_aliases: &std::collections::BTreeMap<String, BTreeSet<candor_classify::policy::ReasonClass>>,
    // ⟨0.20⟩ `.candor/config` `net-partner` hosts, so a `deny Net[unknown-host]` tolerates declared partners
    // and the verdict's `netClass` classifies them (NET-DESTINATION-CLASS-DESIGN.md).
    net_partners: &BTreeSet<String>,
    // ⟨0.24⟩ Returns the WITHHELD `(rule, function)` pairs beside the violations (SPEC §3.1). Not a
    // `Vec<GateViolation>` any more, deliberately: a caller that could ignore the second half would
    // reintroduce the silent-tolerate this rung closed, and the type is the only thing that stops it.
) -> candor_classify::gate::GateOutcome {
    // The SHARED §6.2 parser, with the ⟨0.19⟩ config aliases — one grammar across all four engines.
    let p = candor_classify::policy::parse_policy_with_aliases(policy_text, unknown_aliases);
    // ⟨0.20⟩ Materialize each Net-bearing fn's destination classes ONCE, from this machine's
    // `net-partner` config. The gate used to compute them lazily at the `deny Net[dest…]` site; the set
    // is identical (only a Net-bearing fn could reach that branch) and it is the same derivation scan.rs
    // writes into the report's `netClass`, which is what makes the ⟨0.24⟩ report route byte-equivalent.
    let net_classes = net_class_map(all, inferred, hostsacc, incompleteacc, net_partners);
    candor_classify::gate::gate(
        &p,
        &candor_classify::gate::GateInput {
            all,
            inferred,
            calls,
            hosts: hostsacc,
            cmds: cmdsacc,
            paths: pathsacc,
            tables: tablesacc,
            surface_incomplete: incompleteacc,
            reason_classes: reasonclassacc,
            net_classes: &net_classes,
        },
    )
}

/// ⟨0.20⟩ Each `Net`-bearing fn's destination classes, materialized ONCE from this machine's
/// `net-partner` config. Extracted because the gate and the provable-purity disclosure must read the
/// SAME set: the disclosure asks whether a rule PASSES a function, and since ⟨0.20⟩ that question can
/// turn on a `Net[dest…]` filter, so a disclosure computing its own answer would be a second gate.
pub(crate) fn net_class_map(
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    hostsacc: &HashMap<String, BTreeSet<String>>,
    incompleteacc: &HashMap<String, BTreeSet<&'static str>>,
    net_partners: &BTreeSet<String>,
) -> HashMap<String, Vec<String>> {
    all.iter()
        .filter(|q| inferred.get(*q).is_some_and(|s| s.contains("Net")))
        .map(|q| (q.clone(), net_classes_of(q, hostsacc, incompleteacc, net_partners)))
        .collect()
}

/// The provable-purity DISCLOSURE (eval/fixloop/DISPATCH-NOTE.md): functions that PASS a `pure`/`deny` layer
/// but are `Unknown` — their compliance is asserted, not verified (the Unknown could hide the forbidden
/// effect; the classic case is a fn/closure-injected port). Advisory — NEVER a violation, so the gate's
/// verdict/exit is untouched; the caller emits it as a note so an author learns their layer isn't PROVABLY
/// clean. Returns `(fn, deny-Unknown upgrade)` per hole. Mirrors `candor-query unverified`.
///
/// ⟨0.24⟩ THE RE-PARSE CARRIES THE ALIASES NOW, and that is this route's half of `ea0df4f`. The verdict
/// path resolves `deny Unknown[<alias>]` through the `.candor/config` beside the policy; this advisory
/// re-parse did not pass them, so the token resolved to nothing, the filter emptied, and the rule
/// WIDENED to a bare `deny Unknown` — under which every hole is a violation and the note has nothing to
/// say. The query verb was fixed at `ea0df4f` and this one was the same defect standing in the other
/// copy: `parse_policy_silent` is `parse_policy_quiet` with the vocabulary, and the QUIET part is still
/// required (the verdict path already warned about this same text — #21).
pub(crate) fn unverified_holes(
    policy_text: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    reasonclassacc: &HashMap<String, BTreeSet<String>>,
    net_classes: &HashMap<String, Vec<String>>,
    unknown_aliases: &std::collections::BTreeMap<String, BTreeSet<candor_classify::policy::ReasonClass>>,
) -> Vec<(String, String)> {
    use candor_classify::policy::{parse_policy_silent, rule_and_upgrade, unverified_hole_rule};
    let rules = parse_policy_silent(policy_text, unknown_aliases).rules;
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let no_classes: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for q in all {
        // Same predicate + upgrade reconstruction as `candor-query unverified` (candor_classify::policy):
        // the two disclosure paths share ONE definition of a hole, so they cannot drift. And since ⟨0.24⟩
        // that definition is the GATE's own firing decision, fed the same two accumulators the gate got.
        let effs: Vec<&str> = inferred.get(q).unwrap_or(&empty).iter().copied().collect();
        let nets = net_classes.get(q).unwrap_or(&no_classes);
        if let Some(r) = unverified_hole_rule(q, &effs, reasonclassacc.get(q), nets, &rules) {
            out.push((q.clone(), rule_and_upgrade(r).1));
        }
    }
    out
}

/// What the AS-EFF-005 baseline guard decided for one crate scan (see [`check_baseline`]).
pub(crate) enum BaselineOutcome {
    /// No baseline file exists — the ratchet is not adopted yet. A one-time stderr note was printed;
    /// the exit code is unchanged (the guard is simply not active — candor-java's absent-file posture).
    Inactive,
    /// Invalid gate input (empty value, unreadable/unparseable file, missing or MISMATCHED producing
    /// version) — the diagnostic was printed and the caller must exit 2 WITHOUT evaluating (the §2.1
    /// stale-baseline posture: never a silent skip, never a stale compare).
    Invalid,
    /// The baseline was valid and same-build; here is every AS-EFF-005 violation (possibly none).
    Checked(Vec<GateViolation>),
}

/// The absent-baseline notes already printed (one per resolved file, process-wide): a workspace whose
/// members share one direct-path baseline value must not repeat the identical note per member.
static NOTED_ABSENT: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();

/// The AS-EFF-005 baseline regression guard (candor-spec §7 item 5) — the stable scanner's arm of the
/// family-wide MUST, with candor-java's `checkBaseline` as the exact model. `value` is the
/// `CANDOR_BASELINE` env / config `baseline` value: a report PREFIX (the `--out` form —
/// `<value>.<crate>.scan.json` per workspace member) or a direct report file path.
///
///   - EMPTY value                      → `Invalid` (exit 2): a configured-but-empty gate input is the
///     §6.2 unreadable-policy class — never a silently skipped gate (matches the bare `policy` posture).
///   - file ABSENT                      → `Inactive` + one stderr note ("guard not active; record one").
///   - present but UNPARSEABLE, or with a MISSING/MISMATCHED producing version (the envelope
///     `candor.version` vs this build) → `Invalid` (exit 2) WITHOUT evaluating: a baseline is comparable
///     only to its OWN producing build (§2.1) — engine upgrades change reports, so evaluating produces a
///     bogus AS-EFF-005 wave and skipping is an unbounded fail-open window.
///   - valid + same build               → compare per-fn TRANSITIVE sets: any fn GAINING an effect vs its
///     baseline set is one violation.
///
/// EXISTENCE — what counts as a "new function" (exempt) vs an "existing function" (guarded):
///   - ⟨0.16⟩ When the baseline **callgraph sidecar** is present (sibling of the resolved report —
///     the report path with `.json` swapped for `.callgraph.json`; SPEC §2.2 records EVERY analyzed fn,
///     including PURE leaves reports omit), existence is keyed on it: a fn that is a callgraph node (even
///     with an EMPTY/∅ baseline effect set — a baseline-pure leaf) that now performs ANY effect is a GAIN
///     violation. This catches the pure→effectful transition — the sharpest supply-chain shape, which the
///     report-only rule missed because a formerly-pure fn is absent from the report and read as exempt
///     "new". A fn genuinely absent from the callgraph stays exempt (real new code). This is the `gains`
///     `origin` existence rule (§3.1 ⟨0.12⟩) applied to the scan-time ratchet — see candor-query diff.rs
///     `gain_origin`.
///   - sidecar ABSENT → degrade to today's report-only existence (pre-⟨0.16⟩): a fn absent from the
///     baseline report is exempt (a formerly-pure fn reads as new — the shared pre-0.16 family semantics).
///     Still catches WIDENING on already-effectful fns. A one-time stderr note that the guard is weaker.
///   - sidecar PRESENT-but-corrupt → `Invalid` (exit 2), mirroring the corrupt-baseline handling above: a
///     broken sidecar must not silently narrow the guard by making its pure leaves read as exempt "new".
///
/// Same-named baseline entries (rlib+bin `main`) are UNIONed, not last-write-wins — the baseline is the
/// over-approximation of what a name was already permitted to reach (mirrors the deep engine).
///
/// `unknown_ratchet` (config `unknown-ratchet` / CANDOR_UNKNOWN_RATCHET, default OFF) flips an Unknown-ONLY
/// gain from advisory to an AS-EFF-005 FAILURE — a fn already Unknown in the baseline is grandfathered (no
/// gain), only a NEWLY-introduced Unknown fails. Default OFF leaves the guard's output byte-identical.
pub(crate) fn check_baseline(
    value: &str,
    dir: &str,
    crate_name: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    unknown_ratchet: bool,
    // The path came from a checked-in `.candor/config` `baseline` line rather than `CANDOR_BASELINE`.
    // A MISSING file then means something different — see the absent branch below.
    declared_in_config: bool,
) -> BaselineOutcome {
    if value.trim().is_empty() {
        eprintln!(
            "candor-scan: baseline is configured with an EMPTY value — failing (exit 2); the guard \
             must not be silently skipped (set a report path/prefix, or remove the key)"
        );
        return BaselineOutcome::Invalid;
    }
    // A direct report file wins when it exists; otherwise the canonical `--out` prefix form.
    let file = if Path::new(value).is_file() {
        value.to_string()
    } else {
        format!("{value}.{crate_name}.scan.json")
    };
    if !Path::new(&file).is_file() {
        // A CHECKED-IN DECLARATION IS NOT THE SAME ABSENCE. `.candor/config` naming a baseline says this
        // repo HAS one, so a missing file was deleted or never committed and the guard passing green
        // over it is the gateless-green class — measured by an adopter review as the second-likeliest
        // first-commit mistake. `CANDOR_BASELINE` is set unconditionally by the adopt workflow, so an
        // absent path THERE means the ratchet is not adopted yet, which stays a note.
        if declared_in_config {
            eprintln!(
                "candor-scan: .candor/config declares `baseline {value}` but {file} is not there — \
                 failing (exit 2). A checked-in declaration says this repo HAS a baseline, so an absent \
                 one was deleted or never committed. Commit it, or record one: candor-scan {dir} --out {value}"
            );
            return BaselineOutcome::Invalid;
        }
        let noted = NOTED_ABSENT.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
        if noted.lock().unwrap().insert(file.clone()) {
            eprintln!(
                "candor-scan: baseline {file} does not exist — the regression guard is not active \
                 (record one: candor-scan {dir} --out {value})"
            );
        }
        return BaselineOutcome::Inactive;
    }
    let regen = format!("regenerate it with this build: candor-scan {dir} --out {value}");
    let Ok(text) = std::fs::read_to_string(&file) else {
        eprintln!("candor-scan: baseline {file} exists but could not be read — failing (exit 2), guard NOT evaluated; {regen}");
        return BaselineOutcome::Invalid;
    };
    // A partially-corrupt baseline (any entry dropped) is as invalid as a whole-file parse failure:
    // a vanished entry's fn would read as new (exempt), silently narrowing the guard.
    let entries = match candor_report::report_entries_counted(&text) {
        Some((entries, 0)) => entries,
        _ => {
            eprintln!(
                "candor-scan: baseline {file} exists but could not be parsed (corrupt/truncated?) — \
                 failing (exit 2), guard NOT evaluated; the guard must not silently pass on an \
                 unreadable baseline (the unreadable-policy class, §6.2); {regen}"
            );
            return BaselineOutcome::Invalid;
        }
    };
    let this_build = format!("scan-{}", env!("CARGO_PKG_VERSION"));
    match candor_report::report_version(&text) {
        None => {
            eprintln!(
                "candor-scan: baseline {file} has no provenance header (a legacy/bare-array report) — \
                 a baseline is comparable only to its producing build (§2.1). Failing (exit 2); {regen}"
            );
            return BaselineOutcome::Invalid;
        }
        Some(v) if v != this_build => {
            eprintln!(
                "candor-scan: baseline {file} was produced by engine build {v} but this is build \
                 {this_build} — coverage changes reports, so an engine swap is baseline-invalidating \
                 and the gate cannot evaluate (exit 2, the unreadable-policy class; never a silent \
                 skip, never a bogus AS-EFF-005 wave). Regenerate deliberately with this build: \
                 candor-scan {dir} --out {value}"
            );
            return BaselineOutcome::Invalid;
        }
        Some(_) => {}
    }
    let mut base: HashMap<String, BTreeSet<String>> = HashMap::new();
    for e in entries {
        base.entry(e.func).or_default().extend(e.inferred);
    }
    // ⟨0.16⟩ Existence keys on the baseline callgraph sidecar when present (SPEC §7 item 5): the
    // sidecar lists PURE leaves the report omits, so a formerly-pure fn is a graph node and no longer
    // reads as exempt "new". Derive the sidecar from the resolved report path (`<stem>.json` →
    // `<stem>.callgraph.json`), mirroring the write in scan.rs and load_callgraph in candor-query.
    let cg_file = file.strip_suffix(".json").map(|s| format!("{s}.callgraph.json")).unwrap_or_else(|| format!("{file}.callgraph.json"));
    // The set of names present in the baseline callgraph (callers AND callees) — the same node set
    // candor-query's `gain_origin` treats as "existing". `None` == no sidecar (degrade to report-only).
    let cg_nodes: Option<BTreeSet<String>> = if Path::new(&cg_file).is_file() {
        let Ok(cg_text) = std::fs::read_to_string(&cg_file) else {
            eprintln!(
                "candor-scan: baseline callgraph {cg_file} exists but could not be read — failing (exit 2), \
                 guard NOT evaluated; a broken sidecar must not silently narrow the guard (§7 item 5); {regen}"
            );
            return BaselineOutcome::Invalid;
        };
        match serde_json::from_str::<BTreeMap<String, Vec<String>>>(&cg_text) {
            Ok(map) => {
                let mut nodes: BTreeSet<String> = BTreeSet::new();
                for (caller, callees) in map {
                    nodes.insert(caller);
                    nodes.extend(callees);
                }
                Some(nodes)
            }
            Err(_) => {
                eprintln!(
                    "candor-scan: baseline callgraph {cg_file} exists but could not be parsed \
                     (corrupt/truncated?) — failing (exit 2), guard NOT evaluated; a broken sidecar must \
                     not silently narrow the guard (§7 item 5, the unreadable-policy class); {regen}"
                );
                return BaselineOutcome::Invalid;
            }
        }
    } else {
        // Pre-⟨0.16⟩ degradation: no sidecar, so existence falls back to report-only. A formerly-pure
        // fn (absent from the report) reads as new and escapes — disclose the weakened guard once.
        let noted = NOTED_ABSENT.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
        if noted.lock().unwrap().insert(cg_file.clone()) {
            eprintln!(
                "candor-scan: baseline callgraph sidecar {cg_file} is absent — the guard degrades to \
                 report-only existence (a formerly-PURE fn that turns effectful reads as new code and \
                 ESCAPES; widening on already-effectful fns is still caught). Regenerate the baseline \
                 with this build to record the sidecar: candor-scan {dir} --out {value}"
            );
        }
        None
    };
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let empty_base: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    let mut unknown_only: Vec<String> = Vec::new(); // ⟨0.16⟩ advisory: gained ONLY Unknown
    for q in all {
        // Existence: in the baseline report, OR (⟨0.16⟩) a baseline-callgraph node — a
        // baseline-pure leaf has no report entry but IS a graph node, so its baseline effect set is ∅
        // and ANY current effect is a gain. A fn in neither is genuinely new and stays exempt.
        let in_cg = cg_nodes.as_ref().is_some_and(|n| n.contains(q));
        let prior = match base.get(q) {
            Some(p) => p,
            None if in_cg => &empty_base, // baseline-pure callgraph node: ∅ baseline effects
            None => continue,             // new function — not a regression
        };
        let gained: Vec<&str> =
            inferred.get(q).unwrap_or(&empty).iter().copied().filter(|e| !prior.contains(*e)).collect();
        if gained.is_empty() {
            continue;
        }
        // ⟨0.16⟩ the ratchet fires only on gaining a REAL boundary effect. An Unknown-ONLY gain
        // is the §4 trust marker, not an effect (`pure` policies exclude it), and on version bumps it is
        // dominated by resolution noise (SOUNDNESS-LOG 2026-07-16) — DISCLOSE it, don't fail on it.
        let real: Vec<&str> = gained.iter().copied().filter(|e| *e != "Unknown").collect();
        if real.is_empty() {
            // ⟨unknown-ratchet⟩ OPT-IN (config `unknown-ratchet` / CANDOR_UNKNOWN_RATCHET, default OFF —
            // candor-java Policy.checkBaseline is the model). This is what makes `deny Unknown` adoptable on
            // legacy DI/reflection-heavy code: the CURRENT Unknown surface is GRANDFATHERED (a fn already
            // Unknown in the baseline shows no gain ⇒ never flagged), and only a NEWLY-introduced Unknown —
            // a blind spot the baseline did not have — fails. So a team freezes today's report as the
            // baseline and the strict gate ratchets the Unknown surface DOWN instead of failing everywhere on
            // day one; grandfather one by regenerating the baseline. Default OFF preserves the ⟨0.16⟩ advisory
            // posture (Unknown-gains = resolution noise), leaving the guard's output byte-identical.
            if unknown_ratchet {
                out.push(GateViolation {
                    rule: "AS-EFF-005".into(),
                    func: q.clone(),
                    effects: vec!["Unknown".to_string()],
                    detail: format!(
                        "`{q}` gained an unresolved call (Unknown) not in the baseline — a NEW blind spot \
                         (unknown-ratchet); resolve it, or regenerate the baseline to grandfather it"
                    ),
                    ..Default::default()
                });
            } else {
                unknown_only.push(q.clone());
            }
            continue;
        }
        out.push(GateViolation {
            rule: "AS-EFF-005".into(),
            func: q.clone(),
            effects: real.iter().map(|s| s.to_string()).collect(),
            detail: format!(
                "`{q}` gained effect {{ {} }} not present in the baseline; an existing function \
                 started performing a new effect",
                real.join(", ")
            ),
            ..Default::default()
        });
    }
    if !unknown_only.is_empty() {
        unknown_only.sort();
        let shown: Vec<&str> = unknown_only.iter().take(3).map(String::as_str).collect();
        let more = if unknown_only.len() > 3 { format!(" (+{} more)", unknown_only.len() - 3) } else { String::new() };
        eprintln!(
            "candor-scan: note — {} function(s) gained an unresolved call (Unknown) vs the baseline but \
             no real effect — advisory, NOT a regression (Unknown is the §4 trust marker, dominated by \
             resolution noise on version bumps): {}{more}",
            unknown_only.len(),
            shown.join(", ")
        );
    }
    out.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    BaselineOutcome::Checked(out)
}

/// `--gate-json <file>` target, set once in `scan_main` (a no-op when unset — the direct-`scan_one` test
/// paths never RECORD). Mirrors the `CFG_FEATURES` OnceLock idiom; a plain path so it threads no ScanOpts.
/// Members record via `record_gate_violations`; `scan_main` writes the single final verdict.
pub(crate) static GATE_JSON_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// SPEC §3.3.1 ⟨0.28⟩ — was `--json` (the REPORT stream) requested? Set once in `scan_main` before the
/// arg loop, so the arming rule can decide *what to write on exit-2* the same way `GATE_JSON_PATH` does
/// for the verdict sink. Rule (4) of the ⟨0.28⟩ report-sink clause: on any exit-2 the fail-closed
/// report is written to stdout, exactly once, as the stream's only content. An empty stream on exit-2
/// throws a JSON consumer back to scraping stderr — the distinction that made the incomplete-analysis
/// defect a defect. Measured on four engines, unknown-flag exit-2: stdout was 0 bytes on every one.
pub(crate) static WANT_JSON_STREAM: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The `unknown-ratchet` / `CANDOR_UNKNOWN_RATCHET` opt-in (config `flag`), resolved once in `scan_main`
/// and read at each `check_baseline` call — a process-wide mode like `GATE_JSON_PATH`, so it threads no
/// ScanOpts through scan_target/run_with_deps. Default OFF (unset OnceLock reads `false`). When ON, an
/// Unknown-ONLY gain vs the baseline FAILS (AS-EFF-005) instead of staying advisory — see `check_baseline`.
pub(crate) static UNKNOWN_RATCHET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Did the baseline path come from a checked-in `.candor/config` line rather than `CANDOR_BASELINE`?
/// Resolved ONCE in `scan_main`, exactly as UNKNOWN_RATCHET above — a config-derived flag every
/// `scan_one` needs, and threading it through `ScanOpts` instead meant touching ~60 test construction
/// sites for a value none of them care about.
pub(crate) static BASELINE_FROM_CONFIG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

pub(crate) fn baseline_from_config() -> bool {
    matches!(BASELINE_FROM_CONFIG.get(), Some(true))
}

/// Whether the `unknown-ratchet` opt-in is active for this process (default OFF).
pub(crate) fn unknown_ratchet() -> bool {
    matches!(UNKNOWN_RATCHET.get(), Some(true))
}

// ⟨0.30⟩ THREAD-LOCAL, not a process global. Workspace members are scanned SEQUENTIALLY on one thread
// (`for d in &dirs`), so a thread-local accumulates across them exactly as the global did — while giving
// each `cargo test` thread its own, which a process static cannot. That distinction is what let the sink
// guard below be removed: recording unconditionally is correct, and only turned into a race because the
// state was shared by every test in the binary. The peek is a SAME-THREAD recursive `scan_one` call
// and records no violations (it scans with no policy), so nothing is lost by the split — an earlier
// version of this comment said it ran on its own thread, which would have mattered the moment
// someone made that true and the peek began recording.
thread_local! {
    /// Violations ACCUMULATED across `scan_one` calls. A `[workspace]` root runs the gate once per
    /// member; writing the verdict per member let the LAST member overwrite the first's violations —
    /// `gate.json` said `ok: true` while the process exited 1 (a clean final member masked an earlier
    /// violator), violating the §3.3 "verdict MUST agree with the exit code" rule. So members only
    /// RECORD here; `scan_main` writes ONCE.
    pub(crate) static GATE_VIOLATIONS: std::cell::RefCell<Vec<GateViolation>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// ⟨0.31⟩ IS THIS FRAME INSIDE THE ⟨0.30⟩ PEEK? The peek re-enters `scan_one` over the files the
    /// scan EXCLUDED, and **nothing it sees may reach the verdict**. It writes no report, so anything it
    /// records is carried by the scan route and by no other: `gate --report` reads the report and cannot
    /// reproduce it. That is a §3.1 route-equality break AND an over-claim, since the gate judged none
    /// of those files.
    ///
    /// A CHOKE POINT RATHER THAN A GUARD PER CALL SITE, because per-site guards were tried and the class
    /// came back. `analyzed` was guarded after being measured at 276 against the report's 129; the
    /// ⟨0.31⟩ `netPartners` key was written months later by someone (me) who had no reason to think
    /// about a peek, and reproduced the defect exactly. The recording sites are where the author's
    /// attention ISN'T. Suppressing centrally makes the default safe instead of making it correct only
    /// when remembered.
    ///
    /// This does not lose the peek's findings, and the existing `outOfScope` shape is why: the peek
    /// RETURNS a report body, and the OUTER frame reads it and records what it decides to. That is the
    /// architecture this enforces — the peek is a source of data, never a writer of verdict state.
    ///
    /// Thread-local, and correct because the peek is a SAME-THREAD recursive call (see the note at
    /// `GATE_VIOLATIONS`). If the peek is ever moved to its own thread it becomes trivially true instead.
    static IN_PEEK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// ⟨0.31⟩ PROOF THAT ONE SCAN RUN IS ONE THREAD.
///
/// `GATE_VIOLATIONS` is a **thread-local** while nine sibling accumulators are process-globals, and that
/// asymmetry is not an oversight — the thread-local exists because `cargo test` runs tests on parallel
/// threads and a process-global violation list let them contaminate each other. But it makes the
/// `[workspace]` member loop load-bearing in a way nothing about the loop says: members must scan
/// **sequentially on one thread**, or each worker accumulates into its own list and cross-member
/// violations are silently lost. The symptom is a WRONG EXIT CODE — one member's certain violation
/// vanishing behind another's "could not evaluate" — with no panic, no diff, and no failing fixture.
///
/// Three sequential loops in this family were parallelised for speed in one night. This one must not be
/// without moving the state first, and a comment saying so is not a mechanism.
///
/// So `scan_one` takes `&RunToken`, and the token is neither `Send` nor `Sync`: a `par_iter()` over the
/// members captures it and **fails to compile**. The author's next move is to construct one inside the
/// closure — which lands them on this comment, because [`begin_run`] is the only way to make one and
/// there is exactly one call to it outside tests (pinned by
/// `exactly_one_run_token_is_minted_outside_tests`).
///
/// It is a forcing function, not a proof. What it converts is a silent runtime under-report into a
/// compile error at the exact line that would cause it.
pub(crate) struct RunToken {
    /// A raw pointer is neither Send nor Sync, and carrying it by `PhantomData` costs nothing at
    /// runtime — `RunToken` is zero-sized. Deliberately NOT `Clone`: a clone per worker would be the
    /// same defect wearing the token's clothes.
    _one_thread: std::marker::PhantomData<*const ()>,
}

/// Mint the token for THIS run. Exactly one call outside tests, in `scan_main`, above the member loop.
///
/// If you are here because a parallel iterator would not compile: the fix is not a second token. It is
/// to move the per-run accumulators out of `thread_local!` and into state threaded through `scan_one`,
/// so that workers merge instead of diverging. Read the note on [`RunToken`] first.
pub(crate) fn begin_run() -> RunToken {
    RunToken { _one_thread: std::marker::PhantomData }
}

/// Run `f` with every gate accumulator suppressed. Restores the PREVIOUS value rather than clearing, so
/// this composes if a peek is ever nested inside another.
pub(crate) fn while_peeking<T>(f: impl FnOnce() -> T) -> T {
    let prev = IN_PEEK.with(|p| p.replace(true));
    let out = f();
    IN_PEEK.with(|p| p.set(prev));
    out
}

/// Would a `record_gate_*` call right here land in a verdict this run publishes?
pub(crate) fn recording_suppressed() -> bool {
    IN_PEEK.with(|p| p.get())
}

/// Record one scan's gate violations toward the final `--gate-json` verdict. A no-op unless the flag was
/// given (the direct-`scan_one` test/selftest paths never record).
/// Does this run already hold a CERTAIN violation? (SPEC §3.1 ⟨0.24⟩ precedence.)
///
/// The order is **violation (1) > refusal (2) > incomplete (2)**, and the first rung is forced rather
/// than chosen: if a rule FIRED on evidence the report carries, `Reject` is upward-closed, so however
/// the unanswerable rule would have resolved cannot un-reject it. Exit 1 is not merely fail-closed
/// there, it is CERTAIN — and strictly more informative, because it names the violation.
/// …and so does the READ, for the same reason: a precedence decision taken against another thread's
/// (empty) accumulator is how a certain violation silently becomes "could not evaluate".
pub(crate) fn holds_violation(_run: &RunToken) -> bool {
    GATE_VIOLATIONS.with(|v| !v.borrow().is_empty())
}

/// ⟨0.31⟩ TAKES THE RUN TOKEN, and that is the whole point of the token rather than a formality.
/// `GATE_VIOLATIONS` is the thread-local this protects, so the proof that a run is one thread belongs
/// exactly HERE — at the write — not at some outer function boundary that merely happens to forward it.
/// Requiring it means no caller can reach the accumulator from a thread that did not carry the token in.
pub(crate) fn record_gate_violations(violations: &[GateViolation], _run: &RunToken) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    // ⟨0.30⟩ UNCONDITIONAL. This returned early unless `--gate-json` was set, so `holds_violation` was
    // blind without it and the ⟨0.30⟩ precedence check answered differently with and without a machine
    // sink — MEASURED on `clap` under `pure`: exit 1 with the flag, 2 without, same tree. An exit code
    // must never depend on a sink being requested. Safe to record always now that the accumulator is
    // thread-local: the first attempt at this kept the process global and turned a latent race into an
    // active one, because `cargo test` runs tests in parallel threads.
    GATE_VIOLATIONS.with(|v| v.borrow_mut().extend(violations.iter().cloned()));
}

/// ⟨0.30⟩ Clear the per-RUN gate accumulators. These are process statics with no reset, which a CLI never
/// noticed (one run per process) and which now matters twice: recording is unconditional above, and a
/// stale violation from an earlier run would suppress a later run's ⟨0.30⟩ exit. Called by `scan_main`;
/// a test driving `scan_one` directly calls it for the same isolation.
pub(crate) fn reset_gate_run_state() {
    GATE_VIOLATIONS.with(|v| v.borrow_mut().clear());
    if let Some(m) = GATE_OUT_OF_SCOPE.get() {
        m.lock().unwrap().clear();
    }
}

/// ⟨0.15 staged⟩ Uncovered packages ACCUMULATED across `scan_one` calls (workspace members union, like
/// GATE_VIOLATIONS) toward the `--gate-json` verdict's ADVISORY `coverage` note. Names only — the
/// advisory shape is `{ uncovered: N, packages: […] }`; the per-package call counts live in each
/// member report's own `coverage` envelope field.
pub(crate) static GATE_COVERAGE: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<String>>> =
    std::sync::OnceLock::new();

/// ⟨0.15 staged⟩ Record one scan's κ-coverage ledger toward the `--gate-json` verdict's advisory
/// `coverage` note (spec §3.3 verb conditionality — a gate verdict over partially-covered code
/// re-discloses the gap, VERDICT-PRESERVING). A no-op unless `--gate-json` was given, mirroring
/// `record_gate_violations`.
pub(crate) fn record_gate_coverage(ledger: &[(String, usize)]) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    if ledger.is_empty() || !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_COVERAGE.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));
    acc.lock().unwrap().extend(ledger.iter().map(|(cr, _)| cr.clone()));
}

/// ⟨0.24⟩ THE AMBIENT-VOCABULARY DISCLOSURE (SPEC §3.1): the `.candor/config` whose `unknown-alias`
/// definitions a policy rule actually resolved through, and **what each of those aliases expanded to**
/// (`7f5b5ba` — the name alone cannot tell a reader which gate ran). Accumulated like the κ ledger so a
/// workspace scan names it ONCE on the shared verdict rather than per member — every member resolves the
/// same policy against the same config, so the map is idempotent by construction.
/// ⟨0.24⟩ `unknown-alias` NAME → the reason-class TOKENS it expands to (SPEC §3.1 `7f5b5ba`). Named
/// because both this accumulator and [`policy_precheck`]'s return carry it, and the two have to be the
/// same type for the disclosure to reach the verdict unchanged.
pub(crate) type UsedAliases = std::collections::BTreeMap<String, BTreeSet<String>>;

pub(crate) static GATE_VOCABULARY: std::sync::OnceLock<std::sync::Mutex<(String, UsedAliases)>> =
    std::sync::OnceLock::new();

pub(crate) fn record_gate_vocabulary(config: &std::path::Path, aliases: &UsedAliases) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    if aliases.is_empty() || !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_VOCABULARY
        .get_or_init(|| std::sync::Mutex::new((String::new(), std::collections::BTreeMap::new())));
    let mut g = acc.lock().unwrap();
    g.0 = config.display().to_string();
    for (name, classes) in aliases {
        g.1.insert(name.clone(), classes.clone());
    }
}

/// ⟨0.21⟩ COMPLETENESS MANIFEST: the analyzed-fn count (summed across workspace members) + the units that
/// could NOT be analyzed, ACCUMULATED toward the `--gate-json` verdict's `analyzed:{count}` (Gap 1, always)
/// and — when non-empty — the fail-closed `incomplete:true`/`unanalyzed` disclosure (Gap 2). A no-op unless
/// `--gate-json` was given, mirroring `record_gate_coverage`.
pub(crate) static GATE_ANALYZED: std::sync::OnceLock<std::sync::Mutex<usize>> = std::sync::OnceLock::new();
pub(crate) static GATE_UNANALYZED: std::sync::OnceLock<std::sync::Mutex<Vec<candor_report::UnanalyzedUnit>>> =
    std::sync::OnceLock::new();

/// ⟨0.30⟩ The peek's findings, for the verdict document. Kept SEPARATE from the exit-code decision on
/// purpose: this accumulator is gated on `--gate-json` being set (as its siblings are), and an exit code
/// must not depend on whether a machine-readable sink was requested. scan.rs decides the exit from the
/// local value; this only feeds the document.
pub(crate) static GATE_OUT_OF_SCOPE: std::sync::OnceLock<
    std::sync::Mutex<Vec<candor_report::OutOfScopeFinding>>,
> = std::sync::OnceLock::new();



/// ⟨0.31⟩ The ambient `net-partner` provenance for the verdict document — same storage shape as
/// `GATE_OUT_OF_SCOPE` beside it, and fed only when a `--gate-json` sink was asked for.
pub(crate) static GATE_NET_PARTNERS: std::sync::OnceLock<
    std::sync::Mutex<Vec<candor_report::NetPartners>>,
> = std::sync::OnceLock::new();

/// ⟨0.31⟩ Record what an ambient `net-partner` moved, for the verdict. A LIST because a workspace scans
/// several members and each anchors its own config; a single crate contributes one record.
pub(crate) fn record_gate_net_partners(rec: Option<&candor_report::NetPartners>) {
    if recording_suppressed() { return; }
    if !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    if let Some(r) = rec {
        let m = GATE_NET_PARTNERS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
        let mut g = m.lock().unwrap();
        if !g.iter().any(|e| e == r) {
            g.push(r.clone());
        }
    }
}

pub(crate) fn record_gate_out_of_scope(findings: &[candor_report::OutOfScopeFinding]) {
    if recording_suppressed() { return; }
    if !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    if !findings.is_empty() {
        GATE_OUT_OF_SCOPE
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .extend(findings.iter().cloned());
    }
}

pub(crate) fn record_gate_analyzed(count: usize, unanalyzed: &[candor_report::UnanalyzedUnit]) {
    if recording_suppressed() { return; }
    if !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    *GATE_ANALYZED.get_or_init(|| std::sync::Mutex::new(0)).lock().unwrap() += count;
    if !unanalyzed.is_empty() {
        GATE_UNANALYZED
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .extend(unanalyzed.iter().cloned());
    }
}

/// ⟨0.24⟩ WHY THIS RUN COULD NOT PRODUCE A VERDICT — set at each exit-2 site that is NOT an incomplete
/// analysis (an unreadable policy, a policy that cannot be honoured as written, an invalid baseline).
/// Read by [`write_gate_json`], which turns it into the fail-closed refusal document SPEC §3.1 requires.
/// `OnceLock`, first-writer-wins: the first refusal is the one that stopped the run.
pub(crate) static GATE_REFUSAL: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub(crate) fn record_gate_refusal(why: impl Into<String>) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    let _ = GATE_REFUSAL.set(why.into());
}

/// ⟨0.27⟩ Exit 2 for a broken gate config, leaving the fail-closed refusal document at the sink —
/// INCLUDING the stream sink (SPEC §3.1's stream-sink clause). The file sink was already covered by
/// arming, so these early exits looked done — but `--gate-json -` is not armed (a stream has no stale
/// previous document, and a placeholder would put two documents in the pipe), so an unknown flag or a
/// valueless gate-adjacent flag exited 2 leaving stdout EMPTY: the consumer of the stream was thrown
/// back to scraping stderr, the same operator mistake answered or not according to which early exit
/// fired. Measured four-way: an unhonourable policy wrote the refusal to stdout in every engine while an
/// unknown flag wrote it in one of four. Routing every pre-verdict exit through the one writer closes
/// the cause split; on a FILE sink this also replaces the armed placeholder with the specific reason,
/// which is strictly more informative and still fail-closed.
pub(crate) fn exit2_refused(why: impl Into<String>) -> ! {
    let why = why.into();
    record_gate_refusal(why.clone());
    write_gate_json(2);
    // ⟨0.28⟩ REPORT STREAM: the same rule the verdict stream gets one hop upstream. If `--json` (report
    // to stdout) was requested and stdout is not already claimed by `--gate-json -` (the two-stream case
    // is refused earlier in `scan_main`), write the ⟨0.21⟩ Row-1 fail-closed report as stdout's only
    // content. Without this, an unknown-flag exit-2 left stdout EMPTY on every engine — the report-sink
    // analog of the defect ⟨0.27⟩ closed for the verdict sink.
    write_json_stream_failclosed("refused", &why);
    std::process::exit(2);
}

/// SPEC §3.3.1 ⟨0.28⟩ (4) — the fail-closed REPORT is written to stdout as its only content on any
/// exit-2, if `--json` (stream) was requested. Shape is the ⟨0.21⟩ Row-1 manifest-carrying empty:
/// `functions: []` + `analyzed.count: 0` + `unanalyzed` naming the cause. A ⟨0.24⟩ consumer already
/// reads this as *nothing was judged, no purity licence*, so no new reader logic is needed. Called
/// from every pre-verdict exit-2 site (via `exit2_refused` and the pre-pass sink refusals).
///
/// A no-op if `--json` was not requested, if `--gate-json -` also claims stdout (the two-stream case
/// is refused with a verdict document earlier), or if the report has already been printed to stdout
/// (a completed scan on `--json`; guarded by `REPORT_STREAM_WRITTEN`).
pub(crate) static REPORT_STREAM_WRITTEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// SPEC §3.3.1 ⟨0.28⟩ (1) — **ARM THE `--out <prefix>` REPORT SET.**
///
/// The verdict sink arms by writing a placeholder to a path the run is about to own. A report PREFIX
/// cannot do that: at parse time the run does not yet know which packages it is going to write, and a
/// workspace fans out to one report per member. The set it DOES know is the one the PREVIOUS run left —
/// `<prefix>.*.json` on disk — and that is exactly the set at risk of being read as current after this
/// run fails. So arming here means REWRITING those to the ⟨0.21⟩ Row-1 manifest-carrying empty; each
/// member that scans successfully overwrites its own with a real report a moment later.
///
/// Measured 2026-08-10 on a three-member workspace: `--out out --zzz-not-a-flag` exited 2 with all six
/// files (3 reports + 3 sidecars) byte-identical to the previous good run.
///
/// **This also neutralises the ORPHAN, which is a separate defect found by the same measurement and is
/// the reason to prefer this shape over a marker file.** Delete a member from the workspace and rerun:
/// its report survives, byte-shaped exactly like a live one, with nothing saying its source is gone —
/// and it still sets gate outcomes (measured: `deny Exec` exited 1 on a function whose crate had been
/// removed). An orphan is definitionally a file this run does not overwrite, so rewriting the previous
/// set first makes every orphan fall back to "no claim" for free.
///
/// Deleting instead is rejected for the reason §3.3.1 already gives: a consumer that treats a missing
/// file as "nothing to report" fails open by a different route. The files stay; they stop asserting.
///
/// SIDECARS ARE NOT TOUCHED, deliberately — whether `.callgraph`/`.hierarchy` must arm alongside their
/// report is an open question against §2.2 ⟨0.26⟩'s own manifest rules, and guessing it here would put a
/// second answer in the tree.
///
/// **AND THE ORPHAN IS RESTORED, NOT KEPT — see [`disarm_unwritten_out_reports`].** The first version of
/// this armer left every un-overwritten file holding the placeholder, which looked like a free fix for
/// the orphan defect and was actually a new wrong answer: a placeholder's non-empty `unanalyzed` is the
/// ⟨0.21⟩ incomplete-analysis trigger, so a COMPLETE scan of the remaining members began refusing with
/// exit 2 and went on refusing until someone deleted the leftover by hand. Measured: `deny Exec` over
/// the prefix went 1 → 2 after a member was removed. The run did not fail to analyze the deleted crate;
/// the crate is not there. Claiming incompleteness the run did not experience is the mirror of the
/// staleness this rung exists to close.
pub(crate) fn arm_out_prefix(prefix: &str, inputs: &[(String, String)]) {
    if prefix.is_empty() {
        return;
    }
    let p = std::path::Path::new(prefix);
    let (dir, stem) = match (p.parent(), p.file_name()) {
        (Some(d), Some(f)) => (if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d }, f.to_string_lossy().into_owned()),
        _ => return,
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let doc = OUT_ARM_DOC.get_or_init(|| format!(
        "{{\n  \"candor\": {{ \"version\": \"scan-{ver}\", \"toolchain\": \"stable\", \"spec\": \"{spec}\" }},\n  \"functions\": [],\n  \"analyzed\": {{ \"count\": 0 }},\n  \"unanalyzed\": [ {{ \"path\": \"<run>\", \"reason\": \"armed: this report was written when the run STARTED and was never replaced, so the run failed before it could describe this package — or the package is no longer part of the scan and this file is a leftover. Either way it is NOT a claim about any code.\" }} ]\n}}\n",
        ver = env!("CARGO_PKG_VERSION"),
        spec = candor_report::SPEC_VERSION,
    ));
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&format!("{stem}.")) || !name.ends_with(".json") {
            continue;
        }
        let full = e.path();
        // ONLY FILES POSITIVELY IDENTIFIED AS §2 REPORTS — never a name denylist.
        //
        // The first version excluded `.callgraph`/`.hierarchy`/`.locs` by suffix and armed everything
        // else. SPEC §2.2 ⟨0.24⟩ (the "reserved set, family-wide" paragraph) lists SEVEN reserved
        // trailing segments — `callgraph`, `hierarchy`, `calibrated`, `layerreach`, `locs`, `gate`, and
        // the `encountered-*` family — and records that the engines were already drifting on it, one
        // carving out six and another two. I carved out three. Measured: this armer overwrote
        // `<prefix>.calibrated.json`, `.layerreach.json`, `.encountered-hosts.json` and — worst —
        // `<prefix>.gate.json`, a GATE VERDICT, each replaced by a report-shaped placeholder.
        //
        // The denylist-over-allowlist rule this project follows is about CLASSIFYING, where
        // over-approximating is the safe direction. For a WRITER it inverts: over-approximating
        // destroys a file. §2.2 says an incomplete denylist there is "loud" because an unregistered
        // suffix merely falls back into a candidate set — here it is silent and destructive. So the
        // safe direction is to write only what this engine positively recognises as its own report,
        // which also cannot drift as the reserved family grows.
        // THE ⟨0.27⟩ (2) INPUT EXEMPTION APPLIES TO THIS WRITER TOO, AND IT IS ASKED FIRST. Arming
        // happens before the run knows its answer, so a prefix whose expansion collides with something
        // this run READS would destroy it — the same hazard that made `--policy P --gate-json P` a
        // machine-readable all-clear. A policy or a chained dep report can perfectly well be named
        // `<prefix>.something.json`.
        //
        // ORDER MATTERS, and candor-swift's arm of this rung got it right where I had it backwards.
        // Identification-first silently skips a `--policy <prefix>.policy.json` that is not JSON — the
        // operator never learns their policy sat in the arming path, and losing a disclosure is the
        // thing this project does not do. The exemption also has to OUTRANK identification for the case
        // where the colliding input IS a valid report (a chained `CANDOR_DEPS` dep report under the same
        // prefix): "do not touch what this run reads" is the stronger claim, whatever the file turns out
        // to be.
        let fs = full.to_string_lossy().into_owned();
        if inputs.iter().any(|(path, _)| crate::scan::same_artifact_pub(&fs, path)) {
            eprintln!(
                "candor-scan: --out {prefix} would arm over {fs}, which this run READS — leaving it \
                 untouched. Give the report set its own prefix."
            );
            continue;
        }
        let is_report = std::fs::read_to_string(&full)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|v| v.get("candor").is_some() && v.get("functions").is_some());
        if !is_report {
            continue;
        }
        // Remember the bytes BEFORE overwriting, so a run that completes can hand back anything it
        // turned out not to own (see `disarm_unwritten_out_reports`).
        if let Ok(prev) = std::fs::read(&full) {
            OUT_ARMED
                .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                .lock()
                .unwrap()
                .push((full.clone(), prev));
        }
        // THE SIDECARS FOLLOW ONLY IF THE REPORT ACTUALLY ARMED. This was `let _ = write(...)` followed
        // by an unconditional delete, so a write that FAILED (read-only tree, full disk) left the STALE
        // report in place and removed its callgraph — strictly worse than the pre-rung state, because
        // the half that survives is the one a gate reads while the half that made `callers` answerable
        // is gone. candor-java's arm of this rung raised it: rust discarded a result java could see.
        // A pair degrades together or not at all.
        if std::fs::write(&full, doc).is_err() {
            eprintln!(
                "candor-scan: could not arm the report at {} — leaving it and its sidecars exactly as \
                 they are; if this run does not complete, that path may still hold a PREVIOUS run's report",
                full.display()
            );
            continue;
        }
        // ⟨0.28⟩ …AND THIS REPORT'S §2.2 SIDECARS GO WITH IT — DELETED, not emptied.
        //
        // An armed report beside a LIVE sidecar is a pair that contradicts itself, and §2.2 gives the
        // sidecar no provenance of its own to arbitrate with. It is not theoretical: `callers`/`whatif`/
        // `rewire` are answered FROM THE SIDECAR, because a currently-pure function is absent from the
        // report by §2 rule 3. Measured — baseline `f` pure with one caller `g`, new version gives `f` an
        // effect and adds caller `h`, run exits 2 — `callers f` answered exit 0 with "reached by 1
        // function(s) (the blast radius if it gained an effect): g". Confident, labelled the blast
        // radius, and wrong. An agent reads it as safe-to-edit.
        //
        // DELETED rather than `{}`, and not by reading the report's anti-deletion rule across: no sidecar
        // consumer treats absence as a claim (§2.2 makes it OPTIONAL, so every consumer has an absence
        // arm and every specified arm is safe), while ⟨0.24⟩ has already ruled empty ≡ absent ≡
        // unparseable for the hierarchy. Measured four-way on the one cell that rule does not cover — an
        // empty-but-valid baseline callgraph — all four engines answer `origin: "unknown"`, so `{}` buys
        // nothing the deletion does not.
        //
        // The reserved-segment names come from §2.2's family-wide list. Here a MISS is safe (a sidecar
        // left behind is the pre-rung state, and the ⟨0.28⟩ pairing rule catches it consumer-side),
        // whereas an over-reach would delete something that is not ours — the opposite of the armer
        // above, where a miss leaves a stale report and an over-reach destroys a file. Both directions
        // are chosen so the WRONG guess costs the least.
        for seg in ["callgraph", "hierarchy", "locs", "calibrated", "layerreach"] {
            let side = full.with_extension("").to_string_lossy().into_owned() + "." + seg + ".json";
            if std::path::Path::new(&side).exists() {
                // Never a path this run READS, on the same rule as the report itself.
                if inputs.iter().any(|(path, _)| crate::scan::same_artifact_pub(&side, path)) {
                    continue;
                }
                // A SYMLINKED SIDECAR IS LEFT ALONE, AND SAID SO. Deleting it and later handing back the
                // bytes (the orphan path) replaces the LINK with a regular file — a third state neither
                // the pre-run tree nor the armed tree ever had, and it severs exactly the shared-artifact
                // CI layout ⟨0.28⟩'s own artifact rule exists to preserve ("write THERE, leaving the link
                // in place"). Measured on rust before this: an orphan's symlinked callgraph came back as
                // a file. Raised by candor-swift's arm of this rung. The pairing rule covers what is left
                // behind, which is why leaving it is the cheap side of the trade.
                if std::fs::symlink_metadata(&side).is_ok_and(|m| m.file_type().is_symlink()) {
                    eprintln!(
                        "candor-scan: {side} is a §2.2 sidecar of an armed report but is a SYMLINK — \
                         leaving it, because removing it would sever the link on restore. Its report is \
                         armed, so treat the pair as unanswerable (SPEC §3.3.1 ⟨0.28⟩)."
                    );
                    continue;
                }
                if let Ok(prev) = std::fs::read(&side) {
                    OUT_ARMED_SIDECARS
                        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                        .lock()
                        .unwrap()
                        .push((std::path::PathBuf::from(&side), prev));
                }
                let _ = std::fs::remove_file(&side);
            }
        }
    }
}

/// What the armer saved so the disarm can hand it back: `(path, the bytes before arming)`, guarded
/// for the lazy `OnceLock` init. Named once so the two ledgers below cannot drift in shape.
type ArmedBytes = std::sync::OnceLock<std::sync::Mutex<Vec<(std::path::PathBuf, Vec<u8>)>>>;

/// `(path, bytes)` for every §2.2 sidecar this run DELETED while arming. Restored beside its report by
/// [`disarm_unwritten_out_reports`] when the run turns out not to have owned that report after all —
/// an orphan's sidecar is as much not-ours as the orphan itself.
static OUT_ARMED_SIDECARS: ArmedBytes = std::sync::OnceLock::new();

/// The exact placeholder bytes, so `disarm` can tell "still armed" from "this run rewrote it".
static OUT_ARM_DOC: std::sync::OnceLock<String> = std::sync::OnceLock::new();
/// ⟨0.28⟩ Latched by `scan_target` once the run has FINISHED ITS WRITE PHASE over the target — the
/// license [`disarm_unwritten_out_reports`] requires before it hands anything back. See that fn for
/// why the license is this and not "control returned".
static OUT_REPORTS_WRITTEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The run scanned its whole target and wrote (or streamed) a report per member. Only `scan_target`
/// calls this, at its exits — it is the one place both the plain and `--deps` routes funnel through,
/// so a route that dies BEFORE reaching it (a missing `Cargo.lock`, or any early return a future
/// change adds to `run_with_deps`) never acquires the license and the placeholders stand.
pub(crate) fn mark_out_reports_written() {
    let _ = OUT_REPORTS_WRITTEN.set(true);
}
/// `(path, bytes-before-arming)` for every report this run armed under an `--out` prefix.
static OUT_ARMED: ArmedBytes = std::sync::OnceLock::new();

/// SPEC §3.3.1 ⟨0.28⟩ — **HAND BACK WHAT THIS RUN TURNED OUT NOT TO OWN.**
///
/// Arming cannot know at parse time which packages the run will write, so it arms the whole previous
/// set. Once the run has finished writing, a file STILL holding the placeholder is one the run never
/// claimed — a leftover from a package that is no longer in the scan. That is not an incomplete
/// analysis, and leaving the ⟨0.21⟩ placeholder there asserts one: measured, it turned a complete scan
/// of the remaining members into a permanent exit-2 refusal that only manual deletion cleared.
///
/// So the previous bytes go back, and the orphan is left exactly as this run found it. **The orphan
/// remains an open defect** — a deleted crate's report still describes code that is gone, and still
/// reaches a gate over the prefix — and that is deliberate: it is a PRE-EXISTING defect with its own
/// design question (delete it? mark it not-in-scan? both need a wire answer, and a prefix can legitimately
/// be shared), and quietly resolving it inside a staleness fix would be deciding it by accident.
///
/// Deleting the placeholder instead of restoring is also rejected, for §3.3.1's own reason: a consumer
/// that treats a missing file as "nothing to report" fails open by a different route.
///
/// **LICENSED BY [`mark_out_reports_written`], NOT BY BEING CALLED.** This used to run whenever control
/// came back to `scan_main`, and `run_with_deps` RETURNS 2 on a missing `Cargo.lock` instead of
/// exiting — so a `--deps` run that failed before writing anything handed the previous run's green
/// reports back, which is precisely the state this rung exists to destroy (⟨0.24⟩: "not left holding a
/// previous run's answer"). Gating on the call site would fix one spelling and leave the next early
/// `return` to re-open it silently; the license is keyed on the thing the hand-back actually requires —
/// THIS run wrote its report set, so a file still holding the placeholder is one the run did not own —
/// and only `scan_target`'s exits grant it. The orphan hand-back on a COMPLETED run (exit 0, 1 or a
/// gate refusal's 2 — all after the write phase) is unchanged.
pub(crate) fn disarm_unwritten_out_reports() {
    if !matches!(OUT_REPORTS_WRITTEN.get(), Some(true)) {
        return;
    }
    let Some(armed) = OUT_ARMED.get() else { return };
    let Some(doc) = OUT_ARM_DOC.get() else { return };
    for (path, prev) in armed.lock().unwrap().iter() {
        // Only files this run left untouched since arming — anything it rewrote is a real report.
        if std::fs::read(path).is_ok_and(|now| now == doc.as_bytes()) {
            let _ = std::fs::write(path, prev);
            // ⟨0.28⟩ …and this report's sidecars come back with it. A report the run turned out not to
            // own is an ORPHAN, left exactly as found — and "as found" included its sidecars. Restoring
            // the report while leaving its sidecars deleted would be a THIRD state neither the pre-run
            // tree nor the armed tree ever had, and it would silently degrade every `callers`/`whatif`
            // answer over that package to the absence arm with nothing saying why.
            if let Some(sides) = OUT_ARMED_SIDECARS.get() {
                let stem = path.with_extension("").to_string_lossy().into_owned();
                for (sp, sprev) in sides.lock().unwrap().iter() {
                    if sp.to_string_lossy().starts_with(&(stem.clone() + ".")) && !sp.exists() {
                        let _ = std::fs::write(sp, sprev);
                    }
                }
            }
        }
    }
}

pub(crate) fn write_json_stream_failclosed(reason_key: &str, why: &str) {
    if !matches!(WANT_JSON_STREAM.get(), Some(true)) { return; }
    if matches!(GATE_JSON_PATH.get(), Some(Some(p)) if p == "-") { return; }
    if matches!(REPORT_STREAM_WRITTEN.get(), Some(true)) { return; }
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " ");
    let doc = format!(
        "{{\n  \"candor\": {{ \"version\": \"scan-{ver}\", \"toolchain\": \"stable\", \"spec\": \"{spec}\" }},\n  \"functions\": [],\n  \"analyzed\": {{ \"count\": 0 }},\n  \"unanalyzed\": [ {{ \"path\": \"<run>\", \"reason\": \"{key}: {reason}\" }} ]\n}}",
        ver = env!("CARGO_PKG_VERSION"),
        spec = candor_report::SPEC_VERSION,
        key = esc(reason_key),
        reason = esc(why),
    );
    println!("{doc}");
    let _ = REPORT_STREAM_WRITTEN.set(true);
}

/// ⟨0.24⟩ THE RULES THIS RUN COULD NOT DECIDE (SPEC §3.1 `fc4b5f6`) — accumulated across workspace
/// members like the violations, written once onto the verdict as `unevaluated`.
///
/// On THIS route the only unanswered rules are the WITHHELD `(rule, function)` pairs: `allow` and
/// `forbid` are both evaluable here, which is why `gate --report`'s two whole-policy refusals have no
/// counterpart. ONE ENTRY PER RULE — the first function that defeats it is the example, since naming all
/// of them would bury the rule the operator has to fix.
pub(crate) static GATE_UNEVALUATED: std::sync::OnceLock<
    std::sync::Mutex<Vec<candor_report::Unevaluated>>,
> = std::sync::OnceLock::new();

pub(crate) fn record_gate_unevaluated(items: &[candor_report::Unevaluated]) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    if items.is_empty() || !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_UNEVALUATED.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut g = acc.lock().unwrap();
    for it in items {
        if !g.iter().any(|e| e.rule == it.rule) {
            g.push(it.clone());
        }
    }
}

/// ⟨0.28⟩ SPEC §6.2 — THE POLICY LINES THE PARSE DROPPED, toward the verdict's `ignored` disclosure.
/// The line-level leniency is unchanged (an unrecognized line is ignored-with-a-warning); this is what
/// that leniency COMPOSES TO on the machine channel: every line ignored is a gate that asked nothing,
/// and a verdict that omits them claims the policy on disk is the policy that ran. Distinct from
/// `unevaluated` (rules that PARSED and could not be answered). A no-op unless `--gate-json` was
/// given, mirroring the other recorders; deduplicated by line number for the workspace case, where the
/// same policy text is parsed once but a future second parse must not double-report.
pub(crate) static GATE_IGNORED: std::sync::OnceLock<
    std::sync::Mutex<Vec<candor_report::IgnoredLine>>,
> = std::sync::OnceLock::new();

pub(crate) fn record_gate_ignored(items: &[candor_report::IgnoredLine]) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    if items.is_empty() || !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_IGNORED.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut g = acc.lock().unwrap();
    for it in items {
        if !g.iter().any(|e| e.line == it.line) {
            g.push(it.clone());
        }
    }
}

/// ⟨0.27⟩ THE RULES WHOSE SCOPE BOUND NO FUNCTION (SPEC §4/§3.1 `zeroMatch`) — accumulated across
/// workspace members like the violations, written once onto the verdict. A `BTreeSet` because the pinned
/// collation is code-point sorted + deduplicated (a workspace's members all evaluate the same policy, so
/// the same raw line arrives once per member), and `BTreeSet<String>` yields exactly that order for free.
/// A no-op unless `--gate-json` was given, mirroring the other recorders — the stderr disclosure is
/// printed by the scan itself either way.
pub(crate) static GATE_ZERO_MATCH: std::sync::OnceLock<std::sync::Mutex<std::collections::BTreeSet<String>>> =
    std::sync::OnceLock::new();

pub(crate) fn record_gate_zero_match(rules: &[String]) {
    if recording_suppressed() { return; }   // ⟨0.31⟩ the peek writes no verdict state
    if rules.is_empty() || !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_ZERO_MATCH.get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));
    acc.lock().unwrap().extend(rules.iter().cloned());
}

/// Write the structured gate verdict `{ spec, ok, violations }` (candor-spec §3.3 ⟨0.8⟩) — the machine
/// analog of the AS-EFF console lines, accumulated from the SAME `policy_violations` that set the exit
/// code, so it can never disagree with the gate. Called ONCE, by `scan_main`, after the whole scan (every
/// workspace member) completes. `-` streams to stdout. A no-op unless `--gate-json` was given.
/// ⟨0.24⟩ ARM THE VERDICT FAIL-CLOSED, at the first instant of the run.
///
/// Any exit before a verdict is written must not leave the PREVIOUS run's document in place — a CI
/// wrapper that reads the artifact instead of the exit code would then report a pass over a run that
/// refused. An adversarial review found exactly that on the ⟨0.27⟩ engine-pin refusal: exit 2 with a
/// seeded `{"ok":true}` still on disk, in rust, ts AND swift, while candor-java overwrote it.
///
/// Arming at the START is what makes this a CLASS fix rather than one branch: every exit path — the pin,
/// a corrupt baseline, a panic, a kill — leaves a refusal unless the run got far enough to replace it.
/// candor-java's `armGateJson` is the model, and the wording is deliberately about the RUN, not the code.
pub(crate) fn arm_gate_json() {
    let Some(Some(path)) = GATE_JSON_PATH.get() else { return };
    if path == "-" { return; }                 // a stream has no previous document to be stale
    let doc = format!(
        "{{\n \"spec\": \"{}\",\n \"ok\": false,\n \"refused\": true,\n \"reason\": \"{}\"\n}}\n",
        candor_report::SPEC_VERSION,
        "the gate did not complete — this document was written when the run STARTED and was never \
         replaced by a verdict, so the run failed, crashed or was killed before it could decide. It is \
         NOT a verdict about the code; see the run's stderr for the cause."
    );
    if let Err(e) = std::fs::write(path, doc) {
        eprintln!(
            "candor-scan: could not arm --gate-json {path} fail-closed ({e}) — if this run does not \
             complete, that path may still hold a PREVIOUS run's verdict"
        );
    }
}

pub(crate) fn write_gate_json(exit_code: i32) {
    let Some(Some(path)) = GATE_JSON_PATH.get() else { return };
    // ⟨0.21⟩ COMPLETENESS MANIFEST: the accumulated analyzed count + the units that couldn't be analyzed.
    let analyzed_count = *GATE_ANALYZED.get_or_init(|| std::sync::Mutex::new(0)).lock().unwrap();
    let unanalyzed: Vec<candor_report::UnanalyzedUnit> = GATE_UNANALYZED
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clone();
    // Two distinct exit-2 causes: (a) INCOMPLETE ANALYSIS (a source parse failure) → emit a structured
    // incomplete verdict (Tom's call 2026-07-17, refining §3.3.1 to "no ok:true GUESS": ok:false +
    // incomplete:true + the `unanalyzed` list is honest, never a fabricated pass — a machine learns WHY
    // the gate couldn't certify); (b) a broken gate CONFIG (an unreadable or unhonourable policy).
    //
    // ⟨0.24⟩ **(b) NOW WRITES A REFUSAL DOCUMENT TOO — the rule has no exempt cause** (SPEC §3.1
    // `1503368` (b)). It used to write NOTHING, on the reasoning that a policy nobody could parse has no
    // faithful verdict to emit. True, and beside the point: the argument that MANDATES a document is that
    // a CI wrapper of the shape `candor-scan … --gate-json v.json || true; jq .ok v.json` re-reads **the
    // previous run's document as current**, and a stale green does not care why this run declined to
    // overwrite it. The hazard is identical for both causes; only the measurement that prompted the
    // original clause differed.
    //
    // A refusal document is not a fabricated verdict, which is what makes this consistent rather than a
    // reversal: `gate_refusal_json` carries `ok:false`, `refused:true`, the reason, and **NO `violations`
    // key at all**. The shape already says "no claim about violations", and that is the honest thing to
    // say when the policy could not be read. Its naive read is the fail-closed one, which is the whole
    // standard this format holds itself to.
    //
    // ⟨0.24⟩ **…AND ONLY WHEN THE RUN HAS ESTABLISHED NO VIOLATION — the third conjunct, and the one this
    // predicate was missing** (SPEC §3.1 `4c79958`). It used to read `exit_code == 2 && unanalyzed
    // .is_empty()`, which conflates *"this run ended refused"* with *"this run evaluated nothing"* — the
    // exact conflation the clause forbids. MEASURED 2026-07-28: a pure fn gains an `Fs` call against a
    // frozen baseline; with no policy, exit 1 and `violations: ["AS-EFF-005"]`; add ANY policy carrying a
    // bad token and the run exited 2 with NO `violations` key. **A typo in a policy token deleted a
    // certain baseline regression from the machine channel**, while the `[AS-EFF-005]` line stayed on
    // stderr — the human kept the finding, CI lost it.
    //
    // The conflation was invisible because the AS-EFF-005 baseline guard is a DIFFERENT violation
    // producer from the policy gate, runs deliberately EARLIER (so both record toward one verdict), and
    // the precedence repair had been scoped to the policy gate's own list. Keying on the shared
    // accumulator instead of on any one producer is what makes the fix general: it covers the
    // baseline-regression-then-unreadable-policy case, the baseline-regression-then-unhonourable-policy
    // case, and the baseline-regression-then-sole-withholding case, without naming any of them.
    // ⟨0.30⟩ read from the thread-local (see its declaration for why it is no longer a process global).
    // The shared serializer (candor_report::gate_verdict_json_v24) also fixes the violation ORDER —
    // (rule, detail), the same order the console prints — so the verdict is deterministic and
    // byte-comparable across backends. Members already record in that order per crate.
    let mut violations = GATE_VIOLATIONS.with(|v| v.borrow().clone());
    // ⟨0.30⟩ the peeked functions performing a denied effect — the second `incomplete` cause.
    let out_of_scope: Vec<candor_report::OutOfScopeFinding> = GATE_OUT_OF_SCOPE
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clone();
    // ⟨0.30⟩ …AND the run established no out-of-scope finding. Without this conjunct a ⟨0.30⟩ exit 2 —
    // which HAS a faithful verdict to emit — would be written as a config REFUSAL document, losing the
    // findings and telling the operator their policy failed to load. Same conflation the ⟨0.24⟩ note
    // above records for `violations`, one cause later.
    if exit_code == 2 && unanalyzed.is_empty() && violations.is_empty() && out_of_scope.is_empty() {
        let why = GATE_REFUSAL.get().cloned().unwrap_or_else(|| {
            "the gate config did not load (exit 2) — see stderr for the specific cause".to_string()
        });
        // ⟨0.24⟩ …carrying the unanswered rules, which on a SOLE refusal is the whole of what the
        // consumer can act on (SPEC §3.1 `fc4b5f6`).
        let sole_unevaluated: Vec<candor_report::Unevaluated> = GATE_UNEVALUATED
            .get()
            .map(|m| m.lock().unwrap().clone())
            .unwrap_or_default();
        match candor_report::gate_refusal_json_v24(&why, &sole_unevaluated) {
            Ok(json) => {
                if path == "-" {
                    println!("{json}");
                } else if let Err(e) =
                    candor_report::write_atomic(std::path::Path::new(path), format!("{json}\n").as_bytes())
                {
                    eprintln!(
                        "candor-scan: could not write the refusal document to --gate-json {path} ({e}) — \
                         a consumer reading that path will see the PREVIOUS run's verdict, which is \
                         stale. Delete it, or treat exit 2 as a failure."
                    );
                }
            }
            Err(e) => eprintln!("candor-scan: could not serialize the refusal document ({e})"),
        }
        return;
    }
    // ⟨0.15 staged⟩ the advisory coverage note: present only when a scanned target's κ ledger was
    // non-empty. Verdict-preserving — ok/violations/exit are computed exactly as before (the ⟨0.9⟩
    // provable-purity auto-disclosure precedent); a fully-covered scan's verdict is byte-unchanged.
    let packages: Vec<String> = GATE_COVERAGE
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeSet::new()))
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let coverage =
        (!packages.is_empty()).then_some(candor_report::GateCoverage { uncovered: packages.len(), packages });
    // ⟨0.21⟩ the full verdict carries `analyzed:{count}` (Gap 1); on a COMPLETE gate `unanalyzed` is empty,
    // so `incomplete`/`unanalyzed` are omitted — byte-compatible with a pre-rung verdict + coverage.
    //
    // ONE code path now serves every non-config exit, and that is deliberate. The incomplete arm used to
    // be a separate `match` that hard-coded `&mut none` for the violations and `None` for coverage, which
    // is how a real `deny Net` finding got deleted from the document on a crate that also had one
    // unparseable file (measured 2026-07-28; the same defect the `gate --report` route carried, since
    // that route was written to mirror this one for §3.1 byte-equality). Violations, coverage and the
    // manifest all ride the ONE verdict; `gate_verdict_json_full` computes `ok = no violations AND not
    // incomplete`, so exit 1, exit 2-with-a-manifest and exit 0 all get an accurate document.
    // ⟨0.24⟩ …and the ambient vocabulary that participated, if any (SPEC §3.1).
    let vocabulary = GATE_VOCABULARY.get().and_then(|m| {
        let g = m.lock().unwrap();
        (!g.1.is_empty()).then(|| candor_report::GateVocabulary {
            config: g.0.clone(),
            aliases: g.1.clone(),
        })
    });
    // ⟨0.27⟩ WHEN THIS RUN REFUSED WHILE HOLDING A VIOLATION, THE DOCUMENT IS A VERDICT AND THE REFUSAL
    // TRAVELS AS `unevaluated` — NOT as `refused`/`reason` (SPEC §3.1's composed-document clause). This
    // engine used to put `refused: true` beside `violations`, reasoning that dropping the refusal half
    // would let an operator read `{ok:false, violations:[AS-EFF-005]}` as "the gate was enforced and this
    // is all it found". The harm was real and the channel was wrong: `refused: true` is the refusal
    // document's DISCRIMINATOR, and its pinned meaning — "the gate is making no claim about violations" —
    // contradicts a document that carries them. A consumer keying on `refused` filed a certain violation
    // under "no claim". The disclosure that says "the policy never ran" is the `unevaluated` list, one
    // entry per rule of the refused policy (recorded at the refusal site), which answers the operator's
    // actual question — WHICH rules went unenforced — instead of re-using the other document's flag.
    //
    // On a policy-free composed run (a baseline regression beside, say, an unreadable baseline sibling)
    // there is no policy to enumerate and `unevaluated` is rightly empty; the refusal reason still
    // reaches the human on stderr, and reaches the machine only when the run ends REFUSED (exit 2, the
    // sole-refusal document above).
    let unevaluated: Vec<candor_report::Unevaluated> = GATE_UNEVALUATED
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();
    // ⟨0.27⟩ …and the zero-match disclosure (SPEC §4 `zeroMatch`): the same list the stderr lines carry,
    // in the machine channel — a typo'd scope was invisible to a wrapper that reads the document.
    let zero_match: Vec<String> = GATE_ZERO_MATCH
        .get()
        .map(|m| m.lock().unwrap().iter().cloned().collect())
        .unwrap_or_default();
    // ⟨0.28⟩ …and the policy lines the parse DROPPED (SPEC §6.2 `ignored`): the same facts the per-line
    // stderr warnings carry, on the machine channel — omitted when nothing was dropped, so a clean
    // policy's verdict is byte-identical.
    let ignored: Vec<candor_report::IgnoredLine> = GATE_IGNORED
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();
    // ⟨0.31⟩ the ambient partner provenance the producer recorded, carried into the verdict.
    let net_partners: Vec<candor_report::NetPartners> = GATE_NET_PARTNERS
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();
    match candor_report::gate_verdict_json_v31(
        &mut violations,
        coverage.as_ref(),
        analyzed_count,
        &unanalyzed,
        vocabulary.as_ref(),
        &unevaluated,
        &zero_match,
        &ignored,
        &out_of_scope,
        &net_partners,
    ) {
        Ok(json) if path == "-" => println!("{json}"),
        Ok(json) => {
            if let Err(e) = candor_report::write_atomic(std::path::Path::new(path), format!("{json}\n").as_bytes()) {
                eprintln!("candor-scan: could not write --gate-json {path}: {e}");
            }
        }
        Err(e) => eprintln!("candor-scan: could not serialize gate verdict: {e}"),
    }
}

pub(crate) fn host_part(h: &str) -> String {
    let a = h.split_once("://").map(|(_, r)| r).unwrap_or(h);
    let a = a.split('/').next().unwrap_or(a);
    a.rsplit_once('@').map(|(_, h)| h).unwrap_or(a).to_string()
}
