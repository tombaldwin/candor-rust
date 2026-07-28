//! ⟨0.24⟩ THE GATE — SPEC §6.2 matching over an ALREADY-ACCUMULATED signature, and the ONLY copy of
//! that matching in the stable toolchain.
//!
//! **THE SEAM.** [`GateInput`] is the boundary between *what produced the signature* and *what §6.2
//! does with it*. Every field is already accumulated: this module runs no fixpoint, opens no file and
//! consults no scan state, so the same matching code serves both routes in —
//!
//!   - `candor-scan … --policy <f>` builds a `GateInput` from the classifier's transitive accumulators
//!     (`crate::gate::policy_violations`, which is now a thin wrapper);
//!   - `candor-query gate --report <loc> --policy <f>` (SPEC §3.1 ⟨0.24⟩) builds one from a WRITTEN
//!     report and nothing else.
//!
//! That split is the whole point of §3.1 ⟨0.24⟩: until it existed the gate was reachable only THROUGH
//! the classifier, so a defect in the gate and a defect in the classifier were indistinguishable from
//! any test that could be written. Do NOT re-implement the matching on the report side — the §6.2
//! clause that mandates the verb was written about exactly that mistake.

use crate::policy::{literal_allowed, reason_class_matches, scope_matches, ParsedPolicy};
use candor_report::GateViolation;
use std::collections::{BTreeSet, HashMap};

/// ⟨0.20⟩ The `Net` destination classes an fn reaches (transitive) — the SINGLE derivation shared by the
/// report's `netClass` field (candor-scan's writer) and the gate: an exact host-literal match
/// ([`crate::net_dest_class`]) for the visible hosts, plus the fail-closed `unknown-host` when the Net
/// surface is masked (`incomplete` has Net) OR carries no visible host (a runtime endpoint). Call only
/// for an fn known to have Net; returns sorted.
///
/// It lives beside the gate rather than in the scanner because the report field and the gate filter MUST
/// be the same set: `gate --report` reads `netClass` off the wire and the scan derives it here, and the
/// §3.1 ⟨0.24⟩ byte-equivalence obligation is exactly the claim that those two agree.
pub fn net_classes_of<E: AsRef<str> + Ord>(
    q: &str,
    hostsacc: &HashMap<String, BTreeSet<String>>,
    incompleteacc: &HashMap<String, BTreeSet<E>>,
    partners: &BTreeSet<String>,
) -> Vec<String> {
    let mut classes: BTreeSet<String> = hostsacc
        .get(q)
        .into_iter()
        .flatten()
        .map(|h| crate::net_dest_class(h, partners).to_string())
        .collect();
    let masked = incompleteacc.get(q).is_some_and(|s| s.iter().any(|e| e.as_ref() == "Net"));
    let no_hosts = hostsacc.get(q).map(|s| s.is_empty()).unwrap_or(true);
    if masked || no_hosts {
        classes.insert("unknown-host".to_string());
    }
    classes.into_iter().collect()
}

/// ⟨0.24⟩ THE GATE'S INPUT — one signature per function, every field already TRANSITIVE.
///
/// `E` is the effect-name representation: `&'static str` on the scan route (the classifier's interned
/// vocabulary) and `String` on the report route (the wire's names, taken VERBATIM — a report naming an
/// effect this build's vocabulary does not list must still trip a `pure` rule, so the names are never
/// filtered through a known-effect allowlist on the way in).
pub struct GateInput<'a, E: AsRef<str> + Ord> {
    /// Every function the gate ranges over, in the caller's order.
    pub all: &'a [String],
    /// Per fn, the TRANSITIVE effect set — the model's `S`, with candor's `Unknown` marker carried as a
    /// member (this engine's encoding of `D ≠ ∅`).
    pub inferred: &'a HashMap<String, BTreeSet<E>>,
    /// The call graph AS-EFF-009 walks.
    pub calls: &'a HashMap<String, BTreeSet<String>>,
    /// Per fn, the TRANSITIVE literal surface AS-EFF-008 certifies against.
    pub hosts: &'a HashMap<String, BTreeSet<String>>,
    pub cmds: &'a HashMap<String, BTreeSet<String>>,
    pub paths: &'a HashMap<String, BTreeSet<String>>,
    pub tables: &'a HashMap<String, BTreeSet<String>>,
    /// Per fn, the effects whose literal surface is structurally INCOMPLETE — the AS-EFF-008 fail-closed
    /// marker, without which a benign visible literal masks an invisible forbidden endpoint.
    pub surface_incomplete: &'a HashMap<String, BTreeSet<E>>,
    /// Per fn, the TRANSITIVE reason-class tokens — the model's `D` (§6.2 ⟨0.19⟩). The Unknown EFFECT
    /// propagates along the call graph, so its REASON must too: else `deny E Unknown[reflect]` at a
    /// caller inheriting Unknown from a reflect-caused callee sees no class and does NOT fire.
    pub reason_classes: &'a HashMap<String, BTreeSet<String>>,
    /// Per `Net`-bearing fn, its ⟨0.20⟩ destination classes, ALREADY derived — by [`net_classes_of`] on
    /// the scan route, read verbatim from the report's `netClass` on the report route. Absent ⇒ empty.
    pub net_classes: &'a HashMap<String, Vec<String>>,
}

/// Apply a parsed §6.2 policy to an already-accumulated signature. THE ONLY matching code in the stable
/// toolchain — `candor-scan --policy` and `candor-query gate --report` both land here, which is what
/// makes "the same verdict from the same signature" a property of the code rather than of two
/// consistent authors. Returns the violations, sorted by (rule, detail).
pub fn gate<E: AsRef<str> + Ord>(p: &ParsedPolicy, gi: &GateInput<E>) -> Vec<GateViolation> {
    let empty: BTreeSet<E> = BTreeSet::new();
    let no_classes: Vec<String> = Vec::new();
    let mut out = Vec::new();
    // ONE VERDICT PER (rule, function), whatever the caller's enumeration. `all` is a list of UNITS on
    // the scan route, and two units can share one qualified name — `#[cfg(unix)] fn f` beside
    // `#[cfg(not(unix))] fn f` is the everyday case. Their signatures were already merged into one
    // `inferred` entry keyed by that name, so the gate saw ONE signature and reported it TWICE: two
    // byte-identical `GateViolation` records, an inflated `N policy violation(s)` count, and a
    // `--gate-json` document that could not be equal to the one the ⟨0.24⟩ report route produces (a
    // report is keyed by name, so the duplicate is not reachable there). FOUND BY the §3.1 byte-equality
    // obligation, on 15 of 90 rows over ebman/pgman/the candor workspace — which is the whole argument
    // for the verb: no end-to-end test could have separated this from a classifier defect.
    let mut seen_fn: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for q in gi.all {
        if !seen_fn.insert(q.as_str()) {
            continue;
        }
        let inf = gi.inferred.get(q).unwrap_or(&empty);
        // AS-EFF-006 — deny/pure: forbidden effects in the transitive set.
        for r in &p.rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            let mut hits: Vec<&str> = if r.effects.is_empty() {
                // `pure` — every EFFECT, but NOT `Unknown`: the §4 trust marker is not an effect
                // (AS-EFF-003's concern; `deny Unknown <scope>` is the explicit knob). The reference
                // engine and the deep backend exclude it identically — this engine wrongly counted an
                // Unknown-only fn as a `pure` violation until 2026-07-09 (a cross-engine verdict split
                // on the same policy file).
                inf.iter().map(AsRef::as_ref).filter(|e| *e != "Unknown").collect()
            } else {
                inf.iter().map(AsRef::as_ref).filter(|e| r.effects.contains(e)).collect()
            };
            // Reason-scoped Unknown: a `deny E Unknown[classes]` (non-empty filter) keeps its Unknown hit
            // ONLY for a fn whose TRANSITIVE reason classes include one of those classes; else tolerate it
            // (wrong reason-class). Concrete effects in `hits` are untouched — only Unknown is scoped.
            if hits.contains(&"Unknown") && !r.unknown_classes.is_empty() {
                let want: BTreeSet<&str> = r.unknown_classes.iter().map(|c| c.token()).collect();
                // An Unknown with NO recorded reason is `unresolved` (conservative — stays in
                // `[*]`/`[unresolved]`). THIS IS A NET, NOT A ROUTE. It is per FUNCTION and keys on the
                // ABSENCE of a class set, so any other reason on the same function hides whatever it was
                // covering — which is how a reasonless chained-dep `Unknown` went ungated on every consumer
                // that also had a reason of its own. That case now CONTRIBUTES `unresolved` where the
                // signature is BUILT (candor-scan's `reason_class_direct`; `gate_input_from_report` on the
                // report route) instead of arriving here by absence. What is left for the absence arm is
                // the RELEASE-mode gap: the writer's §4 invariant is a `debug_assert`, so a future path
                // that puts `Unknown` into `direct` with no reason fails closed here rather than escaping
                // the gate. Not dead — it is pinned by
                // `reason_scoped_unknown_gate_fires_on_match_tolerates_mismatch`.
                //
                // ⟨0.24⟩ The rule itself lives in `crate::policy::reason_class_matches` because
                // `unverified --class` must select over exactly the set this gate scopes over: a gate and
                // the disclosure naming the holes that gate did not prove, disagreeing, is the defect.
                let matched = reason_class_matches(gi.reason_classes.get(q), &want);
                if !matched {
                    hits.retain(|e| *e != "Unknown");
                }
            }
            // Net destination-class: a `deny Net[dest…]` (non-empty filter) keeps its Net hit ONLY for a fn
            // reaching one of those destination classes; else tolerate (only asserted-safe destinations).
            // Fail-closed: a masked surface / a Net with no visible host is unknown-host (net_classes_of).
            if hits.contains(&"Net") && !r.net_classes.is_empty() {
                let fn_net = gi.net_classes.get(q).unwrap_or(&no_classes);
                let matched = fn_net.iter().any(|c| r.net_classes.contains(c));
                if !matched {
                    hits.retain(|e| *e != "Net");
                }
            }
            if !hits.is_empty() {
                // §6.2: when Unknown is denied, report ALL reason classes on the fn (transitive), so the
                // consumer sees every reason the strict gate bit — not just the class the rule matched.
                let reason_class = if hits.contains(&"Unknown") {
                    gi.reason_classes.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default()
                } else {
                    Vec::new()
                };
                // ⟨0.20⟩ when Net is denied, report ALL of the fn's destination classes (transitive).
                let net_class = if hits.contains(&"Net") {
                    gi.net_classes.get(q).cloned().unwrap_or_default()
                } else {
                    Vec::new()
                };
                out.push(GateViolation {
                    rule: "AS-EFF-006".into(),
                    func: q.clone(),
                    effects: hits.iter().map(|s| s.to_string()).collect(),
                    detail: format!("`{q}` performs {{ {} }}, forbidden by policy: `{}`", hits.join(", "), r.raw),
                    reason_class,
                    net_class,
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
            if !inf.iter().any(|e| e.as_ref() == r.effect) {
                continue;
            }
            let lits = match r.effect {
                // `Llm` ⟨0.13⟩ rides the Net host surface (SPEC §1) — `allow Llm <host>` certifies the same
                // captured hosts as `allow Net`, restricted to the MODEL hosts (a model call's host WAS
                // captured as a Net literal). Matches candor-java's checkAllowlist("Llm", hostFixpoint, …).
                "Net" | "Llm" => gi.hosts.get(q),
                "Exec" => gi.cmds.get(q),
                "Db" => gi.tables.get(q),
                _ => gi.paths.get(q),
            };
            // An INCOMPLETE surface (a structurally-invisible reach) can't be certified even with visible
            // hosts — else a benign literal masks the invisible forbidden endpoint (the masking evasion).
            // `Llm` keys off the NET incompleteness (it rides the Net host literal): a runtime/masked model
            // host that fails-closes Net must fail-close `allow Llm` too (incompleteAsLlm in candor-java).
            let inc_key = if r.effect == "Llm" { "Net" } else { r.effect };
            let surface_incomplete =
                gi.surface_incomplete.get(q).is_some_and(|s| s.iter().any(|e| e.as_ref() == inc_key));
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
                            ..Default::default()
                        });
                    }
                }
                _ => out.push(GateViolation {
                    rule: "AS-EFF-008".into(),
                    func: q.clone(),
                    effects: vec![r.effect.to_string()],
                    detail: format!("`{q}` performs {} with no visible literal — the surface cannot be certified: `{}`", r.effect, r.raw),
                    ..Default::default()
                }),
            }
        }
        // AS-EFF-009 — layering: no fn in scope A may transitively reach scope B.
        for r in &p.layer_rules {
            if !scope_matches(q, &r.from) {
                continue;
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<&str> =
                gi.calls.get(q).map(|cs| cs.iter().map(String::as_str).collect()).unwrap_or_default();
            let mut hit: Option<&str> = None;
            while let Some(n) = stack.pop() {
                if !seen.insert(n) {
                    continue;
                }
                if scope_matches(n, &r.to) {
                    hit = Some(n);
                    break;
                }
                if let Some(cs) = gi.calls.get(n) {
                    stack.extend(cs.iter().map(String::as_str));
                }
            }
            if let Some(h) = hit {
                out.push(GateViolation {
                    rule: "AS-EFF-009".into(),
                    func: q.clone(),
                    effects: Vec::new(), // a layer-flow has no single effect
                    detail: format!("`{q}` reaches into a forbidden layer (via `{h}`): `{}`", r.raw),
                    ..Default::default()
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
