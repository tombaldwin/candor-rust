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

use crate::policy::{literal_allowed, reason_class_matches, scope_matches, scope_matches_permitted,
                    ParsedPolicy, PolicyRule};
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

/// ⟨0.24⟩ ONE `(rule, function)` THE GATE COULD NOT EVALUATE — SPEC §3.1: *"a rule FIRES on a function
/// only where the match is evidenced by that function's own entry, and is WITHHELD exactly where it is
/// not. Withholding is per `(rule, function)`, never whole-policy."*
///
/// A withheld pair is NOT a tolerated one. Tolerating means the evidence was read and did not match;
/// withholding means there was no evidence to read, and the two must not arrive at a consumer wearing the
/// same face. The caller decides the disposition — a violation elsewhere dominates (exit 1, disclose), a
/// sole withholding is a refusal (exit 2) — but it can only do that if the fact reaches it, which is why
/// this rides out of [`gate`] beside the violations instead of being logged here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withheld {
    /// The rule's source line, verbatim (`PolicyRule::raw`).
    pub rule: String,
    /// The function the rule could not be evaluated ON. The same rule may fire on another.
    pub func: String,
    /// Which narrowing filter had nothing to read — `"Unknown"` or `"Net"`.
    pub filter: &'static str,
}

/// ⟨0.24⟩ What [`gate`] returns: the violations it is SURE of, and the `(rule, function)` pairs it
/// WITHHELD. Both halves travel, because the verdict is both (SPEC §3.1).
#[derive(Debug, Default)]
pub struct GateOutcome {
    /// Sorted by (rule, detail).
    pub violations: Vec<GateViolation>,
    /// Sorted by (rule, func). Empty on every policy whose filters the signature can answer.
    pub withheld: Vec<Withheld>,
    /// ⟨0.27⟩ SPEC §4 — the RAW TEXT of every rule whose SCOPE bound no function, sorted. A rule that
    /// bound nothing was evaluated and matched nothing, so it cannot have caught anything; scoring it as
    /// satisfied makes a one-character typo in a layer name a permanently green gate. This is a
    /// DISCLOSURE beside the verdict, never a new verdict: the caller prints it and MUST NOT let it
    /// change the exit code (a zero-match rule is legitimate when one policy is shared across repos).
    pub zero_match: Vec<String>,
}

/// ⟨0.24⟩ What one §6.2 `deny`/`pure` rule DOES to one function's signature — see [`rule_hits`].
pub struct RuleHits<'a> {
    /// The effects this rule CHARGES on this function, after both narrowing filters. Empty ⇒ the rule
    /// does not fire here, which is what the disclosure calls PASSING.
    pub hits: Vec<&'a str>,
    /// The filters — `"Unknown"` / `"Net"` — that had no evidence to read, in that order. The hit was
    /// dropped from `hits` AND the fact rides out, because dropping it silently is the mirror defect.
    pub withheld: Vec<&'static str>,
}

/// ⟨0.24⟩ WHAT ONE §6.2 `deny`/`pure` RULE DOES TO ONE FUNCTION'S SIGNATURE — the firing decision,
/// extracted so it has exactly one implementation.
///
/// **WHY IT IS A FUNCTION NOW.** It was inline in [`gate`], and the provable-purity disclosure
/// ([`crate::policy::unverified_hole_rule`]) carried its own second copy that asked a coarser question:
/// "does the rule NAME an effect this function has?", computed from `r.effects` alone. That copy could
/// not see a narrowing filter, so it disagreed with the gate on exactly the rules the ⟨0.24⟩ rung added
/// — and the disagreement ran the LOSING way. A hole is a function that PASSES its rule while `Unknown`,
/// so a rule the gate TOLERATES (`deny Unknown[reflect]` over an `indirect` hole) was read by the
/// disclosure as a violation-that-isn't and dropped from the output: `unverified` printed **"every
/// function in a pure/deny layer is PROVABLY clean ✓"** over a function the gate had just declined to
/// clear. MEASURED 2026-07-28 on a one-function crate; reachable with NO alias in play, one layer below
/// the alias-widening defect `ea0df4f` closed.
///
/// The caller does the SCOPE test and owns the disposition of `withheld`; this answers only "given that
/// this rule governs this function, what does it charge, and what could it not evaluate?".
///
/// `reason_classes` is the ACCUMULATED (post-fixpoint) class set — `None`/empty means the signature
/// carries none, which is NOT determinable and is withheld, never floored. `net_classes` is the fn's
/// ⟨0.20⟩ destination classes, likewise already derived.
pub fn rule_hits<'a>(
    r: &PolicyRule,
    effects: &[&'a str],
    reason_classes: Option<&BTreeSet<String>>,
    net_classes: &[String],
) -> RuleHits<'a> {
    let mut withheld: Vec<&'static str> = Vec::new();
    let mut hits: Vec<&str> = if r.effects.is_empty() {
        // `pure` — every EFFECT, but NOT `Unknown`: the §4 trust marker is not an effect
        // (AS-EFF-003's concern; `deny Unknown <scope>` is the explicit knob). The reference
        // engine and the deep backend exclude it identically — this engine wrongly counted an
        // Unknown-only fn as a `pure` violation until 2026-07-09 (a cross-engine verdict split
        // on the same policy file).
        effects.iter().copied().filter(|e| *e != "Unknown").collect()
    } else {
        effects.iter().copied().filter(|e| r.effects.contains(e)).collect()
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
        //
        // ⟨0.24⟩ **BUT THE FLOOR IS ASKED FIRST, AND SEPARATELY — SPEC §3.1.** `reason_class_matches`
        // answers "could this rule apply?", and its absence/empty arm floors at `unresolved` so a
        // hole nobody classified never slips out of a filter that names its own class. That is the
        // right fail-closed default for a MATCHER and the WRONG basis for a FIRING: read as grounds
        // to emit a violation it asserts a reason NOBODY RECORDED. The two questions shared this one
        // helper safely only while the report route's refusal short-circuited before `gate()` ran;
        // `8b97e5c` removed that short-circuit (correctly — a certain violation must reach the
        // document) and the identical constant, on identical data, became a FABRICATION.
        //
        // MEASURED 2026-07-28, `deny Unknown[unresolved] app.opaque` over an entry with `inferred:
        // ["Unknown"]` and no `direct`, no `unknownWhy`, no `calls`: exit 1 with a violation record
        // in `--gate-json`, for a function whose determinable class set is EMPTY. The record was
        // self-refuting — it carried no `reasonClass` key at all, because the floor exists only
        // inside the predicate and never in the data.
        //
        // So the three-way split. NOT determinable ⇒ **WITHHELD**: the hit is dropped AND the pair
        // rides out to the caller, because dropping it silently is the mirror defect (a narrowed
        // filter tolerating for lack of evidence is the fail-open this whole rung exists to close).
        // Determinable ⇒ the shared matcher decides, unchanged, and the `Some(cs)` arm it lands on
        // is the only one a firing may rest on.
        //
        // THE MIRROR IS PINNED, because this is where an under-report gets introduced: an entry
        // whose `unresolved` is INHERITED — a `calls` edge to a reasonless direct `Unknown` — has a
        // determinable set of `{unresolved}` (contributed at the ENTRY, before the fixpoint) and
        // MUST still fire. That is `R1_EXPECT["unresolved"]`'s `app.a_reasonless_only`, and
        // `a_withheld_unknown_filter_does_not_take_the_inherited_one_with_it` beside it.
        let classes = reason_classes;
        let determinable = classes.is_some_and(|cs| !cs.is_empty());
        if !determinable {
            hits.retain(|e| *e != "Unknown");
            withheld.push("Unknown");
        } else if !reason_class_matches(classes, &want) {
            hits.retain(|e| *e != "Unknown");
        }
    }
    // Net destination-class: a `deny Net[dest…]` (non-empty filter) keeps its Net hit ONLY for a fn
    // reaching one of those destination classes; else tolerate (only asserted-safe destinations).
    // Fail-closed: a masked surface / a Net with no visible host is unknown-host (net_classes_of).
    //
    // ⟨0.24⟩ SAME THREE-WAY SPLIT AS THE REASON FILTER, and this side is where the shape is easiest
    // to see because it never fabricated: with no destination classes to read, `any()` over the
    // empty set is false and the Net hit was DROPPED — the *other* half of the same defect, an
    // absence-keyed relaxation of a fail-closed gate. Silently tolerating and silently charging are
    // the two ways to answer a question the evidence cannot settle; WITHHOLDING is the third, and
    // the only one that stays true. Costs nothing on a signature this engine produced:
    // `net_classes_of` floors every Net-bearing fn at `unknown-host`, so an empty set here means
    // "this producer did not carry the field", never "this function reaches nothing".
    if hits.contains(&"Net") && !r.net_classes.is_empty() {
        let fn_net = net_classes;
        if fn_net.is_empty() {
            hits.retain(|e| *e != "Net");
            withheld.push("Net");
        } else if !fn_net.iter().any(|c| r.net_classes.contains(c)) {
            hits.retain(|e| *e != "Net");
        }
    }
    RuleHits { hits, withheld }
}


/// Apply a parsed §6.2 policy to an already-accumulated signature. THE ONLY matching code in the stable
/// toolchain — `candor-scan --policy` and `candor-query gate --report` both land here, which is what
/// makes "the same verdict from the same signature" a property of the code rather than of two
/// consistent authors. Returns the violations, sorted by (rule, detail), AND the withheld pairs.
pub fn gate<E: AsRef<str> + Ord>(p: &ParsedPolicy, gi: &GateInput<E>) -> GateOutcome {
    let empty: BTreeSet<E> = BTreeSet::new();
    let no_classes: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut withheld: Vec<Withheld> = Vec::new();
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
        // Materialized ONCE per function rather than per (function, rule): `rule_hits` is generic over
        // nothing, so the two routes' effect representations (`&'static str` interned / `String` off the
        // wire) converge here instead of inside the matcher.
        let effs: Vec<&str> = inf.iter().map(AsRef::as_ref).collect();
        // AS-EFF-006 — deny/pure: forbidden effects in the transitive set.
        for r in &p.rules {
            if let Some(s) = &r.scope {
                if !scope_matches(q, s) {
                    continue;
                }
            }
            // ONE implementation of the firing decision, shared with the provable-purity disclosure
            // ([`crate::policy::unverified_hole_rule`]) — see [`rule_hits`] for what the second copy cost.
            let RuleHits { hits, withheld: wh } = rule_hits(
                r,
                &effs,
                gi.reason_classes.get(q),
                gi.net_classes.get(q).map(Vec::as_slice).unwrap_or(&no_classes),
            );
            for filter in wh {
                withheld.push(Withheld { rule: r.raw.clone(), func: q.clone(), filter });
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
        // ⟨0.29⟩ AS-EFF-009 — `only A -> B …`: a fn in A may reach A and the listed scopes, NOTHING else.
        //
        // The same walk as `forbid` above with the test INVERTED, and the inversion is the point rather
        // than the code. `forbid` fails OPEN — what you did not prohibit is permitted — so a leaf package
        // can only be protected by enumerating what it must not reach, and that list does not cover a
        // package added tomorrow. `only` fails SAFE: the dependency you forgot to permit is a violation,
        // loudly, on the day it appears.
        //
        // THE WALK STOPS AT A PERMITTED SCOPE. A permitted callee's own dependencies are governed by the
        // rules about IT; descending past it would make `only` demand the transitive closure of everything
        // you permit, which is the same enumeration-that-rots one level down. `from` IS descended through
        // — a fn in A calling another fn in A that reaches infra is still A reaching infra.
        for r in &p.only_rules {
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
                // ⟨0.29⟩ EXACT segment match on a PERMITTED scope — see `scope_matches_permitted`. The
                // shared prefix matcher is fail-CLOSED for every other rule kind and fail-OPEN here.
                if r.to.iter().any(|t| scope_matches_permitted(n, t)) {
                    continue; // permitted, and its own callees are not this rule's business
                }
                if !scope_matches(n, &r.from) {
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
                    effects: Vec::new(),
                    detail: format!(
                        "`{q}` reaches `{h}`, which this permission rule does not permit: `{}`",
                        r.raw
                    ),
                    ..Default::default()
                });
            }
        }
    }
    // Sort by (rule, detail) — identical order to the old rendered-line sort (the "[rule] detail" render
    // puts the constant '[' first and all AS-EFF codes are same-length), without allocating two Strings
    // per comparison.
    out.sort_by(|a, b| (a.rule.as_str(), a.detail.as_str()).cmp(&(b.rule.as_str(), b.detail.as_str())));
    // Deterministic for the same reason the violations are: a disclosure a consumer diffs between runs
    // must not reorder because a HashMap iterated differently.
    withheld.sort_by(|a, b| (&a.rule, &a.func).cmp(&(&b.rule, &b.func)));
    withheld.dedup();
    // ⟨0.27⟩ ZERO-MATCH DISCLOSURE. Counted over the SAME key set the gate iterated, so "bound nothing"
    // means here exactly what it means to the gate. A `deny`/`pure` with NO scope applies to every
    // function and so can never be this kind of typo — excluded. A layer rule counts a match on either
    // endpoint, over the call-graph keys it binds across.
    let mut zero: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for r in &p.rules {
        if r.scope.is_some() {
            zero.entry(r.raw.as_str()).or_insert(0);
        }
    }
    for r in &p.layer_rules {
        zero.entry(r.raw.as_str()).or_insert(0);
    }
    // ⟨0.29⟩ an `only` rule binds nothing when NEITHER endpoint names anything in this tree — the same
    // typo channel `forbid` has, and the more dangerous one to leave silent: a `forbid` that binds
    // nothing merely fails to prohibit, while an `only` that binds nothing withholds a promise the
    // operator believes they made.
    for r in &p.only_rules {
        zero.entry(r.raw.as_str()).or_insert(0);
    }
    if !zero.is_empty() {
        let mut names: std::collections::BTreeSet<&str> =
            gi.all.iter().map(|q| q.as_str()).collect();
        names.extend(gi.calls.keys().map(String::as_str));
        for n in names {
            for r in &p.rules {
                if let Some(s) = &r.scope {
                    if scope_matches(n, s) {
                        *zero.entry(r.raw.as_str()).or_insert(0) += 1;
                    }
                }
            }
            for r in &p.layer_rules {
                if scope_matches(n, &r.from) || scope_matches(n, &r.to) {
                    *zero.entry(r.raw.as_str()).or_insert(0) += 1;
                }
            }
            // ON `from` ONLY, and deliberately NOT on either endpoint the way a `forbid` counts. A
            // `forbid`'s subject is the pair; an `only`'s subject is `from` — it is a promise ABOUT that
            // scope — so a rule whose destinations all exist while its `from` names nothing has bound
            // nothing at all, and is exactly the typo that leaves an operator believing a leaf is
            // protected. Counting the destinations would hide it behind a scope that happens to resolve.
            for r in &p.only_rules {
                if scope_matches(n, &r.from) {
                    *zero.entry(r.raw.as_str()).or_insert(0) += 1;
                }
            }
        }
    }
    let zero_match: Vec<String> = zero
        .into_iter()
        .filter(|(_, c)| *c == 0)
        .map(|(raw, _)| raw.to_string())
        .collect();
    GateOutcome { violations: out, withheld, zero_match }
}
