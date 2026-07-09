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
                inf.iter().copied().collect() // `pure` — ANY effect (Unknown included: not certifiably pure)
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
                "Net" => hostsacc.get(q),
                "Exec" => cmdsacc.get(q),
                "Db" => tablesacc.get(q),
                _ => pathsacc.get(q),
            };
            // An INCOMPLETE surface (a structurally-invisible reach) can't be certified even with visible
            // hosts — else a benign literal masks the invisible forbidden endpoint (the masking evasion).
            let surface_incomplete = incompleteacc.get(q).is_some_and(|s| s.contains(r.effect));
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
    match candor_report::gate_verdict_json(&mut violations) {
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
