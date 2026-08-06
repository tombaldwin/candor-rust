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
pub(crate) fn policy_precheck(
    policy_text: &str,
    unknown_aliases: &std::collections::BTreeMap<String, BTreeSet<candor_classify::policy::ReasonClass>>,
) -> (Vec<String>, UsedAliases) {
    let p = candor_classify::policy::parse_policy_silent(policy_text, unknown_aliases);
    // ⟨0.24⟩ FATAL errors only. `ParsedPolicy::errors` now also carries the lines the parser DROPPED but
    // could survive (a malformed `forbid`, an unknown rule kind) — those are `parsepolicy`'s to report,
    // and refusing a build on them would be the opposite defect.
    (p.fatal_messages().into_iter().map(str::to_string).collect(), p.used_aliases)
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

/// Violations ACCUMULATED across `scan_one` calls. A `[workspace]` root runs the gate once per member;
/// writing the verdict per member let the LAST member overwrite the first's violations — `gate.json` said
/// `ok: true` while the process exited 1 (a clean final member masked an earlier violator), violating the
/// §3.3 "verdict MUST agree with the exit code" rule. So members only RECORD here; `scan_main` writes ONCE.
pub(crate) static GATE_VIOLATIONS: std::sync::OnceLock<std::sync::Mutex<Vec<GateViolation>>> = std::sync::OnceLock::new();

/// Record one scan's gate violations toward the final `--gate-json` verdict. A no-op unless the flag was
/// given (the direct-`scan_one` test/selftest paths never record).
pub(crate) fn record_gate_violations(violations: &[GateViolation]) {
    if !matches!(GATE_JSON_PATH.get(), Some(Some(_))) {
        return;
    }
    let acc = GATE_VIOLATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    acc.lock().unwrap().extend(violations.iter().cloned());
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

pub(crate) fn record_gate_analyzed(count: usize, unanalyzed: &[candor_report::UnanalyzedUnit]) {
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
    let _ = GATE_REFUSAL.set(why.into());
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

/// Write the structured gate verdict `{ spec, ok, violations }` (candor-spec §3.3 ⟨0.8⟩) — the machine
/// analog of the AS-EFF console lines, accumulated from the SAME `policy_violations` that set the exit
/// code, so it can never disagree with the gate. Called ONCE, by `scan_main`, after the whole scan (every
/// workspace member) completes. `-` streams to stdout. A no-op unless `--gate-json` was given.
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
    let acc = GATE_VIOLATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    // The shared serializer (candor_report::gate_verdict_json_v24) also fixes the violation ORDER —
    // (rule, detail), the same order the console prints — so the verdict is deterministic and
    // byte-comparable across backends. Members already record in that order per crate.
    let mut violations = acc.lock().unwrap().clone();
    if exit_code == 2 && unanalyzed.is_empty() && violations.is_empty() {
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
    // ⟨0.24⟩ …and when this exit-2 run refused WHILE holding a violation, the document carries BOTH: the
    // violation that dominates (SPEC §3.1 `4c79958` — precedence binds the verdict) and the refusal that
    // says the policy never ran. Dropping the second half would be the mirror defect: an operator reading
    // `{ok:false, violations:[AS-EFF-005]}` would conclude the gate had been enforced and passed.
    //
    // NARROW BY CONSTRUCTION: `unanalyzed.is_empty()` keeps this off the incomplete-analysis path, whose
    // `incomplete`/`unanalyzed` keys already carry the reason and whose `gate --report` counterpart has no
    // refusal to disclose — attaching a second reason channel there would break §3.1's byte-equality MUST
    // to say something already said.
    //
    // The fallback reason is the SAME one the pure-refusal arm uses, and it matters here for the same
    // reason: reaching this line means the run refused, so an unrecorded `why` must still produce
    // `refused: true` rather than a document that looks like an ordinary exit-2.
    let refusal = (exit_code == 2 && unanalyzed.is_empty()).then(|| {
        GATE_REFUSAL
            .get()
            .cloned()
            .unwrap_or_else(|| "the gate config did not load (exit 2) — see stderr for the specific cause".to_string())
    });
    // ⟨0.24⟩ …and the rules this run could NOT decide (SPEC §3.1 `fc4b5f6`), beside the verdict rather
    // than instead of it. On this route that is only the WITHHELD pairs; `allow`/`forbid` are evaluable
    // here, so `gate --report`'s two whole-policy entries have no counterpart and the byte-equality MUST
    // is untouched on every policy both routes answer in full.
    let unevaluated: Vec<candor_report::Unevaluated> = GATE_UNEVALUATED
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();
    match candor_report::gate_verdict_json_v24_refused(
        &mut violations,
        coverage.as_ref(),
        analyzed_count,
        &unanalyzed,
        vocabulary.as_ref(),
        refusal.as_deref(),
        &unevaluated,
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
