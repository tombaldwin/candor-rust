//! The classify()-consulting half of the coverage-gate generator. `generate.py` does the syntactic
//! extraction (parse each vendored crate's real source, self-scan it with `candor-scan`) and hands this
//! binary the result as JSON; this binary is the ONLY place that calls the real `candor_classify::classify`
//! (rather than re-deriving its rules in Python, which would be a second, driftable copy of the truth).
//!
//! For every candidate whose self-scan `inferred` set contains a CORE effect — originally just
//! Fs/Net/Db/Exec, widened 2026-08-28 to the FULL effect vocabulary (+Clipboard/Ipc/Env/Clock/Rand/Log;
//! see `generate.py`'s header for why Unknown/bare-`invisible` alone still don't qualify) after an
//! audit found `arboard::{Get,Set}::file_list` invisible to this gate for exactly the reason a narrow
//! trigger set predicts: Clipboard was outside it, so a missing Clipboard verb was structurally
//! invisible regardless of phrasing — this tries every guessed consumer-facing path and asks
//! `classify(crate, guess)`:
//!   - any guess resolves        -> COVERED (written to covered.tsv, the HARD/regression-proof list)
//!   - no guess resolves         -> OPEN (written to open.tsv, the ratchet list)
//!
//! Usage: classify_check <entries.json> <out_covered.tsv> <out_open.tsv>

use std::collections::{HashMap, HashSet};
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct Entry {
    #[serde(rename = "crate")]
    #[allow(dead_code)]
    krate: String,
    #[allow(dead_code)]
    module: Option<String>,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    ty: Option<String>,
    #[serde(rename = "fn")]
    #[allow(dead_code)]
    fnname: String,
    #[allow(dead_code)]
    file: String,
    guesses: Vec<String>,
    #[allow(dead_code)]
    self_scan_key: String,
    #[allow(dead_code)]
    self_scan_found: bool,
    inferred: Option<Vec<String>>,
    #[allow(dead_code)]
    invisible: Option<Vec<String>>,
}

// The full concrete effect vocabulary classify() can return (see crates/candor-classify/src/lib.rs).
// Originally just the four "dangerous" effects the ten real incidents were shaped like (Fs/Net/Db/Exec);
// WIDENED 2026-08-28 (see REPORT for the commit) after `arboard::{Get,Set}::file_list` shipped a real,
// silent Clipboard gap that this generator was structurally incapable of ever flagging — Clipboard
// wasn't in the trigger set, so a missing Clipboard verb never became a candidate at all, regardless of
// how the crate's rule was phrased. The narrowing to just Fs/Net/Db/Exec was a real, deliberate,
// MEASURED call (see the retained comment below) — but it traded away recall on every OTHER concrete
// effect, not just the four it was tuned against, and the arboard incident is proof that trade landed on
// a real crate, not just a hypothetical one. `Llm` is deliberately NOT listed here even though it is a
// real classify() return: it is a REFINEMENT that always co-occurs with `Net` on the same call (see
// lib.rs:2611's doc comment — a statically-known model-provider host adds Llm IN ADDITION to Net, never
// instead of it), so any candidate that could trigger on Llm alone already triggers on Net; listing it
// separately would be redundant, not additive. `Unknown` and a bare non-empty `invisible` with empty
// `inferred` remain excluded for the reason below.
//
// Retained from the original, narrower cut — the same reasoning now applies to EACH of the newly-added
// effects, not just Log/Env/Clock/Rand/Ipc, so re-verify it didn't just relocate the flood: self-scan
// picks up low-stakes, ubiquitous instrumentation (nearly every function transitively logs or reads a
// clock), and a bare non-empty `invisible` with empty `inferred` carries no discriminating signal
// (near-universal — almost everything transitively touches an uncalibrated dependency). `Unknown` alone
// is ALSO excluded even though it is a much rarer/stronger signal than `invisible`: it would have caught
// more of the ten incidents (diesel's `establish`, `tokio_postgres::connect_raw`, `sea_orm::
// connect_proxy` all cross into a DIFFERENT uncalibrated crate and self-scan can only say `Unknown`, not
// `Db`/`Net`) but it also roughly triples the open-list size (measured: 987 -> 2423 candidates, 260 ->
// 1323 uncovered) — see REPORT for the full measurement. Precision over recall was the deliberate call.
const CORE: [&str; 10] = [
    "Fs", "Net", "Db", "Exec", "Clipboard", "Ipc", "Env", "Clock", "Rand", "Log",
];

// KNOWN SELF-SCAN FALSE POSITIVES — traced, not guessed. `async_process::Command::{arg,arg0,args,
// current_dir,env,env_clear,env_remove,envs,new}` are documented pure builder setters, and
// classify.rs's OWN `async_process`/`portable_pty` rule (lib.rs:1305) already excludes exactly this set
// by name. Self-scanning `async_process`'s own source still reports them `Exec` because each one's body
// delegates to `self.inner.<verb>(..)`, where `self.inner: std::process::Command` — and `std::process::
// Command`'s rule (lib.rs:1276) is coarser than async_process's hand-tuned one: it excludes only the
// `get_*` READ-BACK getters, not the pure setters, so `classify("std", "std::process::Command::args")`
// legitimately returns `Some("Exec")` (see the `classify("std","std::process::Command::new")` test at
// lib.rs:2755). Self-scan resolved the REAL receiver type and reported a REAL rule match — just the
// wrong rule for what a consumer sees, since a consumer spells this call `async_process::Command::args`,
// which async_process's OWN rule correctly excludes. Not a coverage gap; a std::process::Command
// over-charge question (the SAFE direction) for a separate pass, so left out of both lists rather than
// silently miscounted as either "covered" or "needs a rule".
const KNOWN_FALSE_POSITIVES: &[(&str, &str)] = &[
    ("async_process", "Command::arg"),
    ("async_process", "Command::arg0"),
    ("async_process", "Command::args"),
    ("async_process", "Command::current_dir"),
    ("async_process", "Command::env"),
    ("async_process", "Command::env_clear"),
    ("async_process", "Command::env_remove"),
    ("async_process", "Command::envs"),
    ("async_process", "Command::new"),
];

fn is_known_false_positive(krate: &str, guesses: &[String]) -> bool {
    KNOWN_FALSE_POSITIVES.iter().any(|(k, suffix)| {
        *k == krate && guesses.iter().any(|g| g.ends_with(suffix))
    })
}

// The other half of `KNOWN_FALSE_POSITIVES`'s job, but for entries a human read and found genuinely
// pure rather than mis-resolved by self-scan: `candor_classify::REVIEWED_PURE_ENTRIES` (the coverage
// gate's own escape hatch, see its doc comment in crates/candor-classify/src/lib.rs). Without this check
// a `REVIEWED_PURE_ENTRIES` entry would keep reappearing in every regenerated `open.tsv` forever — the
// gate has no OTHER way to learn that a human already closed the question, since `classify()` correctly
// still returns `None` for something that performs no effect.
fn is_reviewed_pure(krate: &str, guesses: &[String]) -> bool {
    candor_classify::REVIEWED_PURE_ENTRIES
        .iter()
        .any(|(k, p)| *k == krate && guesses.iter().any(|g| g == p))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: classify_check <entries.json> <out_covered.tsv> <out_open.tsv>");
        std::process::exit(2);
    }
    let (entries_path, covered_path, open_path) = (&args[1], &args[2], &args[3]);

    let data = std::fs::read_to_string(entries_path)
        .unwrap_or_else(|e| panic!("reading {entries_path}: {e}"));
    let map: HashMap<String, Vec<Entry>> = serde_json::from_str(&data)
        .unwrap_or_else(|e| panic!("parsing {entries_path}: {e}"));

    // Keyed by (crate, self_scan_key) — NOT by (crate, guess) — because a guess string is not unique:
    // sibling types/fns in different modules (async/blocking/wasm variants of the same name is the
    // dominant real shape — see generate.py's `root_reexport_prefixes` docstring, 610 such guess-strings
    // measured shared across >1 distinct self-scan key in the 74 calibrated crates) can legitimately
    // guess the IDENTICAL crate-root alias. Keying insertion on the guess string let a second entry
    // silently overwrite (and thereby DROP — the exact silent-under-report shape this whole gate exists
    // to prevent, now happening inside the gate's own generator) a first, real, still-open entry the
    // moment both entries' effects happened to intersect CORE at once — invisible under the narrower
    // Fs/Net/Db/Exec-only CORE (the two colliding entries rarely shared a CORE-qualifying effect), and
    // exposed once CORE widened to cover more of the vocabulary. `self_scan_key` (module + type + fn) is
    // the thing that's actually guaranteed unique per candidate; the guess is only ever used for display
    // and for the classify() lookup itself, never as identity.
    let mut covered: HashMap<(String, String), (String, Vec<String>, &'static str)> = HashMap::new();
    let mut open: HashMap<(String, String), (String, Vec<String>, String)> = HashMap::new();
    let mut core_total = 0usize;

    for (krate, entries) in &map {
        for e in entries {
            let Some(inf) = &e.inferred else { continue };
            if !inf.iter().any(|x| CORE.contains(&x.as_str())) {
                continue;
            }
            if is_known_false_positive(krate, &e.guesses) {
                continue;
            }
            if is_reviewed_pure(krate, &e.guesses) {
                continue;
            }
            core_total += 1;
            let dedup_key = (krate.clone(), e.self_scan_key.clone());
            // Capture the ACTUAL classify() return value alongside the matched guess — not just
            // whether one exists. `inf` (self-scan's `inferred` set) is a DIFFERENT oracle's superset
            // of every effect reachable in the crate's real implementation (e.g. async_nats::connect
            // legitimately touches Fs/Log/Net/Rand/Unknown on the way to opening a socket); classify()
            // instead returns ONE label for the call site. The two can disagree by design — Consumer::
            // request_batch's self-scan set is `Log` alone while classify() correctly returns `Net` for
            // it — so `inf` is documentation, never a value classify()'s result can be checked against.
            // Recording what classify() itself said at generation time is the only way a later run can
            // detect a rule NARROWED to a still-non-None but WRONG effect (a `deny Net` policy waving
            // through a `Net` call site relabelled `Log`), which membership-testing `inf` cannot catch:
            // `Log` was already a legitimate member of `connect`'s inferred set before this fix existed.
            let matched = e.guesses.iter().find_map(|g| candor_classify::classify(krate, g).map(|eff| (g, eff)));
            if let Some((g, classified_as)) = matched {
                covered.insert(dedup_key, (g.clone(), inf.clone(), classified_as));
            } else {
                // Report the SHORTEST guess (usually the crate-root re-export alias, i.e. the shape a
                // real consumer would actually spell — `ignore::Walk::new`, not `ignore::walk::Walk::new`).
                let mut gs = e.guesses.clone();
                gs.sort_by_key(|g| g.len());
                let chosen = gs.first().cloned().unwrap_or_default();
                open.insert(dedup_key, (chosen, inf.clone(), e.file.clone()));
            }
        }
    }

    // Two DIFFERENT self_scan_keys can still print the same consumer_path guess (the reqwest
    // async_impl/wasm case is a REAL, correct instance: mutually exclusive cfg targets both legitimately
    // re-export `ClientBuilder` at the crate root). Rendering to text lines and deduping THOSE (rather
    // than deduping on the map key, which is what caused the original bug) collapses only genuine
    // duplicate rows — two entries that also agree on effects — while still keeping two rows visible
    // when the effects actually differ, which is real information a reader should see.
    let mut cov_lines: Vec<String> = covered
        .into_iter()
        .map(|((krate, _sk), (g, eff, classified_as))| {
            format!("{krate}\t{g}\t{}\t{classified_as}", eff.join(","))
        })
        .collect();
    cov_lines.sort();
    cov_lines.dedup();
    let mut cov_out = String::from(
        "# crate\tconsumer_path\tself_scan_effects\tclassified_as\n\
         # GENERATED by eval/coverage-gate/generate.py — see that file's header for the method.\n\
         # self_scan_effects is the self-scan oracle's full inferred set for this entry point (real\n\
         # local call-graph reachability over the crate's OWN source) — documentation, never a value to\n\
         # check classify() against: the two oracles legitimately disagree (self-scan's set for\n\
         # `async_nats::Consumer::request_batch` is `Log` alone; classify() correctly returns `Net`).\n\
         # classified_as is the actual `classify(crate, consumer_path)` return value THIS gate asserts:\n\
         # crates/candor-classify/tests/coverage_gate.rs requires it hold EXACTLY (not just Some(_)) —\n\
         # a rule narrowed to a different-but-still-Some effect (Net -> Log) is a regression this equality\n\
         # check catches; `is_some()` alone would not, since the narrowed value can already be a member\n\
         # of self_scan_effects. Regenerate via `python3 eval/coverage-gate/generate.py`.\n",
    );
    for line in &cov_lines {
        cov_out.push_str(line);
        cov_out.push('\n');
    }
    std::fs::write(covered_path, cov_out).unwrap_or_else(|e| panic!("writing {covered_path}: {e}"));

    let mut uncovered_pkgs: HashSet<String> = HashSet::new();
    let mut open_lines: Vec<String> = open
        .into_iter()
        .map(|((krate, _sk), (g, eff, f))| {
            uncovered_pkgs.insert(krate.clone());
            format!("{krate}\t{g}\t{}\t{f}", eff.join(","))
        })
        .collect();
    open_lines.sort();
    open_lines.dedup();
    let mut open_out = String::from(
        "# crate\tconsumer_path\teffects\tsource_file\n\
         # GENERATED by eval/coverage-gate/generate.py — the RATCHET list (may shrink, must never grow\n\
         # without review — see .github/workflows/coverage-gate-refresh.yml). Each row is a public entry\n\
         # point self-scan found reaching a real effect (see generate.py / this file's CORE constant for\n\
         # the current trigger vocabulary) that classify() does not (yet) recognize under any guessed\n\
         # consumer-facing spelling, OR REVIEWED_PURE_ENTRIES does not list.\n\
         # NOT individually hand-verified — this is a generated worklist, not a set of confirmed defects;\n\
         # triage before treating a row as an action item (see REPORT for a worked false-positive example).\n",
    );
    for line in &open_lines {
        open_out.push_str(line);
        open_out.push('\n');
    }
    std::fs::write(open_path, open_out).unwrap_or_else(|e| panic!("writing {open_path}: {e}"));

    println!(
        "core-effect candidates: {core_total} | covered: {} | open: {} across {} crates",
        cov_lines.len(),
        open_lines.len(),
        uncovered_pkgs.len()
    );
}
