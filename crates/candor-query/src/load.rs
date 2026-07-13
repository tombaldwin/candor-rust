//! Report/sidecar loading: the prefix globs, the report entries, the callgraph
//! and hierarchy sidecars, and the backend/provenance probes.

use crate::*;

// ── report loading ──────────────────────────────────────────────────────────────────────────────

/// Report file PATHS for a prefix — the `<base>.<crate>.<type>.json` reports, sorted, excluding the
/// `.calibrated`/`.encountered-*` sidecars. Thin wrapper over `candor_report::report_files`, which
/// owns the ONE discrimination rule shared with the lint (so the two can't disagree).
pub(crate) fn glob_reports(prefix: &str) -> Vec<PathBuf> {
    report_files(prefix).into_iter().map(|r| r.path).collect()
}

/// All report entries across the matching files. A report file that is present but won't parse is
/// DISCLOSED on stderr (not silently skipped): dropping it silently would make its effectful functions
/// read as "no effect" — a silent under-report against the never-silently-pure promise. Still tolerant
/// (one corrupt file doesn't kill the merged query), but the blind spot is now visible. With atomic
/// report writes (candor_report::write_atomic) this only fires on genuine corruption, not a mid-write race.
pub(crate) fn load_entries(prefix: &str) -> Vec<ReportEntry> {
    load_entries_inner(prefix).0
}

/// The shared loader. Returns the merged entries AND whether a matched file wholly FAILED to read or
/// parse (`hard_fail`) — the loud wrapper needs that bit to tell "an empty-but-valid report" apart from
/// "every report we found was corrupt", which must never read as an empty (all-clear) answer.
fn load_entries_inner(prefix: &str) -> (Vec<ReportEntry>, bool) {
    let mut out = Vec::new();
    let mut hard_fail = false;
    for path in glob_reports(prefix) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("candor: report {} could not be read — its functions are OMITTED from this query", path.display());
            hard_fail = true;
            continue;
        };
        match report_entries_counted(&text) {
            Some((es, dropped)) => {
                if dropped > 0 {
                    // A per-entry drop is the same under-report as a whole-file failure — disclose it,
                    // never let a corrupt function silently vanish (and read as pure) from the query.
                    eprintln!(
                        "candor: report {} — {dropped} function entr{} could not be parsed and {} OMITTED from this query (corrupt or mid-write); re-run the scan",
                        path.display(),
                        if dropped == 1 { "y" } else { "ies" },
                        if dropped == 1 { "is" } else { "are" },
                    );
                }
                out.extend(es);
            }
            None => {
                eprintln!(
                    "candor: report {} failed to parse — its functions are OMITTED from this query (corrupt or mid-write); re-run the scan",
                    path.display()
                );
                hard_fail = true;
            }
        }
    }
    (out, hard_fail)
}

/// `load_entries`, but a prefix matching NO report files fails LOUD (exit 2) instead of reading as an
/// empty report. A typo'd prefix otherwise yields an authoritative-looking empty answer — `map`/`where`
/// printed `{}` and exited 0, which an agent (or a gate built on the output) reads as "no effects"
/// rather than "wrong path" (found by dogfooding the umbrella AGENTS.md route; `diff`/`rewire` were
/// already guarded). A legitimately effect-free crate still writes a report file, so "no files" is
/// always an error, never an empty answer.
pub(crate) fn load_entries_loud(prefix: &str) -> Result<Vec<ReportEntry>, i32> {
    if glob_reports(prefix).is_empty() {
        eprintln!("candor: no report files at prefix `{prefix}` — check the path.");
        return Err(2);
    }
    let (entries, hard_fail) = load_entries_inner(prefix);
    // A report file was FOUND but yielded no usable entries because it failed to read/parse — that is a
    // corrupt report, NOT an effect-free crate. Returning `Ok(empty)` here would let a caller read the
    // emptiness as "no effects": `tour` prints "nothing hidden", a policy `map`/gate PASSES — the §4
    // cardinal-sin false all-clear. A legitimately effect-free crate still writes a report that LISTS its
    // functions, so empty-after-a-parse-failure is always the corrupt case. Fail loud (the disclosure
    // above already named the file). One corrupt file among several still merges — that stays tolerant
    // (entries non-empty → Ok), only a net-empty parse failure is fatal.
    if entries.is_empty() && hard_fail {
        eprintln!("candor: every report found at prefix `{prefix}` failed to load — refusing to report an empty (all-clear) answer over a corrupt report; re-run the scan.");
        return Err(2);
    }
    Ok(entries)
}

// ── audit ───────────────────────────────────────────────────────────────────────────────────────

/// The basename of a prefix (`.candor/report` → `report`), used to label per-crate report files.
pub(crate) fn prefix_base(prefix: &str) -> String {
    Path::new(prefix).file_name().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

/// Encountered-crate sidecars: `<base>.encountered-*.json` (not matched by the `.*.*.json` report
/// glob — only two dot-segments). Each holds a JSON array of crate names candor *saw called*.
pub(crate) fn glob_encountered(prefix: &str) -> Vec<PathBuf> {
    // Strip a `.json` direct-file locator (§3.3.1) so the encountered-crate sidecars glob matches
    // `<stem>.encountered-*.json` — otherwise audit/receipt κ-coverage silently drops them.
    let prefix = prefix.strip_suffix(".json").unwrap_or(prefix);
    let dir = match Path::new(prefix).parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let base = prefix_base(prefix);
    let needle = format!("{base}.encountered-");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            if let Some(n) = ent.file_name().to_str()
                && let Some(mid) = n.strip_prefix(&needle).and_then(|m| m.strip_suffix(".json")) {
                    // an encountered sidecar is `<base>.encountered-<crate>-<kind>.json` — a SINGLE
                    // dotless segment. Excluding any further dot avoids mis-claiming the REPORT of a
                    // crate literally named `encountered-…` (`<base>.encountered-foo.lib.json`).
                    if !mid.contains('.') {
                        out.push(ent.path());
                    }
                }
        }
    }
    out
}

/// The external crates candor saw resolved calls into — the union of the `<prefix>.encountered-*.json`
/// sidecars. Shared by `audit` (coverage gaps) and `receipt`.
pub(crate) fn encountered_set(prefix: &str) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in glob_encountered(prefix) {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(arr) = serde_json::from_str::<Vec<String>>(&text) {
                seen.extend(arr);
            }
    }
    seen
}

/// The calibrated-coverage sidecar `<dir>/<base>.calibrated.json` → (crates, prefixes, path_crates).
/// `path_crates` are crates the engine matches by path-prefix (tokio/async_std/mio) — covered, but
/// absent from the crate-name list — so the coverage check doesn't mislabel them as blind spots.
pub(crate) fn load_calibrated(prefix: &str, base: &str) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
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

/// Load the type-hierarchy sidecar(s) (`<prefix>.*.hierarchy.json`, 0.7), or empty if absent (→ the
/// frontier falls back to a simple-name match, which over-lists — the safe lower-bound direction).
pub(crate) fn load_hierarchy(prefix: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // A `--report <file>.json` locator arrives here verbatim (§3.3.1 direct-file load); its sidecars are
    // named `<stem>.hierarchy.json`, NOT `<stem>.json.*.hierarchy.json`, so strip the `.json` before
    // forming the glob prefix — otherwise the sidecar is silently dropped (a report loaded by `.json` path
    // would lose its type hierarchy / call graph, under-reporting transitive queries at exit 0).
    let prefix = prefix.strip_suffix(".json").unwrap_or(prefix);
    let p = Path::new(prefix);
    let dir = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let Some(base) = p.file_name().and_then(|s| s.to_str()) else {
        return out;
    };
    let pfx = format!("{base}.");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&pfx) || !name.ends_with(".hierarchy.json") {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(ent.path())
            && let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text)
        {
            for (k, v) in map {
                out.entry(k).or_default().extend(v);
            }
        }
    }
    out
}

/// Load + merge every `<prefix>.*.callgraph.json` sidecar into one `caller -> [callees]` map (by path).
pub(crate) fn load_callgraph(prefix: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Strip a `.json` (a §3.3.1 direct-file locator) so the sidecar glob matches `<stem>.callgraph.json`
    // and never silently drops the call graph — see load_hierarchy for the full note.
    let prefix = prefix.strip_suffix(".json").unwrap_or(prefix);
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
        // A corrupt/unreadable callgraph file silently shrinks the graph — so a blast-radius query
        // (whatif/rewire/callers) UNDER-reports, and `whatif` can return a false GREEN on the security
        // boundary (an in-scope caller whose edge lived in the dropped file vanishes). Sweep-2 fixed this
        // asymmetry on the REPORT path (load_entries discloses) but not here. Disclose both failure arms,
        // mirroring that fix — never silently drop graph edges a gate verdict depends on.
        let path = ent.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("candor: callgraph {} could not be read — its edges are OMITTED, so blast-radius queries (whatif/rewire/callers) UNDER-report", path.display());
            continue;
        };
        match serde_json::from_str::<BTreeMap<String, Vec<String>>>(&text) {
            Ok(map) => {
                for (k, v) in map {
                    out.entry(k).or_default().extend(v);
                }
            }
            Err(_) => eprintln!("candor: callgraph {} failed to parse — its edges are OMITTED, so blast-radius queries (whatif/rewire/callers) UNDER-report (corrupt or mid-write); re-run the scan", path.display()),
        }
    }
    out
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
pub(crate) fn report_backend(prefix: &str) -> &'static str {
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
pub(crate) fn is_scan_artifact(base: &str, name: &str) -> bool {
    name.strip_prefix(base)
        .and_then(|r| r.strip_prefix('.'))
        .is_some_and(|rest| rest.contains(".scan.") || rest.ends_with(".scan.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A report file that is FOUND but wholly fails to parse must make `load_entries_loud` return
    /// Err(2) — NOT Ok(empty). Ok(empty) would let a caller (tour, a policy map/gate) read the emptiness
    /// as "no effects": the §4 cardinal-sin false all-clear over a corrupt report. A valid report always
    /// lists its functions, so empty-after-a-parse-failure is always the corrupt case (dogfood find).
    #[test]
    fn corrupt_report_fails_loud_not_empty() {
        let dir = std::env::temp_dir().join(format!("candor-loud-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let prefix = dir.join("report");
        let prefix_s = prefix.to_string_lossy().into_owned();

        // (a) no report files at all → loud (the pre-existing guard).
        assert_eq!(load_entries_loud(&prefix_s).err(), Some(2), "no files must fail loud");

        // (b) a FOUND but truncated/garbage report → loud (the fix): parse fails, entries empty.
        let corrupt = dir.join("report.demo.scan.json");
        std::fs::File::create(&corrupt).unwrap()
            .write_all(br#"{ "candor": {}, "functions": [ { "fn": "x."#).unwrap();
        assert_eq!(load_entries_loud(&prefix_s).err(), Some(2), "a corrupt report must fail loud, never Ok(empty)");

        // (c) a VALID report → Ok with entries (the tolerant path is unchanged).
        let valid = serde_json::json!({
            "meta": { "version": "t", "toolchain": "stable", "spec": candor_report::SPEC_VERSION },
            "crate": "demo",
            "functions": [ { "fn": "a::f", "inferred": ["Fs"], "direct": ["Fs"] } ]
        });
        std::fs::write(&corrupt, serde_json::to_string(&valid).unwrap()).unwrap();
        let got = load_entries_loud(&prefix_s).expect("a valid report loads");
        assert!(!got.is_empty(), "a valid report yields entries");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
