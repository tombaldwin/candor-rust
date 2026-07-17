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
use candor_classify::policy::{parse_policy, rule_and_upgrade, unverified_hole_rule, PolicyRule};
use candor_report::ReportEntry;

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
    let rules = match std::fs::read_to_string(&pp) {
        Ok(t) => parse_policy(&t).rules,
        Err(e) => {
            eprintln!("candor: policy `{pp}` could not be read ({e}) — nothing checked.");
            return 2;
        }
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
    // `--class <c,…>` (SPEC §3.1 ⟨0.20⟩): keep only holes whose Unknown is of a matching reason class.
    let class_filter = g.class.as_deref().map(crate::containment::parse_class_filter);
    let class_matches = |e: &ReportEntry| -> bool {
        use candor_classify::policy::ReasonClass;
        match &class_filter {
            None => true,
            Some(set) => e.unknown_why.iter().any(|w| set.contains(&ReasonClass::classify(w))),
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
