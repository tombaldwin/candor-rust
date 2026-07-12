//! Baseline comparison: `diff`, `gains`, `receipt`.

use crate::*;

// ── diff ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
pub(crate) struct FnInfo {
    pub(crate) inferred: BTreeSet<String>,
    pub(crate) direct: BTreeSet<String>,
    pub(crate) calls: BTreeSet<String>,
}

/// fn -> its effect info, last write wins (mirrors Python's `out[e['fn']] = …`).
pub(crate) fn load_fninfo(prefix: &str) -> BTreeMap<String, FnInfo> {
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
pub(crate) struct Change {
    #[serde(rename = "fn")]
    pub(crate) func: String,
    pub(crate) gained: Vec<String>,
    pub(crate) introduced: Vec<String>,
    pub(crate) inherited: Vec<String>,
    pub(crate) lost: Vec<String>,
    pub(crate) status: String,
}

#[derive(Serialize)]
pub(crate) struct DiffJson<'a> {
    pub(crate) baseline_version: &'a str,
    pub(crate) engine_version: &'a str,
    pub(crate) changes: Vec<Change>,
}

pub(crate) fn cmd_diff(args: &[String]) -> i32 {
    // `diff <current> <baseline> [--json]` — the two-locator comparative verb (SPEC §3.3.1 exception: it
    // does NOT discover). Locators resolve by the shared --report rule (dir/.json/prefix). The optional
    // `<baseline_ver> <engine_ver>` stamps trail the two locators (the version banner). The DEPRECATED old
    // form put JSON as a `<0|1>` sentinel in the THIRD positional slot: `diff <cur> <base> <0|1> <bver>
    // <ever>` — still accepted with a stderr note.
    let mut want_json = args.iter().any(|a| a == "--json");
    let pos: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    if pos.len() < 2 {
        eprintln!("usage: candor-query diff <current> <baseline> [--json] [<baseline_ver> <engine_ver>]");
        return 2;
    }
    // Old form: a `0|1` sentinel in the third slot (before the version stamps). Detect + strip it.
    let mut rest: Vec<String> = pos[2..].to_vec();
    if let Some(first) = rest.first() {
        if first == "0" || first == "1" {
            deprecation_note("a trailing `0|1` JSON sentinel");
            if first == "1" {
                want_json = true;
            }
            rest.remove(0);
        }
    }
    let cur_loc = resolve_locator(&pos[0]);
    let base_loc = resolve_locator(&pos[1]);
    let (cur_pre, base_pre) = (&cur_loc, &base_loc);
    let bver = rest.first().map(String::as_str).unwrap_or("");
    let ever = rest.get(1).map(String::as_str).unwrap_or("");

    // A prefix that matches NO report files must fail LOUD, not read as an empty report: a typo'd
    // `cur` would otherwise show zero gains (a gained-effect gate built on this output would silently
    // PASS with the wrong path), and a typo'd baseline would show every effect as newly gained. A
    // legitimately effect-free crate still writes a report file, so "no files" is always an error.
    for (which, pre) in [("current", cur_pre), ("baseline", base_pre)] {
        if glob_reports(pre).is_empty() {
            eprintln!("candor: no report files at {which} prefix `{pre}` — check the path.");
            return 2;
        }
    }
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
pub(crate) fn rank(c: &Change, top_level: &BTreeSet<String>) -> u8 {
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

// ── receipt ─────────────────────────────────────────────────────────────────────────────────────

/// The Claude Code receipt's report-derived fields, emitted as shell-friendly `key<TAB>value` lines
/// so `candor-run.sh` reads them without a JSON parser (it used inline Python heredocs). Fields:
/// `fns`, `effects` (count-prefixed, in the receipt's display order), `unresolved`, `calibrated`
/// (`<crates>|<prefixes>`), `encountered` (crates candor saw resolved calls into).
pub(crate) fn cmd_receipt(args: &[String]) -> i32 {
    let Some(pre) = args.first().map(String::as_str) else {
        eprintln!("usage: candor-query receipt <prefix>");
        return 2;
    };
    let base = prefix_base(pre);
    let fns = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };
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
pub(crate) fn cmd_gains(args: &[String]) -> i32 {
    // `gains <current> <baseline> [--json]` — the two-locator comparative verb (does NOT discover, like
    // `diff`). Locators resolve by the shared --report rule. The DEPRECATED old form allowed a trailing
    // `1` sentinel for JSON (handled below alongside `--json`).
    let mut want_json = args.iter().any(|a| a == "--json");
    let pos: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    // Old trailing `1` sentinel → JSON (with a note); a `0` is the explicit non-JSON old form.
    let (cur_loc, base_loc) = match pos.as_slice() {
        [a, b, ..] => (resolve_locator(a), resolve_locator(b)),
        _ => {
            eprintln!("usage: candor-query gains <current> <baseline> [--json]");
            return 2;
        }
    };
    if let Some(last) = pos.get(2) {
        if last == "0" || last == "1" {
            deprecation_note("a trailing `0|1` JSON sentinel");
            if last == "1" {
                want_json = true;
            }
        }
    }
    let (cur_pre, base_pre) = (cur_loc.as_str(), base_loc.as_str());
    // Same no-files-fails-loud rule as cmd_diff, and for the same reason: a typo'd current prefix
    // shows zero gains (a gate built on this silently PASSES); a typo'd baseline shows every effect
    // as newly gained.
    for (which, pre) in [("current", cur_pre), ("baseline", base_pre)] {
        if glob_reports(pre).is_empty() {
            eprintln!("candor: no report files at {which} prefix `{pre}` — check the path.");
            return 2;
        }
    }
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
    if want_json {
        // The package-level supply-chain alarm (spec §5.1): `gained` is the UNION of effects the
        // surface gained between the two reports — a dependency that grew a Net/Exec reach between
        // releases — with the per-function detail under `byFunction`. Machine-readable so a CI gate
        // can alarm on a dependency update that quietly gains a capability.
        let mut gained: Vec<&str> = out.iter().map(|(_, e)| e.as_str()).collect();
        gained.sort_unstable();
        gained.dedup();
        let by_function: Vec<_> = out
            .iter()
            .map(|(f, e)| serde_json::json!({ "fn": f, "effect": e }))
            .collect();
        let v = serde_json::json!({ "gained": gained, "byFunction": by_function });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return 0;
    }
    for (func, e) in out {
        println!("{func}\t{e}");
    }
    0
}
