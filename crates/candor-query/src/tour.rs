//! `tour` — the ON-DEMAND, top-N version of the cold-repo opener (SURFACE-BEST-FIND-DESIGN.md, P2).
//!
//! Lists the N most SURPRISING transitive reaches in an existing report — no re-scan. The scan-time note
//! (candor-scan) surfaces the single best find; `tour` is the guided-poke: "show me the next few". It
//! delegates to the SHARED `candor_classify::surface::best_finds` (the same heuristic the scan-note uses,
//! so the ranking can't drift), reading the report + callgraph the scan already wrote.

use crate::*;

/// The report's `package` name (the §2 envelope field), or `None` if absent/unreadable — the tour header
/// prefers it (meaningful, locator-independent) over the prefix basename. A `packages` PLURAL envelope
/// (the JVM shape, SPEC §2) is honoured too: one entry names it verbatim; several name their longest
/// common dotted prefix (`com.uflexi.actions` + `com.uflexi.dao` → `com.uflexi`); none shared → None.
fn report_package(prefix: &str) -> Option<String> {
    let path = glob_reports(prefix).into_iter().next()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(p) = v.get("package").and_then(|p| p.as_str()).filter(|s| !s.is_empty()) {
        return Some(p.to_string());
    }
    let pkgs: Vec<&str> = v
        .get("packages")?
        .as_array()?
        .iter()
        .filter_map(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    packages_label(&pkgs)
}

/// The longest common dot-separated prefix of a plural `packages` list — the tour header's label for a
/// multi-package (JVM-shape) report. Whole segments only (`com.ab` + `com.ac` share `com`, not `com.a`).
fn packages_label(pkgs: &[&str]) -> Option<String> {
    let first: Vec<&str> = pkgs.first()?.split('.').collect();
    let mut n = first.len();
    for p in &pkgs[1..] {
        let segs: Vec<&str> = p.split('.').collect();
        let mut i = 0;
        while i < n.min(segs.len()) && segs[i] == first[i] {
            i += 1;
        }
        n = i;
        if n == 0 {
            return None; // nothing shared — the basename fallback is more honest than a wrong name
        }
    }
    Some(first[..n].join("."))
}

/// `tour [<N>]` — the N (default 10) most surprising reaches in the report's crate, each with a
/// ready-to-run `candor path <fn> <effect>` command. §3.3.1 grammar: report DISCOVERED or via `--report`,
/// `--json` for machines. The optional positional integer sets how many to list. Read-only.
pub(crate) fn cmd_tour(args: &[String]) -> i32 {
    let g = parse(args, Shape { verb_args: 1, sentinel: false, has_policy: false });
    let want_json = g.want_json;
    // The lone optional positional is N (how many to list); default 10. It MUST be a positive integer —
    // a non-integer OR zero is a usage error (exit 2). `tour 0` must never print the honest-sounding
    // "nothing hidden" over an effectful crate: that would be a false all-clear (the §4 cardinal sin).
    let n: usize = match g.positional.first() {
        None => 10,
        Some(s) => match s.parse::<usize>() {
            Ok(v) if v >= 1 => v,
            _ => {
                eprintln!("usage: candor-query tour [<N>] [--report <locator>] [--json]   (N is a positive integer ≥ 1)");
                return 2;
            }
        },
    };
    let Some(pre) = report_or_discover(&g) else {
        eprintln!("candor: no report found (no --report and no .candor/ discovered) — scan the crate first.");
        return 2;
    };
    let pre = pre.as_str();
    // Fail LOUD on a missing/typo'd report (exit 2), like the other verbs — never read as an empty report.
    let entries = match load_entries_loud(pre) {
        Ok(v) => v,
        Err(c) => return c,
    };

    // Build the maps the heuristic wants from the report entries + the callgraph sidecar. `inferred`/
    // `direct` come from the report; `calls` prefers the full callgraph sidecar (records EVERY edge —
    // like the scan held in memory) and falls back to the report's effect-relevant `calls`. `loc` maps a
    // function to its "file:line" for the source callout.
    let mut inferred: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    for e in &entries {
        inferred.insert(e.func.clone(), e.inferred.iter().cloned().collect());
        if !e.direct.is_empty() {
            direct.insert(e.func.clone(), e.direct.iter().cloned().collect());
        }
        if !e.loc.is_empty() {
            loc.insert(e.func.clone(), e.loc.clone());
        }
    }
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let cg = load_callgraph(pre);
    if cg.is_empty() {
        for e in &entries {
            if !e.calls.is_empty() {
                calls.insert(e.func.clone(), e.calls.iter().cloned().collect());
            }
        }
    } else {
        for (k, v) in cg {
            calls.insert(k, v.into_iter().collect());
        }
    }

    let finds = candor_classify::surface::best_finds(&inferred, &direct, &calls, &loc, n);

    // The header names the report's PACKAGE (from the §2 envelope) — meaningful and locator-independent,
    // so every engine and every --report form print the same crate. Falls back to the prefix basename.
    let crate_name = report_package(pre).unwrap_or_else(|| prefix_base(pre));
    if want_json {
        #[derive(Serialize)]
        struct Reach {
            #[serde(rename = "fn")]
            func: String,
            effect: String,
            hops: usize,
            source: String,
            loc: String,
            score: i64,
        }
        let reaches: Vec<Reach> = finds
            .iter()
            .map(|f| Reach {
                func: f.func.clone(),
                effect: f.effect.clone(),
                hops: f.hops,
                source: f.source.clone(),
                loc: f.source_loc.clone(),
                score: f.score,
            })
            .collect();
        let out = serde_json::json!({ "reaches": reaches });
        println!("{}", serde_json::to_string(&out).unwrap());
        return 0;
    }

    if finds.is_empty() {
        // Effectful-but-nothing-surprising vs genuinely-pure both land here; either way the honest line
        // is the useful answer (never a manufactured surprise) — mirrors the scan-note fallback.
        println!("candor: nothing hidden — every effect sits where its name says it should.");
        return 0;
    }
    println!("candor tour — the {} most surprising reach{} in {crate_name}:", finds.len(), if finds.len() == 1 { "" } else { "es" });
    for (i, f) in finds.iter().enumerate() {
        let hop_word = if f.hops == 1 { "hop" } else { "hops" };
        let where_s = if f.source_loc.is_empty() { String::new() } else { format!(" ({})", f.source_loc) };
        println!(
            "  {}. `{}` performs {}, {} {} away via `{}`{}",
            i + 1, f.func, f.effect, f.hops, hop_word, f.source, where_s
        );
        println!("     →  candor path {} {}", f.func, f.effect);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a report + callgraph on disk, run `tour`, and confirm the ranked list names the
    /// benign-deep reach (a "settings"-named fn that reaches Net a few hops down).
    #[test]
    fn tour_names_the_benign_deep_reach() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("candor-tour-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let prefix = dir.join("report");
        let prefix_s = prefix.to_string_lossy().into_owned();

        // A report where settings::Settings::load inherits Net 3 hops down via net_layer::do_send, plus
        // an effecty api::fetch that must NOT win (excluded by the leaf lexicon).
        let report = serde_json::json!({
            "meta": { "version": "scan-test", "toolchain": "stable", "spec": candor_report::SPEC_VERSION },
            "crate": "demo",
            "functions": [
                { "fn": "net_layer::do_send", "loc": "src/net.rs:9", "inferred": ["Net"], "direct": ["Net"] },
                { "fn": "core::sync_state", "inferred": ["Net"], "calls": ["net_layer::do_send"] },
                { "fn": "core::refresh", "inferred": ["Net"], "calls": ["core::sync_state"] },
                { "fn": "settings::Settings::load", "loc": "src/lib.rs:5", "inferred": ["Net"], "calls": ["core::refresh"] },
                { "fn": "api::fetch", "inferred": ["Net"], "calls": ["net_layer::do_send"] }
            ]
        });
        let report_path = dir.join("report.demo.scan.json");
        let mut fh = std::fs::File::create(&report_path).unwrap();
        fh.write_all(serde_json::to_string(&report).unwrap().as_bytes()).unwrap();

        // The callgraph sidecar (caller -> callees), the same edges the scan held in memory.
        let cg = serde_json::json!({
            "core::sync_state": ["net_layer::do_send"],
            "core::refresh": ["core::sync_state"],
            "settings::Settings::load": ["core::refresh"],
            "api::fetch": ["net_layer::do_send"]
        });
        let cg_path = dir.join("report.demo.scan.callgraph.json");
        let mut cgh = std::fs::File::create(&cg_path).unwrap();
        cgh.write_all(serde_json::to_string(&cg).unwrap().as_bytes()).unwrap();

        // Re-derive the maps and call the heuristic directly (the same path cmd_tour takes), asserting
        // the ranked shape without capturing stdout.
        let entries = load_entries_loud(&prefix_s).expect("report loads");
        let mut inferred: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut direct: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut loc: HashMap<String, String> = HashMap::new();
        for e in &entries {
            inferred.insert(e.func.clone(), e.inferred.iter().cloned().collect());
            if !e.direct.is_empty() {
                direct.insert(e.func.clone(), e.direct.iter().cloned().collect());
            }
            if !e.loc.is_empty() {
                loc.insert(e.func.clone(), e.loc.clone());
            }
        }
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (k, v) in load_callgraph(&prefix_s) {
            calls.insert(k, v.into_iter().collect());
        }
        let finds = candor_classify::surface::best_finds(&inferred, &direct, &calls, &loc, 10);
        assert!(!finds.is_empty(), "the benign-deep reach should surface");
        assert_eq!(finds[0].func, "settings::Settings::load");
        assert_eq!(finds[0].effect, "Net");
        assert_eq!(finds[0].hops, 3);
        assert_eq!(finds[0].source, "net_layer::do_send");
        // api::fetch (effecty) is excluded — it never appears in the tour.
        assert!(finds.iter().all(|f| f.func != "api::fetch"));

        // And the verb itself returns success (exit 0) against the on-disk report.
        assert_eq!(cmd_tour(&["--report".into(), report_path.to_string_lossy().into_owned()]), 0);
        assert_eq!(cmd_tour(&["5".into(), "--report".into(), report_path.to_string_lossy().into_owned()]), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
