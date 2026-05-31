//! candor-query — the read-only report queries (show / where / callers / map / diff) that used to be
//! inline Python heredocs in `cargo-candor`. One typed binary over the shared `candor-report` types,
//! so the JSON shape is defined once (in the lint and here) instead of re-parsed ad hoc in every
//! script. The CLI keeps the *exact* argv convention and output of the Python it replaces, so the
//! bash wrapper only swaps `python3 - … <<'PY'` for `candor-query …`; everything downstream (the
//! integration tests, the MCP server, the agent) sees identical bytes.
//!
//! Usage (positional, mirroring the old `sys.argv`):
//!   candor-query show    <prefix> <query>  <0|1>
//!   candor-query where   <prefix> <effect> <0|1>
//!   candor-query callers <prefix> <query>  <0|1>
//!   candor-query map     <prefix>          <0|1>
//!   candor-query diff    <cur_prefix> <base_prefix> <0|1> <baseline_ver> <engine_ver>
//! The trailing 0|1 is the want-JSON flag (the wrapper computes it from `--json`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use candor_report::{report_entries, ReportEntry};
use regex::Regex;
use serde::Serialize;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    let code = match cmd {
        "audit" => cmd_audit(rest),
        "show" => cmd_show(rest),
        "where" => cmd_where(rest),
        "callers" => cmd_callers(rest),
        "map" => cmd_map(rest),
        "diff" => cmd_diff(rest),
        other => {
            eprintln!("candor-query: unknown command '{other}' (audit|show|where|callers|map|diff)");
            2
        }
    };
    std::process::exit(code);
}

// ── report loading ──────────────────────────────────────────────────────────────────────────────

/// Files matching the Python glob `pre + '.*.*.json'`: same directory, basename `<base>.`, ending
/// `.json`, with at least the two `.`-separated wildcard segments between (≥3 dots in the suffix).
/// Sorted for deterministic output. A directoryless prefix reads the current dir.
fn glob_reports(prefix: &str) -> Vec<PathBuf> {
    let p = Path::new(prefix);
    let dir = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = match p.file_name().and_then(|s| s.to_str()) {
        Some(b) => b,
        None => return Vec::new(),
    };
    let needle = format!("{base}.");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with(&needle) && name.ends_with(".json") {
                let suffix = &name[base.len()..]; // starts with '.'
                if suffix.matches('.').count() >= 3 {
                    out.push(ent.path());
                }
            }
        }
    }
    out.sort();
    out
}

/// All report entries across the matching files (skipping unreadable / unparsable ones, like Python).
fn load_entries(prefix: &str) -> Vec<ReportEntry> {
    let mut out = Vec::new();
    for path in glob_reports(prefix) {
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if let Some(es) = report_entries(&text) {
            out.extend(es);
        }
    }
    out
}

// ── audit ───────────────────────────────────────────────────────────────────────────────────────

/// The basename of a prefix (`.candor/report` → `report`), used to label per-crate report files.
fn prefix_base(prefix: &str) -> String {
    Path::new(prefix).file_name().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

/// Encountered-crate sidecars: `<base>.encountered-*.json` (not matched by the `.*.*.json` report
/// glob — only two dot-segments). Each holds a JSON array of crate names candor *saw called*.
fn glob_encountered(prefix: &str) -> Vec<PathBuf> {
    let dir = match Path::new(prefix).parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = prefix_base(prefix);
    let needle = format!("{base}.encountered-");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            if let Some(n) = ent.file_name().to_str() {
                if n.starts_with(&needle) && n.ends_with(".json") {
                    out.push(ent.path());
                }
            }
        }
    }
    out
}

/// `cargo candor audit` default view: an at-a-glance effect profile aggregated across the crates.
/// Args: `<prefix> <engine_ver> <suspect_file>`. Always exits 0 (a report renderer).
fn cmd_audit(args: &[String]) -> i32 {
    let (pre, ver, suspect_path) = match args {
        [a, b, c, ..] => (a.as_str(), b.as_str(), c.as_str()),
        _ => {
            eprintln!("usage: candor-query audit <prefix> <engine_ver> <suspect_file>");
            return 2;
        }
    };
    let base = prefix_base(pre);

    // entries + per-crate counts, in sorted-glob order.
    let mut fns: Vec<ReportEntry> = Vec::new();
    let mut percrate: Vec<(String, usize)> = Vec::new();
    for path in glob_reports(pre) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            percrate.push((String::new(), 0));
            continue;
        };
        let es = report_entries(&text).unwrap_or_default();
        // label = filename with the `<base>.` prefix and `.json` suffix stripped → `<crate>.<type>`.
        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let label = fname
            .strip_prefix(&format!("{base}."))
            .and_then(|s| s.strip_suffix(".json"))
            .unwrap_or("")
            .to_string();
        percrate.push((label, es.len()));
        fns.extend(es);
    }

    if fns.is_empty() {
        println!("candor: no effectful functions found (everything candor can see is pure).");
        return 0;
    }

    const EFFS: [&str; 10] =
        ["Net", "Db", "Fs", "Exec", "Ipc", "Env", "Clock", "Rand", "Clipboard", "Log"];
    let mut tally: BTreeMap<&str, usize> = EFFS.iter().map(|e| (*e, 0)).collect();
    let mut unresolved: Vec<String> = Vec::new();
    for e in &fns {
        for x in &e.inferred {
            if let Some(n) = tally.get_mut(x.as_str()) {
                *n += 1;
            }
        }
        if e.unresolved || e.inferred.iter().any(|x| x == "Unknown") {
            unresolved.push(e.func.clone());
        }
    }

    println!("candor @{ver}");
    let pc = percrate.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(" · ");
    println!("{} effectful functions  ·  {}", fns.len(), pc);
    println!();
    // ranked: effects with count>0, by (count desc, name desc) — matches Python's reverse sort of (n,e).
    let mut ranked: Vec<(usize, &str)> = EFFS.iter().filter(|e| tally[**e] > 0).map(|e| (tally[*e], *e)).collect();
    ranked.sort_by(|a, b| b.cmp(a));
    let eff_line = ranked.iter().map(|(n, e)| format!("{n} {e}")).collect::<Vec<_>>().join(" · ");
    println!("  effects   {eff_line}");
    println!();

    if !unresolved.is_empty() {
        let u: Vec<String> = unresolved.iter().cloned().collect::<BTreeSet<_>>().into_iter().collect();
        let extra = if u.len() > 5 { format!("  (+{} more)", u.len() - 5) } else { String::new() };
        println!("  ⚠ {} make calls candor can't resolve — their effect set may be incomplete (read these):", u.len());
        println!("      {}{extra}", u.iter().take(5).cloned().collect::<Vec<_>>().join(", "));
        println!();
    }

    // coverage: external crates candor saw called but does not calibrate, that LOOK effectful.
    let (calib_c, calib_p) = load_calibrated(pre, &base);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in glob_encountered(pre) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(arr) = serde_json::from_str::<Vec<String>>(&text) {
                seen.extend(arr);
            }
        }
    }
    let suspect = std::fs::read_to_string(suspect_path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let suspect = suspect.and_then(|s| Regex::new(&s).ok());
    let calibrated = |c: &str| -> bool {
        calib_c.contains(c)
            || calib_c.iter().any(|k| c == k || c.starts_with(&format!("{k}_")))
            || calib_p.iter().any(|p| c.starts_with(p))
    };
    if let Some(re) = &suspect {
        let gaps: Vec<String> = seen.iter().filter(|c| re.is_match(c) && !calibrated(c)).cloned().collect();
        if !gaps.is_empty() {
            println!("  ⚠ coverage: {} uncalibrated — effects through them may be under-counted", gaps.join(", "));
            println!();
        }
    }

    // the functions with the widest reach into the outside world.
    let width = |e: &ReportEntry| e.inferred.iter().filter(|x| *x != "Unknown").count();
    let mut top: Vec<&ReportEntry> = fns.iter().filter(|e| width(e) > 0).collect();
    top.sort_by(|a, b| width(b).cmp(&width(a)).then_with(|| a.func.cmp(&b.func)));
    top.truncate(8);
    if !top.is_empty() {
        let w = top.iter().map(|e| e.func.chars().count()).max().unwrap_or(0);
        println!("  broadest effect surface");
        for e in &top {
            let effs: Vec<String> = sorted(&e.inferred).into_iter().filter(|x| x != "Unknown").collect();
            println!("    {:<w$}  {{ {} }}", e.func, effs.join(" "), w = w);
        }
        println!();
    }

    println!("  full per-function view:  cargo candor audit --all");
    println!("  guard against new effects:  cargo candor snapshot .candor/baseline");
    0
}

/// The calibrated-coverage sidecar `<dir>/<base>.calibrated.json` → (crates, prefixes).
fn load_calibrated(prefix: &str, base: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let dir = Path::new(prefix).parent().filter(|d| !d.as_os_str().is_empty()).map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let path = dir.join(format!("{base}.calibrated.json"));
    let Ok(text) = std::fs::read_to_string(&path) else { return Default::default() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Default::default() };
    let pick = |key: &str| -> BTreeSet<String> {
        v.get(key)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default()
    };
    (pick("crates"), pick("prefixes"))
}

// ── show ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ShowJson {
    #[serde(rename = "fn")]
    func: String,
    inferred: Vec<String>,
    direct: Vec<String>,
    /// Fs read/write detail, omitted when absent — see `ReportEntry::fs`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fs: Vec<String>,
    unresolved: bool,
}

fn cmd_show(args: &[String]) -> i32 {
    let (pre, q, want_json) = match three(args) {
        Some(t) => t,
        None => {
            eprintln!("usage: candor-query show <prefix> <query> <0|1>");
            return 2;
        }
    };
    let mut fns: Vec<ReportEntry> = load_entries(pre).into_iter().filter(|e| e.func.contains(q)).collect();
    fns.sort_by(|a, b| a.func.cmp(&b.func));

    if want_json {
        let out: Vec<ShowJson> = fns
            .iter()
            .map(|e| ShowJson {
                func: e.func.clone(),
                inferred: sorted(&e.inferred),
                direct: sorted(&e.direct),
                fs: e.fs.clone(),
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
    for e in &fns {
        let direct: BTreeSet<&String> = e.direct.iter().collect();
        let parts: Vec<String> = sorted(&e.inferred)
            .into_iter()
            .map(|x| {
                let star = if direct.contains(&x) { "*" } else { "" };
                // Refine Fs with its read/write detail when known: `Fs*(write)`.
                if x == "Fs" && !e.fs.is_empty() {
                    format!("Fs{star}({})", e.fs.join(","))
                } else {
                    format!("{x}{star}")
                }
            })
            .collect();
        let unk = if e.unresolved { "  ⚠ unresolved (set may be incomplete)" } else { "" };
        println!("  {:<w$}  {{ {} }}{}", e.func, parts.join(" "), unk, w = w);
    }
    let fs_note = if any_fs { ";  Fs(read/write) = the filesystem access seen" } else { "" };
    println!("  (* = performed in the function's own body; unmarked = via a callee{fs_note})");
    0
}

// ── where ───────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct WhereJson {
    effect: String,
    directly: Vec<String>,
    inherited: Vec<String>,
}

fn cmd_where(args: &[String]) -> i32 {
    let (pre, eff, want_json) = match three(args) {
        Some(t) => t,
        None => {
            eprintln!("usage: candor-query where <prefix> <Effect> <0|1>");
            return 2;
        }
    };
    let all = load_entries(pre);
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

// ── callers ─────────────────────────────────────────────────────────────────────────────────────

fn cmd_callers(args: &[String]) -> i32 {
    let (pre, q, want_json) = match three(args) {
        Some(t) => t,
        None => {
            eprintln!("usage: candor-query callers <prefix> <query> <0|1>");
            return 2;
        }
    };
    // callee (matching q) -> set of its callers
    let mut hits: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in load_entries(pre) {
        for callee in &e.calls {
            if callee.contains(q) {
                hits.entry(callee.clone()).or_default().insert(e.func.clone());
            }
        }
    }
    if want_json {
        let out: BTreeMap<String, Vec<String>> =
            hits.iter().map(|(c, cs)| (c.clone(), cs.iter().cloned().collect())).collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }
    if hits.is_empty() {
        println!("candor: nothing matching `{q}` is called by an effectful function (callers of pure functions aren't tracked).");
        return 0;
    }
    for (callee, cs) in &hits {
        println!("  {callee}  ← called by {}:", cs.len());
        for c in cs {
            println!("      {c}");
        }
    }
    0
}

// ── map ─────────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct MapJson {
    effects: Vec<String>,
    functions: usize,
}

fn cmd_map(args: &[String]) -> i32 {
    let (pre, want_json) = match two(args) {
        Some(t) => t,
        None => {
            eprintln!("usage: candor-query map <prefix> <0|1>");
            return 2;
        }
    };
    // module -> (effects, count). Module = fn name with a leading '<' stripped, up to the first '::'.
    let mut mods: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
    for e in load_entries(pre) {
        let stripped = e.func.strip_prefix('<').unwrap_or(&e.func);
        let m = stripped.split("::").next().filter(|s| !s.is_empty()).unwrap_or("(root)").to_string();
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

// ── diff ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct FnInfo {
    inferred: BTreeSet<String>,
    direct: BTreeSet<String>,
    calls: BTreeSet<String>,
}

/// fn -> its effect info, last write wins (mirrors Python's `out[e['fn']] = …`).
fn load_fninfo(prefix: &str) -> BTreeMap<String, FnInfo> {
    let mut out = BTreeMap::new();
    for e in load_entries(prefix) {
        out.insert(
            e.func.clone(),
            FnInfo {
                inferred: e.inferred.into_iter().collect(),
                direct: e.direct.into_iter().collect(),
                calls: e.calls.into_iter().collect(),
            },
        );
    }
    out
}

#[derive(Serialize, Clone)]
struct Change {
    #[serde(rename = "fn")]
    func: String,
    gained: Vec<String>,
    introduced: Vec<String>,
    inherited: Vec<String>,
    lost: Vec<String>,
    status: String,
}

#[derive(Serialize)]
struct DiffJson<'a> {
    baseline_version: &'a str,
    engine_version: &'a str,
    changes: Vec<Change>,
}

fn cmd_diff(args: &[String]) -> i32 {
    // diff <cur_pre> <base_pre> <0|1> <bver> <ever>
    if args.len() < 5 {
        eprintln!("usage: candor-query diff <cur_prefix> <base_prefix> <0|1> <baseline_ver> <engine_ver>");
        return 2;
    }
    let (cur_pre, base_pre, want_json, bver, ever) =
        (&args[0], &args[1], args[2] == "1", args[3].as_str(), args[4].as_str());

    let cur = load_fninfo(cur_pre);
    let base = load_fninfo(base_pre);
    let empty = BTreeSet::new();

    let mut changes: Vec<Change> = Vec::new();
    let keys: BTreeSet<&String> = cur.keys().chain(base.keys()).collect();
    for fn_ in keys {
        let ci = cur.get(fn_).map(|v| &v.inferred).unwrap_or(&empty);
        let bi = base.get(fn_).map(|v| &v.inferred).unwrap_or(&empty);
        if !cur.contains_key(fn_) {
            // function gone (was in baseline)
            if !bi.is_empty() {
                changes.push(Change {
                    func: fn_.clone(),
                    gained: vec![],
                    introduced: vec![],
                    inherited: vec![],
                    lost: bi.iter().cloned().collect(),
                    status: "removed".into(),
                });
            }
            continue;
        }
        let gained: Vec<String> = ci.difference(bi).cloned().collect();
        let lost: Vec<String> =
            if base.contains_key(fn_) { bi.difference(ci).cloned().collect() } else { vec![] };
        if gained.is_empty() && lost.is_empty() {
            continue;
        }
        // A gained effect is INTRODUCED here if it's in this function's own `direct` set; otherwise
        // it's INHERITED from a callee — the source vs. the blast radius.
        let cd = &cur[fn_].direct;
        let introduced: Vec<String> = gained.iter().filter(|e| cd.contains(*e)).cloned().collect();
        let inherited: Vec<String> = gained.iter().filter(|e| !cd.contains(*e)).cloned().collect();
        changes.push(Change {
            func: fn_.clone(),
            gained,
            introduced,
            inherited,
            lost,
            status: if base.contains_key(fn_) { "changed".into() } else { "new".into() },
        });
    }

    if want_json {
        let mut sorted_changes = changes.clone();
        sorted_changes.sort_by(|a, b| a.func.cmp(&b.func));
        let out = DiffJson { baseline_version: bver, engine_version: ever, changes: sorted_changes };
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return 0;
    }

    if changes.is_empty() {
        println!("candor: no effect changes vs {base_pre} (@{}).", q_or(bver));
        return 0;
    }

    println!("candor diff — current (@{ever}) vs {base_pre} (@{})", q_or(bver));
    if !bver.is_empty() && !ever.is_empty() && bver != ever {
        println!("  ⚠ baseline @{bver} ≠ engine @{ever} — some changes may be the engine reclassifying, not your code.");
    }
    println!();

    // Selectivity (§9): a gained effect SURFACES at its top-level gainers — those not called by any
    // other gainer (the entry point / public API). The chain between source and there is plumbing.
    let calls_of: BTreeMap<&String, &BTreeSet<String>> =
        cur.iter().map(|(fn_, v)| (fn_, &v.calls)).collect();
    let sources: BTreeSet<String> =
        changes.iter().filter(|c| !c.introduced.is_empty()).map(|c| c.func.clone()).collect();

    let all_effects: BTreeSet<String> = changes.iter().flat_map(|c| c.gained.iter().cloned()).collect();
    let mut top_level: BTreeSet<String> = BTreeSet::new();
    let mut endpoints_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &all_effects {
        let gainers: BTreeSet<String> =
            changes.iter().filter(|c| c.gained.contains(e)).map(|c| c.func.clone()).collect();
        let mut called: BTreeSet<String> = BTreeSet::new();
        for g in &gainers {
            if let Some(cs) = calls_of.get(g) {
                called.extend(cs.intersection(&gainers).cloned());
            }
        }
        let tl: BTreeSet<String> = gainers.difference(&called).cloned().collect();
        top_level.extend(tl.iter().cloned());
        endpoints_of.insert(e.clone(), tl.difference(&sources).cloned().collect());
    }
    let consequential: BTreeSet<String> = sources.union(&top_level).cloned().collect();

    // Lead with sources, then top-level endpoints, then other gainers, then pure-lost — name within.
    changes.sort_by(|a, b| rank(a, &top_level).cmp(&rank(b, &top_level)).then_with(|| a.func.cmp(&b.func)));

    let mut shown: BTreeSet<String> = BTreeSet::new();
    for c in &changes {
        if !consequential.contains(&c.func) && c.lost.is_empty() {
            continue; // pure intermediate plumbing — collapsed below
        }
        shown.insert(c.func.clone());
        let mut parts: Vec<String> = Vec::new();
        parts.extend(c.introduced.iter().map(|e| format!("+{e}*")));
        parts.extend(c.inherited.iter().map(|e| format!("+{e}")));
        parts.extend(c.lost.iter().map(|e| format!("-{e}")));
        let mark = if !c.gained.is_empty() { "+" } else { "-" };
        let mut tags: Vec<&str> = Vec::new();
        if c.status == "new" {
            tags.push("new fn");
        }
        if c.status == "removed" {
            tags.push("removed fn");
        }
        if top_level.contains(&c.func) && !sources.contains(&c.func) {
            tags.push("top-level");
        }
        if c.gained.iter().any(|e| e == "Unknown") {
            tags.push("⚠ now unresolvable");
        }
        let tag = if tags.is_empty() { String::new() } else { format!("  ({})", tags.join(", ")) };
        println!("  {mark} {}{tag}   {{ {} }}", c.func, parts.join(" "));
    }
    let mut hidden: Vec<String> =
        changes.iter().filter(|c| !shown.contains(&c.func)).map(|c| c.func.clone()).collect();
    hidden.sort();
    if !hidden.is_empty() {
        let head: Vec<String> = hidden.iter().take(4).cloned().collect();
        let names = if hidden.len() > 4 {
            format!("{}, +{} more", head.join(", "), hidden.len() - 4)
        } else {
            head.join(", ")
        };
        println!("  … {} intermediate caller(s) also inherit it: {names}", hidden.len());
    }
    println!();
    println!("  * = introduced here;  (top-level) = where the effect surfaces (an entry point / public API).");
    for e in &all_effects {
        let mut srcs: Vec<String> =
            changes.iter().filter(|c| c.introduced.contains(e)).map(|c| c.func.clone()).collect();
        srcs.sort();
        let gainers: BTreeSet<String> =
            changes.iter().filter(|c| c.gained.contains(e)).map(|c| c.func.clone()).collect();
        let eps = endpoints_of.get(e).cloned().unwrap_or_default();
        if !srcs.is_empty() {
            let reach = if !eps.is_empty() { format!(" → reaches {}", eps.join(", ")) } else { String::new() };
            let extra = gainers.len() as i64 - srcs.len() as i64 - eps.len() as i64;
            let tail = if extra > 0 {
                format!("  (+{extra} intermediate)")
            } else if eps.is_empty() {
                "  (stays local)".to_string()
            } else {
                String::new()
            };
            println!("  {e}: introduced in {}{reach}{tail}", srcs.join(", "));
        } else if !gainers.is_empty() {
            let names = if !eps.is_empty() { eps.join(", ") } else { gainers.iter().cloned().collect::<Vec<_>>().join(", ") };
            println!("  {e}: reaches {names} (source outside this crate/baseline)");
        }
    }
    if changes.iter().any(|c| c.gained.iter().any(|e| e == "Unknown")) {
        println!("  ⚠ a new Unknown means candor can no longer prove that function's effect set is complete — review it.");
    }
    0
}

/// Sort key for the human diff listing: sources first, then top-level endpoints, then other gainers,
/// then pure-lost (mirrors the Python lambda).
fn rank(c: &Change, top_level: &BTreeSet<String>) -> u8 {
    if !c.introduced.is_empty() {
        0
    } else if top_level.contains(&c.func) {
        1
    } else if !c.gained.is_empty() {
        2
    } else {
        3
    }
}

// ── small helpers ───────────────────────────────────────────────────────────────────────────────

fn sorted(v: &[String]) -> Vec<String> {
    let mut out = v.to_vec();
    out.sort();
    out
}

fn q_or(s: &str) -> &str {
    if s.is_empty() { "?" } else { s }
}

/// (prefix, query, want_json) from `[prefix, query, 0|1]`.
fn three(args: &[String]) -> Option<(&str, &str, bool)> {
    match args {
        [a, b, c, ..] => Some((a.as_str(), b.as_str(), c == "1")),
        _ => None,
    }
}

/// (prefix, want_json) from `[prefix, 0|1]`.
fn two(args: &[String]) -> Option<(&str, bool)> {
    match args {
        [a, b, ..] => Some((a.as_str(), b == "1")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report glob must pick up `<base>.<crate>.<type>.json` (the `.*.*.json` shape) but NOT the
    /// `<base>.calibrated.json` / `<base>.encountered-*.json` sidecars (only two dot-segments) — and
    /// `glob_encountered` must do the reverse. Getting this wrong folds coverage data into entries.
    #[test]
    fn globs_discriminate_reports_from_sidecars() {
        let dir = std::env::temp_dir().join("candor-query-glob-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "report.mycrate.lib.json",       // report ✓
            "report.mycrate.Executable.json", // report ✓
            "report.calibrated.json",        // sidecar ✗ (2 dots)
            "report.encountered-mycrate.json", // sidecar ✗ for reports, ✓ for encountered
            "report.single.json",            // ✗ (2 dots)
            "other.a.b.json",                // different base ✗
        ] {
            std::fs::write(dir.join(f), "[]").unwrap();
        }
        let prefix = dir.join("report");
        let prefix = prefix.to_str().unwrap();

        let reports: Vec<String> =
            glob_reports(prefix).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(reports, vec!["report.mycrate.Executable.json", "report.mycrate.lib.json"]);

        let enc: Vec<String> =
            glob_encountered(prefix).iter().map(|p| p.file_name().unwrap().to_str().unwrap().to_string()).collect();
        assert_eq!(enc, vec!["report.encountered-mycrate.json"]);

        assert_eq!(prefix_base(prefix), "report");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
