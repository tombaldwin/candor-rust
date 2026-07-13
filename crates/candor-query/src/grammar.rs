//! The canonical query CLI grammar (SPEC §3.3.1 ⟨0.10⟩) shared by every query verb.
//!
//!   candor-query <verb> <verb-args…> [--report <locator>] [--policy <file>] [--json] [--strict] [--include-unknown]
//!
//! The report is DISCOVERED by default (walk UP from CWD for a `.candor/` dir → `<that>/.candor/report`;
//! a `CANDOR_REPORT` env var overrides discovery). `--report <locator>` overrides, resolving by ONE rule:
//! a directory → `<dir>/.candor/report`; a path ending `.json` → the full report path; else a prefix.
//! `--json` selects JSON; `--policy <file>` supplies a policy; `--strict`/`--include-unknown` keep their
//! §3.1 meaning. Verb args are positional and never include the report.
//!
//! BACK-COMPAT (deprecated aliases, SPEC §3.3.1 — removed no earlier than the next breaking bump): the
//! OLD grammar drove the report as a LEADING positional, JSON as a TRAILING `0|1` sentinel, and policy as
//! a positional. These are still accepted; using one prints a ONE-LINE deprecation note to STDERR (never
//! stdout — stdout stays pure JSON). The conformance suite still drives the old grammar, so it must stay
//! green.

use crate::*;
use std::cell::Cell;

thread_local! {
    /// The deprecation note is printed at most once per process, no matter how many old-form tokens a
    /// single invocation trips (a leading positional AND a trailing sentinel is still one note).
    static WARNED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn deprecation_note(what: &str) {
    WARNED.with(|w| {
        if !w.get() {
            eprintln!(
                "candor-query: {what} is a DEPRECATED grammar (candor-spec §3.3.1) — use \
                 `--report <locator>`, `--json`, `--policy <file>`; the old positional forms are removed \
                 at the next major."
            );
            w.set(true);
        }
    });
}

/// Resolve a `--report`/`CANDOR_REPORT` `<locator>` to a report reference per the §3.3.1 ONE rule
/// (resolved IDENTICALLY for both sources):
///   - a directory → `<dir>/.candor/report` (a prefix);
///   - a path ending `.json` → that SINGLE report file, loaded directly (returned verbatim; ANY `.json`
///     filename, whatever its internal dot-segments — `report_files` recognises an existing `.json` file
///     as a direct single-report reference, not a prefix);
///   - otherwise → a prefix, used verbatim.
pub(crate) fn resolve_locator(loc: &str) -> String {
    let p = Path::new(loc);
    if p.is_dir() {
        return p.join(".candor").join("report").to_string_lossy().into_owned();
    }
    // A `.json` locator names a single report file directly — pass it through unchanged so the loaders
    // (via `report_files`) load THAT file, regardless of whether its name matches the canonical
    // `<base>.<crate>.<backend>.json` shape.
    loc.to_string()
}

/// Discover the report prefix with no `--report`: `CANDOR_REPORT` if set, else walk UP from CWD for a
/// `.candor/` directory and use `<that>/.candor/report`. `None` if neither is found (the caller then
/// fails loud via the usual no-files path).
pub(crate) fn discover_report_prefix() -> Option<String> {
    if let Ok(env) = std::env::var("CANDOR_REPORT")
        && !env.is_empty()
    {
        // `CANDOR_REPORT` resolves by the SAME §3.3.1 locator rule as `--report`: a directory →
        // `<dir>/.candor/report`; a `.json` path → that single report file; else a prefix. Passing the
        // raw value through as a prefix made `CANDOR_REPORT=<dir>` glob `<dir>.<crate>.<backend>.json`
        // and fail "no report files" instead of finding `<dir>/.candor/report.*`.
        return Some(resolve_locator(&env));
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(".candor").is_dir() {
            return Some(dir.join(".candor").join("report").to_string_lossy().into_owned());
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// The parsed canonical grammar for a single-report verb: the report prefix (discovered or from
/// `--report`), the verb's own POSITIONAL args (report stripped), and the flags.
pub(crate) struct Query {
    pub(crate) report: Option<String>,
    pub(crate) positional: Vec<String>,
    pub(crate) want_json: bool,
    pub(crate) policy: Option<String>,
    pub(crate) strict: bool,
    pub(crate) include_unknown: bool,
}

/// A verb's positional shape, used to tell the canonical grammar from the deprecated old forms.
///
/// The NEW grammar gives a verb exactly `verb_args` positionals (the report is a flag): `where`=1
/// (Effect), `show`/`callers`/`impact`=1 (fn), `path`/`whatif`/`fix`=2 (fn+Effect),
/// `map`/`reachable`/`blindspots`/`fix-gate`/`unverified`=0. The OLD grammar prefixed a leading report
/// positional, and (some verbs) appended a `0|1` JSON sentinel and/or a positional policy. Detection is by
/// ARITY: any positional beyond `verb_args` after the tail (`0|1`, policy) is peeled off is the OLD report.
pub(crate) struct Shape {
    /// The verb's own positional argument count in the CANONICAL grammar (report excluded).
    pub(crate) verb_args: usize,
    /// Whether the OLD grammar appended a `0|1` JSON sentinel to this verb (all report queries did).
    pub(crate) sentinel: bool,
    /// Whether this verb ever accepted a positional policy (whatif/fix/fix-gate/unverified).
    pub(crate) has_policy: bool,
}

/// Parse the canonical grammar, accepting the deprecated old forms with a stderr note.
pub(crate) fn parse(args: &[String], shape: Shape) -> Query {
    let mut report: Option<String> = None;
    let mut policy: Option<String> = None;
    let mut want_json = false;
    let mut strict = false;
    let mut include_unknown = false;
    let mut positional: Vec<String> = Vec::new();

    // Pass 1: pull the named flags out; keep the rest positional (order preserved).
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--json" => want_json = true,
            "--strict" => strict = true,
            "--include-unknown" => include_unknown = true,
            "--report" => {
                if let Some(v) = args.get(i + 1) {
                    report = Some(resolve_locator(v));
                    i += 1;
                } else {
                    // A `--report` with no value is a LOUD failure (SPEC §3.3.1), never a silent
                    // fall-through to discovery against a DIFFERENT report — that would answer a query
                    // the user did not ask (the §4 cardinal-sin posture on the wrong report).
                    eprintln!("candor-query: --report requires a <locator> argument (a directory, a `.json` report file, or a prefix)");
                    std::process::exit(2);
                }
            }
            "--policy" => {
                if let Some(v) = args.get(i + 1) {
                    policy = Some(v.clone());
                    i += 1;
                } else {
                    eprintln!("candor-query: --policy requires a <file> argument");
                }
            }
            _ => positional.push(args[i].clone()),
        }
        i += 1;
    }

    // Pass 2: peel the DEPRECATED old-form positionals off the TAIL, then decide the leading report by
    // arity. Old form: `<report> <verb-args…> [policy] [0|1]`. We strip from the end inward so the
    // `verb_args` in the middle are never disturbed.

    // (a) trailing `0|1` JSON sentinel (all report queries appended it). Only when there's an extra
    //     positional beyond the verb's own args — never strip a lone verb argument that happens to be "0".
    if shape.sentinel
        && positional.len() > shape.verb_args
        && matches!(positional.last().map(String::as_str), Some("0") | Some("1"))
    {
        deprecation_note("a trailing `0|1` JSON sentinel");
        if positional.last().map(String::as_str) == Some("1") {
            want_json = true;
        }
        positional.pop();
    }

    // (b) positional policy (whatif/fix/fix-gate/unverified) — the OLD grammar put it AFTER the report +
    //     verb args. After the sentinel is gone, if we still have MORE than `<report?> + verb_args`, the
    //     surplus tail token is the policy. We recognise it only when `--policy` wasn't given.
    //     The report may or may not be a positional yet (step c decides), so "expected with report" =
    //     verb_args + 1. A surplus beyond that is the policy.
    if shape.has_policy && policy.is_none() && positional.len() > shape.verb_args + 1 {
        deprecation_note("a positional policy file");
        policy = Some(positional.pop().unwrap());
    }

    // (c) leading-positional report: after the tail is peeled, any positional beyond `verb_args` is the
    //     OLD leading report (matched by ARITY — the old grammar ALWAYS led with the report, even a typo'd
    //     one that resolves to no files, which must still fail loud rather than silently discover). Only
    //     when `--report` didn't already supply it.
    if report.is_none() && positional.len() > shape.verb_args {
        deprecation_note("a leading-positional report");
        report = Some(resolve_locator(&positional.remove(0)));
    }

    Query { report, positional, want_json, policy, strict, include_unknown }
}

/// Resolve the report prefix for a verb: the parsed `--report`/old-positional if present, else discovery.
/// `None` (no `--report`, nothing discovered) is returned so the caller can fail loud on no-files exactly
/// as before.
pub(crate) fn report_or_discover(q: &Query) -> Option<String> {
    q.report.clone().or_else(discover_report_prefix)
}
