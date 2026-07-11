//! `candor fix` — the boundary fix (integrations/FIX-SPEC.md). When an edit makes a function perform an
//! effect its layer forbids, this computes the *architectural* remedy: where the effect should live (hoist
//! it to the nearest allowed-layer caller) and which functions become pure and thread the value. Read-only
//! over the same report + policy the gate uses; the inverse of `whatif`. Advisory structure, never syntax;
//! the gate re-scan remains the ground truth.

use crate::load::load_entries;
use crate::matching::{best_tier, q_match};
use candor_report::ReportEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

/// The deny/`pure` scope (the "layer") that forbids `effect` at `fname`, or None if performing `effect`
/// there is allowed. Mirrors `whatif`'s violation predicate exactly (SPEC §6): a `deny` fires when it names
/// the effect; a `pure` rule (empty effects) forbids every real effect but not `Unknown`.
fn denied_layer(fname: &str, effect: &str, rules: &[candor_classify::policy::PolicyRule]) -> Option<String> {
    for rule in rules {
        let denies = if rule.effects.is_empty() {
            effect != candor_classify::policy::UNKNOWN
        } else {
            rule.effects.contains(effect)
        };
        let in_scope = rule
            .scope
            .as_deref()
            .is_none_or(|s| candor_classify::policy::scope_matches(fname, s));
        if denies && in_scope {
            return Some(rule.scope.clone().unwrap_or_default());
        }
    }
    None
}

pub(crate) fn cmd_fix(args: &[String]) -> i32 {
    if args.len() < 3 {
        eprintln!("usage: candor-query fix <prefix> <fn> <Effect> [policy-file] [0|1]");
        return 2;
    }
    let (prefix, target, effect) = (&args[0], &args[1], args[2].as_str());
    if candor_classify::cap_from_name(effect).is_none() && effect != "Unknown" {
        eprintln!("candor: unknown effect `{effect}` (expected a candor effect name, e.g. Net/Fs/Db/Exec, or Unknown)");
        return 2;
    }
    let mut policy_path: Option<String> = None;
    let mut want_json = false;
    for a in &args[3..] {
        match a.as_str() {
            "0" => want_json = false,
            "1" | "--json" => want_json = true,
            other => policy_path = Some(other.to_string()),
        }
    }
    if policy_path.is_none() {
        policy_path = std::env::var("CANDOR_POLICY").ok();
    }
    // A fix is defined RELATIVE to the boundary it crosses — no policy, no boundary, nothing to fix.
    let Some(pp) = policy_path else {
        eprintln!("candor fix: a policy is required (pass a policy file or set CANDOR_POLICY) — the fix is the refactor that restores the boundary the edit crossed.");
        return 2;
    };
    let rules = match std::fs::read_to_string(&pp) {
        Ok(t) => candor_classify::policy::parse_policy(&t).rules,
        Err(e) => {
            eprintln!("candor: policy `{pp}` could not be read ({e}) — no fix computed.");
            return 2;
        }
    };

    let entries = load_entries(prefix);
    if entries.is_empty() {
        eprintln!("candor fix: no report for `{prefix}` — scan the crate first.");
        return 2;
    }
    let by_name: HashMap<&str, &ReportEntry> = entries.iter().map(|e| (e.func.as_str(), e)).collect();

    let tier = best_tier(entries.iter().map(|e| e.func.as_str()), target);
    let Some(start) = entries
        .iter()
        .find(|e| &e.func == target)
        .or_else(|| entries.iter().find(|e| q_match(&e.func, target, tier)))
    else {
        eprintln!("candor fix: no function matching `{target}`.");
        return 2;
    };

    if !start.inferred.iter().any(|e| e == effect) {
        println!("candor fix: `{}` does not perform {effect} — nothing to hoist.", start.func);
        return 0;
    }
    let Some(layer) = denied_layer(&start.func, effect, &rules) else {
        println!(
            "candor fix: `{}` performs {effect}, but no policy forbids it there — the boundary isn't crossed, nothing to fix.",
            start.func
        );
        return 0;
    };
    let layer_label = if layer.is_empty() { "this".to_string() } else { format!("`{layer}`") };

    // reverse adjacency: callee -> its direct callers (from the embedded call lists).
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &entries {
        for c in &e.calls {
            rev.entry(c.as_str()).or_default().push(e.func.as_str());
        }
    }
    // the affected set: `start` + every transitive caller — all gain `effect`.
    let mut affected: BTreeSet<&str> = BTreeSet::new();
    affected.insert(start.func.as_str());
    let mut st = vec![start.func.as_str()];
    while let Some(n) = st.pop() {
        if let Some(cs) = rev.get(n) {
            for &c in cs {
                if affected.insert(c) {
                    st.push(c);
                }
            }
        }
    }
    // the denied span D: affected functions in a deny-`effect` layer — these must become pure.
    let denied_span: BTreeSet<&str> =
        affected.iter().copied().filter(|f| denied_layer(f, effect, &rules).is_some()).collect();

    // the direct site(s) S: BFS from `start` through effect-carrying callees to the DIRECT source(s).
    let mut sites: BTreeSet<&str> = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    q.push_back(start.func.as_str());
    seen.insert(start.func.as_str());
    while let Some(cur) = q.pop_front() {
        let Some(f) = by_name.get(cur) else { continue };
        if f.direct.iter().any(|e| e == effect) {
            sites.insert(cur);
        }
        for c in &f.calls {
            if let Some(cf) = by_name.get(c.as_str())
                && cf.inferred.iter().any(|e| e == effect)
                && seen.insert(c.as_str())
            {
                q.push_back(c.as_str());
            }
        }
    }

    // the hoist frontier G: allowed-layer functions that call INTO the denied span (the boundary edges).
    let mut hoist: BTreeSet<&str> = BTreeSet::new();
    for &d in &denied_span {
        if let Some(cs) = rev.get(d) {
            for &c in cs {
                if denied_layer(c, effect, &rules).is_none() {
                    hoist.insert(c);
                }
            }
        }
    }

    let allow_edit = if layer.is_empty() {
        format!("allow {effect}")
    } else {
        format!("allow {effect} {layer}")
    };

    if want_json {
        let out = serde_json::json!({
            "fn": start.func,
            "effect": effect,
            "layer": layer,
            "cleanHoist": !hoist.is_empty(),
            "site": sites.iter().collect::<Vec<_>>(),
            "deniedSpan": denied_span.iter().collect::<Vec<_>>(),
            "hoistTo": hoist.iter().collect::<Vec<_>>(),
            "policyAlternative": allow_edit,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }

    let sitelist = |s: &BTreeSet<&str>| -> String {
        if s.is_empty() { "(not a local source — a cross-crate or Unknown effect)".into() }
        else { s.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ") }
    };

    println!("candor fix — hoist {effect} out of the {layer_label} boundary\n");
    println!("  The violation: `{}` performs {effect}, which the {layer_label} layer forbids.", start.func);
    println!("  Performed directly at: {}", sitelist(&sites));
    println!(
        "  Forbidden across {} function(s) in the layer (they inherited it): {}",
        denied_span.len(),
        denied_span.iter().take(6).map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")
            + if denied_span.len() > 6 { ", …" } else { "" }
    );
    println!();

    if !hoist.is_empty() {
        println!("  THE FIX — hoist the effect to the boundary:");
        println!(
            "    · Perform {effect} at: {}  (an allowed layer that already calls into the domain).",
            hoist.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")
        );
        println!("    · Pass the result down as a parameter; the {} function(s) above then stay pure.", denied_span.len());
        println!("    · Re-run the gate — the {layer_label} blast radius for {effect} should be empty.");
        println!();
        println!("  ALTERNATIVE — if the {layer_label} layer is MEANT to perform {effect}, it's a policy bug,");
        println!("  not a code one: relax the boundary with  `{allow_edit}`.");
    } else {
        println!("  NO CLEAN HOIST — every caller up to the entry points is also in a {effect}-forbidding layer.");
        println!("  Two honest options:");
        println!("    (a) Introduce a PORT: have the domain take an interface parameter (a trait) it receives,");
        println!("        implemented by an adapter in an allowed layer that performs {effect} and injects the");
        println!("        result (dependency inversion) — the domain depends on the abstraction, not the I/O.");
        println!("    (b) If the domain legitimately needs {effect}, relax the boundary:  `{allow_edit}`.");
    }
    println!("\n  (Advisory: candor names the shape, you write the code; the gate re-scan verifies the fix.)");
    0
}
