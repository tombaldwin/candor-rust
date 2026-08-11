//! The per-function views: `show`, `where`, `map`.

use crate::*;

// ── show ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct ShowJson {
    #[serde(rename = "fn")]
    pub(crate) func: String,
    pub(crate) inferred: Vec<String>,
    pub(crate) direct: Vec<String>,
    /// Fs read/write detail, omitted when absent — see `ReportEntry::fs`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) fs: Vec<String>,
    /// Literal Net endpoints, omitted when none visible — see `ReportEntry::hosts`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) hosts: Vec<String>,
    /// Literal Db tables, omitted when none visible — see `ReportEntry::tables`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tables: Vec<String>,
    pub(crate) unresolved: bool,
}

pub(crate) fn cmd_show(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 1, sentinel: true, has_policy: false });
    let Some(q) = g.positional.first().map(String::as_str) else {
        eprintln!("usage: candor-query show <fn> [--report <locator>] [--json]");
        return 2;
    };
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let (pre, want_json) = (pre.as_str(), g.want_json);
    let all = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    let tier = best_tier(all.iter().map(|e| e.func.as_str()), q);
    let mut fns: Vec<ReportEntry> = all.into_iter().filter(|e| q_match(&e.func, q, tier)).collect();
    fns.sort_by(|a, b| a.func.cmp(&b.func));

    if want_json {
        let out: Vec<ShowJson> = fns
            .iter()
            .map(|e| ShowJson {
                func: e.func.clone(),
                inferred: sorted(&e.inferred),
                direct: sorted(&e.direct),
                fs: e.fs.clone(),
                hosts: e.hosts.clone(),
                tables: e.tables.clone(),
                unresolved: e.unresolved,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    if fns.is_empty() {
        println!("candor: no effectful function matching `{q}` (pure functions are omitted from the report).");
        return 0;
    }
    let w = fns.iter().map(|e| e.func.chars().count()).max().unwrap_or(0);
    let any_fs = fns.iter().any(|e| !e.fs.is_empty());
    let any_hosts = fns.iter().any(|e| !e.hosts.is_empty());
    for e in &fns {
        let direct: BTreeSet<&String> = e.direct.iter().collect();
        let parts: Vec<String> = sorted(&e.inferred)
            .into_iter()
            .map(|x| {
                let star = if direct.contains(&x) { "*" } else { "" };
                // Refine Fs with its read/write detail (`Fs*(write)`) and Net with the literal
                // endpoint(s) candor could see (`Net*(api.example.com)`), when known.
                if x == "Fs" && !e.fs.is_empty() {
                    format!("Fs{star}({})", e.fs.join(","))
                } else if x == "Net" && !e.hosts.is_empty() {
                    format!("Net{star}({})", e.hosts.join(","))
                } else if x == "Db" && !e.tables.is_empty() {
                    format!("Db{star}({})", e.tables.join(","))
                } else {
                    format!("{x}{star}")
                }
            })
            .collect();
        let unk = if e.unresolved { "  ⚠ unresolved (set may be incomplete)" } else { "" };
        println!("  {:<w$}  {{ {} }}{}", e.func, parts.join(" "), unk, w = w);
    }
    let fs_note = if any_fs { ";  Fs(read/write) = the filesystem access seen" } else { "" };
    let host_note = if any_hosts { ";  Net(host) = a literal endpoint seen (runtime addresses aren't shown)" } else { "" };
    let any_tables = fns.iter().any(|e| !e.tables.is_empty());
    let table_note = if any_tables { ";  Db(table) = a literal table seen (dynamic SQL isn't shown)" } else { "" };
    println!("  (* = performed in the function's own body; unmarked = via a callee{fs_note}{host_note}{table_note})");
    0
}

// ── where ───────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct WhereJson {
    pub(crate) effect: String,
    pub(crate) directly: Vec<String>,
    pub(crate) inherited: Vec<String>,
    /// ⟨0.28⟩ the incompleteness disclosure, INLINE and LAST — flattened rather than attached to a
    /// `serde_json::Value`, because `to_value` sorts and would re-order `effect`/`directly`/`inherited`
    /// on every ordinary run. See [`crate::completeness::ReportCompleteness::fields`].
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub(crate) completeness: Option<crate::completeness::CompletenessFields>,
}

pub(crate) fn cmd_where(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 1, sentinel: true, has_policy: false });
    let Some(eff) = g.positional.first().map(String::as_str) else {
        eprintln!("usage: candor-query where <Effect> [--report <locator>] [--json]");
        return 2;
    };
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let (pre, want_json) = (pre.as_str(), g.want_json);
    let all = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    // A typo'd/unknown effect NAME is a LOUD error (exit 2) — never a false-empty 0-result at exit 0 that
    // reads as an authoritative "nothing performs Net" when the user actually typed "Network" (corpus-audit
    // #3). A KNOWN effect that is simply absent stays a valid 0-result; an unknown name PRESENT in the report
    // (a spec extension effect) is allowed — so error only when the name is NEITHER known nor present.
    const KNOWN_EFFECTS: &[&str] =
        &["Net", "Fs", "Db", "Llm", "Exec", "Env", "Clock", "Ipc", "Log", "Rand", "Clipboard", "Unknown"];
    if !KNOWN_EFFECTS.contains(&eff) && !all.iter().any(|e| e.inferred.iter().any(|x| x == eff)) {
        eprintln!("candor-query where: unknown effect '{eff}' (known: {})", KNOWN_EFFECTS.join(", "));
        return 2;
    }
    let mut direct: Vec<String> =
        all.iter().filter(|e| e.direct.iter().any(|x| x == eff)).map(|e| e.func.clone()).collect();
    let mut inherit: Vec<String> = all
        .iter()
        .filter(|e| e.inferred.iter().any(|x| x == eff) && !e.direct.iter().any(|x| x == eff))
        .map(|e| e.func.clone())
        .collect();
    direct.sort();
    inherit.sort();

    // ⟨0.28⟩ SPEC §2: the re-disclosure binds *any* verb whose output could read as a negative finding,
    // and `{"directly":[],"inherited":[]}` is one of the four the clause names by measurement. See
    // [`crate::completeness`] — same reader, same two channels, no-op on a complete report.
    let comp = crate::completeness::report_completeness(pre);
    comp.warn_unreadable("where");

    if want_json {
        let out = WhereJson {
            effect: eff.to_string(),
            directly: direct,
            inherited: inherit,
            completeness: comp.fields(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    // BEFORE the answer, not after: it qualifies a NON-empty list as much as an empty one. A function in
    // an unread file performs `eff` or not, and neither list below can say which.
    comp.print_note(
        &format!("the function(s) named below are only those candor could see perform {eff}"),
        &format!(
            "A function in one of those is ABSENT from the report, so it cannot appear in either \
             list. {} Re-scan for a complete answer.",
            comp.gate_line()
        ),
    );
    if direct.is_empty() && inherit.is_empty() {
        if comp.must_hedge() {
            // NOT "no function performs {eff}". That sentence is the prose spelling of the empty JSON
            // pair, and over these bytes candor has not examined enough to say it.
            println!(
                "candor: no function candor COULD SEE performs {eff} — but see the INCOMPLETE note \
                 above; this is NOT \"nothing performs {eff}\"."
            );
            return 0;
        }
        println!("candor: no function performs {eff} in the report.");
        return 0;
    }
    println!("{} function(s) perform {eff}:", direct.len() + inherit.len());
    if !direct.is_empty() {
        println!("  directly ({}):", direct.len());
        for fn_ in &direct {
            println!("    {fn_}");
        }
    }
    if !inherit.is_empty() {
        println!("  inherit it via a callee ({}):", inherit.len());
        for fn_ in &inherit {
            println!("    {fn_}");
        }
    }
    0
}

// ── map ─────────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct MapJson {
    pub(crate) effects: Vec<String>,
    pub(crate) functions: usize,
}

pub(crate) fn cmd_map(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 0, sentinel: true, has_policy: false });
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let (pre, want_json) = (pre.as_str(), g.want_json);
    let entries = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
    // module -> (effects, count). Module = fn name with a leading '<' stripped, up to the first '::'.
    let mut mods: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
    for e in entries {
        // Module = the first path component. For a qualified trait-impl path `<Type as Trait>::m`,
        // that's `Type` — stop at the first of ` as `, `>`, or `::` (whichever comes first) so the
        // bucket isn't the malformed `Type as Trait>`.
        let stripped = e.func.strip_prefix('<').unwrap_or(&e.func);
        // `::` (Rust) or `.` (JVM/TS/Swift/fleet reports read by this same binary): the LAST dot
        // bounds the module for dotted names (`src.db.save` -> `src.db`; `Statement.execute` ->
        // `Statement`), the FIRST `::` for Rust paths — found by the Swift interop probe, where
        // map lumped 731 dotted functions into `(root)`.
        let end = if stripped.contains("::") {
            [stripped.find(" as "), stripped.find('>'), stripped.find("::")]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(stripped.len())
        } else {
            stripped.rfind('.').unwrap_or(stripped.len())
        };
        // A name with NO module separator is a crate-root free function: it buckets into `(root)`,
        // NOT its own one-function pseudo-module (SPEC §6.1 — matches the containment layer rule and
        // the JVM engine, which groups root methods under their class). Without this a flat crate of
        // free functions showed every function as its own "module" — a useless overview.
        let m = if end == stripped.len() {
            "(root)".to_string()
        } else {
            match stripped[..end].trim() {
                "" => "(root)".to_string(),
                s => s.to_string(),
            }
        };
        let v = mods.entry(m).or_default();
        v.0.extend(e.inferred.iter().filter(|x| *x != "Unknown").cloned());
        v.1 += 1;
    }
    // ⟨0.28⟩ `map` answers `{}` over a report that judged nothing, and SPEC §2 names `{}` the STRONGEST
    // determined negative there is: every key a consumer reads defaults to empty, so `d.get("db", {})`
    // cannot tell an empty map from an unexamined one. See [`crate::completeness`].
    let comp = crate::completeness::report_completeness(pre);
    comp.warn_unreadable("map");

    if want_json {
        let out: BTreeMap<String, MapJson> = mods
            .iter()
            .map(|(m, (eff, n))| (m.clone(), MapJson { effects: eff.iter().cloned().collect(), functions: *n }))
            .collect();
        // THE ONE DOCUMENT WHOSE TOP LEVEL IS A USER NAMESPACE, so the disclosure keys can in principle
        // land on a real module (`mod incomplete`). `write_json` OVERWRITES, and a module silently
        // replaced by a hedge is a dropped row — the shape this whole rung exists to remove. It cannot
        // be dodged by nesting: the disclosure has to be a TOP-LEVEL key or a consumer branching on
        // `"incomplete" in doc` never sees it. So the collision is DISCLOSED instead of hidden, loudly
        // and by name, and the hedge still wins — a lost module row the operator has been told about
        // beats a false all-clear nobody has.
        let mut out = serde_json::to_value(out).unwrap();
        // Asked of the fields ACTUALLY about to be written, not of a hardcoded name list: `unanalyzed`
        // and `judgedNothing` are each omitted when empty, and warning that a module named `unanalyzed`
        // was displaced when nothing displaced it is a false disclosure — the failure mode `net-partner`
        // taught this family (a key reported ignored while being honoured), pointed the other way.
        let written = comp.fields().map(|f| serde_json::to_value(f).unwrap()).unwrap_or_default();
        for k in written.as_object().into_iter().flat_map(|o| o.keys()) {
            if out.get(k).is_some() {
                eprintln!(
                    "candor map: this report has a module literally named `{k}`, which collides with \
                     the ⟨0.28⟩ incompleteness disclosure this answer must carry — the disclosure wins \
                     and that module's row is NOT in the JSON below. Its effects are in the text \
                     output (drop --json)."
                );
            }
        }
        comp.write_json(&mut out);
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    comp.print_note(
        "the module rows below cover only the source candor read",
        &format!(
            "A module living wholly in one of those is MISSING from the overview, and one that is \
             listed may be missing functions. {} Re-scan for a complete map.",
            comp.gate_line()
        ),
    );
    if mods.is_empty() {
        if comp.must_hedge() {
            println!(
                "candor: no effectful function candor COULD SEE — but see the INCOMPLETE note above; \
                 this is NOT \"the code performs no effects\"."
            );
            return 0;
        }
        println!("candor: no effectful functions in the report.");
        return 0;
    }
    let total: usize = mods.values().map(|(_, n)| *n).sum();
    println!("candor map — {total} effectful functions across {} module(s)", mods.len());
    println!();
    let w = mods.keys().map(|m| m.chars().count()).max().unwrap_or(0);
    // Order: most functions first, then name (matches Python's key=lambda m: (-n, m)).
    let mut order: Vec<&String> = mods.keys().collect();
    order.sort_by(|a, b| {
        let (na, nb) = (mods[*a].1, mods[*b].1);
        nb.cmp(&na).then_with(|| a.cmp(b))
    });
    for m in order {
        let (eff, n) = &mods[m];
        let effs: Vec<String> = eff.iter().cloned().collect();
        let s = if *n != 1 { "s" } else { "" };
        println!("  {:<w$}  {{ {} }}  ({} fn{})", m, effs.join(" "), n, s, w = w);
    }
    0
}
