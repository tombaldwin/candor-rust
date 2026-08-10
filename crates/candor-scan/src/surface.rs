//! Surface the single most SURPRISING transitive reach (the cold-repo hook).
//!
//! After the effect summary + κ ledger, candor-scan emits ONE more stderr line: the most surprising
//! transitive reach in the crate + a ready-to-run `candor path` command. See SURFACE-BEST-FIND-DESIGN.md.
//!
//! The heuristic itself (score, lexicons, tokenizer, BFS, tie-break) lives in the SHARED
//! `candor_classify::surface` crate so this scan-time note and candor-query's `tour` verb CANNOT drift.
//! This module keeps only the scan-time PRESENTATION (the single-line note + the honest "nothing hidden"
//! fallback + the "emit nothing when the crate has no effects" rule). The emitted bytes are unchanged
//! from before the extraction (conformance PART 4f + the candor-scan tests pin it).

use std::collections::{BTreeSet, HashMap};

/// Emit the surface note to STDERR. `loc` maps qual → "file:line" for the source callout.
///
/// Three-way behavior, unchanged from before the shared-crate extraction:
///   - ZERO effectful functions → emit nothing;
///   - effectful, but nothing clears the bar → the honest "nothing hidden" fallback;
///   - a winning reach → the single-line "most surprising reach" note + the `candor path` command.
/// `uncovered_pkgs` is the κ ledger's size — the dependency packages this scan saw calls into but cannot
/// classify. It is the cause the run can PROVE when it is non-empty, which is why it is a parameter
/// rather than a guess baked into the sentence below.
pub fn emit(
    inferred: &HashMap<String, BTreeSet<&'static str>>,
    direct: &HashMap<String, BTreeSet<&'static str>>,
    calls: &HashMap<String, BTreeSet<String>>,
    loc: &HashMap<String, String>,
    uncovered_pkgs: usize,
) {
    if !candor_classify::surface::any_effectful(inferred) {
        return; // zero effectful functions — emit nothing
    }
    let finds = candor_classify::surface::best_finds(inferred, direct, calls, loc, 1);
    let Some(f) = finds.first() else {
        // effectful, but nothing cleared the bar — the honest fallback (never a manufactured surprise).
        // BUT do NOT reassure "nothing hidden" over a meaningfully-Unknown graph: those Unknowns (unresolved
        // calls) ARE the hidden part, their transitive effects unanalyzed. ≥⅓ of effectful fns Unknown →
        // qualify + point at blindspots (corpus re-audit cardinal sin; four-way with candor-ts).
        let total = inferred.values().filter(|s| !s.is_empty()).count();
        let unknown = inferred.values().filter(|s| s.contains("Unknown")).count();
        if total > 0 && unknown * 3 >= total {
            let cause = if uncovered_pkgs > 0 {
                format!(
                    "; the {uncovered_pkgs} package{} not covered by the classifier (named above), so \
                     calls into them resolve to Unknown",
                    if uncovered_pkgs == 1 { " is" } else { "s are" }
                )
            } else {
                "; unresolvable imports are the usual cause".to_string()
            };
            eprintln!(
                // NAME THE CAUSE THAT APPLIES. This used to end "unresolvable imports or missing project
                // config are the usual cause" unconditionally — a guess, and on the sibling engine the
                // same shape blamed a missing tsconfig on a run that had just read one, while the real
                // cause (packages the classifier does not cover) had already been printed two lines
                // above. `uncovered` is that ledger; when it is non-empty it is the answer.
                "candor: no surprising reaches — but {unknown} of {total} function(s) are Unknown \
                 (unresolved calls; their transitive effects are NOT analyzed). Run `candor blindspots`{cause}."
            );
            return;
        }
        eprintln!("candor: nothing hidden — every effect sits where its name says it should.");
        return;
    };
    let where_s = if f.source_loc.is_empty() { "?" } else { f.source_loc.as_str() };
    let hop_word = if f.hops == 1 { "hop" } else { "hops" };
    let benign_note = if f.benign_token.is_empty() {
        String::new()
    } else {
        format!("          a \"{}\"-named function reaching {}.\n", f.benign_token, f.effect)
    };
    eprintln!(
        "candor: most surprising reach — `{}` performs {}, {} {} away via `{}` ({}).\n{}          →  candor path {} {}",
        f.func, f.effect, f.hops, hop_word, f.source, where_s, benign_note, f.func, f.effect
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use candor_classify::surface::best_finds;

    fn set(items: &[&'static str]) -> BTreeSet<&'static str> {
        items.iter().copied().collect()
    }
    fn sset(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scan_note_top1_is_the_benign_deep_reach() {
        // The scan-time note delegates to best_finds(...,1); confirm the wiring surfaces the same
        // benign-deep reach the shared heuristic ranks first (the scan output byte-shape depends on it).
        let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut inferred: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
        let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut loc: HashMap<String, String> = HashMap::new();

        direct.insert("net_layer::do_send".into(), set(&["Net"]));
        inferred.insert("net_layer::do_send".into(), set(&["Net"]));
        loc.insert("net_layer::do_send".into(), "src/net.rs:9".into());

        inferred.insert("core::sync_state".into(), set(&["Net"]));
        calls.insert("core::sync_state".into(), sset(&["net_layer::do_send"]));
        inferred.insert("core::refresh".into(), set(&["Net"]));
        calls.insert("core::refresh".into(), sset(&["core::sync_state"]));
        inferred.insert("settings::Settings::load".into(), set(&["Net"]));
        calls.insert("settings::Settings::load".into(), sset(&["core::refresh"]));

        // effecty candidate excluded.
        inferred.insert("api::fetch".into(), set(&["Net"]));
        calls.insert("api::fetch".into(), sset(&["net_layer::do_send"]));

        let got = best_finds(&inferred, &direct, &calls, &loc, 1);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].func, "settings::Settings::load");
        assert_eq!(got[0].source_loc, "src/net.rs:9");
    }
}
