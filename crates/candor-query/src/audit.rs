//! `audit` — the effect overview + κ-coverage/honesty summary.

use crate::*;

/// Per-effect counts (only the `EFFECTS` vocabulary — `Unknown` is excluded) plus the names of the
/// functions whose effect set may be incomplete (`unresolved`, or containing `Unknown`). Shared by the
/// `audit` and `receipt` views so the tally + unresolved rule are defined exactly once.
pub(crate) fn tally_effects(fns: &[ReportEntry]) -> (BTreeMap<&'static str, usize>, Vec<String>) {
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

/// THE definition of a "gained" effect: present in `cur`'s set, absent from `base`'s. Shared by `diff`
/// (per-function gained list) and `gains` (the self-review pairs) so they can't drift.
pub(crate) fn gained_effects(cur: &BTreeSet<String>, base: &BTreeSet<String>) -> Vec<String> {
    cur.difference(base).cloned().collect()
}

/// `cargo candor audit` default view: an at-a-glance effect profile aggregated across the crates.
/// Args: `<prefix> <engine_ver> <suspect_file>`. Always exits 0 (a report renderer).
pub(crate) fn cmd_audit(args: &[String]) -> i32 {
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
    let rfs = report_files(pre);
    // NO report files is a path error, not an all-pure crate — without this guard the "everything
    // pure" message below printed for a typo'd prefix, the worst possible misreading (same
    // no-files-fails-loud rule as the query commands; an effect-free crate still writes a report).
    if rfs.is_empty() {
        eprintln!("candor: no report files at prefix `{pre}` — check the path.");
        return 2;
    }
    for rf in rfs {
        let label = format!("{}.{}", rf.krate, rf.kind);
        analyzed.insert(rf.krate.clone());
        // Disclose a read/parse failure (mirroring load_entries) — a silent unwrap_or_default() turned a
        // corrupt report into an empty entry list, so the "everything is pure" message below printed for a
        // mid-write/corrupt report: the exact silent-pure misread candor's never-silently-pure rule forbids.
        let es = match std::fs::read_to_string(&rf.path) {
            Ok(t) => match report_entries_counted(&t) {
                Some((es, dropped)) => {
                    if dropped > 0 {
                        eprintln!("candor: report {} — {dropped} function entr{} could not be parsed and {} OMITTED from this audit (corrupt or mid-write); re-run the scan",
                            rf.path.display(),
                            if dropped == 1 { "y" } else { "ies" },
                            if dropped == 1 { "is" } else { "are" });
                    }
                    es
                }
                None => {
                    eprintln!("candor: report {} failed to parse — its functions are OMITTED from this audit (corrupt or mid-write); re-run the scan", rf.path.display());
                    Vec::new()
                }
            },
            Err(_) => {
                eprintln!("candor: report {} could not be read — its functions are OMITTED from this audit", rf.path.display());
                Vec::new()
            }
        };
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
