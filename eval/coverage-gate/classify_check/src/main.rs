//! The classify()-consulting half of the coverage-gate generator. `generate.py` does the syntactic
//! extraction (parse each vendored crate's real source, self-scan it with `candor-scan`) and hands this
//! binary the result as JSON; this binary is the ONLY place that calls the real `candor_classify::classify`
//! (rather than re-deriving its rules in Python, which would be a second, driftable copy of the truth).
//!
//! For every candidate whose self-scan `inferred` set contains a CORE effect (Fs/Net/Db/Exec — see
//! `generate.py`'s header for why Log/Env/Clock/Rand/Ipc/Unknown/bare-`invisible` are excluded), this
//! tries every guessed consumer-facing path and asks `classify(crate, guess)`:
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

// The four "dangerous" effects the ten real incidents were all shaped like. Deliberately excludes
// Log/Env/Clock/Rand/Ipc (self-scan picks these up from ubiquitous, low-stakes instrumentation — nearly
// every function in a real crate transitively logs or reads a clock) and excludes a bare non-empty
// `invisible` with empty `inferred` (near-universal in real-world code — almost everything transitively
// touches an uncalibrated dependency, so it carries no discriminating signal). `Unknown` alone is ALSO
// excluded even though it is a much rarer/stronger signal than `invisible`: it would have caught more of
// last night's ten (diesel's `establish`, `tokio_postgres::connect_raw`, `sea_orm::connect_proxy` all
// cross into a DIFFERENT uncalibrated crate and self-scan can only say `Unknown`, not `Db`/`Net`) but it
// also roughly triples the open-list size (measured: 987 -> 2423 candidates, 260 -> 1323 uncovered) —
// see REPORT for the full measurement. Precision over recall was the deliberate call.
const CORE: [&str; 4] = ["Fs", "Net", "Db", "Exec"];

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

    let mut covered: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut open: HashMap<(String, String), (Vec<String>, String)> = HashMap::new();
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
            core_total += 1;
            let matched = e.guesses.iter().find(|g| candor_classify::classify(krate, g).is_some());
            if let Some(g) = matched {
                covered.insert((krate.clone(), g.clone()), inf.clone());
            } else {
                // Report the SHORTEST guess (usually the crate-root re-export alias, i.e. the shape a
                // real consumer would actually spell — `ignore::Walk::new`, not `ignore::walk::Walk::new`).
                let mut gs = e.guesses.clone();
                gs.sort_by_key(|g| g.len());
                let chosen = gs.first().cloned().unwrap_or_default();
                open.insert((krate.clone(), chosen), (inf.clone(), e.file.clone()));
            }
        }
    }

    let mut cov: Vec<_> = covered.into_iter().collect();
    cov.sort();
    let mut cov_out = String::from(
        "# crate\tconsumer_path\teffects\n\
         # GENERATED by eval/coverage-gate/generate.py — see that file's header for the method.\n\
         # HARD list: crates/candor-classify/tests/coverage_gate.rs asserts classify() still returns\n\
         # Some(effect) for every row. Regenerate via `python3 eval/coverage-gate/generate.py`.\n",
    );
    for ((k, g), eff) in &cov {
        cov_out.push_str(&format!("{k}\t{g}\t{}\n", eff.join(",")));
    }
    std::fs::write(covered_path, cov_out).unwrap_or_else(|e| panic!("writing {covered_path}: {e}"));

    let mut op: Vec<_> = open.into_iter().collect();
    op.sort();
    let mut open_out = String::from(
        "# crate\tconsumer_path\teffects\tsource_file\n\
         # GENERATED by eval/coverage-gate/generate.py — the RATCHET list (may shrink, must never grow\n\
         # without review — see .github/workflows/coverage-gate-refresh.yml). Each row is a public entry\n\
         # point self-scan found reaching a real Fs/Net/Db/Exec effect that classify() does not (yet)\n\
         # recognize under any guessed consumer-facing spelling, OR REVIEWED_PURE_ENTRIES does not list.\n\
         # NOT individually hand-verified — this is a generated worklist, not a set of confirmed defects;\n\
         # triage before treating a row as an action item (see REPORT for a worked false-positive example).\n",
    );
    for ((k, g), (eff, f)) in &op {
        open_out.push_str(&format!("{k}\t{g}\t{}\t{f}\n", eff.join(",")));
    }
    std::fs::write(open_path, open_out).unwrap_or_else(|e| panic!("writing {open_path}: {e}"));

    let uncovered_pkgs: HashSet<&str> = op.iter().map(|((k, _), _)| k.as_str()).collect();
    println!(
        "core-effect candidates: {core_total} | covered: {} | open: {} across {} crates",
        cov.len(),
        op.len(),
        uncovered_pkgs.len()
    );
}
