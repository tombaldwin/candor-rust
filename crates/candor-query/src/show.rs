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
    let mut direct: Vec<String> =
        all.iter().filter(|e| e.direct.iter().any(|x| x == eff)).map(|e| e.func.clone()).collect();
    let mut inherit: Vec<String> = all
        .iter()
        .filter(|e| e.inferred.iter().any(|x| x == eff) && !e.direct.iter().any(|x| x == eff))
        .map(|e| e.func.clone())
        .collect();
    direct.sort();
    inherit.sort();

    if want_json {
        let out = WhereJson { effect: eff.to_string(), directly: direct, inherited: inherit };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    if direct.is_empty() && inherit.is_empty() {
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
    if want_json {
        let out: BTreeMap<String, MapJson> = mods
            .iter()
            .map(|(m, (eff, n))| (m.clone(), MapJson { effects: eff.iter().cloned().collect(), functions: *n }))
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    if mods.is_empty() {
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
