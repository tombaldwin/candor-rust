//! The §6.1 containment descriptor and `blindspots` (the Unknown sources).

use crate::*;

/// The accepted `--class` token set, spelled once (SPEC §6.2 ⟨0.24⟩ THE FLAG'S VALUE GRAMMAR): the six
/// reason classes plus the two aliases. `dynamic` is here because the clause's own normative diagnostic
/// (`--class dynamic` == unfiltered minus setup-only) uses it — an engine that rejected it would fail the
/// test every engine carries.
pub(crate) const CLASS_TOKENS: &str =
    "reflect, dispatch, indirect, native, unresolved, setup (aliases: dynamic, *)";

/// Parse a `--class <c,…>` filter into reason classes (SPEC §3.1 ⟨0.20⟩, value grammar pinned normative at
/// §6.2 ⟨0.24⟩): ONE comma-separated list of the six tokens, `dynamic` (every genuine class — i.e. all six
/// MINUS `setup`), or `*` (all six).
///
/// AN UNRECOGNISED TOKEN IS A USAGE ERROR (`Err`, which every caller turns into exit 2), and it is
/// deliberately NOT the policy side's drop-with-a-warning. The asymmetry is the point and a reviewer will
/// ask about it: on the policy side, dropping a token off `deny E Unknown[reflect,dyanmic]` leaves the
/// WIDER rule standing, so the failure is loud — the gate over-fires and someone comes to look. Here the
/// token is a FILTER, so dropping it leaves a NARROWER one: `--class dyanmic` would quietly answer a
/// question the user never asked, with a SMALLER number, and a smaller number out of `unverified` is
/// indistinguishable from a real all-clear. That is precisely the fail-open §6.2 exists to close, in the
/// one verb whose job is to say "green, but not provably so". A query flag that cannot be honoured is
/// REFUSED, not approximated.
pub(crate) fn parse_class_filter(
    spec: &str,
) -> Result<std::collections::HashSet<candor_classify::policy::ReasonClass>, String> {
    use candor_classify::policy::ReasonClass;
    const ALL: [ReasonClass; 6] = [
        ReasonClass::Reflect, ReasonClass::Dispatch, ReasonClass::Indirect,
        ReasonClass::Native, ReasonClass::Unresolved, ReasonClass::Setup,
    ];
    let mut out = std::collections::HashSet::new();
    let mut star = false;
    for t in spec.split(',') {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if t == "*" {
            star = true;
        } else if t == "dynamic" {
            out.extend(ReasonClass::dynamic_set());
        } else if let Some(rc) = ReasonClass::from_token(t) {
            out.insert(rc);
        } else {
            // Name the offending token and list the accepted set — the user has to be able to fix the
            // line from the message alone (a typo is the overwhelmingly likely cause).
            return Err(format!(
                "candor-query: --class: unrecognised reason-class `{t}`\n  \
                 accepted: {CLASS_TOKENS}\n  \
                 a --class value that cannot be honoured is refused, not dropped: dropping it would \
                 narrow the filter and answer a question you did not ask, with a smaller number \
                 (SPEC §6.2 ⟨0.24⟩)"
            ));
        }
    }
    // `*` is evaluated after the whole list so a `*,dyanmic` still reports the typo rather than
    // short-circuiting past it — the refusal must not depend on token order.
    if star {
        return Ok(ALL.into_iter().collect());
    }
    Ok(out)
}

// ── containment ───────────────────────────────────────────────────────────────────────────────────

/// BOUNDARY effects SHOULD live in a dedicated layer — their dispersion is the architecture signal (NOT
/// raw counts, which are domain-dependent). AMBIENT effects are expected to be cross-cutting (logging /
/// timestamps everywhere is fine), so they're reported but not scored. `Unknown` is excluded. `Clipboard`
/// is a §6.1 boundary effect (external-resource I/O), so it is contained/scored like the rest.
pub(crate) const CONTAINED: &[&str] = &["Db", "Net", "Llm", "Exec", "Fs", "Ipc", "Clipboard"];

pub(crate) const AMBIENT: &[&str] = &["Log", "Clock", "Rand", "Env"];

/// `containment` — how well each BOUNDARY effect (Db/Net/Llm/Exec/Fs/Ipc/Clipboard) stays in one layer: the
/// domain-INDEPENDENT architecture signal behind the "leaky cross-cutting" intuition (a ratio /
/// structure, not a count). With a baseline prefix it's a RATCHET — exit 1 if a boundary effect appears
/// in a layer it wasn't in ("Db → actions"), and NOTE when one leaves a layer ("✓ Db ⊘ legacy").
/// Deliberately a diagnostic + trend gate, NOT a single gameable "score".
/// Args: `<prefix> [baseline_prefix] [--json]`.
/// `impact` — the blast radius of a function: every effectful fn that TRANSITIVELY calls it, and which
/// ENTRY POINTS are downstream ("if I change this, what surfaces at runtime?"). Backward dual of `path`;
/// the transitive, entry-point-scoped `callers`. Reverses the effect-relevant `calls` graph. Read-only.
/// Scoped to effectful targets (the report's `calls` records only effect-carrying edges — honest limit).
/// `blindspots` (SPEC §3.1 ⟨0.6⟩) — the Unknown SOURCES: entries whose OWN body has an unresolvable call
/// (so they carry `unknownWhy`), each ranked by its Unknown blast radius (the transitive callers that
/// inherit `Unknown` through it). The actionable inverse of a widely-propagated `Unknown`: a report can
/// read mostly-Unknown from a handful of root causes — this names them, ranked, to declare/resolve/accept.
/// Reverse-BFS over the report's effect-relevant `calls` edges (the channel `Unknown` propagates along),
/// the same graph `impact` uses.
pub(crate) fn cmd_blindspots(args: &[String]) -> i32 {
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
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &entries {
        for c in &e.calls {
            rev.entry(c.as_str()).or_default().push(e.func.as_str());
        }
    }
    let total_unknown = entries.iter().filter(|e| e.inferred.iter().any(|x| x == "Unknown")).count();
    // `--class <c,…>` (SPEC §3.1 ⟨0.20⟩): keep only Unknown SOURCES whose reason classes intersect the
    // filter — the drill-down companion to `--stats`. None ⇒ no filter (`*`/dynamic expand to the classes).
    let class_filter: Option<std::collections::HashSet<candor_classify::policy::ReasonClass>> =
        match g.class.as_deref().map(parse_class_filter).transpose() {
            Ok(v) => v,
            Err(msg) => {
                eprintln!("{msg}");
                return 2;
            }
        };
    let matches = |e: &candor_report::ReportEntry| -> bool {
        use candor_classify::policy::ReasonClass;
        match &class_filter {
            None => true,
            Some(set) => e.unknown_why.iter().any(|w| set.contains(&ReasonClass::classify(w))),
        }
    };
    // `--stats` (SPEC §3.1 ⟨0.20⟩): the reason-class DISTRIBUTION over the Unknown SOURCES — how much
    // Unknown, by class {reflect,dispatch,indirect,native,unresolved,setup} — so a team can SIZE the
    // blind-spot cost (and separate genuine dynamism from `setup` mis-config) BEFORE `deny E Unknown`.
    if g.stats {
        use candor_classify::policy::ReasonClass;
        const ORDER: [&str; 6] = ["reflect", "dispatch", "indirect", "native", "unresolved", "setup"];
        let mut by_class: HashMap<&str, usize> = ORDER.iter().map(|c| (*c, 0usize)).collect();
        let mut sources_n = 0usize;
        for e in &entries {
            if e.unknown_why.is_empty() || !matches(e) {
                continue;
            }
            sources_n += 1;
            let classes: HashSet<&str> = e.unknown_why.iter().map(|w| ReasonClass::classify(w).token()).collect();
            for c in &classes {
                *by_class.get_mut(c).unwrap() += 1;
            }
        }
        if want_json {
            let bc: serde_json::Map<String, serde_json::Value> =
                ORDER.iter().map(|k| (k.to_string(), serde_json::json!(by_class[k]))).collect();
            println!("{}", serde_json::json!({ "byClass": bc, "sources": sources_n, "totalUnknown": total_unknown }));
            return 0;
        }
        if sources_n == 0 {
            println!("  no Unknown sources — nothing to classify (no direct-Unknown in this report).");
            return 0;
        }
        println!("  {sources_n} Unknown source(s) by reason class (of {total_unknown} Unknown function(s)) — size the blind-spot cost before `deny E Unknown[…]`:");
        let mut rows: Vec<(&str, usize)> = ORDER.iter().map(|k| (*k, by_class[k])).filter(|(_, v)| *v > 0).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1)); // most-common class first
        for (k, v) in rows {
            let hint = if k == "setup" { "   ← fixable: the scan isn't configured, not a real blind spot" } else { "" };
            println!("  {k:<12} {v:>4}{hint}");
        }
        return 0;
    }
    #[derive(Serialize)]
    struct Source {
        #[serde(rename = "fn")]
        func: String,
        why: Vec<String>,
        reaches: usize,
        affected: Vec<String>,
    }
    let mut sources: Vec<Source> = Vec::new();
    for e in &entries {
        if e.unknown_why.is_empty() || !matches(e) {
            continue; // a SOURCE (carries its own unknownWhy) of a matching reason class
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut q: VecDeque<&str> = VecDeque::new();
        q.push_back(e.func.as_str());
        seen.insert(e.func.as_str());
        while let Some(cur) = q.pop_front() {
            if let Some(callers) = rev.get(cur) {
                for &caller in callers {
                    if seen.insert(caller) {
                        q.push_back(caller);
                    }
                }
            }
        }
        let mut affected: Vec<String> =
            seen.iter().copied().filter(|n| *n != e.func.as_str()).map(String::from).collect();
        affected.sort_unstable();
        sources.push(Source { func: e.func.clone(), why: e.unknown_why.clone(), reaches: affected.len(), affected });
    }
    // most-smearing sources first; tie-break by name for a stable cross-engine shape.
    sources.sort_by(|a, b| b.reaches.cmp(&a.reaches).then_with(|| a.func.cmp(&b.func)));
    if want_json {
        #[derive(Serialize)]
        struct Out {
            sources: Vec<Source>,
            #[serde(rename = "totalUnknown")]
            total_unknown: usize,
        }
        println!("{}", serde_json::to_string(&Out { sources, total_unknown }).unwrap());
        return 0;
    }
    if sources.is_empty() {
        println!("  no Unknown sources — every call resolved (or no Unknown in this report).");
        return 0;
    }
    println!(
        "  {} Unknown source(s) explaining {} Unknown function(s) — the blind spots to declare, resolve, or accept:",
        sources.len(), total_unknown
    );
    for s in &sources {
        println!("  {:<52} reaches {:>4}  {:?}", s.func, s.reaches, s.why);
    }
    0
}

pub(crate) fn cmd_containment(args: &[String]) -> i32 {
    // `containment [<baseline-locator>]`: the report discovers / comes via `--report`; the SINGLE
    // canonical positional is the OPTIONAL baseline (verb_args: 1). A lone bare positional is therefore
    // the BASELINE (the gating ratchet), never re-read as the deprecated leading report — which would
    // silently drop to non-gating report mode (the §4 cardinal-sin, gate-off, the bug this fixes). The
    // deprecated old form drove the report as a leading positional AHEAD of the baseline; it is arity-
    // gated (only when positionals EXCEED 1), so `<report> <baseline>` still peels the report and leaves
    // the baseline in slot 0.
    let g = parse(args, Shape { verb_args: 1, sentinel: false, has_policy: false });
    let want_json = g.want_json;
    let Some(cur_pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let cur_pre = cur_pre.as_str();
    // A baseline locator, if given, resolves by the same --report rule (dir/.json/prefix).
    let base_locator: Option<String> = g.positional.first().map(|b| resolve_locator(b));
    // Loud load (load_fninfo_loud): no-files AND found-but-corrupt both exit 2 — a corrupt report
    // read as an empty map here would score every effect fully contained (and the ratchet below would
    // see zero leaks): a false architecture all-clear over corrupt input, the §4 cardinal sin.
    let cur = match load_fninfo_loud(cur_pre, "") {
        Ok(m) => m,
        Err(c) => return c,
    };
    let names: Vec<&String> = cur.keys().collect();
    let pl = common_prefix_len(&names);
    // effect -> (layer -> count of functions performing it DIRECTLY)
    let mut by_eff: BTreeMap<&'static str, BTreeMap<String, usize>> = BTreeMap::new();
    let known: Vec<&'static str> = CONTAINED.iter().chain(AMBIENT.iter()).copied().collect();
    for (fname, info) in &cur {
        let layer = layer_of(fname, pl);
        for eff in &info.direct {
            if let Some(k) = known.iter().find(|e| **e == eff.as_str()) {
                *by_eff.entry(*k).or_default().entry(layer.clone()).or_default() += 1;
            }
        }
    }

    // RATCHET mode: a baseline prefix was given — flag any NEW (contained-effect, layer), note removals.
    if let Some(base_pre) = base_locator.as_deref() {
        // The baseline loads loud too: a corrupt (or typo'd) baseline reads as an empty layer map, so
        // every boundary effect looks like a NEW leak — a false ratchet FAIL is noisier than the
        // all-clear case but just as untrustworthy, and the corrupt report deserves the same refusal.
        let base = match load_fninfo_loud(base_pre, "baseline") {
            Ok(m) => m,
            Err(c) => return c,
        };
        let bnames: Vec<&String> = base.keys().collect();
        let bpl = common_prefix_len(&bnames);
        let mut base_layers: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
        for (fname, info) in &base {
            let layer = layer_of(fname, bpl);
            for eff in &info.direct {
                if let Some(k) = CONTAINED.iter().find(|e| **e == eff.as_str()) {
                    base_layers.entry(*k).or_default().insert(layer.clone());
                }
            }
        }
        let mut leaks: Vec<String> = Vec::new();
        let mut cleanups: Vec<String> = Vec::new();
        for eff in CONTAINED {
            let now: BTreeSet<String> =
                by_eff.get(eff).map(|m| m.keys().cloned().collect()).unwrap_or_default();
            let was = base_layers.get(eff).cloned().unwrap_or_default();
            for l in now.difference(&was) {
                leaks.push(format!("{eff} → {l}"));
            }
            for l in was.difference(&now) {
                cleanups.push(format!("{eff} ⊘ {l}"));
            }
        }
        leaks.sort();
        cleanups.sort();
        if want_json {
            let out = serde_json::json!({ "leaks": leaks, "cleanups": cleanups });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
            return if leaks.is_empty() { 0 } else { 1 };
        }
        if !leaks.is_empty() {
            println!("[AS-EFF-010] a boundary effect leaked into a layer it wasn't in:");
            for l in &leaks {
                println!("  {l}");
            }
        }
        if !cleanups.is_empty() {
            if !leaks.is_empty() {
                println!();
            }
            println!("✓ improved — a boundary effect left a layer:");
            for c in &cleanups {
                println!("  {c}");
            }
        }
        if leaks.is_empty() && cleanups.is_empty() {
            println!("candor containment: unchanged vs {base_pre} (no leaks, no cleanups).");
        } else if leaks.is_empty() {
            println!("\ncandor containment: no regressions ✓");
        }
        if !leaks.is_empty() {
            println!("\nfix: keep the call in its boundary layer, or refresh the baseline if intended.");
        }
        return if leaks.is_empty() { 0 } else { 1 };
    }

    // REPORT mode: the containment diagnostic.
    let owner_of = |layers: &BTreeMap<String, usize>| -> (String, usize) {
        layers.iter().max_by_key(|(_, n)| **n).map(|(k, n)| (k.clone(), *n)).unwrap()
    };
    if want_json {
        let contained: Vec<serde_json::Value> = CONTAINED
            .iter()
            .filter_map(|eff| {
                by_eff.get(eff).map(|layers| {
                    let tot: usize = layers.values().sum();
                    let (owner, on) = owner_of(layers);
                    serde_json::json!({
                        "effect": eff, "containmentPct": 100 * on / tot,
                        "layers": layers.len(), "owner": owner, "placement": layers,
                    })
                })
            })
            .collect();
        let ambient: BTreeMap<&str, usize> =
            AMBIENT.iter().filter_map(|e| by_eff.get(e).map(|m| (*e, m.len()))).collect();
        let out = serde_json::json!({ "contained": contained, "ambient": ambient });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    println!("candor containment — how well each boundary effect stays in one layer");
    println!("(the signal is dispersion across layers, NOT effect count)\n");
    println!("  {:<7} {:>9} {:>7}   owner  ← leaked into", "effect", "contained", "layers");
    let mut any = false;
    for eff in CONTAINED {
        let Some(layers) = by_eff.get(eff) else { continue };
        any = true;
        let tot: usize = layers.values().sum();
        let (owner, on) = owner_of(layers);
        let mut others: Vec<(&String, &usize)> = layers.iter().filter(|(k, _)| **k != owner).collect();
        others.sort_by(|a, b| b.1.cmp(a.1));
        let leaks: String =
            others.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(", ");
        let tail = if leaks.is_empty() { String::new() } else { format!("  ← {leaks}") };
        println!("  {eff:<7} {:>8}% {:>7}   {owner} ({on}){tail}", 100 * on / tot, layers.len());
    }
    if !any {
        println!("  (no boundary effects in the report)");
    }
    let amb: String = AMBIENT
        .iter()
        .filter_map(|e| by_eff.get(e).map(|m| format!("{e} {}L", m.len())))
        .collect::<Vec<_>>()
        .join(", ");
    if !amb.is_empty() {
        println!("\n  ambient (cross-cutting expected, not scored): {amb}");
    }
    println!(
        "\n  containment% = share of an effect's direct uses in its dominant layer; 100% = fully contained.\
         \n  ratchet a baseline: candor-query containment <prefix> <baseline_prefix> (exit 1 on a new leak)."
    );
    0
}
