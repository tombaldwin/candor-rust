//! `candor fix` / `candor fix-gate` — the boundary fix (integrations/FIX-SPEC.md). When an edit makes a
//! function perform an effect its layer forbids, this computes the *architectural* remedy: where the effect
//! should live (hoist it to the nearest allowed-layer caller) and which functions become pure and thread the
//! value. Read-only over the same report + policy the gate uses; the inverse of `whatif`. `fix` answers for
//! one function; `fix-gate` computes a remedy for every deny/`pure` (AS-EFF-006) violation in a report, so
//! the edit-time loop can hand the agent the *fix*, not just the finding. Advisory structure, never syntax;
//! the gate re-scan remains the ground truth.

use crate::grammar::{parse, report_or_discover, Shape};
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

/// The computed remedy for one `(function, effect)` boundary crossing — the deterministic cut between
/// "must stay pure" (`denied_span`) and "may perform the effect" (`hoist_to`). Borrows the report.
pub(crate) struct RemedyPlan<'a> {
    func: &'a str,
    effect: &'a str,
    layer: String,
    sites: BTreeSet<&'a str>,
    denied_span: BTreeSet<&'a str>,
    hoist_to: BTreeSet<&'a str>,
    hoist_higher: BTreeSet<&'a str>,
    clean_hoist: bool,
    allow_edit: String,
}

impl RemedyPlan<'_> {
    fn clean_hoist(&self) -> bool {
        self.clean_hoist
    }
    /// A hoist frontier exists, but it's SANDWICHED — a forbidden fn calls into it, so hoisting there would
    /// leave that caller violating. Distinguishes the two no-clean-hoist shapes in the message.
    fn sandwiched(&self) -> bool {
        !self.clean_hoist && !self.hoist_to.is_empty()
    }
    /// A stable key so `fix-gate` collapses the many inheritors of one root cause (every function in the
    /// denied span carries the effect) to a single remedy: the plan is fixed by its effect, layer, site, and
    /// hoist target — not by which inheriting function tripped the gate.
    fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{:?}|{:?}",
            self.effect,
            self.layer,
            self.sites.iter().collect::<Vec<_>>(),
            self.hoist_to.iter().collect::<Vec<_>>()
        )
    }
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "fn": self.func,
            "effect": self.effect,
            "layer": self.layer,
            "cleanHoist": self.clean_hoist(),
            "site": self.sites.iter().collect::<Vec<_>>(),
            "deniedSpan": self.denied_span.iter().collect::<Vec<_>>(),
            "hoistTo": self.hoist_to.iter().collect::<Vec<_>>(),
            "hoistHigher": self.hoist_higher.iter().collect::<Vec<_>>(),
            "policyAlternative": self.allow_edit,
        })
    }
    fn render_text(&self, out: &mut String) {
        use std::fmt::Write;
        // An empty scope is a GLOBAL rule (`deny Exec` with no layer — forbid it program-wide); render a
        // real word, not the literal "this", which read like an unsubstituted template variable (#12a).
        let layer_label = if self.layer.is_empty() { "global".to_string() } else { format!("`{}`", self.layer) };
        let sitelist = if self.sites.is_empty() {
            "(not a local source — a cross-crate or Unknown effect)".to_string()
        } else {
            self.sites.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")
        };
        let _ = writeln!(out, "candor fix — hoist {} out of the {layer_label} boundary\n", self.effect);
        let _ = writeln!(out, "  The violation: `{}` performs {}, which the {layer_label} layer forbids.", self.func, self.effect);
        let _ = writeln!(out, "  Performed directly at: {sitelist}");
        let span: Vec<_> = self.denied_span.iter().take(6).map(|x| format!("`{x}`")).collect();
        let more = if self.denied_span.len() > 6 { ", …" } else { "" };
        let _ = writeln!(out, "  Forbidden across {} function(s) in the layer (they inherited it): {}{more}", self.denied_span.len(), span.join(", "));
        let _ = writeln!(out);
        if self.clean_hoist() {
            let _ = writeln!(out, "  THE FIX — hoist the effect to the boundary:");
            let _ = writeln!(out, "    · Perform {} at: {}  (an allowed layer that already calls into the domain).", self.effect,
                self.hoist_to.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", "));
            let _ = writeln!(out, "    · Pass the result down as a parameter; the {} function(s) above then stay pure.", self.denied_span.len());
            let _ = writeln!(out, "    · Re-run the gate — the {layer_label} blast radius for {} should be empty.", self.effect);
            if !self.hoist_higher.is_empty() {
                let tops: Vec<_> = self.hoist_higher.iter().take(4).map(|x| format!("`{x}`")).collect();
                let more = if self.hoist_higher.len() > 4 { ", …" } else { "" };
                let _ = writeln!(out, "    · TRADE-OFF — or hoist higher (up to {}{more}): the effect then originates further up,", tops.join(", "));
                let _ = writeln!(out, "      keeping the {} intervening allowed-layer function(s) pure too, at the cost of threading it through more signatures.", self.hoist_higher.len());
            }
            let _ = writeln!(out);
            let _ = writeln!(out, "  ALTERNATIVE — if the {layer_label} layer is MEANT to perform {}, it's a policy bug,", self.effect);
            let _ = writeln!(out, "  not a code one: relax the boundary with  `{}`.", self.allow_edit);
        } else {
            if self.sandwiched() {
                let _ = writeln!(out, "  NO CLEAN HOIST — the nearest allowed layer ({}) is itself CALLED BY a {}-forbidding layer,",
                    self.hoist_to.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", "), self.effect);
                let _ = writeln!(out, "  so hoisting {} there would leave that caller violating (a forbidden layer sandwiching an allowed one).", self.effect);
            } else {
                let _ = writeln!(out, "  NO CLEAN HOIST — every caller up to the entry points is also in a {}-forbidding layer.", self.effect);
            }
            let _ = writeln!(out, "  Three ways to fix it:");
            let _ = writeln!(out, "    (a) HOIST TO A NEW ENTRY POINT (recommended) — add a thin function ABOVE the {layer_label} layer that");
            let _ = writeln!(out, "        performs {} and passes the result DOWN as plain DATA; the {layer_label} functions take it as a", self.effect);
            let _ = writeln!(out, "        parameter and become PROVABLY pure (candor verifies no effect — clean under any policy). candor");
            let _ = writeln!(out, "        says \"no clean hoist\" only because no allowed caller EXISTS yet — you can add one; simplest fix.");
            let _ = writeln!(out, "    (b) INJECT via a fn/closure — give the {layer_label} layer a FUNCTION/CLOSURE parameter, supplied by an");
            let _ = writeln!(out, "        allowed adapter. This clears `deny {}`, but candor can't see THROUGH the injected function, so it", self.effect);
            let _ = writeln!(out, "        reads the {layer_label} as Unknown — a hole a `deny {} Unknown` policy would still flag; prefer (a) for", self.effect);
            let _ = writeln!(out, "        provable purity. Do NOT use a trait/interface port: candor resolves the dispatch back to its");
            let _ = writeln!(out, "        {}-performing impl, so the {layer_label} still trips the gate.", self.effect);
            let _ = writeln!(out, "    (c) If the {layer_label} layer legitimately needs {}, relax the boundary:  `{}`.", self.effect, self.allow_edit);
        }
    }
}

/// The cut itself — pure over the report graph. `start` performs `effect` and sits in a deny-`effect` layer
/// (`layer`); `rev` is the callee→callers adjacency. Returns the site(s), the denied span, and the hoist
/// frontier. Shared by `cmd_fix` (one function) and `cmd_fix_gate` (every violation).
fn compute_remedy<'a>(
    by_name: &HashMap<&'a str, &'a ReportEntry>,
    rev: &BTreeMap<&'a str, Vec<&'a str>>,
    rules: &[candor_classify::policy::PolicyRule],
    start: &'a ReportEntry,
    effect: &'a str,
    layer: String,
) -> RemedyPlan<'a> {
    // direct site(s) S: BFS from `start` through effect-carrying callees to the DIRECT source(s).
    let mut sites: BTreeSet<&str> = BTreeSet::new();
    let mut fseen: BTreeSet<&str> = BTreeSet::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    q.push_back(start.func.as_str());
    fseen.insert(start.func.as_str());
    while let Some(cur) = q.pop_front() {
        let Some(f) = by_name.get(cur) else { continue };
        if f.direct.iter().any(|e| e == effect) {
            sites.insert(cur);
        }
        for c in &f.calls {
            if let Some(cf) = by_name.get(c.as_str())
                && cf.inferred.iter().any(|e| e == effect)
                && fseen.insert(c.as_str())
            {
                q.push_back(c.as_str());
            }
        }
    }

    // ANCHOR on the site(s) (fall back to `start` for a cross-crate/Unknown source with no local site) and
    // walk UP: denied-layer effect-carriers are the pure span; the allowed-layer callers where the climb
    // stops are the hoist frontier. Site-anchored so the span is the SAME whichever inheriting function
    // triggered it (root-independent) — the inheritors of one crossing collapse to one identical remedy.
    let anchors: Vec<&str> = if sites.is_empty() { vec![start.func.as_str()] } else { sites.iter().copied().collect() };
    let mut denied_span: BTreeSet<&str> = BTreeSet::new();
    let mut hoist_to: BTreeSet<&str> = BTreeSet::new();
    let mut up: VecDeque<&str> = VecDeque::new();
    for &a in &anchors {
        if denied_layer(a, effect, rules).is_some() {
            denied_span.insert(a); // a site that is itself in the denied layer
        }
        up.push_back(a);
    }
    while let Some(cur) = up.pop_front() {
        if let Some(cs) = rev.get(cur) {
            for &caller in cs {
                // skip a caller that doesn't route the effect — INCLUDING one absent from the report (a pure
                // callgraph-only node never carries the effect). Matches candor-swift; avoids classifying a
                // pure node into the span/hoist. (/code-review — was `if let Some(ce) = … && !…`.)
                let Some(ce) = by_name.get(caller) else { continue };
                if !ce.inferred.iter().any(|e| e == effect) {
                    continue;
                }
                if denied_layer(caller, effect, rules).is_some() {
                    if denied_span.insert(caller) {
                        up.push_back(caller); // denied → part of the span; keep climbing
                    }
                } else {
                    hoist_to.insert(caller); // allowed → the boundary; the effect should originate here
                }
            }
        }
    }

    // higher hoist options: allowed-layer transitive callers of the minimal frontier that also route the
    // effect — the places you COULD originate it instead. Hoisting higher keeps the frontier pure too, at the
    // cost of threading the value through more signatures (FIX-SPEC: the trade-off, disclosed not hidden).
    // The SANDWICHED-layer check (/code-review): a hoist is CLEAN only if no forbidden function sits ABOVE
    // the frontier. If a denied fn calls into a hoist target (D1 → A, A the frontier), then hoisting the
    // effect to A leaves D1 still inheriting it — so it isn't a clean hoist. Detected in the same upward
    // climb that gathers `hoist_higher` (which collects the allowed ancestors).
    let mut hoist_higher: BTreeSet<&str> = BTreeSet::new();
    let mut sandwiched = false;
    let mut hq: VecDeque<&str> = hoist_to.iter().copied().collect();
    let mut hseen: BTreeSet<&str> = hoist_to.iter().copied().collect();
    while let Some(cur) = hq.pop_front() {
        if let Some(cs) = rev.get(cur) {
            for &caller in cs {
                let Some(ce) = by_name.get(caller) else { continue };
                if !ce.inferred.iter().any(|e| e == effect) {
                    continue;
                }
                if denied_layer(caller, effect, rules).is_some() {
                    sandwiched = true; // a forbidden fn calls into the frontier — hoisting there wouldn't clear it
                } else if hseen.insert(caller) {
                    hoist_higher.insert(caller);
                    hq.push_back(caller);
                }
            }
        }
    }
    let clean_hoist = !hoist_to.is_empty() && !sandwiched;

    let allow_edit = if layer.is_empty() {
        format!("allow {effect}")
    } else {
        format!("allow {effect} {layer}")
    };
    RemedyPlan { func: &start.func, effect, layer, sites, denied_span, hoist_to, hoist_higher, clean_hoist, allow_edit }
}

/// Read + parse a policy, loud-failing (exit 2) on an unreadable path — the same fail-loud contract as
/// `whatif`, so a typo'd policy never yields a confident plan against a silently-empty ruleset. Returns the
/// deny/`pure` rules on success.
fn load_rules(policy_path: Option<String>) -> Result<Vec<candor_classify::policy::PolicyRule>, i32> {
    let policy_path = policy_path.or_else(|| std::env::var("CANDOR_POLICY").ok());
    let Some(pp) = policy_path else {
        eprintln!("candor fix: a policy is required (pass a policy file or set CANDOR_POLICY) — the fix is the refactor that restores the boundary the edit crossed.");
        return Err(2);
    };
    match std::fs::read_to_string(&pp) {
        Ok(t) => Ok(candor_classify::policy::parse_policy(&t).rules),
        Err(e) => {
            eprintln!("candor: policy `{pp}` could not be read ({e}) — no fix computed.");
            Err(2)
        }
    }
}

/// Build the callee→callers adjacency from the embedded call lists.
fn reverse_graph(entries: &[ReportEntry]) -> BTreeMap<&str, Vec<&str>> {
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in entries {
        for c in &e.calls {
            rev.entry(c.as_str()).or_default().push(e.func.as_str());
        }
    }
    rev
}

pub(crate) fn cmd_fix(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 2, sentinel: true, has_policy: true });
    let (Some(target), Some(effect)) = (g.positional.first().cloned(), g.positional.get(1).cloned()) else {
        eprintln!("usage: candor-query fix <fn> <Effect> [--report <locator>] [--policy <file>] [--json]");
        return 2;
    };
    let (target, effect) = (&target, effect.as_str());
    if candor_classify::cap_from_name(effect).is_none() && effect != "Unknown" {
        eprintln!("candor: unknown effect `{effect}` (expected a candor effect name, e.g. Net/Fs/Db/Exec, or Unknown)");
        return 2;
    }
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let policy_path = g.policy.clone();
    let rules = match load_rules(policy_path) {
        Ok(r) => r,
        Err(c) => return c,
    };

    let entries = load_entries(prefix);
    if entries.is_empty() {
        eprintln!("candor fix: no report for `{prefix}` — scan the crate first.");
        return 2;
    }
    let by_name: HashMap<&str, &ReportEntry> = entries.iter().map(|e| (e.func.as_str(), e)).collect();

    // Resolve `target` among the best-tier name matches, PREFERRING one that actually performs the effect —
    // so a bare leaf (`save`) resolves to the violating `Repo.save`, not a same-named pure `Cache.save` that
    // happens to sort first. (Must match candor-ts/candor-java/candor-swift exactly — a divergence here flips
    // a real remedy into a false "nothing to hoist" on some engines. /code-review.)
    let tier = best_tier(entries.iter().map(|e| e.func.as_str()), target);
    let matches: Vec<&ReportEntry> = entries.iter().filter(|e| tier > 0 && q_match(&e.func, target, tier)).collect();
    let Some(start) = matches
        .iter()
        .copied()
        .find(|e| e.inferred.iter().any(|x| x == effect))
        .or_else(|| matches.first().copied())
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

    let rev = reverse_graph(&entries);
    let plan = compute_remedy(&by_name, &rev, &rules, start, effect, layer);

    if want_json {
        println!("{}", serde_json::to_string_pretty(&plan.to_json()).unwrap());
    } else {
        let mut s = String::new();
        plan.render_text(&mut s);
        print!("{s}");
        println!("\n  (Advisory: candor names the shape, you write the code; the gate re-scan verifies the fix.)");
    }
    0
}

/// `fix-gate <prefix> [policy] [--json|0|1]` — a remedy for EVERY deny/`pure` boundary crossing in the
/// report. This is the loop's payoff: the edit-time gate blocks, and this hands the agent the fix for each
/// crossing. Scope is AS-EFF-006 (effect-in-forbidden-layer) only — the one refactor candor can compute;
/// allowlist/layering findings are a different shape and are left to the gate's own message. Advisory: the
/// gate re-scan stays the ground truth.
pub(crate) fn cmd_fix_gate(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: true });
    let Some(prefix) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let prefix = &prefix;
    let want_json = g.want_json;
    let policy_path = g.policy.clone();
    let rules = match load_rules(policy_path) {
        Ok(r) => r,
        Err(c) => return c,
    };

    let entries = load_entries(prefix);
    if entries.is_empty() {
        eprintln!("candor fix-gate: no report for `{prefix}` — scan the crate first.");
        return 2;
    }
    let by_name: HashMap<&str, &ReportEntry> = entries.iter().map(|e| (e.func.as_str(), e)).collect();
    let rev = reverse_graph(&entries);

    // Every (function, effect) that trips a deny/pure rule → its remedy, collapsed to one plan per root
    // cause (dedup_key folds the inheritors of a single crossing together).
    // Iterate functions (and effects) in sorted order so the first-writer-wins `fn` representative of a
    // collapsed remedy is deterministic across engines (load_entries doesn't sort; java/swift/ts all iterate
    // a sorted key set). The BTreeMap already emits remedies in dedup-key order. (/code-review.)
    let mut sorted: Vec<&ReportEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.func.cmp(&b.func));
    let mut plans: BTreeMap<String, RemedyPlan> = BTreeMap::new();
    for e in sorted {
        let mut effs: Vec<&String> = e.inferred.iter().collect();
        effs.sort();
        for effect in effs {
            if let Some(layer) = denied_layer(&e.func, effect, &rules) {
                let plan = compute_remedy(&by_name, &rev, &rules, e, effect, layer);
                plans.entry(plan.dedup_key()).or_insert(plan);
            }
        }
    }

    if want_json {
        let remedies: Vec<_> = plans.values().map(|p| p.to_json()).collect();
        let out = serde_json::json!({ "ok": remedies.is_empty(), "remedies": remedies });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }

    if plans.is_empty() {
        println!("candor fix-gate: no deny/pure boundary crossings in this report ✓");
        return 0;
    }
    let n = plans.len();
    println!(
        "candor fix — {n} boundary {} for this change:\n",
        if n == 1 { "remedy" } else { "remedies" }
    );
    for (i, p) in plans.values().enumerate() {
        if i > 0 {
            println!("  ────────────────────────────────────────");
        }
        let mut s = String::new();
        p.render_text(&mut s);
        print!("{s}");
    }
    println!("\n  (Advisory: candor names the shape, you write the code; the gate re-scan verifies each fix.)");
    0
}
