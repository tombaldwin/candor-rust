//! `candor unverified` — the provable-purity disclosure (a policy-guidance companion to `fix`, from
//! eval/fixloop/DISPATCH-NOTE.md). A `deny <E>` or `pure` rule PASSES a function that carries none of its
//! forbidden effects — but if that function is `Unknown` (candor could not resolve one of its calls), the
//! pass is UNVERIFIED: the Unknown could hide the very effect the rule forbids (the classic case is a
//! fn/closure-injected "port" — the layer reads as Unknown, so `deny Net domain`/`pure domain` clear it even
//! though the domain may reach Net at runtime). This names every such function in a governed layer and the
//! `deny <E> Unknown <scope>` upgrade that makes the intent provable. Advisory: exit 0, or `--strict` → exit
//! 1 so CI can REQUIRE provable purity. The gate's verdict is untouched — this only discloses the gap.

use crate::grammar::{parse, report_or_discover, Shape};
use crate::load::load_entries;
use candor_classify::policy::{rule_and_upgrade, unverified_hole_rule, PolicyRule};
use candor_report::ReportEntry;
use std::collections::{BTreeSet, HashMap};

/// The per-function TRANSITIVE reason-class set, rebuilt from a report — candor-scan's gate-side
/// `reason_class_acc` (scan.rs), recomputed on this side of the report boundary and over the same
/// `propagate_str` least fixpoint, so `unverified --class` selects over exactly the set a
/// `deny E Unknown[class]` gate scopes over.
///
/// TWO FAULTS LIVE HERE, and only fixing BOTH is a fix.
///
/// (1) `unknownWhy` is DIRECT-ONLY by design — §4: a reason names an unresolvable site in the
/// function's OWN body — so a function whose `Unknown` is purely INHERITED from a callee carries no
/// reason of its own. Matching a filter against that field reads a field answering a different
/// question, and the old predicate (`unknownWhy` ∩ filter ≠ ∅) therefore dropped every inherited hole
/// from every filter, INCLUDING one naming the class the callee recorded. Measured on this engine
/// before the fix: 6 of 7 `unverified` holes on candor-scan's own sources, 101 of 124 `Unknown`
/// entries on ebman and 37 of 60 on pgman, carry no direct reason at all. Hence the fixpoint.
///
/// (2) The empty set must FAIL CLOSED, not open. §6.2: a function whose `Unknown` carries no recorded
/// reason CONTRIBUTES `unresolved`. That is `reason_class_matches`'s absence arm — but that arm is a
/// NET keyed on the WHOLE set being empty, so any other reason on the same function swallows it. The
/// case that can co-occur with a reason is contributed HERE instead, per entry, into the DIRECT map so
/// it propagates to callers like any other class.
///
/// THE GATE ON (2) IS THE POINT, and getting it wrong is the mirror fabrication. It is `direct ∋
/// Unknown` with nothing named — the unit INTRODUCED the hole and did not say why — NOT "the reason set
/// is absent", which is also exactly what a correctly-classified INHERITED `Unknown` looks like.
/// Contributing `unresolved` to one of those would trade a fail-open for a fabricated class, and a fix
/// that trades one sin for its mirror is not a fix. rust's report carries `direct` (§2), so the §4
/// condition is checkable verbatim here rather than approximated.
fn reason_class_acc(entries: &[ReportEntry]) -> HashMap<String, BTreeSet<String>> {
    use candor_classify::policy::ReasonClass;
    let mut direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut all: Vec<String> = Vec::with_capacity(entries.len());
    for e in entries {
        all.push(e.func.clone());
        if !e.calls.is_empty() {
            calls.insert(e.func.clone(), e.calls.iter().cloned().collect());
        }
        let mut cs: BTreeSet<String> =
            e.unknown_why.iter().map(|w| ReasonClass::classify(w).token().to_string()).collect();
        if cs.is_empty() && e.direct.iter().any(|d| d == "Unknown") {
            cs.insert(ReasonClass::Unresolved.token().to_string());
        }
        if !cs.is_empty() {
            direct.insert(e.func.clone(), cs);
        }
    }
    candor_classify::propagate::propagate_str(&direct, &calls, &all)
}

pub(crate) fn cmd_unverified(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: true });
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let strict = g.strict;
    let policy_path = g.policy.clone().or_else(|| std::env::var("CANDOR_POLICY").ok());
    let Some(pp) = policy_path else {
        eprintln!("candor unverified: a policy is required (the check is relative to your pure/deny layers).");
        return 2;
    };
    // ⟨0.24⟩ Through the SHARED loader, and here the old bare `parse_policy` LOST A DISCLOSURE rather
    // than adding one: a hole is a function that PASSES its rule while being `Unknown`, so widening
    // `deny Unknown[<alias>]` to a bare `deny Unknown` reclassified real holes as violations-that-aren't
    // and this verb answered "every function in a pure/deny layer is PROVABLY clean ✓". §6.2: the gate
    // and the disclosure MUST apply the same rule.
    let rules = match crate::policy::load_policy_as_the_gate_does("unverified", &pp) {
        Ok(p) => p.rules,
        Err(code) => return code,
    };
    let entries = load_entries(prefix);
    if entries.is_empty() {
        eprintln!("candor unverified: no report for `{prefix}` — scan the crate first.");
        return 2;
    }

    // A hole: a function that is Unknown, sits in a deny/pure scope, and PASSES that rule (carries none of its
    // forbidden real effects). The predicate is `unverified_hole_rule` — the SAME one candor-scan's gate note
    // uses (candor_classify::policy), so the disclosure can never drift between the two paths.
    struct Hole<'a> {
        func: &'a ReportEntry,
        rule: &'a PolicyRule,
    }
    // `--class <c,…>` (SPEC §3.1 ⟨0.20⟩, semantics pinned normative at §6.2 ⟨0.24⟩): keep only holes
    // whose Unknown is of a matching reason class — resolved TRANSITIVELY, over the same reach the
    // `deny E Unknown[class]` gate resolves, and failing CLOSED on a hole nothing classified.
    // §6.2 ⟨0.24⟩: an unrecognised token is a USAGE ERROR (exit 2), never a silently narrowed filter —
    // see `parse_class_filter` for why this half of the rule is not the policy side's drop-with-warning.
    let class_filter = match g.class.as_deref().map(crate::containment::parse_class_filter).transpose() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    let want: Option<std::collections::BTreeSet<&str>> = class_filter
        .as_ref()
        .map(|set| set.iter().map(|c| c.token()).collect());
    // Computed once, and only when a filter was given (it is a fixpoint over the whole report).
    let reason_acc = want.as_ref().map(|_| reason_class_acc(&entries));
    let class_matches = |e: &ReportEntry| -> bool {
        match (&want, &reason_acc) {
            (Some(w), Some(acc)) => candor_classify::policy::reason_class_matches(acc.get(&e.func), w),
            _ => true, // no --class ⇒ no filter
        }
    };
    let holes: Vec<Hole> = entries
        .iter()
        .filter_map(|e| {
            unverified_hole_rule(&e.func, &e.inferred, &rules)
                .filter(|_| class_matches(e))
                .map(|rule| Hole { func: e, rule })
        })
        .collect();

    if want_json {
        let items: Vec<_> = holes
            .iter()
            .map(|h| {
                let (rule, upgrade) = rule_and_upgrade(h.rule);
                serde_json::json!({
                    "fn": h.func.func,
                    "rule": rule,
                    "unknownWhy": h.func.unknown_why,
                    "upgrade": upgrade,
                })
            })
            .collect();
        let out = serde_json::json!({ "ok": items.is_empty(), "unverified": items });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return if strict && !holes.is_empty() { 1 } else { 0 };
    }

    if holes.is_empty() {
        println!("candor unverified: every function in a pure/deny layer is PROVABLY clean (no Unknown holes) ✓");
        return 0;
    }
    println!(
        "candor unverified — {} function(s) PASS their policy but aren't PROVABLY clean:\n",
        holes.len()
    );
    let mut upgrades: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for h in &holes {
        let (rule, upgrade) = rule_and_upgrade(h.rule);
        upgrades.insert(upgrade.clone());
        println!("  `{}`  (in `{rule}`)", h.func.func);
        let why = if h.func.unknown_why.is_empty() {
            "an unresolvable call".to_string()
        } else {
            h.func.unknown_why.join(", ")
        };
        println!("     is Unknown ({why}) — candor can't confirm it's free of the forbidden effect(s);");
        println!("     the Unknown could hide the very effect the rule forbids (e.g. a fn/closure-injected port).");
        println!("     → make it provable:  add  `{upgrade}`");
        println!();
    }
    println!("  The gate still PASSES — this is advisory. To REQUIRE provable purity, add:");
    for u in &upgrades {
        println!("      {u}");
    }
    if strict {
        return 1;
    }
    0
}
