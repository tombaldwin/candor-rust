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

use candor_report::{report_entries, report_files, ReportEntry, EFFECTS};
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
        "containment" => cmd_containment(rest),
        "receipt" => cmd_receipt(rest),
        "gains" => cmd_gains(rest),
        "state" => cmd_state(rest),
        "reports" => cmd_reports(rest),
        "locate" => cmd_locate(rest),
        "engine-version" => cmd_engine_version(rest),
        "merge-hook" => cmd_merge_hook(rest),
        other => {
            eprintln!(
                "candor-query: unknown command '{other}' \
                 (audit|show|where|callers|map|diff|containment|receipt|gains|state|reports|locate|engine-version|merge-hook)"
            );
            2
        }
    };
    std::process::exit(code);
}

// ── report loading ──────────────────────────────────────────────────────────────────────────────

/// Report file PATHS for a prefix — the `<base>.<crate>.<type>.json` reports, sorted, excluding the
/// `.calibrated`/`.encountered-*` sidecars. Thin wrapper over `candor_report::report_files`, which
/// owns the ONE discrimination rule shared with the lint (so the two can't disagree).
fn glob_reports(prefix: &str) -> Vec<PathBuf> {
    report_files(prefix).into_iter().map(|r| r.path).collect()
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
                if let Some(mid) = n.strip_prefix(&needle).and_then(|m| m.strip_suffix(".json")) {
                    // an encountered sidecar is `<base>.encountered-<crate>-<kind>.json` — a SINGLE
                    // dotless segment. Excluding any further dot avoids mis-claiming the REPORT of a
                    // crate literally named `encountered-…` (`<base>.encountered-foo.lib.json`).
                    if !mid.contains('.') {
                        out.push(ent.path());
                    }
                }
            }
        }
    }
    out
}

/// Per-effect counts (only the `EFFECTS` vocabulary — `Unknown` is excluded) plus the names of the
/// functions whose effect set may be incomplete (`unresolved`, or containing `Unknown`). Shared by the
/// `audit` and `receipt` views so the tally + unresolved rule are defined exactly once.
fn tally_effects(fns: &[ReportEntry]) -> (BTreeMap<&'static str, usize>, Vec<String>) {
    let mut tally: BTreeMap<&'static str, usize> = EFFECTS.iter().map(|e| (*e, 0)).collect();
    let mut unresolved: Vec<String> = Vec::new();
    for e in fns {
        for x in &e.inferred {
            if let Some(n) = tally.get_mut(x.as_str()) {
                *n += 1;
            }
        }
        if e.unresolved || e.inferred.iter().any(|x| x == "Unknown") {
            unresolved.push(e.func.clone());
        }
    }
    (tally, unresolved)
}

/// The external crates candor saw resolved calls into — the union of the `<prefix>.encountered-*.json`
/// sidecars. Shared by `audit` (coverage gaps) and `receipt`.
fn encountered_set(prefix: &str) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in glob_encountered(prefix) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(arr) = serde_json::from_str::<Vec<String>>(&text) {
                seen.extend(arr);
            }
        }
    }
    seen
}

/// THE definition of a "gained" effect: present in `cur`'s set, absent from `base`'s. Shared by `diff`
/// (per-function gained list) and `gains` (the self-review pairs) so they can't drift.
fn gained_effects(cur: &BTreeSet<String>, base: &BTreeSet<String>) -> Vec<String> {
    cur.difference(base).cloned().collect()
}

/// `cargo candor audit` default view: an at-a-glance effect profile aggregated across the crates.
/// Args: `<prefix> <engine_ver> <suspect_file>`. Always exits 0 (a report renderer).
fn cmd_audit(args: &[String]) -> i32 {
    let (pre, ver, suspect_path) = match args {
        [a, b, c, ..] => (a.as_str(), b.as_str(), c.as_str()),
        _ => {
            eprintln!("usage: candor-query audit <prefix> <engine_ver> <suspect_file> [--coverage]");
            return 2;
        }
    };
    // `--coverage`: list every external crate candor has no effect rules for (the full blind-spot
    // surface), not just the name-heuristic suspects.
    let coverage = args.iter().any(|a| a == "--coverage" || a == "-c");
    let base = prefix_base(pre);

    // entries + per-crate counts, in sorted order. The `<crate>.<type>` label is taken from the
    // filename (via report_files) regardless of readability, so an unreadable report still shows its
    // label with a count of 0 (matching the Python this replaced).
    let mut fns: Vec<ReportEntry> = Vec::new();
    let mut percrate: Vec<(String, usize)> = Vec::new();
    // Crates that have their OWN report here are analyzed via cross-crate propagation (their effects
    // are inherited, not guessed) — so they're NOT classifier blind spots, even though the classifier
    // has no rule for them. (E.g. a workspace sibling like `candor_report`.)
    let mut analyzed: BTreeSet<String> = BTreeSet::new();
    for rf in report_files(pre) {
        let label = format!("{}.{}", rf.krate, rf.kind);
        analyzed.insert(rf.krate.clone());
        let es = std::fs::read_to_string(&rf.path).ok().and_then(|t| report_entries(&t)).unwrap_or_default();
        percrate.push((label, es.len()));
        fns.extend(es);
    }

    if fns.is_empty() {
        println!("candor: no effectful functions found (everything candor can see is pure).");
        return 0;
    }

    let (tally, unresolved) = tally_effects(&fns);

    println!("candor @{ver}");
    let pc = percrate.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(" · ");
    println!("{} effectful functions  ·  {}", fns.len(), pc);
    println!();
    // ranked: effects with count>0, by (count desc, name desc) — matches Python's reverse sort of (n,e).
    let mut ranked: Vec<(usize, &str)> = EFFECTS.iter().filter(|e| tally[**e] > 0).map(|e| (tally[*e], *e)).collect();
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
    let (calib_c, calib_p, calib_path) = load_calibrated(pre, &base);
    let seen = encountered_set(pre);
    let suspect = std::fs::read_to_string(suspect_path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    // Warn (don't silently skip) when the suspect pattern won't compile — the `regex` crate rejects
    // some constructs Python's `re` accepted, and a silent drop would hide the coverage-gap check.
    let suspect = suspect.and_then(|s| match Regex::new(&s) {
        Ok(re) => Some(re),
        Err(e) => {
            eprintln!("candor: ignoring unparsable suspect pattern ({e}); coverage-gap check skipped");
            None
        }
    });
    let calibrated = |c: &str| -> bool {
        calib_c.contains(c)
            || calib_c.iter().any(|k| c == k || c.starts_with(&format!("{k}_")))
            || calib_p.iter().any(|p| c.starts_with(p))
            || calib_path.contains(c) // path-matched runtimes (tokio/async_std/mio) ARE covered
    };
    // The full honest under-report surface: every external crate candor called into but has neither a
    // classifier rule NOR its own report (a crate with a report is analyzed via cross-crate, so its
    // effects are inherited — not a blind spot). Calls into what remains are assumed PURE.
    let uncovered: Vec<String> =
        seen.iter().filter(|c| !calibrated(c) && !analyzed.contains(*c)).cloned().collect();
    // Default: the suspect heuristic surfaces the *likely-effectful* uncovered crates loudly.
    let suspect_gaps: Vec<String> = match &suspect {
        Some(re) => uncovered.iter().filter(|c| re.is_match(c)).cloned().collect(),
        None => Vec::new(),
    };
    if !suspect_gaps.is_empty() {
        println!("  ⚠ coverage: {} uncalibrated — effects through them may be under-counted", suspect_gaps.join(", "));
        println!();
    }
    if coverage {
        // `--coverage`: the complete auditable list, regardless of the name heuristic. This is the
        // honest answer to "could candor be under-reporting via a dep it doesn't know?".
        if uncovered.is_empty() {
            println!("  coverage: every external crate candor called into is calibrated — no blind spots.");
        } else {
            println!("  coverage — {} external crate(s) candor has NO effect rules for; calls into them", uncovered.len());
            println!("  are assumed PURE, so any I/O they perform is UNDER-REPORTED. Verify the effectful ones:");
            for c in &uncovered {
                println!("      {c}");
            }
        }
        println!();
    } else if !uncovered.is_empty() {
        // Don't leave the rest fully silent: a one-line, neutral pointer to the full audit.
        let other = uncovered.len() - suspect_gaps.len();
        if other > 0 {
            println!("  {other} more external crate(s) have no effect rules (assumed pure) — `cargo candor audit --coverage` to list", );
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
    if !coverage {
        println!("  classifier blind spots:  cargo candor audit --coverage");
    }
    println!("  guard against new effects:  cargo candor snapshot .candor/baseline");
    0
}

/// The calibrated-coverage sidecar `<dir>/<base>.calibrated.json` → (crates, prefixes, path_crates).
/// `path_crates` are crates the engine matches by path-prefix (tokio/async_std/mio) — covered, but
/// absent from the crate-name list — so the coverage check doesn't mislabel them as blind spots.
fn load_calibrated(prefix: &str, base: &str) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
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
    (pick("crates"), pick("prefixes"), pick("path_crates"))
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
    /// Literal Net endpoints, omitted when none visible — see `ReportEntry::hosts`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hosts: Vec<String>,
    unresolved: bool,
}

/// Match a function name against a query. EXACT-wins: if some candidate equals `q` verbatim, only
/// exact names match (`show foo` returns `foo`, not `foobar`); otherwise fall back to substring so a
/// partial query still searches. `exact_exists` is precomputed over the candidate set.
fn q_match(name: &str, q: &str, exact_exists: bool) -> bool {
    if exact_exists { name == q } else { name.contains(q) }
}

fn cmd_show(args: &[String]) -> i32 {
    let (pre, q, want_json) = match three(args) {
        Some(t) => t,
        None => {
            eprintln!("usage: candor-query show <prefix> <query> <0|1>");
            return 2;
        }
    };
    let all = load_entries(pre);
    let exact = all.iter().any(|e| e.func == q);
    let mut fns: Vec<ReportEntry> = all.into_iter().filter(|e| q_match(&e.func, q, exact)).collect();
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
    println!("  (* = performed in the function's own body; unmarked = via a callee{fs_note}{host_note})");
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
    // Prefer the full call-graph sidecar (the engine emits `<prefix>.<crate>.<kind>.callgraph.json`
    // alongside the report). It records EVERY function's callees — including pure ones — so we can
    // answer "who TRANSITIVELY calls X" for any function: the blast radius an agent needs *before*
    // adding an effect to X. The report alone only records effect-relevant edges (can't see a pure X).
    let cg = load_callgraph(pre);
    if !cg.is_empty() {
        return callers_via_callgraph(&cg, q, want_json);
    }

    // Fallback (no call-graph sidecar): the older effect-relevant, direct-only view.
    let entries = load_entries(pre);
    let exact = entries.iter().any(|e| e.calls.iter().any(|c| c == q));
    let mut hits: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for e in &entries {
        for callee in &e.calls {
            if q_match(callee, q, exact) {
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

/// "Who reaches `q`?" over the full call graph: the DIRECT callers and the full TRANSITIVE set (the
/// blast radius if `q` gained an effect). Works for any function, effectful or pure.
fn callers_via_callgraph(cg: &BTreeMap<String, Vec<String>>, q: &str, want_json: bool) -> i32 {
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
    let exact = names.contains(q);
    let targets: Vec<&str> = names.iter().copied().filter(|n| q_match(n, q, exact)).collect();
    if targets.is_empty() {
        if want_json {
            println!("{{}}");
        } else {
            println!("candor: no function matching `{q}` found in the call graph.");
        }
        return 0;
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

/// Load + merge every `<prefix>.*.callgraph.json` sidecar into one `caller -> [callees]` map (by path).
fn load_callgraph(prefix: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let p = Path::new(prefix);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else { return out };
    let pfx = format!("{base}.");
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&pfx) || !name.ends_with(".callgraph.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(ent.path()) else { continue };
        if let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text) {
            for (k, v) in map {
                out.entry(k).or_default().extend(v);
            }
        }
    }
    out
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
        // Module = the first path component. For a qualified trait-impl path `<Type as Trait>::m`,
        // that's `Type` — stop at the first of ` as `, `>`, or `::` (whichever comes first) so the
        // bucket isn't the malformed `Type as Trait>`.
        let stripped = e.func.strip_prefix('<').unwrap_or(&e.func);
        let end = [stripped.find(" as "), stripped.find('>'), stripped.find("::")]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(stripped.len());
        let m = match stripped[..end].trim() {
            "" => "(root)".to_string(),
            s => s.to_string(),
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

// ── diff ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct FnInfo {
    inferred: BTreeSet<String>,
    direct: BTreeSet<String>,
    calls: BTreeSet<String>,
}

/// fn -> its effect info, last write wins (mirrors Python's `out[e['fn']] = …`).
fn load_fninfo(prefix: &str) -> BTreeMap<String, FnInfo> {
    let mut out: BTreeMap<String, FnInfo> = BTreeMap::new();
    for e in load_entries(prefix) {
        // MERGE (union) rather than overwrite: two crates can render a function with the same printed
        // name (`main`, a shared generic monomorphization). Overwriting dropped one crate's effects, so
        // diff/gains could miss a newly-introduced effect in the shadowed crate. Union over-approximates
        // (sound for a regression check — a gain in EITHER is surfaced).
        let info = out.entry(e.func.clone()).or_default();
        info.inferred.extend(e.inferred);
        info.direct.extend(e.direct);
        info.calls.extend(e.calls);
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
        let gained: Vec<String> = gained_effects(ci, bi);
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
        changes.sort_by(|a, b| a.func.cmp(&b.func));
        let out = DiffJson { baseline_version: bver, engine_version: ever, changes };
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

// ── containment ───────────────────────────────────────────────────────────────────────────────────

/// BOUNDARY effects SHOULD live in a dedicated layer — their dispersion is the architecture signal (NOT
/// raw counts, which are domain-dependent). AMBIENT effects are expected to be cross-cutting (logging /
/// timestamps everywhere is fine), so they're reported but not scored. `Unknown` is excluded.
const CONTAINED: &[&str] = &["Db", "Net", "Exec", "Fs", "Ipc"];
const AMBIENT: &[&str] = &["Log", "Clock", "Rand", "Env"];

/// The number of leading `::` segments shared by EVERY function name — the codebase root, so the next
/// segment is the architectural "layer" (`pgman::app::…` → `app`; a multi-crate report → the crate).
fn common_prefix_len(names: &[&String]) -> usize {
    let mut prefix: Option<Vec<&str>> = None;
    for n in names {
        let segs: Vec<&str> = n.split("::").collect();
        match &mut prefix {
            None => prefix = Some(segs),
            Some(p) => {
                let mut i = 0;
                while i < p.len() && i < segs.len() && p[i] == segs[i] {
                    i += 1;
                }
                p.truncate(i);
            }
        }
    }
    prefix.map(|p| p.len()).unwrap_or(0)
}

/// The layer a function belongs to: the MODULE segment after the common root prefix. A free function at
/// the root (`pgman::main`) has no module beyond the crate, so it buckets into `(root)` rather than
/// becoming its own pseudo-layer — the layer is `segs[prefix_len]` only when a leaf follows it.
fn layer_of(name: &str, prefix_len: usize) -> String {
    let segs: Vec<&str> = name.split("::").collect();
    if prefix_len + 1 < segs.len() {
        segs[prefix_len].to_string()
    } else {
        "(root)".to_string()
    }
}

/// `containment` — how well each BOUNDARY effect (Db/Net/Exec/Fs/Ipc) stays in one layer: the
/// domain-INDEPENDENT architecture signal behind the "leaky cross-cutting" intuition (a ratio /
/// structure, not a count). With a baseline prefix it's a RATCHET — exit 1 if a boundary effect appears
/// in a layer it wasn't in ("Db → actions"), and NOTE when one leaves a layer ("✓ Db ⊘ legacy").
/// Deliberately a diagnostic + trend gate, NOT a single gameable "score".
/// Args: `<prefix> [baseline_prefix] [--json]`.
fn cmd_containment(args: &[String]) -> i32 {
    let want_json = args.iter().any(|a| a == "--json");
    let pos: Vec<&String> = args.iter().filter(|a| *a != "--json").collect();
    let Some(cur_pre) = pos.first() else {
        eprintln!("usage: candor-query containment <prefix> [baseline_prefix] [--json]");
        return 2;
    };
    let cur = load_fninfo(cur_pre);
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
    if let Some(base_pre) = pos.get(1) {
        let base = load_fninfo(base_pre);
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
            println!("[containment] a boundary effect leaked into a layer it wasn't in:");
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
    println!("  {:<7} {:>9} {:>7}   {}", "effect", "contained", "layers", "owner  ← leaked into");
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

// ── receipt ─────────────────────────────────────────────────────────────────────────────────────

/// The Claude Code receipt's report-derived fields, emitted as shell-friendly `key<TAB>value` lines
/// so `candor-run.sh` reads them without a JSON parser (it used inline Python heredocs). Fields:
/// `fns`, `effects` (count-prefixed, in the receipt's display order), `unresolved`, `calibrated`
/// (`<crates>|<prefixes>`), `encountered` (crates candor saw resolved calls into).
fn cmd_receipt(args: &[String]) -> i32 {
    let Some(pre) = args.first().map(String::as_str) else {
        eprintln!("usage: candor-query receipt <prefix>");
        return 2;
    };
    let base = prefix_base(pre);
    let fns = load_entries(pre);
    let (tally, unresolved) = tally_effects(&fns);
    let unresolved = unresolved.len();
    // The receipt's own display order (Db-first), preserved byte-for-byte from the Python it replaces.
    // It must list exactly the EFFECTS vocabulary; the assert catches a new effect added to EFFECTS but
    // not here (which would silently drop it from the receipt while `audit` still showed it).
    const ORDER: [&str; 10] =
        ["Db", "Net", "Fs", "Exec", "Env", "Clock", "Ipc", "Rand", "Clipboard", "Log"];
    debug_assert_eq!(
        ORDER.iter().copied().collect::<BTreeSet<_>>(),
        EFFECTS.iter().copied().collect::<BTreeSet<_>>(),
        "receipt ORDER must be a permutation of candor_report::EFFECTS",
    );
    let effects = ORDER
        .iter()
        .filter(|k| tally.get(**k).copied().unwrap_or(0) > 0)
        .map(|k| format!("{} {k}", tally[*k]))
        .collect::<Vec<_>>()
        .join(", ");

    let (mut calib_c, calib_p, calib_path) = load_calibrated(pre, &base);
    calib_c.extend(calib_path); // path-matched runtimes (tokio/…) count as covered for the receipt too
    let encountered = encountered_set(pre);
    let join = |s: &BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join(" ");
    println!("fns\t{}", fns.len());
    println!("effects\t{effects}");
    println!("unresolved\t{unresolved}");
    println!("calibrated\t{}|{}", join(&calib_c), join(&calib_p));
    println!("encountered\t{}", join(&encountered));
    0
}

// ── gains (edit-time self-review) ───────────────────────────────────────────────────────────────

/// Every `<fn>\t<effect>` a function INHERITED or introduced since the baseline (current `inferred`
/// minus baseline `inferred`), sorted. `candor-run.sh`'s opt-in self-review dedups these against its
/// `review-seen` file and formats the prompt — the seen-file state stays in bash so this stays a
/// read-only query.
fn cmd_gains(args: &[String]) -> i32 {
    let (cur_pre, base_pre) = match args {
        [a, b, ..] => (a.as_str(), b.as_str()),
        _ => {
            eprintln!("usage: candor-query gains <cur_prefix> <base_prefix>");
            return 2;
        }
    };
    let cur = load_fninfo(cur_pre);
    let base = load_fninfo(base_pre);
    let empty = BTreeSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for (func, info) in &cur {
        let b = base.get(func).map(|i| &i.inferred).unwrap_or(&empty);
        for e in gained_effects(&info.inferred, b) {
            out.push((func.clone(), e));
        }
    }
    out.sort();
    for (func, e) in out {
        println!("{func}\t{e}");
    }
    0
}

/// `state [<root>]` — print a stable content hash of every `.rs` file under `<root>` (default cwd),
/// excluding `target/` and `.git/`. This is the source-freshness key the wrapper writes to
/// `.candor/state` and later compares, to tell whether a saved report still matches the tree. It
/// replaces a fragile `find … | sort -z | xargs shasum | shasum | cut` pipeline that was copy-pasted
/// into ~10 shell sites — and had already DRIFTED (some copies excluded `.git`, some didn't), so two
/// code paths could hash the same tree differently. One canonical implementation kills that bug class.
/// The hash is FNV-1a over each file's path then bytes (NUL-separated) in sorted order — deterministic
/// and dependency-free; the value need only be stable, not cryptographic (the state file is ephemeral).
fn cmd_state(args: &[String]) -> i32 {
    let root = args.first().map(String::as_str).unwrap_or(".");
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rs(Path::new(root), &mut files);
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64-bit offset basis
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for f in &files {
        // Path relative to root, so the same tree hashes the same regardless of where it lives.
        let rel = f.strip_prefix(root).unwrap_or(f);
        feed(rel.to_string_lossy().as_bytes());
        feed(&[0]);
        if let Ok(bytes) = std::fs::read(f) {
            feed(&bytes);
        }
        feed(&[0]);
    }
    println!("{h:016x}");
    0
}

/// `reports <prefix> [--exists]` — the canonical report-file discovery for a prefix, via
/// `candor_report::report_files` (the SAME `<prefix>.<crate>.<type>.json` shape, with the exact
/// sidecar exclusion: `.calibrated.json` / `.encountered-*` / `.layerreach.json` are NOT reports).
/// Default: print one report path per line. `--exists`: print nothing, exit 0 if any report exists
/// else 1 — a drop-in for the `ls "$prefix".*.*.json >/dev/null` existence checks in the wrapper, so
/// "what counts as a report" is defined once (here) instead of approximated by a shell glob.
/// Which backend produced the report(s) at <prefix>: `scan` (the stable scanner writes `.<crate>.scan`),
/// `lint` (the nightly lint writes `.<crate>.<Rlib|Executable|Cdylib|…>`), or `none`. The single owner
/// of "what backend is this report" — replaces a filename glob duplicated across both bash orchestrators.
fn report_backend(prefix: &str) -> &'static str {
    let files = report_files(prefix);
    if files.is_empty() {
        "none"
    } else if files.iter().any(|f| f.kind == "scan") {
        "scan"
    } else {
        "lint"
    }
}

/// Is a filename (relative to the prefix's directory) a STABLE-backend artifact for <base>? True for
/// `<base>.<crate>.scan.json` and its `.scan.callgraph.json` sidecar — i.e. a `.scan.` segment.
fn is_scan_artifact(base: &str, name: &str) -> bool {
    name.strip_prefix(base)
        .and_then(|r| r.strip_prefix('.'))
        .is_some_and(|rest| rest.contains(".scan.") || rest.ends_with(".scan.json"))
}

/// Remove every report artifact for <prefix> that does NOT belong to the `keep` backend — so a lint
/// report and a scan report never coexist under one prefix. Removes reports + callgraph/encountered/
/// calibrated sidecars of the other backend; never touches the kept backend's files (so a build failure
/// keeps the same-backend last-good report). Returns the count removed.
fn clear_other_reports(prefix: &str, keep: &str) -> usize {
    let p = Path::new(prefix);
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else { return 0 };
    let prefix_dot = format!("{base}.");
    let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
    let mut removed = 0;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix_dot) || !name.ends_with(".json") {
            continue;
        }
        let scan = is_scan_artifact(base, name);
        let remove = match keep {
            "scan" => !scan, // keep the scan report; drop the lint reports + its sidecars
            "lint" => scan,  // keep the lint report(s); drop the scan report + its callgraph
            _ => false,
        };
        if remove && std::fs::remove_file(ent.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn cmd_reports(args: &[String]) -> i32 {
    let exists_only = args.iter().any(|a| a == "--exists");
    let backend = args.iter().any(|a| a == "--backend");
    let clear_keep = args.iter().position(|a| a == "--clear-other").and_then(|i| args.get(i + 1)).cloned();
    let Some(prefix) = args.iter().find(|a| !a.starts_with("--")) else {
        eprintln!("usage: candor-query reports <prefix> [--exists | --backend | --clear-other <scan|lint>]");
        return 2;
    };
    if backend {
        println!("{}", report_backend(prefix));
        return 0;
    }
    if let Some(keep) = clear_keep {
        clear_other_reports(prefix, &keep);
        return 0;
    }
    let files = report_files(prefix);
    if exists_only {
        return if files.is_empty() { 1 } else { 0 };
    }
    for rf in &files {
        println!("{}", rf.path.display());
    }
    0
}

/// `candor-query locate <lib|scan> <dir>...` — print the NEWEST-by-mtime matching artifact across the
/// given dirs (a dylib for `lib`, the `candor-scan` binary for `scan`), or nothing. The single owner of
/// the newest-mtime locator logic (was copied into both bash scripts; `ls | head` there silently picked
/// the alphabetically-first — stale — toolchain dylib after a bump). `query` itself stays a bash
/// bootstrap (it can't locate the binary that does the locating).
/// Newest-by-mtime artifact of `kind` (`lib` = a `libcandor@*.{dylib,so}`, `scan` = the `candor-scan`
/// binary) across `dirs`, or None. Newest mtime — NOT alphabetical (`ls | head` picks the stale
/// toolchain dylib after a bump). Pure (unit-tested); `cmd_locate` just prints it.
fn locate_newest(kind: &str, dirs: &[String]) -> Option<PathBuf> {
    let matches_kind = |name: &str| -> bool {
        match kind {
            "scan" => name == "candor-scan",
            "lib" => name.starts_with("libcandor@") && (name.ends_with(".dylib") || name.ends_with(".so")),
            _ => false,
        }
    };
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if !matches_kind(name) {
                continue;
            }
            let Ok(mtime) = ent.metadata().and_then(|m| m.modified()) else { continue };
            if newest.as_ref().is_none_or(|(t, _)| mtime > *t) {
                newest = Some((mtime, ent.path()));
            }
        }
    }
    newest.map(|(_, p)| p)
}

fn cmd_locate(args: &[String]) -> i32 {
    let Some(kind) = args.first() else {
        eprintln!("usage: candor-query locate <lib|scan> <dir>...");
        return 2;
    };
    match locate_newest(kind, &args[1..]) {
        Some(p) => {
            println!("{}", p.display());
            0
        }
        None => 1,
    }
}

/// `candor-query engine-version <lib-path>` — print the `candor-build-version=` tag build.rs embedded in
/// the dylib (the version of the engine that actually produced a report), or nothing. Replaces a
/// `strings -a | grep -oE` incantation duplicated across both bash scripts (and not portable without
/// `strings`). Reads the file bytes directly.
fn cmd_engine_version(args: &[String]) -> i32 {
    let Some(path) = args.first() else {
        eprintln!("usage: candor-query engine-version <lib-path>");
        return 2;
    };
    let Ok(bytes) = std::fs::read(path) else { return 1 };
    let needle = b"candor-build-version=";
    let Some(start) = bytes.windows(needle.len()).position(|w| w == needle) else { return 1 };
    let v: Vec<u8> = bytes[start + needle.len()..]
        .iter()
        .copied()
        .take_while(|b| b.is_ascii_alphanumeric())
        .collect();
    if v.is_empty() {
        return 1;
    }
    println!("{}", String::from_utf8_lossy(&v));
    0
}

/// `merge-hook <settings.json> <hook-command>` — idempotently merge candor's Stop hook into a Claude
/// Code settings file, NON-destructively. Replaces an inline `python3` heredoc (one less interpreter
/// dependency, and typed + tested). If the file exists but isn't strict JSON (comments / trailing
/// commas, which Claude Code tolerates but a strict parser rejects), it is LEFT UNTOUCHED and a manual
/// snippet is printed — never reset-and-overwritten, which would wipe the user's other settings.
fn cmd_merge_hook(args: &[String]) -> i32 {
    let (Some(path), Some(cmd)) = (args.first(), args.get(1)) else {
        eprintln!("usage: candor-query merge-hook <settings.json> <hook-command>");
        return 2;
    };
    let manual = || {
        eprintln!("  WARNING: {path} isn't plain JSON (comments or trailing commas?) — NOT modifying it.");
        eprintln!(
            "  Add this Stop hook by hand: {{\"matcher\":\"*\",\"hooks\":[{{\"type\":\"command\",\"command\":\"{cmd}\"}}]}}"
        );
    };
    let mut data: serde_json::Value = if Path::new(path).exists() {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(_) => {
                    manual();
                    return 0; // unparseable → leave it; do not risk the user's settings
                }
            },
            Err(e) => {
                eprintln!("candor-query: cannot read {path}: {e}");
                return 1;
            }
        }
    } else {
        serde_json::json!({})
    };
    // Navigate/insert hooks.Stop, bailing (untouched) if any node is the wrong JSON type.
    let Some(obj) = data.as_object_mut() else {
        manual();
        return 0;
    };
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let Some(stop) = hooks
        .as_object_mut()
        .map(|h| h.entry("Stop").or_insert_with(|| serde_json::json!([])))
        .and_then(|s| s.as_array_mut())
    else {
        manual();
        return 0;
    };
    let present = stop.iter().any(|g| {
        g.get("hooks").and_then(|h| h.as_array()).is_some_and(|hs| {
            hs.iter().any(|h| h.get("command").and_then(|c| c.as_str()) == Some(cmd.as_str()))
        })
    });
    if present {
        println!("  Stop hook already present in {path}");
        return 0;
    }
    stop.push(serde_json::json!({"matcher": "*", "hooks": [{"type": "command", "command": cmd}]}));
    let body = serde_json::to_string_pretty(&data).unwrap_or_default();
    if let Err(e) = std::fs::write(path, format!("{body}\n")) {
        eprintln!("candor-query: cannot write {path}: {e}");
        return 1;
    }
    println!("  merged Stop hook into {path}");
    0
}

/// Recursively collect `*.rs` files under `dir`, skipping `target` and `.git` directories (and any
/// symlinked dir, to avoid cycles). Order-independent; the caller sorts.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let path = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            let name = ent.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            collect_rs(&path, out);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
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

    #[test]
    fn containment_layer_derivation() {
        // The common root prefix is stripped; the next MODULE segment is the layer.
        let names: Vec<String> = ["pgman::conn::connect", "pgman::query::run", "pgman::main"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let refs: Vec<&String> = names.iter().collect();
        let pl = common_prefix_len(&refs);
        assert_eq!(pl, 1, "shared root is `pgman`");
        assert_eq!(layer_of("pgman::conn::connect", pl), "conn");
        assert_eq!(layer_of("pgman::query::Q::run", pl), "query");
        // a free function at the crate root has no module → buckets into (root), not its own layer.
        assert_eq!(layer_of("pgman::main", pl), "(root)");
        // multi-crate report: no shared first segment → the crate IS the layer.
        let multi: Vec<String> =
            ["a::x::f".to_string(), "b::y::g".to_string()].to_vec();
        let mrefs: Vec<&String> = multi.iter().collect();
        assert_eq!(common_prefix_len(&mrefs), 0);
        assert_eq!(layer_of("a::x::f", 0), "a");
    }

    #[test]
    fn is_scan_artifact_discriminates() {
        assert!(is_scan_artifact("report", "report.mycrate.scan.json"));
        assert!(is_scan_artifact("report", "report.mycrate.scan.callgraph.json"));
        // lint artifacts are NOT scan
        assert!(!is_scan_artifact("report", "report.mycrate.Rlib.json"));
        assert!(!is_scan_artifact("report", "report.mycrate.Executable.callgraph.json"));
        assert!(!is_scan_artifact("report", "report.calibrated.json"));
        // a different prefix is not ours
        assert!(!is_scan_artifact("report", "other.mycrate.scan.json"));
    }

    #[test]
    fn report_backend_and_clear_other() {
        let dir = std::env::temp_dir().join("candor-query-backend-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pre = dir.join("report");
        let prefix = pre.to_string_lossy().to_string();
        let w = |name: &str| std::fs::write(dir.join(name), b"{}").unwrap();

        // none
        assert_eq!(report_backend(&prefix), "none");
        // a scan report
        w("report.c.scan.json");
        w("report.c.scan.callgraph.json");
        assert_eq!(report_backend(&prefix), "scan");
        // now a lint run lands too → both present; clear the scan side (keep lint)
        w("report.c.Rlib.json");
        w("report.c.Rlib.callgraph.json");
        w("report.calibrated.json");
        assert_eq!(report_backend(&prefix), "scan"); // scan still present
        let removed = clear_other_reports(&prefix, "lint"); // keep lint, drop scan
        assert_eq!(removed, 2); // report.c.scan.json + report.c.scan.callgraph.json
        assert!(!dir.join("report.c.scan.json").exists());
        assert!(dir.join("report.c.Rlib.json").exists());
        assert_eq!(report_backend(&prefix), "lint");
        // and the reverse: keep scan would drop the lint reports + calibrated sidecar
        w("report.c.scan.json");
        let removed = clear_other_reports(&prefix, "scan");
        assert!(removed >= 3); // Rlib.json + Rlib.callgraph.json + calibrated.json
        assert!(dir.join("report.c.scan.json").exists());
        assert_eq!(report_backend(&prefix), "scan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_picks_newest_by_mtime() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join("candor-query-locate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // two toolchain-suffixed dylibs; the alphabetically-FIRST is the OLDER (the stale-pick bug)
        let old = dir.join("libcandor@nightly-2025-01-01-x.dylib");
        let new = dir.join("libcandor@nightly-2026-01-01-x.dylib");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&new, b"x").unwrap();
        // make `new` strictly newer
        let f_old = std::fs::OpenOptions::new().write(true).open(&old).unwrap();
        f_old.set_modified(SystemTime::now() - Duration::from_secs(100)).unwrap();
        let f_new = std::fs::OpenOptions::new().write(true).open(&new).unwrap();
        f_new.set_modified(SystemTime::now()).unwrap();
        let out = locate_newest("lib", &[dir.to_string_lossy().to_string()]);
        assert_eq!(out, Some(new)); // newest mtime, not the alphabetically-first (older) one
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// merge-hook must: add the hook to a fresh/empty file; PRESERVE the user's other settings; be
    /// idempotent; and — the critical safeguard — leave an unparseable file UNTOUCHED rather than
    /// clobber it. (The bug this guards against once wiped a user's permissions/model on re-install.)
    #[test]
    fn merge_hook_is_nondestructive_and_idempotent() {
        let dir = std::env::temp_dir().join("candor-query-merge-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cmd = "X/stop-hook.sh".to_string();
        let arg = |p: &std::path::Path| vec![p.to_string_lossy().to_string(), cmd.clone()];

        // 1) fresh file → hook added, parseable.
        let fresh = dir.join("fresh.json");
        assert_eq!(cmd_merge_hook(&arg(&fresh)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], cmd.as_str());

        // 2) existing unrelated settings preserved.
        let keep = dir.join("keep.json");
        std::fs::write(&keep, r#"{"model":"opus","permissions":{"allow":["Bash"]}}"#).unwrap();
        assert_eq!(cmd_merge_hook(&arg(&keep)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&keep).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["permissions"]["allow"][0], "Bash");
        assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], cmd.as_str());

        // 3) idempotent — a second merge doesn't duplicate the hook.
        assert_eq!(cmd_merge_hook(&arg(&keep)), 0);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&keep).unwrap()).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1);

        // 4) unparseable (comments/trailing comma) → LEFT UNTOUCHED.
        let bad = dir.join("bad.json");
        let original = "{ // comment\n  \"model\": \"x\",\n}";
        std::fs::write(&bad, original).unwrap();
        assert_eq!(cmd_merge_hook(&arg(&bad)), 0);
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), original, "must not touch a non-JSON file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `collect_rs` + the FNV digest must be deterministic, sensitive to `.rs` content, and blind to
    /// `target/` and `.git/` — the contract the ~10 shell sites used to re-implement (inconsistently).
    #[test]
    fn state_hash_is_deterministic_and_scoped() {
        let dir = std::env::temp_dir().join("candor-query-state-test");
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["src", "sub", "target/x", ".git"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("src/a.rs"), "fn a(){}").unwrap();
        std::fs::write(dir.join("sub/b.rs"), "fn b(){}").unwrap();
        std::fs::write(dir.join("target/x/c.rs"), "fn c(){}").unwrap(); // must be ignored
        std::fs::write(dir.join(".git/d.rs"), "fn d(){}").unwrap(); // must be ignored
        std::fs::write(dir.join("src/notrust.txt"), "ignored").unwrap();

        let hash = |root: &Path| -> u64 {
            let mut files = Vec::new();
            collect_rs(root, &mut files);
            files.sort();
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for f in &files {
                for &b in f.strip_prefix(root).unwrap_or(f).to_string_lossy().as_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                for &b in std::fs::read(f).unwrap_or_default().iter() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            h
        };

        // only the two real .rs files are collected (target/, .git/, and .txt excluded).
        let mut files = Vec::new();
        collect_rs(&dir, &mut files);
        assert_eq!(files.len(), 2, "must collect exactly src/a.rs + sub/b.rs");

        let h1 = hash(&dir);
        assert_eq!(h1, hash(&dir), "deterministic");
        // editing an ignored dir must NOT change the hash.
        std::fs::write(dir.join("target/x/c.rs"), "fn c(){ let _=9; }").unwrap();
        std::fs::write(dir.join(".git/d.rs"), "fn d(){ let _=9; }").unwrap();
        assert_eq!(h1, hash(&dir), "target/ and .git/ edits are ignored");
        // editing a real source file MUST change it.
        std::fs::write(dir.join("src/a.rs"), "fn a(){ let _=1; }").unwrap();
        assert_ne!(h1, hash(&dir), "a real .rs edit changes the hash");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
