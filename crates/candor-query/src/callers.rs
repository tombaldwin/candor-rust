//! Call-graph traversals: `callers`, `impact`, `path`, `reachable`.

use crate::*;

// ── callers ─────────────────────────────────────────────────────────────────────────────────────

pub(crate) fn cmd_callers(args: &[String]) -> i32 {
    // --include-unknown ⟨0.7⟩: also disclose the unresolved-dispatch frontier (possibleViaUnknownDispatch).
    // candor-query is the query engine for candor-swift too (swift is analyze-only), so this serves swift
    // reports (which emit `dispatch:owner.member` + a hierarchy sidecar) as well as rust ones. Without the
    // flag, the {of,direct,transitive} shape is unchanged.
    //
    // ⟨0.24⟩ THIS COMMENT USED TO READ "as well as rust reports (no `dispatch:` → empty frontier)", and
    // that was false in both halves: candor-scan emits `dispatch:` for EVERY dispatch reason it raises
    // (20 in a 1062-report census, all `dispatch:untyped cross-package receiver`), and the frontier over a
    // rust report is therefore not empty — it is the DOT-FREE arm below. SPEC §3.1 carried the same
    // sentence and §4 restated it; both were corrected in the same rung. A falsified assertion has as many
    // homes as it has restatements, and fixing the one you found is not fixing it — this is the third.
    //
    // THE FRONTIER SELECTS BY KIND, NOT BY CLASS, and that is load-bearing rather than incidental. §6.2
    // projects `ambiguous:` to class `dispatch`, but an `ambiguous:` entry never formed an owner at all,
    // so there is nothing for condition (3) to resolve against. Keying off the `dispatch:` PREFIX below
    // excludes them for free; keying off `ReasonClass::classify(w) == Dispatch` would admit all 8710 of
    // them on this engine's census. Pinned by
    // `callers_include_unknown_keys_off_the_kind_so_ambiguous_and_off_vocabulary_stay_out`.
    let g = parse(args, Shape { verb_args: 1, sentinel: true, has_policy: false });
    let include_unknown = g.include_unknown;
    let Some(q) = g.positional.first().map(String::as_str) else {
        eprintln!("usage: candor-query callers <fn> [--report <locator>] [--json] [--include-unknown]");
        return 2;
    };
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let (pre, want_json) = (pre.as_str(), g.want_json);
    // Prefer the full call-graph sidecar (the engine emits `<prefix>.<crate>.<kind>.callgraph.json`
    // alongside the report). It records EVERY function's callees — including pure ones — so we can
    // answer "who TRANSITIVELY calls X" for any function: the blast radius an agent needs *before*
    // adding an effect to X. The report alone only records effect-relevant edges (can't see a pure X).
    let mut cg = load_callgraph(pre);
    // The sidecar records EVERY function (incl. pure leaves), so a no-match against it is a DEFINITIVE
    // "no such function" (loud exit 2). The fallback below is effect-relevant edges ONLY, so a pure leaf
    // called only by pure functions is invisible there — a no-match is INCONCLUSIVE, not proof of absence.
    let complete_graph = !cg.is_empty();
    // Fallback (no call-graph sidecar): build a graph from the report's effect-relevant `calls` edges
    // and run the SAME query, so the output shape ({of,direct,transitive}) and JSON contract are
    // identical to the sidecar path. The old fallback emitted a {callee:[callers]} map — diverging from
    // the pinned SPEC §3.1 shape (/code-review). Transitive is necessarily incomplete here (effectful
    // edges only); the sidecar exists to fix that.
    if cg.is_empty() {
        cg = match load_entries_loud(pre) {
            Ok(v) => v.into_iter().map(|e| (e.func, e.calls)).collect(),
            Err(c) => return c,
        };
    }
    if include_unknown {
        let entries = match load_entries_loud(pre) {
            Ok(v) => v,
            Err(c) => return c,
        };
        callers_via_callgraph_frontier(&cg, &entries, &load_hierarchy(pre), q, want_json, complete_graph)
    } else {
        callers_via_callgraph(&cg, q, want_json, complete_graph)
    }
}

/// callers + the unresolved-dispatch frontier (--include-unknown). The CONFIRMED reachers, plus the
/// functions that reach `q` only through a `dispatch:OWNER.member` the engine declined to resolve —
/// disclosed iff a confirmed reacher is an override of OWNER.member (same method AND a subtype of OWNER
/// per the hierarchy; empty hierarchy → simple-name match, over-lists). A DOT-FREE detail names no owner
/// at all, so that test is unanswerable and the source is disclosed verbatim ⟨0.24⟩. Never asserted
/// ("cannot confirm").
pub(crate) fn callers_via_callgraph_frontier(
    cg: &BTreeMap<String, Vec<String>>,
    entries: &[ReportEntry],
    hier: &BTreeMap<String, Vec<String>>,
    q: &str,
    want_json: bool,
    complete: bool,
) -> i32 {
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (caller, callees) in cg {
        for c in callees {
            rev.entry(c.as_str()).or_default().push(caller.as_str());
        }
    }
    let names: BTreeSet<&str> =
        cg.keys().map(|s| s.as_str()).chain(cg.values().flatten().map(|s| s.as_str())).collect();
    let tier = best_tier(names.iter().copied(), q);
    let targets: Vec<String> = names.iter().copied().filter(|n| q_match(n, q, tier)).map(String::from).collect();
    if targets.is_empty() {
        // A nonexistent function is a LOUD error (exit 2), like `path`/`impact` — never an empty result at
        // exit 0, which reads as an authoritative "nothing calls it" for a fn that doesn't exist (corpus-audit
        // #3). Gated on a non-empty call graph so a report without one isn't misreported as "no such fn".
        if names.is_empty() {
            // ⟨0.28⟩ UNANSWERABLE MUST REACH THE MACHINE CHANNEL. This printed `{}` at exit 0, and the
            // human arm said "no call graph in the report" — the split that makes a defect a cardinal
            // sin. A consumer reading `direct`, or defaulting it (the fail-open idiom ⟨0.24⟩ names on
            // every key in this format), was told NOBODY CALLS this fn: a blast radius of "safe to
            // edit" over a pair whose honest answer is "this run judged nothing". The ⟨0.28⟩ sidecar
            // rung turned that from a rare state into the standard one after a failed run, so the
            // corner became the common path. Both channels now fail closed: the document names itself
            // unanswerable AND the exit is non-zero, because a key alone still leaves `d.get("direct",
            // [])` reading as a determined negative.
            let why = "no call graph in the report — the §2.2 sidecar is absent, so who calls this \
                       function is UNANSWERABLE, not empty (SPEC §3.3.1 ⟨0.28⟩)";
            if want_json {
                println!("{{\n  \"of\": [\"{}\"],\n  \"unanswerable\": \"{}\"\n}}", q.replace('"', "\\\""), why);
            } else {
                println!("candor: {why}");
            }
            return 2;
        }
        // Only a COMPLETE graph (the sidecar, which lists every fn incl. pure leaves) can prove a name is
        // absent. On the effect-only fallback (no sidecar), a miss is INCONCLUSIVE — a pure leaf called only
        // by pure fns is simply invisible — so answer empty at exit 0, never a false "no such function" (#5).
        if !complete {
            if want_json { println!("{{}}"); }
            else { println!("candor: no caller of `{q}` in the effect-relevant graph (the full call-graph sidecar is absent; re-scan with --out to see pure-only callers)."); }
            return 0;
        }
        eprintln!("candor-query callers: no function matching '{q}' in the call graph");
        return 2;
    }
    let direct: BTreeSet<String> =
        targets.iter().flat_map(|t| rev.get(t.as_str()).into_iter().flatten().map(|s| s.to_string())).collect();
    let mut all: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<String> = targets.clone();
    while let Some(n) = stack.pop() {
        if let Some(cs) = rev.get(n.as_str()) {
            for &c in cs {
                if all.insert(c.to_string()) {
                    stack.push(c.to_string());
                }
            }
        }
    }
    // Frontier: index confirmed reachers' declaring types by simple method name, then test each
    // dispatch:OWNER.member source whose owner an override (a reacher) is a subtype of.
    let mut confirmed: BTreeSet<&str> = BTreeSet::new();
    for t in &targets {
        confirmed.insert(t.as_str());
    }
    for a in &all {
        confirmed.insert(a.as_str());
    }
    let mut by_method: HashMap<&str, Vec<&str>> = HashMap::new();
    for r in &confirmed {
        by_method.entry(simple_method(r)).or_default().push(declaring_type(r));
    }
    let has_hier = !hier.is_empty();
    let mut possible: Vec<(String, String)> = Vec::new();
    for e in entries {
        if confirmed.contains(e.func.as_str()) {
            continue;
        }
        let mut hits: BTreeSet<&str> = BTreeSet::new();
        for w in &e.unknown_why {
            if let Some(key) = w.strip_prefix("dispatch:") {
                // ⟨0.24⟩ A DOT-FREE detail names no owner and no member — the engine could not form a
                // receiver type at all (candor-scan emits `dispatch:untyped cross-package receiver` for a
                // call into a chained dependency, and a 1062-report census found EVERY dispatch reason on
                // this engine was that form). Condition (3), "some confirmed reacher is an override of
                // OWNER.M", is then UNANSWERABLE, and an unanswerable condition MUST NOT be scored as a
                // failed one: the source is DISCLOSED with the raw detail verbatim. This is the same
                // direction the no-hierarchy fallback takes one rung up — with no sidecar the subtype test
                // is unanswerable and the ruling is to over-list, not to drop. The frontier over-lists by
                // construction and asserts NOTHING into `transitive`, so a spurious entry costs precision
                // while a dropped one is a false all-clear.
                //
                // MEASURED before this guard: `mod.Dotfree.run` carrying `dispatch:untyped cross-package
                // receiver` appeared NOWHERE in the output, in BOTH the hierarchy and the no-hierarchy arm,
                // with no diagnostic naming it — because `simple_method`/`declaring_type` fall back to the
                // WHOLE STRING with no dot, so `by_method.get(m)` could never hit.
                //
                // Detected STRUCTURALLY (contains no '.'), never by matching the scanner's wording: an
                // allowlist of known reason strings silently drops every reason it forgets, which is
                // exactly the defect being closed.
                if !key.contains('.') {
                    hits.insert(key);
                    continue;
                }
                let m = simple_method(key);
                let owner = declaring_type(key);
                if let Some(types) = by_method.get(m)
                    && (!has_hier || types.iter().any(|t| is_subtype_of(t, owner, hier)))
                {
                    hits.insert(m);
                }
            }
        }
        if !hits.is_empty() {
            // `viaDispatchOn` keeps its pinned one-entry-per-fn shape, multiple hits ','-joined. A raw
            // dot-free detail may carry SPACES (the scanner's does) — harmless, the field was never
            // whitespace-delimited. A detail carrying a ',' would be ambiguous to a consumer that splits
            // on ',', and that is accepted deliberately: `viaDispatchOn` is a disclosure string candor
            // itself never re-parses into an owner, no engine emits a comma today, and the alternatives —
            // escaping (a new sub-grammar in a pinned field) or dropping/truncating the detail — would
            // either break every existing consumer or re-open the silent drop this change closes.
            possible.push((e.func.clone(), hits.iter().copied().collect::<Vec<_>>().join(",")));
        }
    }
    possible.sort();
    if want_json {
        let pv: Vec<_> =
            possible.iter().map(|(f, v)| serde_json::json!({"fn": f, "viaDispatchOn": v})).collect();
        let out = serde_json::json!({
            "of": targets,
            "direct": direct.iter().collect::<Vec<_>>(),
            "transitive": all.iter().collect::<Vec<_>>(),
            "possibleViaUnknownDispatch": pv,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    let tgt = targets.join(", ");
    if !all.is_empty() {
        println!("  `{tgt}` is reached by {} function(s) (the blast radius if it gained an effect):", all.len());
        for c in &all {
            let mark = if direct.contains(c) { " (direct)" } else { "" };
            println!("      {c}{mark}");
        }
    }
    if !possible.is_empty() {
        println!("  + {} function(s) MAY also reach `{tgt}` via an unresolved broad dispatch candor declined to resolve (cannot confirm):", possible.len());
        for (f, v) in &possible {
            println!("      {f}  (via dispatch on {v})");
        }
    }
    if all.is_empty() && possible.is_empty() {
        println!("  `{tgt}` has no callers (nothing in this crate calls it).");
    }
    0
}

/// "Who reaches `q`?" over the full call graph: the DIRECT callers and the full TRANSITIVE set (the
/// blast radius if `q` gained an effect). Works for any function, effectful or pure.
pub(crate) fn callers_via_callgraph(cg: &BTreeMap<String, Vec<String>>, q: &str, want_json: bool, complete: bool) -> i32 {
    // reverse adjacency: callee -> its direct callers.
    let mut rev: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (caller, callees) in cg {
        for c in callees {
            rev.entry(c.as_str()).or_default().push(caller.as_str());
        }
    }
    // resolve `q` to the actual node name(s): exact path, or a unique basename / `::q` suffix match.
    let names: BTreeSet<&str> =
        cg.keys().map(|s| s.as_str()).chain(cg.values().flatten().map(|s| s.as_str())).collect();
    let tier = best_tier(names.iter().copied(), q);
    let targets: Vec<&str> = names.iter().copied().filter(|n| q_match(n, q, tier)).collect();
    if targets.is_empty() {
        // A nonexistent function is a LOUD error (exit 2), like `path`/`impact` — never an empty result at
        // exit 0, which reads as an authoritative "nothing calls it" for a fn that doesn't exist (corpus-audit
        // #3). Gated on a non-empty call graph so a report without one isn't misreported as "no such fn".
        if names.is_empty() {
            // ⟨0.28⟩ UNANSWERABLE MUST REACH THE MACHINE CHANNEL. This printed `{}` at exit 0, and the
            // human arm said "no call graph in the report" — the split that makes a defect a cardinal
            // sin. A consumer reading `direct`, or defaulting it (the fail-open idiom ⟨0.24⟩ names on
            // every key in this format), was told NOBODY CALLS this fn: a blast radius of "safe to
            // edit" over a pair whose honest answer is "this run judged nothing". The ⟨0.28⟩ sidecar
            // rung turned that from a rare state into the standard one after a failed run, so the
            // corner became the common path. Both channels now fail closed: the document names itself
            // unanswerable AND the exit is non-zero, because a key alone still leaves `d.get("direct",
            // [])` reading as a determined negative.
            let why = "no call graph in the report — the §2.2 sidecar is absent, so who calls this \
                       function is UNANSWERABLE, not empty (SPEC §3.3.1 ⟨0.28⟩)";
            if want_json {
                println!("{{\n  \"of\": [\"{}\"],\n  \"unanswerable\": \"{}\"\n}}", q.replace('"', "\\\""), why);
            } else {
                println!("candor: {why}");
            }
            return 2;
        }
        // Only a COMPLETE graph (the sidecar, which lists every fn incl. pure leaves) can prove a name is
        // absent. On the effect-only fallback (no sidecar), a miss is INCONCLUSIVE — a pure leaf called only
        // by pure fns is simply invisible — so answer empty at exit 0, never a false "no such function" (#5).
        if !complete {
            if want_json { println!("{{}}"); }
            else { println!("candor: no caller of `{q}` in the effect-relevant graph (the full call-graph sidecar is absent; re-scan with --out to see pure-only callers)."); }
            return 0;
        }
        eprintln!("candor-query callers: no function matching '{q}' in the call graph");
        return 2;
    }

    let direct: BTreeSet<&str> = targets.iter().flat_map(|t| rev.get(t).into_iter().flatten().copied()).collect();
    // transitive closure of callers (reverse BFS).
    let mut all: BTreeSet<&str> = BTreeSet::new();
    let mut stack: Vec<&str> = targets.clone();
    while let Some(n) = stack.pop() {
        if let Some(cs) = rev.get(n) {
            for &c in cs {
                if all.insert(c) {
                    stack.push(c);
                }
            }
        }
    }

    if want_json {
        let out = serde_json::json!({
            "of": targets,
            "direct": direct.iter().collect::<Vec<_>>(),
            "transitive": all.iter().collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    let tgt = targets.join(", ");
    if all.is_empty() {
        println!("  `{tgt}` has no callers (nothing in this crate calls it).");
        return 0;
    }
    println!(
        "  `{tgt}` is reached by {} function(s) (the blast radius if it gained an effect):",
        all.len()
    );
    for c in &all {
        let mark = if direct.contains(c) { " (direct)" } else { "" };
        println!("      {c}{mark}");
    }
    0
}

pub(crate) fn cmd_impact(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 1, sentinel: true, has_policy: false });
    let want_json = g.want_json;
    let Some(fn_arg) = g.positional.first().cloned() else {
        eprintln!("usage: candor-query impact <fn-substring> [--report <locator>] [--json]");
        return 2;
    };
    let fn_arg = &fn_arg;
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let pre = pre.as_str();
    let entries = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let by_name: HashMap<&str, &ReportEntry> =
        entries.iter().map(|e| (e.func.as_str(), e)).collect();
    let target = entries
        .iter()
        .find(|e| e.func == *fn_arg)
        .or_else(|| entries.iter().find(|e| e.func.contains(fn_arg.as_str())));
    let Some(target) = target else {
        eprintln!("candor-query impact: no function matching '{fn_arg}'");
        return 2;
    };
    // Reverse the effect-relevant call graph, then BFS backward from the target.
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &entries {
        for c in &e.calls {
            rev.entry(c.as_str()).or_default().push(e.func.as_str());
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    q.push_back(target.func.as_str());
    seen.insert(target.func.as_str());
    while let Some(cur) = q.pop_front() {
        if let Some(callers) = rev.get(cur) {
            for &caller in callers {
                if seen.insert(caller) {
                    q.push_back(caller);
                }
            }
        }
    }
    // The affected set: every effectful fn that transitively calls the target (the report holds only
    // effectful units, so every reverse-reachable node is one). Sorted for a stable cross-engine shape.
    let mut affected_names: Vec<&str> =
        seen.iter().copied().filter(|n| *n != target.func.as_str()).collect();
    affected_names.sort_unstable();
    let mut roots: Vec<&ReportEntry> = Vec::new();
    if target.entry_point {
        roots.push(target);
    }
    let mut downstream: Vec<&ReportEntry> = seen
        .iter()
        .filter(|n| **n != target.func.as_str())
        .filter_map(|n| by_name.get(n).copied())
        .filter(|e| e.entry_point)
        .collect();
    downstream.sort_by(|a, b| a.func.cmp(&b.func));
    roots.extend(downstream);

    if want_json {
        let eps: Vec<_> = roots
            .iter()
            .map(|r| serde_json::json!({ "fn": r.func, "inferred": r.inferred }))
            .collect();
        let out = serde_json::json!({
            "fn": target.func,
            "affectedCount": affected_names.len(),
            "affected": affected_names,
            "entryPoints": eps
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    println!("candor impact — what changing `{}` affects:\n", target.func);
    println!(
        "  {} effectful function{} transitively call it.",
        affected_names.len(),
        if affected_names.len() == 1 { "" } else { "s" }
    );
    if roots.is_empty() {
        println!(
            "  No entry point reaches it — not on a runtime path (dead, or a library fn called only externally)."
        );
        return 0;
    }
    println!(
        "  {} entry point{} downstream (a change here surfaces at runtime via):",
        roots.len(),
        if roots.len() == 1 { "" } else { "s" }
    );
    for r in &roots {
        println!("    {}   {{ {} }}", r.func, r.inferred.join(", "));
    }
    0
}

/// `path` — the call chain by which a function comes to perform an effect: a shortest-path BFS over the
/// effect-relevant `calls` graph from <fn> to the nearest function that performs <effect> DIRECTLY (the
/// source), through callees that carry the effect. Answers "this performs Net — through WHAT?", the chain
/// `where`/`callers` describe the ends of but don't connect. Mirrors the JVM port's `path`. Read-only.
pub(crate) fn cmd_path(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 2, sentinel: true, has_policy: false });
    let want_json = g.want_json;
    let (Some(fn_arg), Some(effect)) = (g.positional.first().cloned(), g.positional.get(1).cloned()) else {
        eprintln!("usage: candor-query path <fn-substring> <Effect> [--report <locator>] [--json]");
        return 2;
    };
    let (fn_arg, effect) = (&fn_arg, effect.as_str());
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let pre = pre.as_str();
    let entries = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let by_name: HashMap<&str, &ReportEntry> =
        entries.iter().map(|e| (e.func.as_str(), e)).collect();
    let start = entries
        .iter()
        .find(|e| e.func == *fn_arg)
        .or_else(|| entries.iter().find(|e| e.func.contains(fn_arg.as_str())));
    let Some(start) = start else {
        eprintln!("candor-query path: no function matching '{fn_arg}'");
        return 2;
    };
    if !start.inferred.iter().any(|e| e == effect) {
        // An empty `path` is the honest "no local source on a path" answer (SPEC §3.1), NOT an error.
        // In --json mode emit the documented {effect,fn,path:[]} object — printing human text here
        // polluted stdout so a `jq` consumer crashed (adversarial fidelity review; Java/TS emit the JSON).
        if want_json {
            let out = serde_json::json!({ "fn": start.func, "effect": effect, "path": [] });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        } else {
            println!("{} does not perform {effect}  (inferred: {:?})", start.func, start.inferred);
        }
        return 0;
    }
    // BFS through effect-carrying callees to the first DIRECT source.
    let mut prev: HashMap<&str, Option<&str>> = HashMap::new();
    let mut q: VecDeque<&str> = VecDeque::new();
    q.push_back(start.func.as_str());
    prev.insert(start.func.as_str(), None);
    let mut source: Option<&str> = None;
    while let Some(cur) = q.pop_front() {
        let Some(f) = by_name.get(cur) else { continue };
        if f.direct.iter().any(|e| e == effect) {
            source = Some(cur);
            break;
        }
        for c in &f.calls {
            if let Some(cf) = by_name.get(c.as_str())
                && cf.inferred.iter().any(|e| e == effect) && !prev.contains_key(c.as_str()) {
                    prev.insert(c.as_str(), Some(cur));
                    q.push_back(c.as_str());
                }
        }
    }
    let Some(source) = source else {
        // Reached via a cross-crate call or Unknown — the honest empty-path answer (SPEC §3.1), not an
        // error. Emit the JSON object in --json mode (was human text → broke a `jq` consumer).
        if want_json {
            let out = serde_json::json!({ "fn": start.func, "effect": effect, "path": [] });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        } else {
            println!(
                "{} performs {effect} but its source is not a local function \
                 (cross-crate, or via Unknown) — not statically traceable.",
                start.func
            );
        }
        return 0;
    };
    let mut chain: Vec<&str> = Vec::new();
    let mut n = Some(source);
    while let Some(name) = n {
        chain.push(name);
        n = *prev.get(name).unwrap();
    }
    chain.reverse();

    if want_json {
        let steps: Vec<_> = chain
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let loc = by_name.get(name).map(|e| e.loc.clone()).unwrap_or_default();
                serde_json::json!({ "fn": name, "loc": loc, "source": i == chain.len() - 1 })
            })
            .collect();
        let out = serde_json::json!({ "fn": start.func, "effect": effect, "path": steps });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    println!("candor path — how `{}` comes to perform {effect}:\n", start.func);
    for (i, name) in chain.iter().enumerate() {
        let indent = "  ".repeat(i + 1);
        let arrow = if i == 0 { "" } else { "→ " };
        let tag = if i == chain.len() - 1 {
            let loc = by_name.get(name).map(|e| e.loc.as_str()).unwrap_or("");
            if loc.is_empty() {
                format!("   [{effect} source]")
            } else {
                format!("   [{effect} source @ {loc}]")
            }
        } else {
            String::new()
        };
        println!("{indent}{arrow}{name}{tag}");
    }
    0
}

/// `reachable` — the effects the program performs at runtime: the union of `inferred` over the ENTRY
/// POINTS (reachability roots — `main`, `#[no_mangle]` exports; far richer on the JVM port). Since
/// `inferred` is already transitive, a root's set IS its full reachable surface, so the union answers
/// "what does this binary actually do" without a per-fn dump. Mirrors the JVM port's `reachable`.
pub(crate) fn cmd_reachable(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: false });
    let want_json = g.want_json;
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let pre = pre.as_str();
    let entries = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let roots: Vec<&ReportEntry> = entries.iter().filter(|e| e.entry_point).collect();
    let mut by_eff: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &roots {
        for eff in &e.inferred {
            by_eff.entry(eff.clone()).or_default().push(e.func.clone());
        }
    }

    if want_json {
        let effects: serde_json::Map<String, serde_json::Value> = by_eff
            .iter()
            .map(|(eff, who)| {
                (eff.clone(), serde_json::json!({ "count": who.len(), "via": who }))
            })
            .collect();
        let out = serde_json::json!({ "entryPoints": roots.len(), "effects": effects });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }

    println!(
        "candor reachable — effects the program performs at runtime (union over {} entry point{})",
        roots.len(),
        if roots.len() == 1 { "" } else { "s" }
    );
    if roots.is_empty() {
        println!("  (no entry points in this report — nothing is marked runtime-invoked)");
        return 0;
    }
    // Boundary effects first (Clipboard rides in CONTAINED now), then ambient, then the Unknown caveat.
    // Any other effect trails.
    let order: Vec<&str> = CONTAINED
        .iter()
        .chain(AMBIENT.iter())
        .copied()
        .chain(["Unknown"])
        .collect();
    let mut seen: Vec<&String> = by_eff.keys().collect();
    seen.sort_by_key(|e| order.iter().position(|o| *o == e.as_str()).unwrap_or(order.len()));
    for eff in seen {
        let who = &by_eff[eff];
        let examples =
            who.iter().take(3).map(|s| reachable_leaf(s)).collect::<Vec<_>>().join(", ");
        let more = if who.len() > 3 { ", …" } else { "" };
        let tag = if eff == "Unknown" { "   ← visibility caveat, not a performed effect" } else { "" };
        println!("  {eff:<10} {:>3}  ({examples}{more}){tag}", who.len());
    }
    let pure = roots.iter().filter(|e| e.inferred.is_empty()).count();
    let n = roots.len();
    println!("\n  {n} entry point{}; {pure} perform no effect (pure roots).", if n == 1 { "" } else { "s" });
    0
}

/// Last two `::`-segments of a fully-qualified path, for compact examples.
pub(crate) fn reachable_leaf(fname: &str) -> String {
    let mut segs: Vec<&str> = fname.rsplitn(3, "::").take(2).collect();
    segs.reverse();
    segs.join("::")
}
