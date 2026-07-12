//! candor-query — the read-only report queries (show / where / callers / map / diff) that used to be
//! inline Python heredocs in `cargo-candor`. One typed binary over the shared `candor-report` types,
//! so the JSON shape is defined once (in the lint and here) instead of re-parsed ad hoc in every
//! script. The CLI keeps the *exact* argv convention and output of the Python it replaces, so the
//! bash wrapper only swaps `python3 - … <<'PY'` for `candor-query …`; everything downstream (the
//! integration tests, the MCP server, the agent) sees identical bytes.
//!
//! Usage — the canonical query grammar (candor-spec §3.3.1, spec 0.10):
//!   candor-query <verb> <verb-args…> [--report <locator>] [--policy <file>] [--json] [--strict] [--include-unknown]
//!   candor-query show    <fn>          [--report <locator>] [--json]
//!   candor-query where   <Effect>      [--report <locator>] [--json]
//!   candor-query map                   [--report <locator>] [--json]
//!   candor-query diff    <current> <baseline> [--json]      (two locators; does NOT discover)
//! The report is DISCOVERED (walk up for `.candor/`; `CANDOR_REPORT` overrides) when `--report` is absent.
//! The OLD positional forms (a leading report, a trailing `0|1` JSON sentinel, a positional policy) stay
//! accepted as DEPRECATED aliases with a stderr note (removed no earlier than the next breaking bump).

pub(crate) use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use candor_report::{report_entries_counted, report_files, ReportEntry, EFFECTS};
pub(crate) use regex::Regex;
pub(crate) use serde::Serialize;

mod grammar;
mod load;
mod matching;
mod audit;
mod show;
mod callers;
mod policy;
mod diff;
mod containment;
mod state;
mod fix;
mod unverified;
mod tour;

// One flat crate namespace, like the candor-scan split: modules re-export
// crate-wide so no call site changed (byte-identical outputs gated the move).
pub(crate) use grammar::*;
pub(crate) use load::*;
pub(crate) use matching::*;
pub(crate) use audit::*;
pub(crate) use show::*;
pub(crate) use callers::*;
pub(crate) use policy::*;
pub(crate) use diff::*;
pub(crate) use containment::*;
pub(crate) use state::*;
pub(crate) use fix::*;
pub(crate) use unverified::*;
pub(crate) use tour::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    // Top-level --version / --help, handled BEFORE the subcommand match so they don't fall through to
    // the unknown-command error (exit 2). Aligned with the candor-scan binary's format. Fully offline.
    if cmd == "-V" || cmd == "--version" {
        print_version();
        std::process::exit(0);
    }
    if cmd == "-h" || cmd == "--help" {
        print_help();
        std::process::exit(0);
    }
    let rest = &args[args.len().min(1)..];
    let code = match cmd {
        "audit" => cmd_audit(rest),
        "show" => cmd_show(rest),
        "where" => cmd_where(rest),
        "callers" => cmd_callers(rest),
        "map" => cmd_map(rest),
        "diff" => cmd_diff(rest),
        "containment" => cmd_containment(rest),
        "reachable" => cmd_reachable(rest),
        "path" => cmd_path(rest),
        "impact" => cmd_impact(rest),
        "blindspots" => cmd_blindspots(rest),
        "tour" => cmd_tour(rest),
        "whatif" => cmd_whatif(rest),
        "fix" => cmd_fix(rest),
        "fix-gate" => cmd_fix_gate(rest),
        "unverified" => cmd_unverified(rest),
        "rewire" => cmd_rewire(rest),
        "receipt" => cmd_receipt(rest),
        "gains" => cmd_gains(rest),
        "state" => cmd_state(rest),
        "reports" => cmd_reports(rest),
        "locate" => cmd_locate(rest),
        "gate-verdict" => cmd_gate_verdict(rest),
        "engine-version" => cmd_engine_version(rest),
        "merge-hook" => cmd_merge_hook(rest),
        "parsepolicy" => cmd_parsepolicy(rest),
        // The agent contract for THE INSTALLED VERSION, embedded at build time — doc and binary
        // cannot drift (the §2.1 version-trust rule applied to documentation).
        "--agents" | "agents" => {
            println!("<!-- candor-query {} · the agent contract for this installed version -->", env!("CARGO_PKG_VERSION"));
            print!("{}", include_str!("../AGENTS.md"));
            0
        }
        other => {
            eprintln!(
                "candor-query: unknown command '{other}' \
                 (audit|show|where|callers|map|diff|containment|reachable|path|impact|blindspots|tour|whatif|fix|fix-gate|unverified|rewire|parsepolicy|receipt|gains|state|reports|locate|gate-verdict|engine-version|merge-hook|--agents)"
            );
            2
        }
    };
    std::process::exit(code);
}

/// `--version` / `-V` — the installed build + the spec contract it speaks, then the upgrade line.
/// Mirrors the candor-scan binary EXACTLY (build from `CARGO_PKG_VERSION`, spec from the shared
/// `candor_report::SPEC_VERSION` that also stamps the report envelope, so the two can never drift).
/// Fully offline.
fn print_version() {
    println!("candor-query {} (spec {})", env!("CARGO_PKG_VERSION"), candor_report::SPEC_VERSION);
    println!("upgrade: cargo install candor-query --force");
}

/// `--help` / `-h` — banner + USAGE + the dispatched subcommands (each with a one-line description
/// derived from its `usage:` string / doc) + the github footer. The list MUST match the real set the
/// `match cmd` dispatches (and the unknown-command list); keep all three in lockstep.
fn print_help() {
    println!(
        "candor-query {} — read-only effect-report queries over candor's reports (candor-spec {})",
        env!("CARGO_PKG_VERSION"),
        candor_report::SPEC_VERSION
    );
    println!();
    println!("USAGE:  candor-query <command> [args…]");
    println!("        candor-query --version | --help");
    println!();
    println!("COMMANDS:");
    // The query verbs take the canonical §3.3.1 grammar: the report is DISCOVERED (or via `--report`),
    // `--json` selects JSON, `--policy` supplies a policy. The old positional forms (leading report, `0|1`
    // sentinel, positional policy) still work as deprecated aliases. Descriptions in dispatch order.
    let rows: &[(&str, &str)] = &[
        ("audit    <prefix> <engine_ver> <suspect_file> [--coverage]", "at-a-glance effect profile across the crates"),
        ("show     <fn> [--report <loc>] [--json]", "the effects a function performs (direct vs via a callee)"),
        ("where    <Effect> [--report <loc>] [--json]", "which functions perform an effect (directly / inherited)"),
        ("callers  <fn> [--report <loc>] [--json] [--include-unknown]", "who reaches a function — its blast radius"),
        ("map      [--report <loc>] [--json]", "per-module effect map (effects + function counts)"),
        ("diff     <current> <baseline> [--json] [<bver> <ever>]", "per-function effect changes vs a baseline"),
        ("containment [<baseline>] [--report <loc>] [--json]", "effects at the entry points, optionally vs a baseline"),
        ("reachable [--report <loc>] [--json]", "the effects the program actually performs at runtime"),
        ("path     <fn-substring> <Effect> [--report <loc>] [--json]", "the call chain by which a fn comes to perform an effect"),
        ("impact   <fn-substring> [--report <loc>] [--json]", "the blast radius of a fn: effectful callers + entry points"),
        ("blindspots [--report <loc>] [--json]", "the Unknown sources, ranked by their Unknown blast radius"),
        ("tour     [<N>] [--report <loc>] [--json]", "the N most surprising transitive reaches (default 10)"),
        ("whatif   <fn> <Effect> [--report <loc>] [--policy <f>] [--json]", "pre-edit verdict: blast radius + policy violations"),
        ("fix      <fn> <Effect> [--report <loc>] [--policy <f>] [--json]", "the boundary fix: where the effect belongs + the hoist refactor"),
        ("fix-gate [--report <loc>] [--policy <f>] [--json]", "a fix for EVERY boundary crossing — the loop's block-message remedy"),
        ("unverified [--report <loc>] [--policy <f>] [--strict]", "pure/deny layers that PASS but are Unknown (not PROVABLY clean)"),
        ("rewire   <cur_prefix> <base_prefix> [0|1]", "call edges a function DROPPED vs a baseline (de-wiring)"),
        ("parsepolicy <policy-file>", "dump a parsed CANDOR_POLICY as canonical JSON (conformance)"),
        ("receipt  <prefix>", "the Claude Code receipt's report-derived fields (key<TAB>value)"),
        ("gains    <current> <baseline> [--json]", "every effect a fn gained since the baseline (self-review)"),
        ("state    [<root>]", "a stable content hash of the .rs tree (source-freshness key)"),
        ("reports  <prefix> [--exists|--backend|--clear-other <scan|lint>]", "list/probe/clean the report artifacts for a prefix"),
        ("locate   <lib|scan> <dir>…", "print the newest matching report file under the dirs"),
        ("gate-verdict <parts-file> <out|->", "assemble the §3.3 gate verdict from NDJSON violation records"),
        ("engine-version <lib-path>", "print the candor-build-version tag embedded in a binary"),
        ("merge-hook <settings.json> <hook-command>", "idempotently merge candor's Stop hook into a settings file"),
        ("--agents", "print the agent contract embedded in this installed build"),
    ];
    let w = rows.iter().map(|(u, _)| u.len()).max().unwrap_or(0);
    for (usage, desc) in rows {
        println!("  {usage:<w$}  {desc}");
    }
    println!();
    println!("  -V, --version    print the installed build + spec contract (offline) and the upgrade line");
    println!("  -h, --help       print this help");
    println!();
    println!("See https://github.com/tombaldwin/candor");
}

#[cfg(test)]
mod tests;
