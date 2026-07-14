//! Baseline comparison: `diff`, `gains`, `receipt`.

use crate::*;

/// The receipt's own display order (Db-first), preserved byte-for-byte from the Python it replaces.
/// It MUST be exactly a permutation of the `candor_report::EFFECTS` vocabulary — a new effect added to
/// EFFECTS but not here would silently drop from the receipt (while `audit` still showed it). The
/// invariant is enforced by the `receipt_order_is_a_permutation_of_effects` unit test below, which runs
/// in CI (unlike the old `debug_assert_eq!`, which was compiled out in the release binary CI ships).
const RECEIPT_ORDER: [&str; 11] =
    ["Db", "Net", "Llm", "Fs", "Exec", "Env", "Clock", "Ipc", "Rand", "Clipboard", "Log"];

// ── diff ────────────────────────────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
pub(crate) struct FnInfo {
    pub(crate) inferred: BTreeSet<String>,
    pub(crate) direct: BTreeSet<String>,
    pub(crate) calls: BTreeSet<String>,
}

/// The shared fn-info loader (fn -> merged effect info, mirroring Python's `out[e['fn']] = …`).
/// Returns the map AND the `hard_fail` bit `load_entries_inner` threads (load.rs): a matched report
/// wholly failed to read/parse, or parsed but yielded zero usable entries while non-empty (the
/// semantic-corruption rule). The loud wrapper needs the bit to tell an empty-but-VALID report apart
/// from "every report we found was corrupt". Every fn-info consumer (diff/gains/containment) loads
/// via `load_fninfo_loud` below.
fn load_fninfo_flagged(prefix: &str) -> (BTreeMap<String, FnInfo>, bool) {
    let mut out: BTreeMap<String, FnInfo> = BTreeMap::new();
    let (entries, hard_fail) = load_entries_inner(prefix);
    for e in entries {
        // MERGE (union) rather than overwrite: two crates can render a function with the same printed
        // name (`main`, a shared generic monomorphization). Overwriting dropped one crate's effects, so
        // diff/gains could miss a newly-introduced effect in the shadowed crate. Union over-approximates
        // (sound for a regression check — a gain in EITHER is surfaced).
        let info = out.entry(e.func.clone()).or_default();
        info.inferred.extend(e.inferred);
        info.direct.extend(e.direct);
        info.calls.extend(e.calls);
    }
    (out, hard_fail)
}

/// `load_fninfo`, but both a no-files prefix AND a found-but-corrupt report fail LOUD (exit 2) instead
/// of reading as an empty map — `load_entries_loud`'s rule (load.rs:75) applied to the comparative
/// verbs. diff/gains/containment previously loaded via the QUIET path, so a FOUND-but-corrupt current
/// report yielded an empty map and an exit-0 empty answer (`{"gained":[],"byFunction":[]}` — a
/// supply-chain all-clear over corrupt input, the §4 cardinal sin). A legitimately effect-free crate
/// still writes a report that LISTS its functions, so empty-because-of-hard-fail is always the corrupt
/// case; a CLEAN-empty valid report stays Ok. `which` labels the side in the message ("current" /
/// "baseline"; "" for the single-report verbs) — the pre-existing no-files wording is preserved.
pub(crate) fn load_fninfo_loud(prefix: &str, which: &str) -> Result<BTreeMap<String, FnInfo>, i32> {
    let at = if which.is_empty() { String::new() } else { format!("{which} ") };
    if glob_reports(prefix).is_empty() {
        eprintln!("candor: no report files at {at}prefix `{prefix}` — check the path.");
        return Err(2);
    }
    let (map, hard_fail) = load_fninfo_flagged(prefix);
    if map.is_empty() && hard_fail {
        eprintln!("candor: every report found at {at}prefix `{prefix}` failed to load — refusing to report an empty (all-clear) answer over a corrupt report; re-run the scan.");
        return Err(2);
    }
    Ok(map)
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
    if matches!(rest.first().map(String::as_str), Some("0") | Some("1")) {
        deprecation_note("a trailing `0|1` JSON sentinel");
        if rest.first().map(String::as_str) == Some("1") {
            want_json = true;
        }
        rest.remove(0);
    }
    let cur_loc = resolve_locator(&pos[0]);
    let base_loc = resolve_locator(&pos[1]);
    let (cur_pre, base_pre) = (&cur_loc, &base_loc);
    let bver = rest.first().map(String::as_str).unwrap_or("");
    let ever = rest.get(1).map(String::as_str).unwrap_or("");

    // Both sides load LOUD (load_fninfo_loud): a prefix matching NO report files must fail, not read
    // as an empty report — a typo'd `cur` would otherwise show zero gains (a gained-effect gate built
    // on this output would silently PASS with the wrong path), and a typo'd baseline would show every
    // effect as newly gained. And a FOUND-but-corrupt report must fail the same way, never read as an
    // empty (all-clear) diff — see load_fninfo_loud.
    let cur = match load_fninfo_loud(cur_pre, "current") {
        Ok(m) => m,
        Err(c) => return c,
    };
    let base = match load_fninfo_loud(base_pre, "baseline") {
        Ok(m) => m,
        Err(c) => return c,
    };
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
    // Receipt display order (`RECEIPT_ORDER`, module-level): a permutation of `candor_report::EFFECTS`,
    // enforced by the `receipt_order_is_a_permutation_of_effects` CI test (the old in-fn `debug_assert_eq!`
    // was compiled out of the release binary, so drift shipped silently).
    let effects = RECEIPT_ORDER
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
/// The origin ladder for a gained-effect fn (⟨0.12 staged⟩, gains --json): baseline-report hit →
/// "existing"; else a baseline-callgraph node (a baseline-PURE fn has no report entry but is still a
/// graph caller/callee) → "existing"; else a graph that is empty OR PARTIAL (a matched sidecar was
/// dropped — the fn may have lived there) → "unknown"; else "new". Factored out of cmd_gains so the
/// partial-graph rule is unit-pinned (a partial graph must never mint "new").
fn gain_origin(
    func: &str,
    base: &BTreeMap<String, FnInfo>,
    cg_nodes: &BTreeSet<&str>,
    cg_empty_or_partial: bool,
) -> &'static str {
    if base.contains_key(func) || cg_nodes.contains(func) {
        "existing"
    } else if cg_empty_or_partial {
        "unknown"
    } else {
        "new"
    }
}

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
    if matches!(pos.get(2).map(String::as_str), Some("0") | Some("1")) {
        deprecation_note("a trailing `0|1` JSON sentinel");
        if pos.get(2).map(String::as_str) == Some("1") {
            want_json = true;
        }
    }
    let (cur_pre, base_pre) = (cur_loc.as_str(), base_loc.as_str());
    // Same loud rule as cmd_diff, and for the same reason: a typo'd (or FOUND-but-corrupt) current
    // prefix shows zero gains — an exit-0 `{"gained":[]}` supply-chain all-clear a gate silently
    // PASSES on; a typo'd baseline shows every effect as newly gained. See load_fninfo_loud.
    let cur = match load_fninfo_loud(cur_pre, "current") {
        Ok(m) => m,
        Err(c) => return c,
    };
    let base = match load_fninfo_loud(base_pre, "baseline") {
        Ok(m) => m,
        Err(c) => return c,
    };
    let empty = BTreeSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for (func, info) in &cur {
        let b = base.get(func).map(|i| &i.inferred).unwrap_or(&empty);
        for e in gained_effects(&info.inferred, b) {
            out.push((func.clone(), e));
        }
    }
    out.sort();
    // §2.1 provenance: which engine BUILD produced each side (the report's `candor.version`,
    // `meta.version` for older shapes; "" when unknown — the candor-ts/candor-java parity shape).
    // When both are known and differ, a "gained capability" may be the newer engine RECLASSIFYING,
    // not the dependency changing — disclosed in BOTH output modes (java/swift parity; the TSV form
    // is what candor-run.sh's self-review consumes, and it deserves the warning most — max-review
    // find: the disclosure was --json-scoped, so one engine presented a reclassification as real).
    let baseline_version = report_build_version(base_pre);
    let engine_version = report_build_version(cur_pre);
    if !baseline_version.is_empty() && !engine_version.is_empty() && baseline_version != engine_version {
        eprintln!("candor: ⚠ baseline @{baseline_version} ≠ engine @{engine_version} — a \"gained capability\" may be the engine reclassifying, not the dependency changing; regenerate both reports with one build to compare releases.");
    }
    if want_json {
        // The package-level supply-chain alarm (spec §5.1): `gained` is the UNION of effects the
        // surface gained between the two reports — a dependency that grew a Net/Exec reach between
        // releases — with the per-function detail under `byFunction`. Machine-readable so a CI gate
        // can alarm on a dependency update that quietly gains a capability.
        //
        // ⟨0.12 staged⟩ each entry carries `origin` — the candor-gains prototype's key finding
        // promoted into the open query. A gain on a fn that EXISTED at the baseline (shipped pure,
        // now does Net — the supply-chain attack signal) is a different alarm from a NEW fn that
        // does Net (a feature). Reports OMIT pure functions (§2), so existence is keyed on the
        // baseline CALLGRAPH (a baseline-pure fn is a graph node with no report entry):
        //   "existing" — in the baseline report, or a baseline-callgraph node (caller or callee);
        //   "new"      — in neither (the fn did not exist at the baseline);
        //   "unknown"  — absent from the baseline report AND the baseline callgraph is
        //                PARTIAL-OR-ABSENT (no sidecar found, or a matched sidecar failed to
        //                read/parse — the fn may have lived in the dropped file): existence is
        //                undecidable, DISCLOSED rather than guessed (§4). Without the partial arm a
        //                dropped sidecar made its fns read "new", downgrading the supply-chain
        //                attack signal (an EXISTING fn gaining an effect) to a feature.
        // JSON-only: the human `fn\teffect` TSV is a pinned consumer surface (candor-run.sh's
        // seen-file dedup matches whole lines) and stays byte-stable.
        let (base_cg, base_cg_partial) = load_callgraph_flagged(base_pre);
        let base_cg_nodes: BTreeSet<&str> = base_cg
            .iter()
            .flat_map(|(k, vs)| std::iter::once(k.as_str()).chain(vs.iter().map(String::as_str)))
            .collect();
        let cg_unusable = base_cg.is_empty() || base_cg_partial;
        let origin_of = |f: &str| gain_origin(f, &base, &base_cg_nodes, cg_unusable);
        let mut gained: Vec<&str> = out.iter().map(|(_, e)| e.as_str()).collect();
        gained.sort_unstable();
        gained.dedup();
        let by_function: Vec<_> = out
            .iter()
            .map(|(f, e)| serde_json::json!({ "effect": e, "fn": f, "origin": origin_of(f) }))
            .collect();
        // Top-level keys ALPHABETICAL (baseline_version, byFunction, engine_version, gained) — the
        // cross-engine gains shape candor-ts/candor-java emit.
        let v = serde_json::json!({
            "baseline_version": baseline_version,
            "byFunction": by_function,
            "engine_version": engine_version,
            "gained": gained,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return 0;
    }
    for (func, e) in out {
        println!("{func}\t{e}");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receipt's `RECEIPT_ORDER` must stay a permutation of `candor_report::EFFECTS` — a new effect
    /// added to EFFECTS but not to RECEIPT_ORDER would silently vanish from the receipt (`audit` would
    /// still show it — a divergence). This RUNS in CI (`cargo test`), unlike the old in-fn
    /// `debug_assert_eq!` which the release binary CI ships compiled OUT. Fails loud if the arrays drift.
    #[test]
    fn receipt_order_is_a_permutation_of_effects() {
        assert_eq!(
            RECEIPT_ORDER.iter().copied().collect::<BTreeSet<_>>(),
            EFFECTS.iter().copied().collect::<BTreeSet<_>>(),
            "receipt RECEIPT_ORDER must be a permutation of candor_report::EFFECTS \
             (a new effect was added to one array but not the other)",
        );
    }

    /// A FOUND-but-corrupt report must make the comparative verbs fail LOUD (exit 2) — never an exit-0
    /// empty answer. `gains --json` over a corrupt current report printed `{"gained":[],"byFunction":[]}`
    /// — a supply-chain ALL-CLEAR over corrupt input, the §4 cardinal sin (and `diff` the same via its
    /// quiet loads). Pins load_fninfo_loud end-to-end through cmd_gains and cmd_diff.
    #[test]
    fn corrupt_report_fails_gains_and_diff_loud_not_empty() {
        let dir = std::env::temp_dir().join(format!("candor-gains-loud-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let (cur, base) = (dir.join("cur"), dir.join("base"));
        let (cur_s, base_s) = (cur.to_string_lossy().into_owned(), base.to_string_lossy().into_owned());
        let valid = serde_json::json!({
            "candor": { "version": "b1", "toolchain": "stable", "spec": candor_report::SPEC_VERSION },
            "functions": [ { "fn": "a::f", "inferred": ["Fs"], "direct": ["Fs"] } ]
        });
        std::fs::write(dir.join("base.demo.scan.json"), serde_json::to_string(&valid).unwrap()).unwrap();

        // (a) current FOUND but truncated/garbage → exit 2 from gains AND diff, JSON or not.
        std::fs::write(dir.join("cur.demo.scan.json"), r#"{ "candor": {}, "functions": [ { "fn": "x."#).unwrap();
        let args = |j: bool| -> Vec<String> {
            let mut v = vec![cur_s.clone(), base_s.clone()];
            if j { v.push("--json".into()) }
            v
        };
        assert_eq!(cmd_gains(&args(true)), 2, "gains --json over a corrupt current report must exit 2");
        assert_eq!(cmd_gains(&args(false)), 2, "gains (TSV) over a corrupt current report must exit 2");
        assert_eq!(cmd_diff(&args(true)), 2, "diff over a corrupt current report must exit 2");

        // (b) semantic corruption — parses as a bare array but every entry is junk → still loud.
        std::fs::write(dir.join("cur.demo.scan.json"), "[1, 2, 3]").unwrap();
        assert_eq!(load_fninfo_loud(&cur_s, "current").err(), Some(2), "all-junk array must fail loud");

        // (c) a corrupt BASELINE is just as untrustworthy (every effect would read newly gained).
        std::fs::write(dir.join("cur.demo.scan.json"), serde_json::to_string(&valid).unwrap()).unwrap();
        std::fs::write(dir.join("base.demo.scan.json"), "not json").unwrap();
        assert_eq!(cmd_gains(&args(true)), 2, "gains over a corrupt baseline must exit 2");

        // (d) both sides valid → exit 0 (the tolerant path unchanged); a CLEAN-empty report stays Ok.
        std::fs::write(dir.join("base.demo.scan.json"), serde_json::to_string(&valid).unwrap()).unwrap();
        assert_eq!(cmd_gains(&args(true)), 0, "valid reports still succeed");
        let clean_empty = serde_json::json!({
            "candor": { "version": "b1", "toolchain": "stable", "spec": candor_report::SPEC_VERSION },
            "functions": []
        });
        std::fs::write(dir.join("cur.demo.scan.json"), serde_json::to_string(&clean_empty).unwrap()).unwrap();
        assert!(load_fninfo_loud(&cur_s, "current").is_ok(), "a well-formed empty report is not corrupt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A PARTIAL baseline callgraph (a matched sidecar failed to read/parse) must classify a fn absent
    /// from the surviving graph as origin "unknown", never "new" — the fn may have lived in the dropped
    /// file, and "new" downgrades the supply-chain attack signal (an EXISTING fn gaining an effect) to
    /// a feature. A genuinely ABSENT sidecar stays the ordinary empty (→ unknown) case, and a fully
    /// valid graph still mints "new"/"existing" as before.
    #[test]
    fn partial_baseline_callgraph_reads_unknown_not_new() {
        let dir = std::env::temp_dir().join(format!("candor-cg-partial-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let base = dir.join("base");
        let base_s = base.to_string_lossy().into_owned();
        std::fs::write(dir.join("base.a.scan.callgraph.json"), r#"{"a::caller":["a::callee"]}"#).unwrap();
        std::fs::write(dir.join("base.b.scan.callgraph.json"), r#"{"b::caller": ["b::"#).unwrap(); // corrupt

        let (cg, partial) = load_callgraph_flagged(&base_s);
        assert!(partial, "a matched-but-corrupt sidecar must flag the graph PARTIAL");
        assert!(cg.contains_key("a::caller"), "the surviving sidecar still merges");

        let nodes: BTreeSet<&str> =
            cg.iter().flat_map(|(k, vs)| std::iter::once(k.as_str()).chain(vs.iter().map(String::as_str))).collect();
        let report_base: BTreeMap<String, FnInfo> = BTreeMap::new();
        let unusable = cg.is_empty() || partial;
        // the fn that lived only in the CORRUPT sidecar: absent from graph + report → unknown, not new.
        assert_eq!(gain_origin("b::caller", &report_base, &nodes, unusable), "unknown");
        // a surviving graph node is still existing (partial never erases positive evidence).
        assert_eq!(gain_origin("a::callee", &report_base, &nodes, unusable), "existing");

        // the same graph, both sidecars valid → partial=false, and an absent fn is genuinely "new".
        std::fs::write(dir.join("base.b.scan.callgraph.json"), r#"{"b::caller":["b::callee"]}"#).unwrap();
        let (cg2, partial2) = load_callgraph_flagged(&base_s);
        assert!(!partial2, "two valid sidecars are not partial");
        let nodes2: BTreeSet<&str> =
            cg2.iter().flat_map(|(k, vs)| std::iter::once(k.as_str()).chain(vs.iter().map(String::as_str))).collect();
        assert_eq!(gain_origin("never::existed", &report_base, &nodes2, cg2.is_empty()), "new");

        // an ABSENT sidecar (no callgraph files at all) is NOT partial — it's the empty (unknown) case.
        let bare = dir.join("bare");
        let (cg3, partial3) = load_callgraph_flagged(&bare.to_string_lossy());
        assert!(cg3.is_empty() && !partial3, "a genuinely absent sidecar is empty, not partial");
        assert_eq!(gain_origin("anything", &report_base, &BTreeSet::new(), true), "unknown");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The gains provenance sources: report_build_version reads the envelope's `candor.version`, falls
    /// back to `meta.version` (older shapes), and returns "" when absent/unreadable — the "" is what
    /// gains --json emits as baseline_version/engine_version when unknown (candor-ts parity).
    #[test]
    fn report_build_version_reads_envelope_with_meta_fallback() {
        let dir = std::env::temp_dir().join(format!("candor-buildver-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pre = dir.join("r");
        let pre_s = pre.to_string_lossy().into_owned();
        let report = dir.join("r.demo.scan.json");

        std::fs::write(&report, r#"{"candor":{"version":"0.9.9","toolchain":"t"},"functions":[]}"#).unwrap();
        assert_eq!(report_build_version(&pre_s), "0.9.9", "the §2 envelope candor.version");

        std::fs::write(&report, r#"{"meta":{"version":"0.1.1","toolchain":"t"},"functions":[]}"#).unwrap();
        assert_eq!(report_build_version(&pre_s), "0.1.1", "older meta.version shape");

        std::fs::write(&report, r#"{"functions":[]}"#).unwrap();
        assert_eq!(report_build_version(&pre_s), "", "no version anywhere → empty string");

        std::fs::write(&report, "not json").unwrap();
        assert_eq!(report_build_version(&pre_s), "", "unreadable → empty string, never a guess");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
