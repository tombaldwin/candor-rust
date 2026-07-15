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

/// Evaluate a CANDOR_POLICY (parsed by the SHARED §6.2 parser in candor-classify, so this gate can
/// never disagree with the nightly/JVM gates on grammar) over a finished scan. Returns one line per
/// violation: deny/pure (AS-EFF-006) against the transitive `inferred` sets, literal allowlists
/// (AS-EFF-008) against the transitive hosts/cmds/paths/tables surfaces, layering `forbid A -> B`
/// (AS-EFF-009) by reachability over the local call graph.
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
) -> Vec<GateViolation> {
    use candor_classify::policy::{literal_allowed, parse_policy, scope_matches};
    let p = parse_policy(policy_text);
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let mut out = Vec::new();
    for q in all {
        let inf = inferred.get(q).unwrap_or(&empty);
        // AS-EFF-006 — deny/pure: forbidden effects in the transitive set.
        for r in &p.rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            let hits: Vec<&str> = if r.effects.is_empty() {
                // `pure` — every EFFECT, but NOT `Unknown`: the §4 trust marker is not an effect
                // (AS-EFF-003's concern; `deny Unknown <scope>` is the explicit knob). The reference
                // engine and the deep backend exclude it identically — this engine wrongly counted an
                // Unknown-only fn as a `pure` violation until 2026-07-09 (a cross-engine verdict split
                // on the same policy file).
                inf.iter().copied().filter(|e| *e != "Unknown").collect()
            } else {
                inf.iter().copied().filter(|e| r.effects.contains(e)).collect()
            };
            if !hits.is_empty() {
                out.push(GateViolation {
                    rule: "AS-EFF-006".into(),
                    func: q.clone(),
                    effects: hits.iter().map(|s| s.to_string()).collect(),
                    detail: format!("`{q}` performs {{ {} }}, forbidden by policy: `{}`", hits.join(", "), r.raw),
                });
            }
        }
        // AS-EFF-008 — literal allowlists over the transitive literal surfaces.
        for r in &p.allow_rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            if !inf.contains(r.effect) {
                continue;
            }
            let lits = match r.effect {
                // `Llm` ⟨0.13⟩ rides the Net host surface (SPEC §1) — `allow Llm <host>` certifies the same
                // captured hosts as `allow Net`, restricted to the MODEL hosts (a model call's host WAS
                // captured as a Net literal). Matches candor-java's checkAllowlist("Llm", hostFixpoint, …).
                "Net" | "Llm" => hostsacc.get(q),
                "Exec" => cmdsacc.get(q),
                "Db" => tablesacc.get(q),
                _ => pathsacc.get(q),
            };
            // An INCOMPLETE surface (a structurally-invisible reach) can't be certified even with visible
            // hosts — else a benign literal masks the invisible forbidden endpoint (the masking evasion).
            // `Llm` keys off the NET incompleteness (it rides the Net host literal): a runtime/masked model
            // host that fails-closes Net must fail-close `allow Llm` too (incompleteAsLlm in candor-java).
            let inc_key = if r.effect == "Llm" { "Net" } else { r.effect };
            let surface_incomplete = incompleteacc.get(q).is_some_and(|s| s.contains(inc_key));
            match lits {
                Some(ls) if !ls.is_empty() && !surface_incomplete => {
                    let bad: Vec<&str> =
                        ls.iter().filter(|l| !literal_allowed(r.effect, l, &r.literals)).map(String::as_str).collect();
                    if !bad.is_empty() {
                        out.push(GateViolation {
                            rule: "AS-EFF-008".into(),
                            func: q.clone(),
                            effects: vec![r.effect.to_string()],
                            detail: format!("`{q}` reaches {{ {} }} outside the allowlist: `{}`", bad.join(", "), r.raw),
                        });
                    }
                }
                _ => out.push(GateViolation {
                    rule: "AS-EFF-008".into(),
                    func: q.clone(),
                    effects: vec![r.effect.to_string()],
                    detail: format!("`{q}` performs {} with no visible literal — the surface cannot be certified: `{}`", r.effect, r.raw),
                }),
            }
        }
        // AS-EFF-009 — layering: no fn in scope A may transitively reach scope B.
        for r in &p.layer_rules {
            if !scope_matches(q, &r.from) {
                continue;
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<&str> = calls.get(q).map(|cs| cs.iter().map(String::as_str).collect()).unwrap_or_default();
            let mut hit: Option<&str> = None;
            while let Some(n) = stack.pop() {
                if !seen.insert(n) {
                    continue;
                }
                if scope_matches(n, &r.to) {
                    hit = Some(n);
                    break;
                }
                if let Some(cs) = calls.get(n) {
                    stack.extend(cs.iter().map(String::as_str));
                }
            }
            if let Some(h) = hit {
                out.push(GateViolation {
                    rule: "AS-EFF-009".into(),
                    func: q.clone(),
                    effects: Vec::new(), // a layer-flow has no single effect
                    detail: format!("`{q}` reaches into a forbidden layer (via `{h}`): `{}`", r.raw),
                });
            }
        }
    }
    // Sort by (rule, detail) — identical order to the old rendered-line sort (the "[rule] detail" render
    // puts the constant '[' first and all AS-EFF codes are same-length), without allocating two Strings
    // per comparison.
    out.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    out
}

/// The provable-purity DISCLOSURE (eval/fixloop/DISPATCH-NOTE.md): functions that PASS a `pure`/`deny` layer
/// but are `Unknown` — their compliance is asserted, not verified (the Unknown could hide the forbidden
/// effect; the classic case is a fn/closure-injected port). Advisory — NEVER a violation, so the gate's
/// verdict/exit is untouched; the caller emits it as a note so an author learns their layer isn't PROVABLY
/// clean. Returns `(fn, deny-Unknown upgrade)` per hole. Mirrors `candor-query unverified`.
pub(crate) fn unverified_holes(
    policy_text: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
) -> Vec<(String, String)> {
    use candor_classify::policy::{parse_policy, rule_and_upgrade, unverified_hole_rule};
    let rules = parse_policy(policy_text).rules;
    let empty: BTreeSet<&'static str> = BTreeSet::new();
    let mut out = Vec::new();
    for q in all {
        // Same predicate + upgrade reconstruction as `candor-query unverified` (candor_classify::policy):
        // the two disclosure paths share ONE definition of a hole, so they cannot drift.
        let effs: Vec<&str> = inferred.get(q).unwrap_or(&empty).iter().copied().collect();
        if let Some(r) = unverified_hole_rule(q, &effs, &rules) {
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
///   - ⟨0.16 staged⟩ When the baseline **callgraph sidecar** is present (sibling of the resolved report —
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
pub(crate) fn check_baseline(
    value: &str,
    dir: &str,
    crate_name: &str,
    all: &[String],
    inferred: &HashMap<String, BTreeSet<&'static str>>,
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
    // ⟨0.16 staged⟩ Existence keys on the baseline callgraph sidecar when present (SPEC §7 item 5): the
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
    for q in all {
        // Existence: in the baseline report, OR (⟨0.16 staged⟩) a baseline-callgraph node — a
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
        if !gained.is_empty() {
            out.push(GateViolation {
                rule: "AS-EFF-005".into(),
                func: q.clone(),
                effects: gained.iter().map(|s| s.to_string()).collect(),
                detail: format!(
                    "`{q}` gained effect {{ {} }} not present in the baseline; an existing function \
                     started performing a new effect",
                    gained.join(", ")
                ),
            });
        }
    }
    out.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    BaselineOutcome::Checked(out)
}

/// `--gate-json <file>` target, set once in `scan_main` (a no-op when unset — the direct-`scan_one` test
/// paths never RECORD). Mirrors the `CFG_FEATURES` OnceLock idiom; a plain path so it threads no ScanOpts.
/// Members record via `record_gate_violations`; `scan_main` writes the single final verdict.
pub(crate) static GATE_JSON_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

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

/// Write the structured gate verdict `{ spec, ok, violations }` (candor-spec §3.3 ⟨0.8⟩) — the machine
/// analog of the AS-EFF console lines, accumulated from the SAME `policy_violations` that set the exit
/// code, so it can never disagree with the gate. Called ONCE, by `scan_main`, after the whole scan (every
/// workspace member) completes. `-` streams to stdout. On exit 2 (an incomplete scan/gate — unreadable
/// policy, a parse failure) NO verdict is written: there is no faithful verdict to emit. A no-op unless
/// `--gate-json` was given.
pub(crate) fn write_gate_json(exit_code: i32) {
    let Some(Some(path)) = GATE_JSON_PATH.get() else { return };
    if exit_code == 2 {
        eprintln!("candor-scan: --gate-json not written — the scan/gate did not complete (exit 2)");
        return;
    }
    let acc = GATE_VIOLATIONS.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    // The shared serializer (candor_report::gate_verdict_json) also fixes the violation ORDER —
    // (rule, detail), the same order the console prints — so the verdict is deterministic and
    // byte-comparable across backends. Members already record in that order per crate.
    let mut violations = acc.lock().unwrap().clone();
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
    match candor_report::gate_verdict_json_with_coverage(&mut violations, coverage.as_ref()) {
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
