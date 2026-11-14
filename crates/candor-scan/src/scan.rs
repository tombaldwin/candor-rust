//! The scan driver: argument walk (`scan_main`), per-crate scan + report emission
//! (`scan_one`), workspace fan-out (`scan_target`), and `--deps` chaining (`run_with_deps`).

use crate::*;

/// Total CALLS into κ-uncovered dependencies at or above which the scan is assumed to be MISSING AN
/// INPUT — pointed at the crate's own code with nothing standing in for what it depends on — and so
/// earns the scan-completeness nudge under the κ ledger (see the emission site in `scan_one`).
///
/// VOLUME, not dependency COUNT, and that choice is load-bearing: count is the wrong metric in BOTH
/// directions. candor-java's own `build/classes` makes 519 uncovered calls into just 4 packages — the
/// textbook "you pointed it at the classes, not the deployed artifact" scan — which any count-based
/// threshold misses entirely, while a small crate touching 5 tiny util deps would be nudged for
/// nothing. A scan whose dependencies ARE supplied (`--deps` / CANDOR_DEPS chaining) sits at or near
/// zero. Held at candor-java's 50 so both engines nudge on the same evidence.
///
/// ADVISORY ONLY: one stderr line. It never touches the report, the gate verdict, or the exit code.
const UNCOVERED_CALLS_NUDGE_MIN: usize = 50;

/// SPEC §3.3.1 ⟨0.27⟩/⟨0.28⟩ — everything the arming and the collision guards need from argv, learned
/// in ONE side-effect-free walk (`prescan_argv`) early enough to precede the first write. It is not the
/// validator — the parse loop below still owns every diagnostic.
struct PreScan {
    /// Every `--gate-json` this argv names, in order and with duplicates kept ⟨0.28⟩.
    /// `distinct_gate_sinks` reduces them; the parse loop honours the LAST. Keeping only the last here
    /// is exactly the behaviour the rung refuses: measured, three engines wrote the verdict to the last
    /// path and left the first holding a previous run's `{"ok": true}` while the gate fired — the stale
    /// green of the ⟨0.27⟩ arming rule, reached by a spelling nobody had considered.
    gate_sinks: Vec<String>,
    /// The `--policy` path the loop will accept — the next token when it is value-shaped; a
    /// flag-shaped token is NOT a value and stays live (see the walk).
    policy: Option<String>,
    /// Every `--out` prefix this argv names, in order and with duplicates kept ⟨0.28⟩. The first
    /// version of the out pre-pass kept only the FIRST occurrence, and candor-swift's arm caught it by
    /// checking its own loop instead of copying: measured on `--out p1 --out p2 --zzz-not-a-flag`,
    /// `p1` was armed and `p2` — the prefix the run would actually have written — stayed STALE. A
    /// pre-pass that disagrees with the loop it runs ahead of arms the wrong thing.
    ///
    /// ⟨0.28⟩ SPEC §3.3.1 has since DECIDED the question an earlier note here filed: **a repeated
    /// `--out` is the same rule as a repeated `--gate-json`** — refused at exit 2, with the
    /// fail-closed report written to EVERY prefix named, because the two statements cannot both be
    /// honoured and last-wins leaves the losing prefix holding a previous run's reports, readable as
    /// current, with nothing saying otherwise. If anything the report sink is the WORSE case: this
    /// engine fans out, so the stale set at the losing prefix is a whole per-crate report set, and a
    /// `gate --report` over it answers from a scan that never ran. Keeping the whole list (rather
    /// than the last) is what lets `scan_main` see the duplication at all.
    outs: Vec<String>,
    /// The scan TARGET. The last positional is kept, which USED to be what the parse loop did on every
    /// bare token; a second positional is now a usage error that exits 2 there. This pass still runs
    /// first, so it keeps the last — with one positional the two agree, and with two the run refuses a
    /// moment later. Taking the FIRST here would make the guard discover a different tree's config than
    /// the run reads, which is how it once checked the wrong pair and destroyed the policy at exit 0.
    target: Option<String>,
}

/// **THE PRE-PASS CONSUMES A VALUE EXACTLY WHERE THE PARSE LOOP DOES — that agreement is the entire
/// contract, and each divergence from it has been a measured defect.** The shared grammar (SPEC §3.2
/// ⟨0.28⟩, the "given no value" ruling): a value-taking flag consumes the next token only when it is
/// VALUE-SHAPED (`-`, or not `-`-prefixed — `--out` refuses even the bare `-`, it is a prefix). A
/// flag-shaped token is NOT a value: the loop refuses the run there at exit 2, and this walk leaves
/// the token LIVE so the flags after the broken one are still parsed — the run has a broken command
/// line, not a redefined one, so a sink named there is still a sink. The previous shape of this
/// agreement had the loop consuming the flag-shaped token as the value (`--policy --gate-json -` read
/// policy = the file named `--gate-json`), which both swallowed the operator's sink AND made the
/// loop's own "given no value" diagnostic unreachable — no argv could produce it. Measured after the
/// pre-pass/loop alignment: exit 2 with NOTHING on the stream where the `--gate-json -` refusal
/// document belongs (conformance §3.1 (b13)).
///
/// The history that produced the alignment stands: the previous pre-passes (three separate walks) had
/// drifted from the loop, so `--policy --out X` armed `X.*.json` while the loop consumed `--out` as
/// the policy path — X's previous reports became permanent placeholders under an `--out` the parse
/// never accepted. ONE walk feeds every pre-pass consumer, and the loop now agrees token-for-token
/// with it: under the ⟨0.28⟩ ruling `--policy --out X` is a usage error at `--policy`, and `--out X`
/// — parsed, not swallowed — arms X fail-closed before the refusal.
/// THE VALUELESS FLAGS, IN ONE PLACE — SOUNDNESS R264.
///
/// `prescan_argv` and the main parse loop each need to know which flags take no operand, and they used
/// to hold two hand-written copies of that fact. The copies differed by exactly one token: the prescan
/// had `--version` and not `-V`. That is §G — two paths computing one fact — and the cost was DATA LOSS,
/// not a wrong report: with `-V` treated as refusable, `stopped` suppressed the target, `pre_target`
/// fell back to `"."`, and `gate_json_input_collision` was then asked "is this sink under `.`?" instead
/// of "is it under the target the operator gave?". A sink outside the CWD subtree is invisible to that
/// question, so arming proceeded and the fail-closed verdict document was written OVER a source file of
/// the scan, at parse time, before the walk. Measured from an unrelated cwd:
///
///     candor-scan --version <dir> --gate-json <dir>/src/lib.rs   exit 2, nothing written
///     candor-scan -V        <dir> --gate-json <dir>/src/lib.rs   exit 0, lib.rs DESTROYED
///
/// Anything added here must be a flag the main loop accepts with no operand. Adding one that DOES take
/// an operand re-opens the target-suppression bug from the other side.
const VALUELESS_FLAGS: &[&str] = &[
    "--json", "--include-tests", "--incremental", "--deps", "--agents",
    "--version", "-V", "--help", "-h",
];

fn prescan_argv(args: &[String]) -> PreScan {
    let mut ps = PreScan { gate_sinks: Vec::new(), policy: None, outs: Vec::new(), target: None };
    let mut it = args.iter().peekable();
    // Set at the first token the main loop would refuse; gates the TARGET only (see the `_` arm).
    let mut stopped = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            // Consumed and recorded only when the loop would accept it (`-` or non-flag); a
            // flag-shaped token stays LIVE — the loop refuses the run at this flag, and whatever the
            // live token names (another sink, say) must still be honoured by that refusal.
            "--gate-json" => {
                if it.peek().is_some_and(|v| v.as_str() == "-" || !v.starts_with('-')) {
                    ps.gate_sinks.push(it.next().expect("peeked").clone());
                }
            }
            // Same shape rule as `--gate-json` (`-` is accepted here and fails loud as an unreadable
            // policy file a moment later — strictly narrower than refusing it in the grammar).
            "--policy" => {
                if it.peek().is_some_and(|v| v.as_str() == "-" || !v.starts_with('-')) {
                    ps.policy = it.next().cloned();
                }
            }
            // A prefix, so even the bare `-` is refused by the loop — recorded only when non-dashed.
            "--out" => {
                if it.peek().is_some_and(|v| !v.starts_with('-')) {
                    ps.outs.push(it.next().expect("peeked").clone());
                }
            }
            // ⟨0.32⟩ THE TARGET THIS WALK RESOLVES MUST BE THE ONE THE MAIN LOOP WOULD ACCEPT, because
            // the refusal marker is written to ITS default prefix. Two rules, both read off the main
            // loop rather than guessed:
            //
            //   1. THE FIRST POSITIONAL WINS, not the last. This arm used to be `ps.target = Some(..)`
            //      on every bare token, with a comment saying it took the last "specifically to mirror
            //      this loop". That mirror went stale: the main loop now REFUSES a second positional
            //      (`unexpected extra argument`) instead of letting the last one win, so the target it
            //      holds when it refuses is the FIRST.
            //
            //   2. STOP AT A TOKEN THE MAIN LOOP WOULD REFUSE. An unrecognised flag exits 2 there, so
            //      nothing after it is ever accepted — and its operand is not a target. Measured:
            //      `candor-scan . --scope src` resolved `src` here (last-wins picked up the rejected
            //      flag's operand), so the refusal marker was written to `src/.candor/`, CREATING that
            //      directory in the operator's tree and overwriting any marker already there. The
            //      target the operator actually gave, `.`, was discarded. candor-ts and candor-swift
            //      both wrote to `.` on this argv; rust was alone.
            //
            // A flag whose arity we know is skipped above; a KNOWN VALUELESS flag is not a stopping
            // point, since a target may legitimately follow it (`candor-scan --include-tests .`).
            _ => {
                if a.starts_with('-') {
                    // STOP THE TARGET, NEVER THE WALK. The first version of this `break` exited the
                    // whole loop, which also stopped collecting `--gate-json` — and that collection is
                    // DELIBERATELY order-independent: ⟨0.27⟩ (1) requires the verdict sink to be armed
                    // with the refusal whatever the argv order, so `--zzz --gate-json G` must still arm
                    // G. Breaking left yesterday's green at G, a STALE GREEN, which is the direction
                    // that matters. Caught by conformance PART 34 (b), not by this crate's own tests.
                    if !VALUELESS_FLAGS.contains(&a.as_str()) {
                        stopped = true;
                    }
                } else if ps.target.is_none() && !stopped {
                    ps.target = Some(a.clone());
                }
            }
        }
    }
    ps
}

/// Two spellings of ONE path are one sink (the §3.3.1 artifact rule); two different artifacts are the
/// ambiguity this refuses. Returns the distinct sinks, first spelling of each kept for the diagnostic.
fn distinct_gate_sinks(all: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in all {
        if !out.iter().any(|k| k == s || (k != "-" && s != "-" && same_artifact(k, s))) {
            out.push(s.clone());
        }
    }
    out
}

/// Refuse a repeated `--gate-json`, writing the refusal to EVERY path named (SPEC §3.3.1 ⟨0.28⟩).
///
/// Writing to each is the load-bearing half. The reader of a losing path has no way to learn that it
/// lost, so leaving it untouched publishes whatever it held before — and the run that produced that
/// state did not fail, which is what makes this worse than the refusal case arming was built for.
fn refuse_repeated_gate_json(sinks: &[String]) -> ! {
    let named = sinks.join(", ");
    eprintln!("candor-scan: --gate-json given more than once ({named}) — refusing (exit 2).");
    eprintln!("        A gate publishes ONE verdict. Naming two sinks says where it goes twice, and the");
    eprintln!("        reader of the path that loses cannot tell it lost. Name one, or run the gate twice.");
    let doc = candor_report::gate_refusal_json(&format!(
        "--gate-json was given more than once ({named}) — a run publishes one verdict to one sink"
    ))
    .unwrap_or_else(|_| "{\"ok\":false,\"refused\":true}".to_string());
    for s in sinks {
        if s == "-" {
            println!("{doc}");
        } else if let Err(e) = std::fs::write(s, format!("{doc}\n")) {
            eprintln!("candor-scan: could not write the refusal to --gate-json {s} ({e})");
        }
    }
    std::process::exit(2)
}

/// `same_artifact` for the ⟨0.28⟩ `--out` armer in `gate.rs` — one resolver, not a second copy.
pub(crate) fn same_artifact_pub(a: &str, b: &str) -> bool {
    same_artifact(a, b)
}

/// SPEC §3.3.1 — is this one artifact under two names?
///
/// SOUNDNESS R150: this used to be a second, independently-maintained copy of the same question that
/// `candor-query/src/gate.rs` also implemented — and the two had drifted (the gate copy was a bare
/// `canonicalize()` missing the device+inode check and the dangling-symlink pre-resolve below). Both
/// crates already depend on `candor-report`, so the one implementation now lives there
/// (`candor_report::same_artifact`) and this is a thin re-export kept so every call site in this crate
/// stays unchanged. See the doc comment on `candor_report::same_artifact` for the full argument.
fn same_artifact(a: &str, b: &str) -> bool {
    candor_report::same_artifact(a, b)
}

/// Every path this run READS, whatever channel it arrived through (SPEC §3.3.1 ⟨0.27⟩).
///
/// THE FIRST VERSION OF THIS GUARD KEYED ON THE FLAG. With the policy declared by `.candor/config` —
/// the checked-in form, i.e. the one a CI job actually has — `--gate-json <that policy>` destroyed it
/// and exited 0 with `"ok": true` in ALL FOUR ENGINES, because the pre-pass only looked at `--policy`
/// and `CANDOR_POLICY`. A policy does not change what it is according to how the operator handed it over.
///
/// The config is read LENIENTLY here — no exit, no diagnostic — because this runs before the real config
/// load and must not pre-empt its refusal. If it cannot be read we learn nothing from it and the load a
/// moment later fails on its own terms. This read decides only whether a path is an INPUT.
fn run_inputs(target: &str, policy_flag: Option<&str>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    // ⟨0.28⟩ THE TARGET ITSELF. §3.3.1 (3) lists "the target's own source tree" beside the policy and
    // the baseline, and this list did not — every channel was registered except the one every run has.
    // Measured: `candor-scan src/lib.rs --gate-json src/lib.rs` replaced the operator's SOURCE FILE with
    // the armed verdict and exited 0. EXACT-ARTIFACT registration, never containment: `same_artifact`
    // stays exact (see its comment — a report written into `.candor/` INSIDE the tree being scanned is
    // ordinary usage and must keep working), so this refuses only the target under its own name.
    out.push((target.to_string(), "the scan target".into()));
    if let Some(p) = policy_flag {
        out.push((p.to_string(), "--policy".into()));
    }
    for (var, label) in [("CANDOR_POLICY", "CANDOR_POLICY"), ("CANDOR_BASELINE", "CANDOR_BASELINE"),
                         ("CANDOR_CONFIG", "CANDOR_CONFIG")] {
        if let Ok(v) = candor_report::env::operator(var) {
            if !v.is_empty() {
                // A BASELINE VALUE IS A NAME FOR A SET OF FILES, and only the raw string was registered.
                // `check_baseline` resolves a non-file value to `<value>.<crate>.scan.json` and reads
                // `<report>.callgraph.json` beside it, and `same_artifact("base", "base.app.scan.json")`
                // is false — so `CANDOR_BASELINE=base --out base --zzz-not-a-flag` exited 2 having
                // replaced the ratchet's baseline, a file this run READS, with the placeholder. The dep-
                // DIRECTORY lesson (below) un-applied to its sibling channel: register what the value
                // RESOLVES TO, not the value's spelling.
                if var == "CANDOR_BASELINE" {
                    for f in baseline_artifact_files(&v) {
                        out.push((f, "a CANDOR_BASELINE report".into()));
                    }
                }
                out.push((v, label.into()));
            }
        }
    }
    if let Ok(d) = candor_report::env::operator("CANDOR_DEPS") {
        // THE SAME SET THE LOADER USES — `crate::deps::DEP_SEPARATORS`. This comment claimed that
        // while spelling a DIFFERENT set: it omitted `\n` and `\r`, so a newline-separated
        // `CANDOR_DEPS` was ONE unresolvable token here and two real paths in the loader. A
        // `--gate-json` naming one of those reports was then unguarded — arming overwrote it and the
        // run exited 0 with `ok: true` written over the operator's own input. Measured live.
        for one in d.split(crate::deps::DEP_SEPARATORS).filter(|x| !x.is_empty()) {
            out.push((one.to_string(), "a CANDOR_DEPS report".into()));
            // A DIRECTORY DEP IS EVERY REPORT INSIDE IT — the loader walks it and reads each `*.json`,
            // so registering only the DIRECTORY left those files unnamed and `--gate-json
            // <depdir>/lib.json` destroyed the operator's report at exit 0. Expanded HERE rather than
            // by making `same_artifact` directory-aware: the scan TARGET is an input too, and a verdict
            // written into the tree being scanned is ordinary usage. Only a dep directory is READ.
            for f in crate::deps::dep_report_files(one) {
                out.push((f.to_string_lossy().into_owned(), "a CANDOR_DEPS report".into()));
            }
        }
    }
    // …AND THE CONFIG'S OWN KEYS, THROUGH THE ENGINE'S OWN LOADER. This used to re-derive the walk and
    // the parse, and a review took it apart on exactly that: the home directory was computed as
    // parent-of-parent unconditionally where the real loader only steps out of a trailing `.candor/`
    // segment, so an out-of-tree `CANDOR_CONFIG` had its relative values anchored one level too high and
    // the guard protected a path the run never reads. A second parser is a second set of holes.
    out.extend(crate::config::config_inputs(target));
    out
}

/// ⟨0.28⟩ The on-disk artifacts a baseline VALUE resolves to, for the sink guards above.
///
/// One rule for both spellings, and it is ARMING'S OWN GLOB — `<stem>.` prefix, `.json` suffix, in the
/// value's parent directory — so the set this registers and the set `arm_out_prefix` would touch line
/// up exactly. The stem strips a trailing `.json` from the value's file name first: a prefix `base`
/// picks up `base.app.scan.json` and its `base.app.scan.callgraph.json` sidecar, and a direct file
/// `base.json` picks up the `base.callgraph.json` that `check_baseline` derives beside it. Only files
/// that EXIST are returned, which is the whole population at risk: a path with nothing at it has
/// nothing for arming to destroy, and a placeholder landing there fails `check_baseline`'s provenance
/// check LOUDLY (exit 2) rather than comparing silently.
pub(crate) fn baseline_artifact_files(value: &str) -> Vec<String> {
    let p = std::path::Path::new(value);
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let stem = name.strip_suffix(".json").unwrap_or(&name);
    if stem.is_empty() {
        return Vec::new();
    }
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let f = e.file_name().to_string_lossy().into_owned();
        if f.starts_with(&format!("{stem}.")) && f.ends_with(".json") {
            out.push(dir.join(&f).to_string_lossy().into_owned());
        }
    }
    out.sort();
    out
}

/// Refuse the sink if it names ANY input of this run, whatever channel that input arrived through.
fn refuse_gate_json_over_any_input(gate: &str, target: &str, policy_flag: Option<&str>) {
    if gate == "-" {
        return;
    }
    for (path, label) in run_inputs(target, policy_flag) {
        refuse_gate_json_over_input(gate, Some(&path), &label);
    }
    if gate_json_is_parsed_source_under_target(gate, target) {
        eprintln!(
            "candor-scan: --gate-json {gate} lies under the scan target {target} and bears an \
             extension this engine parses (.rs) — refusing (exit 2), and nothing was written there."
        );
        eprintln!("        Arming writes at parse time, BEFORE the file walk, so this would overwrite a");
        eprintln!("        source file this run is about to read and then scan the wreckage. A non-source");
        eprintln!("        sink under the target ({target}/.candor/verdict.json, say) is the recommended");
        eprintln!("        layout and is permitted; give the verdict a path that is not source.");
        std::process::exit(2);
    }
    refuse_gate_json_at_config(gate);
}

/// ⟨0.28⟩ SPEC §3.3.1: **a sink that lies UNDER the scan target AND bears an extension this engine
/// parses is refused** — the residual the exact-artifact rule deliberately left. The exact-artifact
/// registration in `run_inputs` catches `--gate-json <target>` itself; it cannot catch
/// `--gate-json src/lib.rs` while scanning `.`, because the file set the run will parse is not known
/// at the moment arming happens (arming precedes the walk, and deferring it would uncover the
/// argv-error exits the arming rule exists for). MEASURED here before the fix:
/// `candor-scan . --policy P --gate-json src/lib.rs` replaced the operator's SOURCE FILE with the
/// armed verdict, then reported the file it had just destroyed as a parse failure (exit 2, the
/// self-describing arm — candor-ts's spelling of the same defect exited 0, reported as SUCCESS).
///
/// NOT containment in general: `<dir>/.candor/report.json` is under the target and is NOT a source
/// file, so the recommended layout stays permitted — that control is what separates this from the
/// containment fix the ruling explicitly rejects ("one engine tried containment and it took 33 tests
/// with it"). An engine knows its own source extensions before it knows its file list; `.rs` is the
/// whole of this engine's parse set (see the walk's extension check in this file).
fn gate_json_is_parsed_source_under_target(gate: &str, target: &str) -> bool {
    if gate == "-" {
        return false;
    }
    let g = std::path::Path::new(gate);
    if g.extension().and_then(|e| e.to_str()) != Some("rs") {
        return false;
    }
    // The target must resolve (a nonexistent target is its own refusal a moment later, having written
    // nothing — this check runs before arming). The sink may not exist yet — resolve its parent and
    // re-append the name, the same shape `same_artifact` uses.
    let Ok(t) = std::path::Path::new(target).canonicalize() else {
        return false;
    };
    let resolved = g.canonicalize().ok().or_else(|| {
        let parent = g.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        Some(parent.canonicalize().ok()?.join(g.file_name()?))
    });
    resolved.is_some_and(|r| r.starts_with(&t))
}

/// Refuse a `--gate-json` sink that names an INPUT of this run, having written nothing (exit 2).
///
/// Arming writes before the run knows its answer, so pointing the sink at the policy destroys the
/// policy: measured, `--policy P --gate-json P` on violating code exited 0 with `ok: true` because the
/// armed JSON replaced P and every line of it parsed as an unknown rule. The gate ran over zero rules —
/// a machine-readable all-clear produced by deleting the question.
/// ⟨0.28⟩ Is this sink an input? Non-exiting, because the duplicate-sink path must be able to ask the
/// question WITHOUT taking the run down: the exemption covers the offending PATH, and every other sink
/// named in the same argv still has a reader waiting for a verdict.
///
/// **ONE SPELLING OF THE RULE — this reads `run_inputs`, the same set the single-sink refusal loops
/// over and the `--out` armer exempts.** The first version re-derived the set by hand and its copy
/// omitted CANDOR_BASELINE, CANDOR_DEPS and the config's own keys, so the DUPLICATE-sink route wrote
/// the repeated-`--gate-json` refusal OVER a chained dep report the single-sink route refused to touch
/// — measured live: `CANDOR_DEPS=R --gate-json R` exited 2 with R intact, `--gate-json R --gate-json V`
/// exited 2 with R replaced by the refusal document. Two spellings of one rule is how the two routes
/// came to disagree; there is now one.
fn gate_json_input_collision(gate: &str, target: &str, policy: Option<&str>) -> bool {
    if gate == "-" {
        return false;
    }
    if is_gate_json_at_config(gate) {
        return true;
    }
    // ⟨0.28⟩ a parsed-source file under the target is an input by the same reading — asked here too,
    // so the DUPLICATE-sink route cannot write its refusal document over source the single-sink route
    // refuses to touch (the two-spellings-of-one-rule drift this predicate exists to prevent).
    if gate_json_is_parsed_source_under_target(gate, target) {
        return true;
    }
    run_inputs(target, policy).iter().any(|(path, _)| same_artifact(gate, path))
}

fn refuse_gate_json_over_input(gate: &str, other: Option<&str>, flag: &str) {
    let Some(other) = other else { return };
    if !same_artifact(gate, other) {
        return;
    }
    eprintln!("candor-scan: --gate-json {gate} names the SAME FILE as {flag} {other} — refusing (exit 2).");
    // "an input of this run", not "your policy": the same sentence now covers the policy, the baseline,
    // a chained dep report, the config's own keys, and the scan target itself.
    eprintln!("        The verdict is armed before this run reads its inputs, so this would overwrite");
    eprintln!("        an input of this run and then gate on the wreckage. Nothing was written; give");
    eprintln!("        the verdict its own path.");
    std::process::exit(2);
}

/// `.candor/config` is never a verdict sink, wherever it is (SPEC §3.3.1 ⟨0.27⟩).
///
/// The per-input checks can only name inputs the run was TOLD about; the config is DISCOVERED by
/// walking up from the target, so by the time its path is known the arming has already destroyed it.
/// A check on the SHAPE needs no discovery, so it can run before the first write and it covers a config
/// found anywhere up the tree. No legitimate run writes a gate verdict to `config` inside `.candor`.
fn is_gate_json_at_config(gate: &str) -> bool {
    if gate == "-" {
        return false;
    }
    let p = std::path::Path::new(gate);
    p.file_name().is_some_and(|n| n == "config")
        && p.parent()
            .and_then(|d| {
                let d = if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d };
                d.canonicalize().ok().or_else(|| Some(d.to_path_buf()))
            })
            .and_then(|d| d.file_name().map(|n| n == ".candor"))
            .unwrap_or(false)
}

fn refuse_gate_json_at_config(gate: &str) {
    if gate == "-" {
        return;
    }
    let p = std::path::Path::new(gate);
    let is_config = p.file_name().is_some_and(|n| n == "config")
        && p.parent()
            .and_then(|d| {
                let d = if d.as_os_str().is_empty() { std::path::Path::new(".") } else { d };
                d.canonicalize().ok().or_else(|| Some(d.to_path_buf()))
            })
            .and_then(|d| d.file_name().map(|n| n == ".candor"))
            .unwrap_or(false);
    if !is_config {
        return;
    }
    eprintln!("candor-scan: --gate-json {gate} is a .candor/config — refusing (exit 2). The verdict is");
    eprintln!("        armed before the config is read, so this would destroy the config that configures");
    eprintln!("        this run. Nothing was written; give the verdict its own path.");
    std::process::exit(2);
}

/// SOUNDNESS R1634 — THE FAULT-INJECTION HOOK IS COMPILED OUT OF THE RELEASED BINARY. `CANDOR_PANIC_ON_FILE=<rel>` made the walk of that file
/// panic so the containment above has a deterministic test; it is a result-moving variable by SPEC §3.4's definition (the file becomes
/// `unanalyzed` and the gate exits 2), and only the in-process unit tests ever set it. SPEC §3.4 lets an engine that "compiles its switches out of
/// the released artifact" conform trivially, so the hook exists in the test build only and the released scanner has no such switch to name.
/// It deliberately does NOT go through `candor_report::env`: its only caller is a unit test that sets it process-wide, and a registry
/// entry there would mark every later report in that process as switched (and stop the incremental cache storing). The source gate
/// (`tests/env_lint.rs`) skips `#[cfg(test)]` items, which is what makes this read legal and the release read impossible.
#[cfg(test)]
fn injected_fault_for(rel: &str) -> bool {
    std::env::var("CANDOR_PANIC_ON_FILE").is_ok_and(|v| v == rel)
}
#[cfg(not(test))]
fn injected_fault_for(_rel: &str) -> bool {
    false
}

pub(crate) fn scan_main() {
    // ⟨0.30⟩ a RUN starts with no inherited verdict state — see `reset_gate_run_state`.
    crate::gate::reset_gate_run_state();
    // ⟨0.31⟩ …and a run is ONE THREAD. This is the only mint outside tests; everything below carries it
    // by reference. See the note on `gate::RunToken` for what breaks if that stops being true.
    let run_tok = crate::gate::begin_run();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = ".".to_string();
    let mut prefix = String::new();
    let mut want_json = false;
    let mut include_tests = false;
    let mut policy_path: Option<String> = None;
    let mut gate_json_path: Option<String> = None;
    let mut deps_mode = false;
    let mut incremental = false;
    let mut saw_positional = false;
    // ⟨0.28⟩ REPORT STREAM: register whether `--json` was requested BEFORE any exit-2 can fire, so
    // `exit2_refused` and the pre-pass sink refusals below can write the fail-closed report to stdout
    // as the stream's only content. On this engine `--json` is stdout-only (a following non-flag token
    // is a second positional, refused later), so a bare `args.iter().any(|a| a == "--json")` is exact.
    let _ = crate::gate::WANT_JSON_STREAM.set(args.iter().any(|a| a == "--json"));
    // ── SPEC §3.3.1 ⟨0.27⟩ ARM FIRST, and never over an input.
    //
    // This pre-pass learns the sink and this run's inputs with NO side effects, so the collision check
    // below can run before anything is written and the arming can precede EVERY other exit — including
    // the unknown-flag exit in the loop, which §3.3 names as a broken-gate-config exit-2 cause that must
    // leave a refusal behind. Arming inside the loop made the contract depend on argv ORDER.
    // ⟨0.28⟩ BEFORE the collision check and before arming: a repeated `--gate-json` is refused outright,
    // and every path named gets the refusal. Ordering matters for the same reason (2) is ordered the way
    // it is — this writes, so it must not run before the input-collision guard has a chance to refuse a
    // sink that is an input. It is placed AFTER that guard below, not here; here we only learn the list.
    let pre = prescan_argv(&args);
    let (all_gate_sinks, pre_policy, pre_target) = (pre.gate_sinks, pre.policy, pre.target);
    // The sink the parse loop will honour is the LAST accepted one — same rule as `--out` below.
    let pre_gate = all_gate_sinks.last().cloned();
    if let Some(gp) = pre_gate.as_deref() {
        // Order matters and got this wrong: the nonexistent-target refusal below used to run FIRST and
        // it WRITES, so `candor-scan /nope --policy P --gate-json P` destroyed P via the very refusal
        // that exists to keep a red gate red. Every write is now downstream of this check.
        // ⟨0.28⟩ `--json` BESIDE `--gate-json -`: a report and a verdict cannot share one stream. Decided
        // HERE, in the pre-pass, so the refusal is stdout's only content — refusing after the report has
        // gone out leaves the consumer with two documents, which is the defect rather than the fix.
        // `--json <file>` is not this case; on this engine `--json` is stdout-only, so the sink alone
        // decides it.
        if gp == "-" && args.iter().any(|a| a == "--json") {
            eprintln!("candor-scan: --json and --gate-json - both name STDOUT — refusing (exit 2).");
            eprintln!("        `--json` writes the REPORT there and `--gate-json -` the VERDICT, so this");
            eprintln!("        would put two JSON documents on one stream and a consumer parsing it gets");
            eprintln!("        neither. Send one to a file, or run the scan twice.");
            // Written directly: this check runs BEFORE `GATE_JSON_PATH` is registered (arming is
            // deliberately downstream of the input guards), so `exit2_refused` would have nowhere to put
            // it — measured as exit 2 with a zero-byte stream, which is the very shape being fixed. `gp`
            // is `-` here by construction, so stdout IS the sink.
            let doc = candor_report::gate_refusal_json(
                "--json and --gate-json - both name stdout — a report and a verdict cannot share one stream",
            )
            .unwrap_or_else(|_| "{\"ok\":false,\"refused\":true}".to_string());
            println!("{doc}");
            std::process::exit(2);
        }
        // ⟨0.28⟩ THE DUPLICATE CASE IS DECIDED FIRST, because the single-sink guard below exits on `gp`
        // alone — the LAST sink — and with `--gate-json - --gate-json <the policy>` that took the run
        // down before the STREAM could be told anything. Measured: exit 2, stdout zero bytes, while the
        // spec requires the fail-closed document on the stream for ANY exit-2 cause. Deciding the
        // duplicate first is what lets the exemption stay scoped to the offending PATH.
        let distinct = distinct_gate_sinks(&all_gate_sinks);
        if distinct.len() > 1 {
            // ⟨0.28⟩ THE INPUT EXEMPTION COVERS THE PATH, NOT THE RUN. `refuse_gate_json_over_any_input`
            // exits 2 having written nothing, which is right for the offending path — but it used to take
            // the whole run with it, so the OTHER named sink kept whatever it held. Measured: exit 2, the
            // policy correctly intact, and the innocent sink still publishing a previous run's
            // `{"ok": true}` to whoever reads it. Refuse the input FIRST (writing nothing anywhere), and
            // let the duplicate refusal reach every path that is not an input.
            let tgt = pre_target.as_deref().unwrap_or(".");
            let offending: Vec<&String> = distinct
                .iter()
                .filter(|s| gate_json_input_collision(s, tgt, pre_policy.as_deref()))
                .collect();
            if !offending.is_empty() {
                // Nothing is written to the offending path — but the OTHER sinks still get the refusal,
                // and a `-` among them always does: a stream has no input to destroy, so (2)'s
                // justification cannot reach it.
                eprintln!(
                    "candor-scan: --gate-json {} names an INPUT of this run — refusing (exit 2), and nothing was written there.",
                    offending.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                );
                let safe: Vec<String> = distinct
                    .iter()
                    .filter(|s| !offending.iter().any(|o| o == s))
                    .cloned()
                    .collect();
                if !safe.is_empty() {
                    refuse_repeated_gate_json(&safe);
                }
                std::process::exit(2);
            }
            refuse_repeated_gate_json(&distinct);
        }
        // SOUNDNESS R264 — AN UNKNOWN TARGET MUST NOT BE READ AS `.`, BECAUSE THE GUARD BELOW ASKS A
        // QUESTION ABOUT THE TARGET AND `.` ANSWERS A DIFFERENT ONE.
        //
        // When a token the parse loop will refuse arrives BEFORE the target (`-V`, a typo, an
        // unrecognised flag), `stopped` suppresses the target and `pre_target` is None. Falling back to
        // `"."` then asks `gate_json_input_collision` *"is this sink under the CWD?"* instead of *"is it
        // under the target the operator named?"* — and a sink outside the CWD subtree is invisible to
        // that question. Arming proceeded and the fail-closed verdict document was written OVER a source
        // file of the scan, at parse time, before the walk. Measured from an unrelated cwd:
        // `candor-scan -V <dir> --gate-json <dir>/src/lib.rs` exited 0 having replaced `lib.rs`.
        //
        // We cannot answer the containment question without a target. But we can still answer the half
        // that makes it DESTRUCTIVE — does the sink bear an extension this engine parses — and that half
        // needs no target at all. So when the target is unknown, refuse any sink that looks like source
        // and arm nothing. A legitimate sink (`.candor/verdict.json`) is unaffected: it is not `.rs`.
        //
        // Refusing to arm here costs at most a stale green on a run that is exiting 2 anyway and says
        // so; arming costs the operator's file. That is not a close trade.
        match pre_target.as_deref() {
            Some(t) => refuse_gate_json_over_any_input(gp, t, pre_policy.as_deref()),
            None => {
                if gp != "-"
                    && std::path::Path::new(gp).extension().and_then(|e| e.to_str()) == Some("rs")
                {
                    eprintln!(
                        "candor-scan: --gate-json {gp} bears an extension this engine parses (.rs) and this \
                         run has no resolved target to check it against — refusing (exit 2), and nothing \
                         was written there."
                    );
                    eprintln!(
                        "        A flag the parse loop refuses (`-V`, a typo) BEFORE the target leaves the \
                         target unresolved, so the input guard cannot tell whether this sink is a file the \
                         scan reads. Name the target first, or use a non-source sink."
                    );
                    std::process::exit(2);
                }
            }
        }
        // ARM HERE — earlier than any exit this process can take. It used to be armed after the arg
        // loop, so the loop's own `unknown flag` exit(2) left the PREVIOUS run's green document on
        // disk; §3.3 names an unknown flag as a broken-gate-config exit-2 cause, which MUST leave a
        // refusal. Nothing between this line and the verdict can exit without replacing it.
        let _ = GATE_JSON_PATH.set(Some(gp.to_string()));
        crate::gate::arm_gate_json();
    }
    // ⟨0.28⟩ ARM THE REPORT SET, before the arg loop below can exit on an unknown flag.
    //
    // **ONLY AN EXPLICITLY NAMED `--out`, NEVER THE DEFAULT PREFIX.** The first version armed the
    // default `<dir>/.candor/report` too, on the reasoning that an operator who passes no `--out` still
    // has yesterday's reports there to go stale. That reasoning is right about staleness and wrong about
    // OWNERSHIP, and the difference destroys data: measured, `candor-scan <repo> --zzz-not-a-flag`
    // overwrote a COMMITTED `.candor/report.<crate>.scan.json` with the placeholder — in candor-rust's
    // own tree, which commits reports for six crates, and committed reports/baselines are the pattern
    // this project recommends. A run that dies in argv parsing was never going to write there, and it
    // had not been told it owned that path.
    //
    // ⟨0.27⟩'s arming rule never had to face this because `--gate-json` has no default: every verdict
    // sink is named. So the rule as written — "arm at the instant the sink is known" — presumes a sink
    // the operator NAMED, and that presumption is now explicit here. With `--out p` the operator has
    // declared p is this run's output and arming is correct even if p is checked in; with no flag at
    // all there is no such declaration.
    //
    // Found by candor-ts's arm of this rung, which tripped over it while running a conformance probe
    // and left rust's committed report dirty.
    // ⟨0.28⟩ A REPEATED `--out` IS THE SAME RULE AS A REPEATED `--gate-json` (SPEC §3.3.1): `--out A
    // --out B` names where the reports go, twice; the two statements cannot both be honoured, and
    // last-wins leaves `A` holding a previous run's whole per-crate report set, readable as current.
    // Refused at exit 2, **with the fail-closed report written to every prefix named** — which under
    // this sink's own arming rules means arming EACH prefix: the set at risk is the one the previous
    // run left there, rewritten to the ⟨0.21⟩ Row-1 no-claim shape (the armer already asks the input
    // exemption first, per file, and takes the §2.2 sidecars with each report). Two spellings of ONE
    // path are one sink (`distinct_gate_sinks` is the same artifact rule — `--out` never accepts `-`,
    // so the stream arm in it is inert here). The exit routes through `exit2_refused`, so a
    // `--gate-json` sink registered above gets the specific refusal document and a `--json` stream
    // gets the fail-closed report — and the run exits before ever scanning, so the hand-back never
    // runs and the placeholders STAND, which is the fail-closed reading a run that scanned nothing is
    // entitled to.
    let distinct_outs = distinct_gate_sinks(&pre.outs);
    if distinct_outs.len() > 1 {
        let named = distinct_outs.join(", ");
        eprintln!("candor-scan: --out given more than once ({named}) — refusing (exit 2).");
        eprintln!("        A run writes ONE report set to ONE prefix. Naming two says where the reports");
        eprintln!("        go twice, and the reader of the prefix that loses cannot tell it lost — it");
        eprintln!("        goes on holding a previous run's reports as if they were current. Name one,");
        eprintln!("        or run the scan twice.");
        let inputs = run_inputs(pre_target.as_deref().unwrap_or("."), pre_policy.as_deref());
        for p in &distinct_outs {
            crate::gate::arm_out_prefix(p, &inputs);
        }
        crate::gate::exit2_refused(format!(
            "--out was given more than once ({named}) — a run writes one report set to one prefix"
        ));
    }
    // ⟨0.32⟩ the target is latched first, so a refusal during parsing still names what it was pointed at.
    crate::gate::note_scan_target(pre_target.as_deref().unwrap_or("."));
    if let Some(pre_pfx) = pre.outs.last() {
        let inputs = run_inputs(pre_target.as_deref().unwrap_or("."), pre_policy.as_deref());
        crate::gate::arm_out_prefix(pre_pfx, &inputs);
        crate::gate::note_report_prefix(pre_pfx);
    } else if let Some(t) = pre_target.as_deref() {
        // ⟨0.32⟩ NO `--out`, SO THE DEFAULT PREFIX — AND LATCHED HERE, AT PRE-PARSE, NOT LATER.
        //
        // This is the window the marker exists to close and the one that separates it from arming. A
        // refusal during argv parsing (an unknown flag is the everyday case) happens BEFORE the scan
        // resolves its target and computes this prefix, so latching it downstream leaves exactly the
        // shape that bit: a stale green survives and `gate --report` certifies it. The TARGET is already
        // known here, and the default prefix is a pure function of it.
        //
        // Arming could not be moved this early — that is the measured data loss, a run that died in
        // parsing replacing a committed report. Writing a marker BESIDE the reports can be, because it
        // destroys nothing: the earliest safe moment and the earliest useful moment are the same moment.
        //
        // SOUNDNESS R520 — …BUT ONLY IF THE TARGET IS THERE. `candor-scan nonexistent-target /bogus`
        // exits 2 correctly and used to leave `./nonexistent-target/.candor/report.refused.json`
        // behind: the marker's `create_dir_all` MADE a directory tree in the operator's CWD, named
        // after a mistyped argument (`nope/deeper/still/.candor/` on a deeper typo). Measured
        // four-way: rust and ts created it, java and swift created nothing, all four exit 2 — so the
        // refusal was never in question and only the filesystem effect differed. FOUND AS LITTER IN
        // THIS FAMILY'S OWN REPO, not by any gate.
        //
        // The reason it is wrong and not merely untidy is the marker's own charter. It exists so a
        // STALE report cannot be read as current — it SHADOWS a previous run's document. The default
        // prefix is derived FROM the target, so when the target does not exist neither can any report
        // under it: there was nothing to shadow, and the write has nothing to fail closed over.
        //
        // The boundary is the declaration/convention line this comment already draws, and it is why
        // the guard sits on this arm and not inside `write_refusal_marker`:
        //   • `--out <pfx>` (the arm above) is a path the operator NAMED, independent of the target.
        //     A previous run over a DIFFERENT, existing target may have left reports there, so a typo
        //     in today's target must still shadow them. That arm is untouched, deliberately.
        //   • `<target>/.candor/report` is a convention computed from the target. An absent target
        //     cannot have reports, so there is nothing for the convention to protect.
        // The existence check ~200 lines below refuses an absent target at exit 2 without a marker
        // already, so no refusal reachable with an absent target loses one it used to have.
        if std::path::Path::new(t).exists() {
            crate::gate::note_report_prefix(&format!("{t}/.candor/report"));
        }
    }
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => {
                // The dash-check `--gate-json` records as fixed, applied here too: `--out --policy P`
                // swallowed `--policy` as the prefix, so the displaced bare `P` became the scan target
                // and the run went GATELESS — exit 0, no gate, writing a report named `--policy.*`.
                // A valueless gate-adjacent flag must fail, never silently drop the gate.
                // The flag-shaped case names the token (SPEC §3.2 ⟨0.28⟩ "given no value"): the
                // operator typed a flag where a prefix belongs, and `./--weird` is the escape hatch.
                match it.next() {
                    Some(v) if !v.starts_with('-') => prefix = v.clone(),
                    Some(v) if v != "-" => {
                        eprintln!("candor-scan: --out was given no value — the next token '{v}' is a flag, not a prefix (a path really named that is spelled ./{v})");
                        // ⟨0.27⟩ every pre-verdict exit leaves the refusal document at the sink — the
                        // stream sink included (SPEC §3.1); see `exit2_refused`.
                        crate::gate::exit2_refused(format!("--out was given no value (the next token '{v}' is a flag)"));
                    }
                    _ => {
                        eprintln!("candor-scan: --out requires a path prefix (a following non-flag value)");
                        crate::gate::exit2_refused("--out requires a path prefix (a following non-flag value)");
                    }
                }
            }
            "--json" => want_json = true,
            "--include-tests" => include_tests = true,
            "--incremental" => incremental = true,
            "--policy" => {
                // A valueless trailing `--policy` (no path follows) must ERROR, not silently fall
                // back to no-gate — matching the strict posture of a set-but-unreadable policy.
                // Silently dropping the gate would let a violation ship under an intended-gated run.
                // A FLAG-SHAPED next token is the same case (SPEC §3.2 ⟨0.28⟩): "given no value" MEANS
                // the next token is flag-shaped, or the clause is unimplementable — consuming it as a
                // filename made this very diagnostic unreachable, and `--policy --gate-json -` read
                // the operator's verdict sink as an unreadable policy path. Measured on this engine:
                // exit 2 with NOTHING on the stream where the refusal document belongs (conformance
                // §3.1 (b13)). The pre-pass leaves the flag-shaped token LIVE, so the sink it names
                // was already armed when this refusal fires and `exit2_refused` reaches it.
                match it.next().cloned() {
                    Some(p) if p == "-" || !p.starts_with('-') => policy_path = Some(p),
                    Some(p) => {
                        eprintln!("candor-scan: --policy was given no value — the next token '{p}' is a flag, not a path (a file really named that is spelled ./{p})");
                        // ⟨0.27⟩ the stream sink gets the refusal too — see `exit2_refused`.
                        crate::gate::exit2_refused(format!("--policy was given no value (the next token '{p}' is a flag)"));
                    }
                    None => {
                        eprintln!("candor-scan: --policy requires a path argument");
                        crate::gate::exit2_refused("--policy requires a path argument");
                    }
                }
            }
            "--gate-json" => {
                // The structured gate verdict target (candor-spec §3.3). Valueless OR flag-shaped fails
                // closed (exit 2): without the dash-check, `--gate-json --policy pol` swallowed `--policy`
                // as the verdict path AND the displaced `pol` token replaced the scan dir (last-positional
                // -wins) — a gateless exit-0 run over the wrong target (max-review find, shipped in 0.8.x).
                // `-` (stream the verdict to stdout) stays valid.
                match it.next().cloned() {
                    Some(p) if p == "-" || !p.starts_with('-') => gate_json_path = Some(p),
                    Some(p) => {
                        eprintln!("candor-scan: --gate-json was given no value — the next token '{p}' is a flag, not a path (a file really named that is spelled ./{p})");
                        // Through `exit2_refused`, not a bare exit(2): another `--gate-json` in the
                        // same argv may already have armed a sink, and a broken command line does not
                        // un-name it — the refusal document must still reach it (SPEC §3.2 ⟨0.28⟩).
                        crate::gate::exit2_refused(format!("--gate-json was given no value (the next token '{p}' is a flag)"));
                    }
                    None => {
                        eprintln!("candor-scan: --gate-json requires a path argument (or `-` for stdout)");
                        crate::gate::exit2_refused("--gate-json requires a path argument (or `-` for stdout)");
                    }
                }
            }
            "--deps" => deps_mode = true,
            "-V" | "--version" => {
                // Two lines, fully OFFLINE: the installed build + the spec contract it speaks, then
                // the upgrade incantation. <spec> reuses candor_report::SPEC_VERSION — the same source
                // that stamps the report envelope's `spec` field, so the two can never drift.
                println!("candor-scan {} (spec {})", env!("CARGO_PKG_VERSION"), candor_report::SPEC_VERSION);
                println!("upgrade: cargo install candor-scan --force");
                return;
            }
            // The agent contract for THE INSTALLED VERSION, embedded at build time — doc and
            // binary cannot drift (the §2.1 version-trust rule applied to documentation). Agents
            // are told to run this instead of trusting a vendored/remote copy.
            "--agents" => {
                println!("<!-- candor-scan {} · the agent contract for this installed version -->", env!("CARGO_PKG_VERSION"));
                print!("{}", include_str!("../AGENTS.md"));
                return;
            }
            "-h" | "--help" => {
                // The family house-style help: identity line, model paragraph, USAGE, OPTIONS,
                // ENVIRONMENT/CONFIG, EXAMPLES, footer. tests/cli.rs pins `USAGE` + exit 0.
                println!("candor-scan — the Rust effect scanner that reads a crate without building it.");
                println!();
                println!("Stable toolchain, purely syntactic: parses the source with syn — no nightly, no");
                println!("rustc-dev, no compile. Emits the same report every family engine speaks, through");
                println!("the shared classifier, and it deliberately under-reports rather than fabricates:");
                println!("Unknown marks only boundaries it can see (callbacks, FFI, untrusted chained reports).");
                println!();
                println!("USAGE");
                println!("  candor-scan [<dir>] [options]        scan a crate (default: .). A [workspace] root");
                println!("                                       scans every member — one report per member under");
                println!("                                       the one prefix; a nested dir with its own");
                println!("                                       Cargo.toml is a different package and is never");
                println!("                                       folded into the parent's report");
                println!("  candor-scan --agents                 print the agent contract embedded in this build");
                println!("  candor-scan --version | --help");
                println!();
                println!("OPTIONS");
                println!("  --out <prefix>       report path prefix (default: <dir>/.candor/report);");
                println!("                       writes <prefix>.<crate>.scan.json + a call-graph sidecar");
                println!("  --json               print the report to stdout instead of writing files");
                println!("  --include-tests      also scan tests/ benches/ examples/ and #[cfg(test)] modules");
                println!("                       (off by default — the report describes the crate, not its harness)");
                println!("  --incremental        reuse a per-file parse/decl cache under <dir>/.candor/cache so an");
                println!("                       edit-then-rescan skips re-parsing unchanged files (~7x on a one-file");
                println!("                       edit). Produces a BYTE-IDENTICAL report to a full scan; a candor-scan");
                println!("                       upgrade or a decl-changing edit invalidates the cache automatically");
                println!("  --deps               scan the Cargo.lock dependency tree first (registry sources from");
                println!("                       ~/.cargo/registry/src) into <dir>/.candor/deps/, then scan <dir>");
                println!("                       CHAINED over those reports — effects cross every crate boundary");
                println!("                       without the classifier needing to know the crates");
                println!("  --policy <file>      enforce a CANDOR_POLICY file (deny/pure/allow/forbid rules);");
                println!("                       exit 1 on a violation");
                println!("  --gate-json <f|->    write the structured gate verdict {{ spec, ok, violations }} as JSON");
                println!("                       over this scan; `-` streams it to stdout; exit 1 on violation.");
                println!("                       ADVISORY FLOOR: the syntactic backend under-reports, so a miss can");
                println!("                       pass — the nightly engine is the sound gate");
                println!("  -V, --version        print the installed build and its contract version (offline), and");
                println!("                       the upgrade line");
                println!("  -h, --help           print this help");
                println!();
                println!("ENVIRONMENT / CONFIG");
                println!("  CANDOR_POLICY=<f>    the policy file when --policy is absent; the checked-in floor");
                println!("                       under both is .candor/config (keys this engine reads: policy,");
                println!("                       baseline, deps — discovered from the scan target; a $CANDOR_CONFIG");
                println!("                       path overrides discovery)");
                println!("  CANDOR_BASELINE=<p>  the effect-regression guard (rule AS-EFF-005): compare this scan");
                println!("                       against a saved report (a path or --out prefix; also the `baseline`");
                println!("                       config key). A fn GAINING an effect vs the baseline exits 1; absent");
                println!("                       baseline → note, guard inactive; unparseable or produced by a");
                println!("                       DIFFERENT build → exit 2 — never a stale compare.");
                println!("                       Record one: candor-scan <dir> --out <prefix>");
                println!("  CANDOR_DEPS=<p:…>    chain sibling reports (files or directories of *.json): an");
                println!("                       unclassified call into a crate a report covers inherits that");
                println!("                       function's effects + literal surfaces. Scan the dep once, chain it");
                println!("                       everywhere; the coverage ledger names what to scan next.");
                println!("                       TWO KINDS OF REPORT GRANT NO COVERAGE, so a key they do not");
                println!("                       answer discloses instead of reading pure: one produced by a");
                println!("                       DIFFERENT engine build (§2.1 — its entries are also downgraded");
                println!("                       to Unknown) and one that declares itself INCOMPLETE (a non-empty");
                println!("                       ⟨0.21⟩ `unanalyzed` — its entries are KEPT unchanged, since they");
                println!("                       came from source it did read; only its silence hedges)");
                println!();
                println!("EXAMPLES");
                println!("  candor-scan .");
                println!("  candor-scan . --policy candor.policy --gate-json verdict.json");
                println!("  candor-scan . --deps");
                println!("  candor-scan crates/mycrate --json");
                println!();
                println!("Docs: candor.poly.io   ·   Verify an install: candor doctor");
                return;
            }
            other => {
                // An unknown flag must FAIL, not become a path: an agent following a newer doc
                // against an older binary ran `candor-scan --agents` and scanned a directory
                // literally named `--agents`; a typo'd `--polcy` would silently drop the gate.
                if other.starts_with('-') {
                    eprintln!("candor-scan: unknown flag '{other}' (see --help)");
                    // ⟨0.27⟩ §3.3 names an unknown flag as a broken-gate-config exit-2 cause, and the
                    // fail-closed document has no exempt cause AND no exempt sink: the file sink was
                    // covered by arming, the stream sink was not — `--gate-json -` beside a typo'd flag
                    // exited 2 with an EMPTY stdout, measured in three of four engines (swift wrote).
                    crate::gate::exit2_refused(format!("unknown flag '{other}' (see --help)"));
                }
                // A SECOND POSITIONAL IS A USAGE ERROR, NOT A SILENT TARGET REPLACEMENT. Until this
                // check, `dir = a.clone()` ran on EVERY bare token, so the last positional silently won
                // and `candor-scan A B` scanned B while saying nothing about A. That is a green gate over
                // violating code, measured on a two-crate fixture: `candor-scan A --policy 'deny Fs'`
                // exits 1, and `candor-scan A B --policy 'deny Fs'` exits 0 with `functions: []` and
                // `analyzed.count 1` — not a disclosed gap but a positive ⟨0.21⟩ purity claim over a unit
                // it never read. A shell glob that matches two paths, or an empty CI variable in
                // `candor-scan "$DIR" "$EXTRA"`, is a permanent all-clear.
                //
                // It was known and worked AROUND rather than rejected: `prescan_sink_and_inputs` takes the
                // LAST positional specifically to mirror this loop, with a comment explaining that taking
                // the first "checked the wrong pair" when there were two. The right answer to two targets
                // was never to pick one. candor-swift already refused this; found four-way by the argv
                // combination sweep in candor/bin/probe-causes.sh and pinned by conformance PART 36 (b18).
                if saw_positional {
                    let why = format!(
                        "unexpected extra argument `{a}` — the scan takes ONE target (got `{dir}` and `{a}`). \
                         Did you mean a flag? See --help."
                    );
                    eprintln!("candor-scan: {why}");
                    crate::gate::exit2_refused(why);
                }
                saw_positional = true;
                dir = a.clone();
            }
        }
    }
    // A SCAN TARGET THAT DOES NOT EXIST IS UNEVALUABLE, NOT CLEAN. This engine scanned a nonexistent
    // path, found nothing, and exited 0 with `ok: true` and `analyzed.count 0` — so a typo'd path in CI
    // is a PERMANENT GREEN, and the stderr even claimed it had "wrote 0 effectful functions" there.
    // java and ts refuse; swift refuses. Found by a review probing what the new arming would be
    // faithfully replaced BY.
    if !std::path::Path::new(&dir).exists() {
        eprintln!("candor-scan: no such path: {dir}");
        eprintln!("        point candor-scan at a crate or workspace directory. Exit 2 (unevaluable) —");
        eprintln!("        a target that is not there is not a clean scan.");
        let _ = GATE_JSON_PATH.set(gate_json_path.clone());
        crate::gate::record_gate_refusal(format!("no such path: {dir}"));
        crate::gate::write_gate_json(2);
        std::process::exit(2);
    }
    // (the --policy/--gate-json collision is refused in the pre-pass at the top of this fn, which is
    // the only place EARLIER than the first write — see refuse_gate_json_over_input.)
    // (armed in the pre-pass at the top of this fn — SPEC §3.3.1 ⟨0.27⟩. This `set` is the no-op that
    // keeps the path correct when no --gate-json was given at all.)
    let _ = GATE_JSON_PATH.set(gate_json_path);
    // `.candor/config` (candor-spec §config): the checked-in floor under the env vars. Discovery is
    // anchored to the SCAN TARGET (walk up from `dir` to the repo root's .candor/config), never the CWD;
    // $CANDOR_CONFIG overrides discovery. FAIL-CLOSED when configured-but-unusable (exit 2 — the §6.2
    // unreadable-policy posture); only genuine absence is empty.
    let cfg = load_candor_config(&dir);
    // The policy source is resolved HERE, once (flag wins, CANDOR_POLICY env next, the config file as the
    // floor) — never inside scan_one, so --deps dependency scans can't inherit the root gate via the env.
    let policy = policy_path
        .or_else(|| candor_report::env::operator("CANDOR_POLICY").ok())
        .or_else(|| cfg.get("policy").cloned());
    // The AS-EFF-005 baseline (spec §7 item 5), resolved once like the policy: CANDOR_BASELINE env
    // over the config `baseline` key (already home-anchored by load_candor_config). Dependency scans
    // under --deps run guard-free — a dep's internals are not this repo's ratchet.
    // WHICH SOURCE supplied the baseline decides what a MISSING file means (see `check_baseline`):
    // `CANDOR_BASELINE` is set unconditionally by the adopt workflow, so an absent path there is "the
    // ratchet is not adopted yet"; a checked-in `baseline` line DECLARES this repo has one, so an absent
    // path there was deleted or never committed — and passing green over it is a gate that stopped
    // gating in silence.
    let _ = crate::gate::BASELINE_FROM_CONFIG.set(candor_report::env::operator("CANDOR_BASELINE").is_err() && cfg.contains_key("baseline"));
    let baseline = candor_report::env::operator("CANDOR_BASELINE").ok().or_else(|| cfg.get("baseline").cloned());
    // ⟨unknown-ratchet⟩ OPT-IN on the AS-EFF-005 guard (config `unknown-ratchet` / CANDOR_UNKNOWN_RATCHET,
    // default OFF): when ON, a NEWLY-introduced Unknown vs the baseline FAILS instead of staying advisory —
    // making `deny Unknown` adoptable on legacy code. Resolved once here (env presence wins, else the config
    // truthy value) and read by every check_baseline via the UNKNOWN_RATCHET global — see check_baseline.
    let _ = crate::gate::UNKNOWN_RATCHET.set(crate::config::flag(&cfg, "unknown-ratchet", "CANDOR_UNKNOWN_RATCHET"));
    // The --gate-json target rides a global (like INCREMENTAL below) so it threads no ScanOpts. Members
    // RECORD violations (record_gate_violations); the verdict is written ONCE here after the whole scan —
    // per-member writes let a clean last member overwrite an earlier violator's verdict (ok:true vs exit 1).
    // Dependency scans under --deps run gate-free (policy=None in scan_one), so they record nothing.
    // THE PIN CHECK RUNS AFTER THE ARMING, and the order is the whole point. It used to run 25 lines
    // earlier, so its exit 2 left the PREVIOUS run's `--gate-json` document on disk — a false green on
    // the machine channel from the release's flagship guard. Anything that can exit must come after the
    // verdict is armed fail-closed.
    enforce_engine_pin(&dir);
    // ⟨0.42⟩ does this run HAVE a gate? A policy or a baseline asks a question; a bare scan asks none (SPEC §3.4 (c)).
    let gating = policy.is_some() || baseline.is_some();
    if deps_mode {
        let code = run_with_deps(&dir, prefix, want_json, include_tests, policy, baseline, &run_tok);
        // ⟨0.41⟩ settle the whole run's `accept` entries (a no-op without any).
        let code = crate::gate::finalize_accept(code, want_json, &run_tok);
        // ⟨0.42⟩ SPEC §3.3 cause (e): a gate under a set result-moving switch is unanswerable.
        let code = crate::gate::refuse_under_switches(code, gating);
        // ⟨0.28⟩ the run finished writing: hand back any armed report it turned out not to own.
        crate::gate::disarm_unwritten_out_reports();
        write_gate_json(code);
        std::process::exit(code);
    }
    // Incremental is OPT-IN and SAFE: a full scan (no flag) never reads the cache, and `--incremental`
    // with no/invalid cache transparently does a full scan + populates it (the gates downgrade any
    // stale entry to a re-derivation). The flag rides in a thread-local so it doesn't thread through
    // every signature between `main` and `scan_one` (scan_target/run_with_deps are unchanged).
    INCREMENTAL.with(|c| c.set(incremental));
    // Cross-crate report chaining (spec §2): CANDOR_DEPS names sibling reports (a `:`-separated
    // list of files and/or directories of *.json); an unclassified qualified call into a crate one
    // of them covers inherits that function's recorded effects + literal surfaces. The stable
    // scanner's half of the dep-scan story: scan the dep once, chain it everywhere.
    let deps_spec = candor_report::env::operator("CANDOR_DEPS").ok().or_else(|| cfg.get("deps").cloned());
    let mut deps_idx = load_dep_reports(deps_spec.as_deref());
    r1743_facade_owner_keys(&mut deps_idx, &dir);
    // scan_target handles both a single crate and a `[workspace]` root (one report per member under
    // one prefix — candor-query's multi-crate merge consumes them together; the policy gates each).
    let code = scan_target(&dir, prefix, want_json, include_tests, policy, baseline, &deps_idx, &run_tok);
    // ⟨0.41⟩ SPEC §6.2 `accept`: the AUTHORITATIVE settlement, over every member's rows at once (a no-op without
    // an entry). Stale entries and cross-member twins can only be seen here, and only ever make the run redder.
    let code = crate::gate::finalize_accept(code, want_json, &run_tok);
    // ⟨0.42⟩ SPEC §3.3 cause (e): a gate under a set result-moving switch is unanswerable.
    let code = crate::gate::refuse_under_switches(code, gating);
    // ⟨0.28⟩ the run finished writing: hand back any armed report it turned out not to own.
    crate::gate::disarm_unwritten_out_reports();
    write_gate_json(code);
    std::process::exit(code);
}

/// Options for one crate scan. `policy` and `baseline` are RESOLVED by the caller (flag/env/config) —
/// scan_one itself never reads the env, so dependency scans under --deps can genuinely run
/// gate-free (review: the env fallback inside scan_one ran the root policy 328 times against
/// dependency internals). `quiet` suppresses the per-scan receipts (dep scans; the --deps summary
/// line speaks for them).
pub(crate) struct ScanOpts<'a> {
    pub(crate) prefix: String,
    pub(crate) want_json: bool,
    pub(crate) include_tests: bool,
    pub(crate) policy: Option<String>,
    /// The AS-EFF-005 baseline value (`CANDOR_BASELINE` env / config `baseline` key): a saved report's
    /// path or `--out` prefix. See `check_baseline` for the full guard contract.
    pub(crate) baseline: Option<String>,
    pub(crate) quiet: bool,
    /// ⟨0.31⟩ TRUE when this call is one MEMBER of a workspace invocation. The unevaluable-target
    /// refusal is PER-INVOCATION, not per-member: a workspace with one live member and one scaffolded
    /// one must stay green, and a member that judged nothing must still publish its ⟨0.24⟩ count-0
    /// report — the wire table at SPEC §2 defines that shape and a per-member refusal would delete it.
    /// The same rule holds elsewhere for the same reason: swift `binary`/`system` targets carry zero
    /// sources by design, a maven aggregator has no classes, and a ts solution root unions its members.
    pub(crate) ws_member: bool,
    pub(crate) deps_idx: &'a DepIndex,
    /// ⟨0.29⟩ THE PEEK. When true this run INVERTS the file selection: it analyzes exactly the files a
    /// normal run excludes, and nothing else. Set only by the recursive call `scan_one` makes on itself,
    /// so the peek goes through the IDENTICAL parse / Pass A / Pass B / classifier pipeline as the gate.
    ///
    /// That identity is the whole design constraint. A hand-written second pass over `build.rs` would be
    /// a SECOND OPINION, and a drifted second opinion reported as a warning is worse than no warning —
    /// the operator cannot tell a real finding from a disagreement between two code paths. Reusing the
    /// entry point makes "same classifier, different file set" true by construction rather than by
    /// review. It also means a peek NEVER peeks again: this flag suppresses the recursion.
    pub(crate) peek_excluded: bool,
}

/// SOUNDNESS R1605 — the dependency traits whose RECEIVER-LESS members a crate-local implementor (written, or a known
/// derive's skeleton, `mbe::splice_sea_orm_entities`) is typed through, as `(crate, trait leaf)`. A table rather than every
/// dependency trait because reading a trait's members means parsing that dependency's source, and a crate that implements
/// `serde::Serialize` must not pay for it; a row is added with its own executed fixture.
const R1605_TRAITS: &[(&str, &str)] = &[("sea_orm", "EntityTrait")];

/// SOUNDNESS R1591 — the calibration-list identifiers that are an EXTERN ALIAS, not a package name, with the package
/// each stands for. The κ exemption is asked of the real package; an alias entry exempts only a rename to its package.
const R1591_CALIBRATED_ALIASES: &[(&str, &str)] = &[("native_tls_crate", "native_tls")];

/// SOUNDNESS R1578 — TYPE AN UNCHAINED DEPENDENCY'S VALUE FROM ITS SOURCE. The collector records where an untyped
/// receiver's value came from (`<cr>::<untyped>::<callee>::<method>`, and since round 3 the full provenance in
/// `Call::prov`), and that marker was consumed only for a CHAINED dependency. Unchained it reached nothing: the κ ledger
/// counted the crate only through the factory CALL, so a classifier-covered crate (calibrated, or no lockfile to exceed a
/// ceiling) and a factory a rule already classifies left the method SILENT — `let js = async_nats::jetstream::new(c);
/// js.query_account()` sends a request and read ABSENT.
///
/// `r1578_resolve` walks the provenance through the source the project's Cargo.lock pins (`DepSigs::ret_of`; the owner's
/// ruling of 2026-11-09: no lock, an absent source, a semver guess — nothing typed): a factory's return; `?` / `unwrap` /
/// `expect` peel a `Result`/`Option` the signature declares; `.await` resolves an `async fn`; a METHOD step only where
/// R1200's probe provably reaches the dependency's INHERENT method (owned receiver + `self`, `&` + `&self`, `&mut` +
/// `&mut self`, the receiver's form read from its declared type). The marker then becomes the typed call
/// `<cr>::<module>::<Type>::<method>`. The second element is whether the consumer DECLARES a nominal type of the same leaf
/// (such a call must not take the local route, round 2 hole 4).
fn r1578_type(
    c: &Call,
    deps: &std::collections::HashSet<String>,
    renames: &HashMap<String, String>,
    deps_idx: &DepIndex,
    dir: &str,
    sigs: &std::cell::RefCell<crate::depsig::DepSigs>,
    returns: &ReturnIndex,
) -> Option<(Call, bool)> {
    let (cr, method, ty) = match r1578_resolve(c, deps, renames, deps_idx, dir, sigs)? {
        R1578::Typed { cr, method, ty } => (cr, method, ty),
        R1578::Std => return None,
    };
    sigs.borrow_mut().note_typed(&cr);
    let ty = r1619_public(&cr, renames, dir, sigs, ty, &method);
    let leaf = ty.rsplit("::").next().unwrap_or(&ty);
    let twin = returns.contains_key(&crate::model::nominal_key(leaf));
    let path = format!("{cr}::{ty}::{method}");
    if candor_report::env::trace("CANDOR_R1578_INSTR") {
        eprintln!("R1578TYPED\t{}\t{path}\ttwin={twin}\t{}", c.path, c.prov.as_deref().unwrap_or("-")); // §E1 REACH COUNTER
    }
    Some((Call { path, leaf: method, typed: true, method: true, dot: true, ..c.clone() }, twin))
}

/// SOUNDNESS R1605 — a BUILDER STEP on a typed `R1605_TRAITS`-crate value. The collector types `post::Entity::find()` (and
/// `E::find()` under `E: EntityTrait`) as the dependency's `Select`; a `let` over a further step (`let q = E::find().filter(c);
/// q.all(db)`) records that typed base and the steps (`<cr>::<untyped>::<Type>::filter::all`, or `…::<Type>::<chain>a.b::m`),
/// because the strict binder does not assume a step returns its receiver. Here each step is checked against the
/// lock-pinned source (`DepSigs::step_map`: every declaration the step could reach agrees on the type it yields), and
/// only then is the call formed on the resulting type, where the classifier answers.
/// `None` for anything else: the published behaviour.
fn r1605_chain_type(
    c: &Call,
    deps: &std::collections::HashSet<String>,
    renames: &HashMap<String, String>,
    deps_idx: &DepIndex,
    dir: &str,
    sigs: &std::cell::RefCell<crate::depsig::DepSigs>,
) -> Option<(Call, bool)> {
    if candor_report::env::switch("CANDOR_R1605_OFF") {
        return None;
    }
    let (cr, rest) = c.path.split_once(&format!("::{UNTYPED_RECV_MARKER}::"))?;
    let cr_real: &str = renames.get(cr).map(String::as_str).unwrap_or(cr);
    if !R1605_TRAITS.iter().any(|(k, _)| *k == cr_real) || !deps.contains(cr) || deps_idx.crates.contains(cr_real) {
        return None;
    }
    let (callee, method) = rest.rsplit_once("::")?;
    let (base, steps): (&str, Vec<&str>) = match callee.split_once("::<chain>") {
        Some((b, s)) => (b, s.split('.').collect()),
        None => {
            let (b, s) = callee.rsplit_once("::")?;
            (b, vec![s])
        }
    };
    let leaf = base.rsplit("::").next()?;
    if base.contains('<') || !crate::lang::is_type_ident(leaf) || steps.iter().any(|s| s.is_empty() || s.contains('<')) {
        return None;
    }
    let _ = leaf;
    let mut sigs_b = sigs.borrow_mut();
    let mut cur = base.to_string();
    for st in &steps {
        let cur_leaf = cur.rsplit("::").next().unwrap_or(&cur).to_string();
        let map = sigs_b.step_map(dir, cr, cr_real, &cur_leaf);
        let (_, next) = map.into_iter().find(|(m, _)| m == st)?;
        cur = next;
    }
    let base = cur.as_str();
    sigs_b.note_typed(cr);
    let path = format!("{cr}::{base}::{method}");
    if candor_report::env::trace("CANDOR_R1605_INSTR") {
        eprintln!("R1605STEP\t{}\t{path}", c.path); // §E1 REACH COUNTER
    }
    Some((Call { path, leaf: method.to_string(), typed: true, method: true, dot: true, ..c.clone() }, false))
}

/// SOUNDNESS R1617(4) — A DEPENDENCY BUILDER CHAIN RE-TYPED FROM THE LOCK-PINNED SOURCE. `c.get(u).send()` over `c:
/// &reqwest::Client` is typed by the chain walk as its ROOT (`Client::send`), on the assumption every step returns its
/// receiver; the collector files a `<chain>` guessed-owner marker beside that call (R861), which only a CHAINED dependency's
/// report could answer, and an unchained one read nothing — so a rule keyed on the step's real type (`RequestBuilder::send`,
/// `ExponentialBackoff::next_backoff`) never matched. Here the marker's provenance is walked through the dependency's
/// signatures exactly as an `<untyped>` marker's is (`r1578_resolve`), and where every step is answered the marker becomes
/// the typed call on the type the chain really reaches. ADDED beside the guessed call, which stays. `None` otherwise.
fn r1617_chain_type(
    c: &Call,
    deps: &std::collections::HashSet<String>,
    renames: &HashMap<String, String>,
    deps_idx: &DepIndex,
    dir: &str,
    sigs: &std::cell::RefCell<crate::depsig::DepSigs>,
) -> Option<(Call, bool)> {
    if candor_report::env::switch("CANDOR_R1617H_OFF") {
        return None;
    }
    c.prov.as_ref()?;
    let (cr, _) = c.path.split_once(&format!("::{CHAIN_GUESS_MARKER}::"))?;
    let pseudo = Call { path: format!("{cr}::{UNTYPED_RECV_MARKER}::{CHAIN_GUESS_MARKER}::{}", c.leaf), ..c.clone() };
    let R1578::Typed { cr, method, ty } = r1578_resolve(&pseudo, deps, renames, deps_idx, dir, sigs)? else { return None };
    sigs.borrow_mut().note_typed(&cr);
    let ty = r1619_public(&cr, renames, dir, sigs, ty, &method);
    let path = format!("{cr}::{ty}::{method}");
    if candor_report::env::trace("CANDOR_R1617_INSTR") {
        eprintln!("R1617CHAIN\t{}\t{path}\t{}", c.path, c.prov.as_deref().unwrap_or("-")); // §E1 REACH COUNTER
    }
    Some((Call { path, leaf: method, typed: true, method: true, dot: true, prov: None, ..c.clone() }, false))
}

/// SOUNDNESS R1619 — THE TYPED CALL IS SPELLED THE WAY A CONSUMER CAN SPELL IT. R1578 places a value on the type's
/// DECLARING qual (`tokio_postgres::transaction_builder::TransactionBuilder`), a path in a private module no consumer can
/// write, while a classifier rule keyed on the consumer's spelling names the public re-export
/// (`tokio_postgres::TransactionBuilder::start`): the typed call then met no rule and `let b = c.build_transaction();
/// b.start()` read ABSENT over an executed BEGIN. Where the declaring spelling classifies nothing and the type's public path
/// (`DepSigs::public_qual`, through `pub use`s from the root) does, that path is used. Either spelling names the same type,
/// so the switch can only find a rule the other missed. `ty` unchanged otherwise.
fn r1619_public(
    cr: &str,
    renames: &HashMap<String, String>,
    dir: &str,
    sigs: &std::cell::RefCell<crate::depsig::DepSigs>,
    ty: String,
    method: &str,
) -> String {
    if candor_report::env::switch("CANDOR_R1619_OFF") {
        return ty;
    }
    let cr_real: &str = renames.get(cr).map(String::as_str).unwrap_or(cr);
    if candor_classify::classify(cr_real, &format!("{cr_real}::{ty}::{method}")).is_some() {
        return ty;
    }
    let Some(p) = sigs.borrow_mut().public_qual(dir, cr, cr_real, &ty) else { return ty };
    if candor_classify::classify(cr_real, &format!("{cr_real}::{p}::{method}")).is_some() {
        if candor_report::env::trace("CANDOR_R1619_INSTR") {
            eprintln!("R1619PUB\t{cr}::{ty}::{method}\t{cr}::{p}::{method}"); // §E1 REACH COUNTER
        }
        return p;
    }
    ty
}

/// SOUNDNESS R1578 — where an `<untyped>` marker's receiver provably lands.
pub(crate) enum R1578 {
    /// a type the dependency declares (crate-relative qual)
    Typed { cr: String, method: String, ty: String },
    /// a std type, or a `Result`/`Option` still wrapping the value: the call is std's if std has the method
    Std,
}

/// SOUNDNESS R1578 — resolve an `<untyped>` marker for an UNCHAINED declared dependency through its provenance (see
/// `r1578_type`). `None`: the receiver could not be placed.
pub(crate) fn r1578_resolve(
    c: &Call,
    deps: &std::collections::HashSet<String>,
    renames: &HashMap<String, String>,
    deps_idx: &DepIndex,
    dir: &str,
    sigs: &std::cell::RefCell<crate::depsig::DepSigs>,
) -> Option<R1578> {
    use crate::depsig::RetTy;
    if candor_report::env::switch("CANDOR_R1578_OFF") {
        return None;
    }
    let (cr, rest) = c.path.split_once(&format!("::{UNTYPED_RECV_MARKER}::"))?;
    let cr_real: &str = renames.get(cr).map(String::as_str).unwrap_or(cr);
    if cr.contains("::") || !deps.contains(cr) || deps_idx.crates.contains(cr_real) {
        return None;
    }
    let method = rest.rsplit_once("::")?.1.to_string();
    let prov = c.prov.as_deref()?;
    let mut parts = prov.split('|');
    let base = parts.next()?;
    let strip = |p: &str| -> Option<String> {
        let (root, rest) = p.split_once("::")?;
        (root == cr).then(|| rest.to_string())
    };
    let mut sigs = sigs.borrow_mut();
    // the state: async-ness, wrappers (outermost first), the placed type, and the receiver's form for a method step
    let (mut is_async, mut wraps, mut ty, mut form): (bool, Vec<char>, RetTy, char) = if let Some(f) = base.strip_prefix("F:") {
        let r = sigs.ret_of(dir, cr, cr_real, &strip(f)?, None)?;
        (r.is_async, r.wraps, r.ty, 'o')
    } else {
        let (t, f) = base.strip_prefix("T:")?.rsplit_once(':')?;
        (false, Vec::new(), RetTy::Dep(strip(t)?), f.chars().next()?)
    };
    for st in parts {
        match st {
            "a" if is_async => is_async = false,
            "?" | "u" if !is_async && matches!(wraps.first(), Some('R' | 'O')) => {
                wraps.remove(0);
            }
            // SOUNDNESS R1619 — `Result::ok` (std's inherent `fn ok(self) -> Option<T>`, the first probe step on an owned
            // `Result`): the value is unchanged, its wrapper becomes `Option` (`cfg.create_pool(..).ok()?`)
            "m:ok" if !is_async && wraps.first() == Some(&'R') && !candor_report::env::switch("CANDOR_R1619_OFF") => {
                wraps[0] = 'O';
            }
            // SOUNDNESS R1627 — a step that returned `&mut Self` / `&Self` hands the next method a reference receiver
            m if m.starts_with("m:") && !is_async && (wraps.is_empty() || (matches!(wraps.as_slice(), ['M' | 'S']) && !candor_report::env::switch("CANDOR_R1627_OFF") && !candor_report::env::switch("CANDOR_R1627_REF_OFF"))) => {
                let RetTy::Dep(q) = &ty else { return None };
                match wraps.first() {
                    Some('M') => form = 'm',
                    Some('S') => form = 'r',
                    _ => {}
                }
                // SOUNDNESS R1627 — the form rule (R1200's probe) refuses an owned receiver and a `&self` method
                // (`ExponentialBackoffBuilder::new().build()`); where the consumer provably has no trait in scope that
                // declares the method, the inherent one is the one that runs whatever the form
                let mut r = sigs.ret_of(dir, cr, cr_real, q, Some((&m[2..], form)));
                if r.is_none() && !candor_report::env::switch("CANDOR_R1627_PROBE_OFF") && sigs.probe_clear(dir, &m[2..], deps, renames).is_ok() {
                    r = sigs.ret_of(dir, cr, cr_real, q, Some((&m[2..], 'a')));
                }
                let r = r?;
                (is_async, wraps, ty, form) = (r.is_async, r.wraps, r.ty, 'o');
            }
            _ => return None,
        }
    }
    if is_async {
        return None;
    }
    match ty {
        RetTy::Dep(q) if wraps.is_empty() || (matches!(wraps.as_slice(), ['M' | 'S']) && !candor_report::env::switch("CANDOR_R1627_OFF") && !candor_report::env::switch("CANDOR_R1627_REF_OFF")) => {
            Some(R1578::Typed { cr: cr.to_string(), method, ty: q })
        }
        _ => Some(R1578::Std),
    }
}

/// ⟨typeSurface.returns⟩ THE PRODUCER. `{crate}#{fn qual}` -> `{crate}#{type qual}`, both FULLY
/// QUALIFIED, for the factory functions whose returned type has at least one non-pure member here.
///
/// Three things this has to get right, each one a confirmed defect of the reverted attempt:
///
/// 1. **The type id is FULLY QUALIFIED, and the report's own hashes are the authority for it.** The
///    attempt published `{crate}#{leaf}`, so `sync::Client` and `mock::Client` were one string and a
///    PURE `mock_client()` factory let `sync::Client::send`'s Net be charged to a caller that cannot
///    reach it. Here the id is the entry hash's PREFIX (`cr#conn::Pool::acquire` -> `cr#conn::Pool`),
///    which is exactly the shape the consumer appends `::<method>` to and asks `by_key` for.
/// 2. **The MODULE is not the type.** An earlier bug took the segment right after `#`, which on any
///    modular crate is the module — invisible on a flat fixture, and it made the rung near-inert. The
///    owning type is the segment BEFORE the method; a lowercase one is a MODULE holding a free fn
///    (`cr#util::helper`), never a type, and is skipped.
/// 3. **A leaf resolving to more than one non-pure type is REFUSED, not picked.** That is defect 1's
///    exact shape and the only place a wrong answer here could fabricate.
///
/// BOUNDED to types with a non-pure member: if the returned type has no effectful and no
/// `Unknown`-carrying member in this report, typing the receiver changes no answer — the lookup it
/// enables succeeds and yields pure, which is what the consumer's silence already yields.
fn build_type_surface(
    crate_name: &str,
    fns: &[FnInfo],
    entries: &[ReportEntry],
) -> candor_report::TypeSurface {
    // The FULL quals of types carrying a non-pure member, straight off the report's own hashes.
    let mut nonpure: BTreeSet<&str> = BTreeSet::new();
    for e in entries {
        if e.inferred.is_empty() {
            continue;
        }
        let Some((_, rest)) = e.hash.split_once('#') else { continue };
        let Some((ty_qual, _method)) = rest.rsplit_once("::") else { continue };
        let leaf = ty_qual.rsplit("::").next().unwrap_or(ty_qual);
        // A TYPE, not a module. `cr#util::helper` is a free fn in module `util`; taking `util` as the
        // owning type is the keying mismatch that made the whole rung inert once before, one layer up.
        if !leaf.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        nonpure.insert(ty_qual);
    }
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut near_misses = 0usize;
    let mut collided: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in fns {
        let Some(tp0) = &f.ret_bound_type else { continue };
        // SOUNDNESS R1579 (R1657 stage 2) — a return written at a RE-EXPORT spelling (`-> crate::nats::Context` over
        // `pub use context::Context`) names the definition the re-export table names, when it names exactly one.
        let tp_norm: Option<String> = (!nonpure.contains(tp0.as_str()))
            .then(|| crate::lang::reexport_lookup(tp0))
            .flatten()
            .filter(|v| !v.contains(crate::decls::ALIAS_ALT_SEP))
            .and_then(|v| v.strip_prefix("crate::").map(str::to_string));
        let tp = tp_norm.as_ref().unwrap_or(tp0);
        // EXACT match on the full qual, and that is the whole disambiguation. Both sides are now
        // module-qualified in this crate's own namespace — `ret_bound_type` resolves a bare name
        // against its DECLARING module — so `mock::Client` and `sync::Client` are different strings
        // and neither can stand in for the other. A suffix/leaf match here is defect 1.
        if !nonpure.contains(tp.as_str()) {
            if nonpure.iter().any(|q| q.rsplit("::").next() == tp.rsplit("::").next()) {
                near_misses += 1; // same type LEAF, different module — the case that MUST not resolve
                if candor_report::env::trace("CANDOR_TYPESURFACE_DEBUG") { eprintln!("TSNEAR\t{}\t{tp}", f.qual); }
            }
            continue;
        }
        let key = format!("{crate_name}#{}", f.qual);
        let val = format!("{crate_name}#{tp}");
        // Two FnInfos under one qual (a `#[cfg]`-duplicated impl) publishing DIFFERENT types: drop the
        // key rather than let walk order decide, the same never-guess rule `by_key` applies.
        if collided.contains(&key) {
            continue;
        }
        match map.get(&key) {
            Some(prev) if *prev != val => {
                map.remove(&key);
                collided.insert(key);
            }
            Some(_) => {}
            None => {
                map.insert(key, val);
            }
        }
    }
    // The COUNTS are the diagnostic, not the output — a bound that admits nothing on a real modular
    // crate is a keying bug, and it is invisible in a diff (standing bar item 8).
    if candor_report::env::trace("CANDOR_TYPESURFACE_DEBUG") {
        let bound: usize = fns.iter().filter(|f| f.ret_bound_type.is_some()).count();
        eprintln!(
            "TYPESURFACE {crate_name}: fns={} bound_returns={bound} nonpure_type_quals={} \
             published={} leaf_near_misses={near_misses}",
            fns.len(), nonpure.len(), map.len()
        );
    }
    candor_report::TypeSurface { returns: map, ..Default::default() }
}

/// One crate scan, end to end (parse -> passes -> report -> receipt -> policy gate). Returns the
/// process exit code. Factored out of `main` so `--deps` can scan a dependency tree IN-PROCESS —
/// candor-scan's own self-gate (`deny Exec`) rightly forbids the spawn-yourself shortcut.
pub(crate) fn scan_one(dir: &str, opts: ScanOpts, run: &crate::gate::RunToken)
    -> (i32, Option<String>) {
    let ScanOpts { prefix, want_json, include_tests, policy: policy_path, baseline: baseline_value, quiet, ws_member, deps_idx, peek_excluded } = opts;
    let root = Path::new(dir);
    let crate_name = read_crate_name(root).unwrap_or_else(|| "crate".to_string());
    // Install this crate's cfg-feature picture (active = default closure, declared = all). A
    // `#[cfg(feature="X")]` compiled OUT under the default build is then skipped, so its effects don't
    // count as the crate's behaviour (winnow's debug-trace `std::env::var` fabricated Env). Set before the
    // parallel Pass B reads it; scan_one runs sequentially per workspace member, so members don't race.
    // Read the manifest HERE (the scan I/O layer); `lang::parse_features` stays a pure text pass (an
    // unreadable/absent Cargo.toml → empty string → no features, the same result as before the hoist).
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    set_cfg_features(parse_features(&cargo_toml));
    // SOUNDNESS R1495 — empty for Pass A (a cached Pass A fact cannot depend on it); installed before Pass B.
    *crate::lang::dyn_alias_cell().write().unwrap() = Default::default();
    // SOUNDNESS R1657 stage 2 — likewise empty for Pass A; installed after the merge (`decls::build_reexport_names`).
    *crate::lang::reexport_names_cell().write().unwrap() = Default::default();
    // SOUNDNESS R1697/R1702 — and the module namespaces, so a workspace member's Pass A never reads the previous member's.
    *crate::lang::ns_cell().write().unwrap() = None;

    // Parse every in-scope .rs file ONCE (syn parses are reused across both passes below). The walk +
    // path-shape filters run SEQUENTIALLY (cheap directory traversal, and the filter set is the report's
    // scope contract); the per-file READ + `syn::parse_file` — profiled at ~77% parse + ~19% I/O of
    // wall-clock, and embarrassingly parallel since each file parses independently — is fanned out across
    // cores with rayon below. ORDER IS PRESERVED: paths are collected in walk order, `par_iter().collect()`
    // writes each result back at its own index (completion order is irrelevant), and the post-filter of
    // read/parse failures keeps the survivors' relative order — so `parsed` is byte-identical to the old
    // sequential push, and the report's fn order (which derives from it) does not move.
    let mut paths: Vec<(std::path::PathBuf, String)> = Vec::new();
    // ⟨0.29⟩ THE SCOPE, RECORDED AS IT IS DECIDED. Each `continue` below is a deliberate exclusion with a
    // written rationale — and that is exactly why none of them was measured: a limitation in a comment
    // reads as considered. Recording the decision at the point it is MADE is the only place it cannot
    // drift from what actually happened; deriving it afterwards would be a second walk that could
    // disagree with this one.
    let mut excluded: Vec<(String, &'static str)> = Vec::new();
    // R459 — the mod-site memo, owned by this walk. Parent module files are parsed once each and reused
    // for every file they declare, which is what keeps a per-FILE question from costing a per-file parse.
    let mut modsite = crate::decls::ModSiteCache::default();
    // SOUNDNESS R977 — the admitted files that sit under a module compiled out of the default build.
    let mut cfg_off_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    // When peeking, every `continue` below becomes a KEEP and every keep becomes a skip — one flag, one
    // walk, so the two file sets are exact complements and no file can fall between them.
    let peeking = peek_excluded;

    for entry in crate::deps::ordered_walk(root)
        .into_iter()
        // A nested directory carrying its own Cargo.toml is a DIFFERENT package (Cargo's own
        // semantics) — folding its files into this crate collides same-named fns across packages
        // and cross-wires the merged call graph (the repo-root self-scan merged 194 eval-fixture
        // `main`s into one unit). It gets its own scan: workspace member, --deps, or directly.
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            // Prune build/tooling dirs by NAME first — cheap, and it skips DESCENT into huge `target/`
            // and `.git/` trees (the dominant cost on a warm checkout) before the per-dir Cargo.toml
            // stat. A name starting with `.` is a hidden tooling dir (`.git`/`.github`/`.cargo`/…).
            let name = e.file_name().to_str().unwrap_or("");
            if name == "target" || (name.starts_with('.') && name != "." && name != "..") {
                return false;
            }
            // A nested dir carrying its own Cargo.toml is a DIFFERENT package (Cargo's own semantics):
            // folding its files into this crate collides same-named fns across packages and cross-wires
            // the merged call graph. It gets its own scan (workspace member, --deps, or directly).
            !e.path().join("Cargo.toml").is_file()
        })
        .filter_map(Result::ok)
    {
        let p = entry.path();
        if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        // All path-shape filters run on the path RELATIVE to the scan root — an absolute prefix can itself
        // contain `target`/`.cargo`/… (a vendored crate lives under `~/.cargo/registry/...`), which must
        // not trip them.
        let rel = p.strip_prefix(root).unwrap_or(p);
        // target/ build artifacts; hidden dirs (`.git`, `.github`, `.cargo`, …) holding tooling/CI scripts,
        // not library code (smol_str's `.github/ci.rs` otherwise reported a phantom `Exec`).
        if rel.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| s == "target" || (s.starts_with('.') && s != "." && s != ".."))
        }) {
            excluded.push((rel.to_string_lossy().into_owned(), "build-output"));
            if peeking { paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned())); }
            continue;
        }
        // The Cargo BUILD SCRIPT is `<crate-root>/build.rs` — it runs at COMPILE time (ring's build.rs
        // execs nasm), never the crate's runtime behaviour, so skip it. But ONLY at the root: a nested
        // `src/build.rs` is an ordinary source module that merely shares the name (git2's `src/build.rs`
        // is `RepoBuilder` — the whole clone/fetch NETWORK surface), and dropping it silently under-reports
        // (an A/B found `git2::Repository::clone` reporting no `Net` because its module had vanished).
        if is_build_script(rel) {
            excluded.push((rel.to_string_lossy().into_owned(), "build-script"));
            if peeking { paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned())); }
            continue;
        }
        // Cargo's non-library compilation targets (tests/, benches/, examples/) — and the common nonstandard
        // singular `test/` tree (e.g. nix) — describe what the crate's HARNESS does (spawn a server, read
        // fixtures, seed RNG), not what the crate itself does. Scanning them conflates the two (redis's bench
        // harness alone showed Exec/Net/Fs/Env/Rand on 200+ fns). Skip by default; `--include-tests` keeps them.
        if !include_tests
            && rel.components().any(|c| {
                matches!(
                    c.as_os_str().to_str(),
                    Some("tests") | Some("test") | Some("benches") | Some("examples")
                )
            })
        {
            excluded.push((rel.to_string_lossy().into_owned(), "non-library-target"));
            if peeking { paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned())); }
            continue;
        }
        // A `tests.rs` / `*_tests.rs` / `*_test.rs` FILE module is usually a `#[cfg(test)]` tree whose
        // effects (a seeded RNG, a temp file) would be mis-read as the crate's — base64's
        // `engine/tests.rs` otherwise reported a phantom `Rand`. This used to skip on the NAME alone,
        // and its own reason text named the hazard it then walked into: *"test-ness is declared at the
        // `mod` site, invisible when walking files."* It is not invisible — R457 — so the stem now only
        // selects a CANDIDATE and `test_stem_file_is_test_module` looks for the evidence (the file's own
        // `#![cfg(test)]`, or the declaring `mod`'s `#[cfg]`), keeping today's answer only where no
        // declaration can be found. A production file that merely LOOKS like a test module — `regex-cli`'s
        // `cmd/compile_test.rs`, the actual source of that binary — is now scanned.
        // SOUNDNESS R459 — **AND THE MIRROR OF THAT, WHICH IS A FABRICATION AND SO RANKS HIGHER.** The
        // block above still read the FILENAME as the question and only consulted the mod site for files
        // whose stem already looked like a test. The same heuristic fails in the other direction:
        // `#[cfg(test)] mod mock;` over `src/mock.rs` (hyper), `mod mocks;` (tokio), `mod test_helpers;`
        // (tower-http, diesel, axum 0.7) — none of those filenames matches the convention, so test code
        // was scanned as production and charged into the crate's report. The mod-site verdict now decides
        // for EVERY file and the filename rule is the fallback for the one case it cannot answer:
        //
        //     Some(true)  -> exclude, on evidence
        //     Some(false) -> scan, on evidence (this is R457's 17 production files)
        //     None        -> no declaration reachable: keep today's answer, which IS the filename rule
        //
        // So the change can only move a file whose declaration was found, and `None` reproduces the old
        // behaviour exactly — including R457's conservative EXCLUDE for an undeclared stem candidate.
        if !include_tests {
            let verdict = crate::decls::file_module_test_verdict(root, rel, &mut modsite);
            let stem_says_test =
                p.file_stem().and_then(|s| s.to_str()).is_some_and(is_test_file_stem);
            // REACH, and the ONLY marker worth counting here: `R459PROBE` fires once per file, so it
            // measures the walk, not the change. This one fires exactly when the EVIDENCE overrode the
            // NAME — in either direction — which is the population that can move a row. Counted by
            // `bin/corpus-ab.py --mark R459MOVE`; the tag shares no prefix with `R459PROBE`, because a
            // substring mark that matches both reports the whole walk as the change's reach (measured
            // on R460, where `R460MASK` swallowed `R460MASKNAMING` and read 409 for 217).
            if verdict.is_some_and(|v| v != stem_says_test)
                && candor_report::env::trace("CANDOR_TESTMOD_DEBUG")
            {
                eprintln!("R459MOVE {} {}",
                          if verdict == Some(true) { "now-excluded" } else { "now-scanned" },
                          rel.display());
            }
            if verdict.unwrap_or(stem_says_test) {
                excluded.push((rel.to_string_lossy().into_owned(), "test-module"));
                if peeking { paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned())); }
                continue;
            }
        }
        if peeking {
            continue;   // an in-scope file is exactly what the peek is NOT about
        }
        // SOUNDNESS R977 — is this file a module the default build does not compile? Then its walks run
        // inside a `CfgOffScope` (Pass A and Pass B below), so its nested feature arms are all kept.
        if crate::decls::file_module_cfg_off(root, rel, &mut modsite) {
            if crate::lang::reach_debug() {
                eprintln!("R977CFGOFF {}", rel.display());
            }
            cfg_off_files.insert(rel.to_string_lossy().into_owned());
        }
        paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned()));
    }

    // ⟨0.31⟩ THE WALK-ADMITTED FILE COUNT, captured HERE and under its own name.
    //
    // `paths` is SHADOWED ~450 lines below by an unrelated `HashMap<String, BTreeSet<String>>` of Fs path
    // literals, and the unevaluable-target refusal lives past that point. Reading `paths.is_empty()` there
    // asks "did this crate record any Fs path literals", which is true of every clean crate — measured:
    // it made a normal crate with a real `src/lib.rs` exit 2, the third attempt at this fix failing the
    // same regression row as the second. Same identifier, two meanings, 450 lines apart.
    let walk_admitted_files = paths.len();

    // ── PARSE + Pass A + Pass B, with an OPTIONAL per-file cache (`--incremental`) ──────────────────
    // The non-incremental path is the original: parallel parse every file, run Pass A then Pass B over
    // all. The incremental path reuses an unchanged file's cached Pass A decls (skipping its parse) and,
    // when the merged decl index is unchanged, its cached Pass B FnInfos too — producing a byte-identical
    // assembled FnInfo set (the merges below replay the original walk-order accumulation exactly). See
    // the cache section above for the soundness argument.
    use rayon::prelude::*;
    let incremental = INCREMENTAL.with(|c| c.get());
    let schema = cache_schema(include_tests);
    let cache_dir = Path::new(dir).join(".candor").join("cache");
    let cache_path = cache_dir.join("scan-cache.json");

    // Load the SINGLE consolidated cache file (`rel -> FileCache`) in one read+deserialize — far cheaper
    // than 1 open per source file. A cache whose schema doesn't match this binary is discarded wholesale.
    let mut prior: HashMap<String, FileCache> = if incremental {
        std::fs::read(&cache_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<ScanCache>(&b).ok())
            .filter(|c| c.schema == schema)
            .map(|c| c.files)
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    // CONTENT HASHES (cheap parallel reads, no parse). The cached entry for a file is reusable iff its
    // stored content_hash matches the bytes on disk now.
    // SOUNDNESS R145 — the files the walk admitted, canonical, so an `include!` of one of them is left to
    // the walk (see `lang::splice_includes`), and the environment the splice resolves paths against.
    let walked_canon: std::collections::HashSet<std::path::PathBuf> =
        paths.par_iter().filter_map(|(p, _)| p.canonicalize().ok()).collect();
    let include_env = crate::lang::IncludeEnv { root, manifest_dir: root, walked: &walked_canon, include_tests };
    // SOUNDNESS R1004 (the cross-file residual) — the crate's `macro_rules!` definitions, so an item-position
    // invocation of a macro defined in ANOTHER file (`#[macro_use] mod mac;`) is expanded like a same-file
    // one (`mbe::CrateMacros`). Read from every walked file that names `macro_rules`, cached or not: a warm
    // file's expansion depends on these bytes, so their digest joins every file's key below.
    let xmacros: crate::mbe::CrateMacros = crate::mbe::merge_crate_macros(
        paths
            .par_iter()
            .filter_map(|(p, _)| {
                let text = std::fs::read_to_string(p).ok()?;
                if !text.contains("macro_rules") {
                    return None;
                }
                let (file, _) = crate::lang::parse_file_2015_tolerant(&text)?;
                Some(crate::mbe::file_macro_defs(&file.items))
            })
            .collect::<Vec<_>>(),
    );
    let xmacros_key = (!xmacros.is_empty()).then(|| crate::mbe::crate_macros_digest(&xmacros));
    // SOUNDNESS R1154 — the package's own library name, as its bins/examples/tests spell it (`mylib::f()`). Not
    // when a manifest dependency carries the same name: then the head is ambiguous and today's reading stands.
    let self_lib: Option<String> = crate::deps::self_lib_name(root)
        .filter(|l| !crate::deps::cargo_deps(dir).0.contains(l));

    // A file whose last parse READ other files (`include!` targets) is keyed on their bytes too — see
    // `include_closure_hash`; with no targets the key is the plain content hash, exactly as before.
    let hashes: Vec<(String, String)> = paths
        .par_iter()
        .map(|(p, rel)| {
            let own = std::fs::read(p).map(|b| fnv1a(&b)).unwrap_or_default();
            let key = match prior.get(rel) {
                Some(fc) if !fc.decls.include_targets.is_empty() => {
                    crate::lang::include_closure_hash(&own, &fc.decls.include_targets, &walked_canon)
                }
                _ => own,
            };
            // SOUNDNESS R977 — the cfg-off verdict comes from ANOTHER file (the declaring `mod`), so it is
            // part of what this file's cached decls and FnInfos were derived under: fold it into the key.
            let key = if cfg_off_files.contains(rel) { format!("{key}+cfgoff") } else { key };
            let key = match &xmacros_key { Some(x) => format!("{key}+xmac{x}"), None => key };
            (rel.clone(), key)
        })
        .collect();
    let per_file: Vec<(String, String, Option<FileCache>)> = hashes
        .into_iter()
        .map(|(rel, content_hash)| {
            let cached = prior
                .remove(&rel)
                .filter(|fc| fc.content_hash == content_hash);
            (rel, content_hash, cached)
        })
        .collect();

    // ROUND 1 PARSE (parallel): every file whose Pass A decls are NOT validly cached. A read/parse
    // failure yields `None` (the original `else { continue }`), so its slot carries no parsed file and
    // contributes nothing — identical to before.
    // Each entry is `Option<(SendFile, locs)>`: the `locs` are this file's `file:line:col`s in walk order,
    // resolved HERE on the parse worker because proc-macro2's span line/col only resolves against the
    // parsing thread's source map (see `fn_locs`/`SendFile`). They ride alongside the moved file so Pass B
    // (single-threaded) can zip them onto each FnInfo without re-resolving a now-dead span.
    // Each parse also returns the `include!` targets it consulted (R145) — the fresh cache key needs them.
    let round1: Vec<Option<(ParsedFile, Vec<String>)>> = per_file
        .par_iter()
        .map(|(rel, _, cached)| {
            if cached.is_some() {
                return None; // decls reusable from cache — defer the parse (it may not be needed at all)
            }
            let p = &paths.iter().find(|(_, r)| r == rel)?.0;
            let text = std::fs::read_to_string(p).ok()?;
            // SOUNDNESS R308 — see `parse_file_2015_tolerant`: a file `syn` rejects gets ONE retry with
            // Rust-2015 bare closure-trait objects normalised, because one elided `dyn` currently drops
            // every function in the file.
            let (mut file, _relaxed) = crate::lang::parse_file_2015_tolerant(&text)?;
            // SOUNDNESS R145 — splice readable `include!` text in place, tag the unreadable rest. Done
            // HERE, on the parsing thread, because the spliced items' spans resolve only against this
            // thread's source map (see `fn_locs`).
            let mut read = Vec::new();
            if text.contains("include!") {
                let dir = p.parent().unwrap_or(root);
                crate::lang::splice_includes(&mut file.items, dir, &include_env, &mut Vec::new(), &mut read);
            }
            if text.contains("pin_project") || text.contains("link!") {
                crate::lang::splice_pin_project(&mut file.items); // R988, R960
            }
            if text.contains("macro_rules") || (!xmacros.is_empty() && text.contains('!')) {
                crate::mbe::splice_local_macros(&mut file.items, &xmacros); // R1004
            }
            crate::mbe::splice_derived_comparisons(&mut file.items); // R1146 (ungated: an `include!` target may hold the derive)
            if text.contains("DeriveEntity") {
                crate::mbe::splice_sea_orm_entities(&mut file.items); // R1605
            }
            let mut locs = Vec::new();
            fn_locs(&file.items, rel, include_tests, &mut locs);
            // SAFETY: see `SendFile` — freshly parsed, uniquely owned, moved once, then single-threaded.
            Some(((SendFile(file), locs), read))
        })
        .collect();

    // DISCLOSE files that failed to read/parse (no cache AND round-1 None): their effects are NOT in
    // the report. A silent skip violates "never silently pure" — the query side already discloses an
    // unparseable REPORT; mirror it for unparseable SOURCE (adversarial review).
    let unparsed: Vec<&str> = per_file
        .iter()
        .zip(&round1)
        .filter(|(pf, parsed)| pf.2.is_none() && parsed.is_none())
        .map(|(pf, _)| pf.0.as_str())
        .collect();
    if !unparsed.is_empty() {
        let shown = unparsed.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = if unparsed.len() > 8 { format!(" + {} more", unparsed.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: {} source file(s) failed to read/parse — effects in them are NOT in this report (re-check the source): {shown}{more}",
            unparsed.len()
        );
    }
    // Remember whether ANY in-scope source failed to parse: the policy gate below must FAIL non-zero
    // when a policy is configured AND analysis was incomplete — a gateless-green over unanalyzed code
    // is a missed-effect = false-pure hole. (`unparsed` borrows `per_file`, consumed below; keep a flag.)
    // `mut` because a file can also fail LATER, in the pass-B walk: a parser abort is contained per file
    // (see the `catch_unwind` below) and must reach this flag, or a configured gate would evaluate over a
    // file whose effects are simply absent and call it clean — the false-pure this flag exists to prevent.
    let mut had_parse_failure = !unparsed.is_empty();
    // ⟨Gap 2⟩ Also carry the unparsed set into the REPORT (owned, so it survives `per_file`'s consumption):
    // the stderr warning above is invisible to a machine reading `--json`, so a bare report looked complete.
    // The gated path still exits 2 with no verdict (SPEC §3.3.1); this is the bare-report disclosure.
    let mut unanalyzed_units: Vec<candor_report::UnanalyzedUnit> = unparsed
        .iter()
        .map(|p| candor_report::UnanalyzedUnit { path: p.to_string(), reason: "source failed to read/parse".into() })
        .collect();

    // Per-file Pass A decls (cache or fresh) + a place to hold a parsed file for Pass B. A file dropped
    // by a read/parse failure (no cache AND round-1 parse failed) is excluded entirely, preserving the
    // original survivor set + walk order.
    let mut decls_per_file: Vec<(String, String, FileDecls)> = Vec::new(); // (rel, content_hash, decls)
    let mut parsed_files: HashMap<String, syn::File> = HashMap::new();     // rel -> parsed (round 1)
    let mut parsed_locs: HashMap<String, Vec<String>> = HashMap::new();    // rel -> per-fn loc (walk order)
    // rel -> (decl_index_hash, fninfos) for entries whose FnInfos are REUSABLE. An entry carrying
    // `FileCache::aborted` never lands here — see the `aborted` arm below.
    let mut cached_fninfos: HashMap<String, (String, Vec<FnInfo>)> = HashMap::new();
    // Files whose on-disk entry was already valid for BOTH content + the decl index it recorded — no
    // re-write needed unless the merged index moves (checked after the digest). Lets a no-op / body-only
    // re-scan skip rewriting the whole cache dir (the dominant cost when nothing changed).
    let mut disk_decl_hash: HashMap<String, String> = HashMap::new();
    for ((rel, ch, cached), r1) in per_file.into_iter().zip(round1) {
        match cached {
            Some(fc) => {
                // The DECLS are reusable either way: they were derived from a successful parse of these
                // exact bytes, before the walk that may have aborted. The FnInfos are conditionally
                // reusable (the decl-index check, after the digest) — unless the entry is POISONED.
                decls_per_file.push((rel.clone(), ch, fc.decls));
                if fc.aborted.is_some() {
                    // A CACHED ABORT IS A MARKER THAT THESE FNINFOS WERE NEVER DERIVED, NOT AN ANSWER TO
                    // REPLAY. `39bbc8b` persisted the abort so a warm run could not read `fninfos: []`
                    // as "analysed, no functions" (the false all-clear); it then replayed the disclosure
                    // off a content-hash + decl-index match, on the argument that those two are exactly
                    // the conditions under which reusing the FnInfos is sound. That argument does not
                    // hold, because the abort is not a function of either. `4f7b704` established the
                    // mechanism: proc-macro2's fallback `Span` indexes a THREAD-LOCAL source map, and
                    // whether a moved span lands past the walking thread's map depends on how much each
                    // rayon worker happened to parse — i.e. on the rest of the crate and on the work
                    // split, not on this file's bytes. A one-off therefore LATCHED, and `--incremental`
                    // is exactly the mode nobody re-runs from cold: a permanent spurious `unanalyzed`
                    // and a gate that can never go green (the mirror of the sin, but still a cached
                    // wrong answer).
                    //
                    // So: keep neither the FnInfos nor the `disk_decl_hash` shortcut. Every downstream
                    // gate reads `cached_fninfos`, so dropping the entry HERE is the single decision —
                    // the round-2 re-parse picks the file up (its `cached_fninfos` miss reads as stale),
                    // pass B walks it, and it either aborts again and discloses by the same cold path,
                    // byte for byte, or it produces the answer it always owed and the marker clears.
                    continue;
                }
                disk_decl_hash.insert(rel.clone(), fc.decl_index_hash.clone());
                cached_fninfos.insert(rel, (fc.decl_index_hash, fc.fninfos));
            }
            None => {
                // A freshly-parsed file (or a parse failure → skip the file entirely, as before).
                let Some(((sf, locs), read)) = r1 else { continue };
                let _cfg_off = crate::lang::CfgOffScope::enter(cfg_off_files.contains(&rel)); // R977
                let mut fd = file_decls(&sf.0.items, include_tests, Path::new(&rel));
                // R145 — key the entry on what THIS parse read, not on what the previous one did.
                let ch = if read.is_empty() {
                    ch
                } else {
                    let own = paths
                        .iter()
                        .find(|(_, r)| r == &rel)
                        .and_then(|(p, _)| std::fs::read(p).ok())
                        .map(|b| fnv1a(&b))
                        .unwrap_or_default();
                    let k = crate::lang::include_closure_hash(&own, &read, &walked_canon);
                    // R977 — the same suffix the `hashes` key carries, or a warm run never matches it.
                    if cfg_off_files.contains(&rel) { format!("{k}+cfgoff") } else { k }
                };
                fd.include_targets = read;
                decls_per_file.push((rel.clone(), ch, fd));
                parsed_locs.insert(rel.clone(), locs);
                parsed_files.insert(rel, sf.0);
            }
        }
    }

    // Pass A MERGE — replay the original accumulation in WALK ORDER over the per-file decls, so the
    // crate-wide index is byte-identical to the old sequential `collect_decls` loop.
    let mut merged = MergedDecls::default();
    for (_, _, fd) in &decls_per_file {
        merge_decls(&mut merged, fd);
    }
    // SOUNDNESS R1483 — an alias CHAIN (`type A1 = A0; type A0 = R;`) recorded one hop per entry, so `A1` expanded
    // to `A0` and named nothing. Chased to its end here, once the map is crate-wide; see `chase_alias_chains`.
    let r1483 = !candor_report::env::switch("CANDOR_R1483_OFF");
    // SOUNDNESS R1489 — the crate's nominal type quals, for a MIXED `#[cfg]` twin (an alias in one arm, a struct in the
    // other). See `add_nominal_twin_arms`. Empty under the kill switch, which reproduces the pre-R1489 map exactly.
    let nominal_quals: HashSet<String> = if !candor_report::env::switch("CANDOR_R1489_OFF") {
        decls_per_file
            .iter()
            .flat_map(|(_, _, fd)| fd.ts.types.iter())
            .filter(|(_, kind, _)| kind == crate::typesurf::KIND_VALUE)
            .map(|(q, _, _)| q.clone())
            .collect()
    } else {
        HashSet::new()
    };
    add_nominal_twin_arms(&mut merged.mod_aliases, &nominal_quals);
    if r1483 {
        chase_alias_chains_n(&mut merged.mod_aliases, &nominal_quals);
    }
    // SOUNDNESS R1657 stage 2 — the crate-wide RE-EXPORT TABLE (`lang::reexport_lookup`): every local named re-export,
    // chased to the definition(s) it denotes. Installed BEFORE `alias_expand_decls`, which re-expands field / return /
    // payload types through `expand` and so normalises a type written at its re-export spelling.
    // The crate's MODULE paths: every file's, every module a type or a re-export edge lives in or comes from, and every
    // ancestor of those (an inline module holding only fns is not seen — it can only cost a value-only refusal's absence).
    let r1657s2_mods: HashSet<String> = {
        let mut m: HashSet<String> = HashSet::new();
        let mut add = |p: &str| {
            let mut cur = p;
            while !cur.is_empty() && m.insert(cur.to_string()) {
                cur = cur.rsplit_once("::").map_or("", |(a, _)| a);
            }
        };
        for (rel, _, fd) in &decls_per_file {
            add(&module_path(Path::new(rel)));
            for (q, _, _) in &fd.ts.types {
                add(q.rsplit_once("::").map_or("", |(a, _)| a));
            }
        }
        for e in &merged.reexports {
            add(&e.module);
            for f in &e.from {
                add(f);
            }
        }
        m
    };
    let r1657s2_declared: HashSet<String> = if crate::lang::r1657s2_off() {
        HashSet::new()
    } else {
        decls_per_file
            .iter()
            .flat_map(|(_, _, fd)| fd.ts.types.iter().map(|(q, _, _)| q.clone()))
            .collect()
    };
    let r1657s2_deps = cargo_deps(dir).0;
    // SOUNDNESS R1710 — every declared type qual (the stage-2 set is empty under its own switch).
    let r1710_types: Vec<String> =
        decls_per_file.iter().flat_map(|(_, _, fd)| fd.ts.types.iter().map(|(q, _, _)| q.clone())).collect();
    let install_reexport_names = |merged: &MergedDecls| {
        let t = if crate::lang::r1657s2_off() {
            HashMap::new()
        } else {
            {
                // Exports first, then the private bindings (a name both bind is a `#[cfg]` pair: the union).
                // A re-exported EXTERN CRATE (`pub use super::libc;`, `use super::libc;` under `extern crate libc;`) is the
                // dependency, not a local item: routed through the table, `libc::sendto` became a crate-local path and the
                // classifier's `Net` was lost (pnet_sys, first cut). Dependency and toolchain crate names stay out.
                let edges: Vec<crate::model::Reexport> = merged
                    .reexports
                    .iter()
                    .chain(merged.private_uses.iter())
                    .filter(|e| {
                        // SOUNDNESS R1697/R1702 — a namespace MARKER names a module's bindings, not a re-exported item
                        (e.from.is_empty() && e.alias.starts_with('\u{1}'))
                            || !r1657s2_deps.contains(&e.name) && !matches!(e.name.as_str(), "std" | "core" | "alloc")
                    })
                    .cloned()
                    .collect();
                let ns = (!crate::lang::r1697_off()).then(|| crate::decls::NsIndex::from_edges(&edges, &merged.private_uses, &r1657s2_mods, &r1657s2_deps));
                let t = crate::decls::build_reexport_names(
                    &edges,
                    &merged.mod_aliases,
                    &r1657s2_declared,
                    &r1657s2_mods,
                    &merged.extern_aliases,
                    ns.as_ref(),
                );
                *crate::lang::ns_cell().write().unwrap() = ns;
                t
            }
        };
        let n = t.len();
        *crate::lang::reexport_names_cell().write().unwrap() = t;
        // SOUNDNESS R1710 — which local glob bases may supply a root extern-prelude name (`lang::PRELUDE_SHADOW`); reads the
        // table just installed.
        crate::lang::install_prelude_shadow(
            &merged.extern_aliases,
            merged.mod_places.keys(),
            r1710_types.iter(),
            merged.use_bound.iter(),
            merged.macro_modules.iter(),
        );
        n
    };
    let r1657s2_names = install_reexport_names(&merged);
    // R99 (SHAPE 2) — SECOND MERGE, over decls whose type paths have been re-expanded against the
    // crate-wide alias map that only exists once the first merge is done. Pass A walks each file in
    // isolation, so a field/return/variant type spelled through a module alias (`c: facade::Command`)
    // was recorded as the literal written path and never resolved; the method using it was ABSENT from
    // `functions[]` under every policy form. See `alias_expand_decls` for the measurement and for why
    // the DECLARING file's `modpath` — known only here — is the only sound context to re-expand in.
    //
    // Guarded twice so the ordinary crate pays nothing: no aliases at all, or no file whose decls
    // actually move, and the first merge stands. `decls_per_file` is NEVER mutated — it is what the
    // cache persists, and an entry is keyed by ONE file's content hash, so baking a crate-wide fact
    // into it would survive an edit to the file that supplied the fact.
    // SOUNDNESS R978 — kept past the merge: the field-twin check below must compare the twins' field
    // types AFTER alias expansion, or two twins that both wrote `housekeeper: Option<HouseKeeperArc>`
    // (one alias per module, two different pointees) read as agreeing and the leaf-keyed index kept the
    // last file's pointee for both.
    let mut expanded: Vec<Option<FileDecls>> = Vec::new();
    if !merged.mod_aliases.is_empty() || r1657s2_names > 0 {
        expanded = decls_per_file
            .iter()
            .map(|(rel, _, fd)| {
                alias_expand_decls(fd, &module_path(Path::new(rel)), &merged.mod_aliases, &r1657s2_deps)
            })
            .collect();
        if expanded.iter().any(Option::is_some) {
            merged = MergedDecls::default();
            for ((_, _, fd), ex) in decls_per_file.iter().zip(&expanded) {
                merge_decls(&mut merged, ex.as_ref().unwrap_or(fd));
            }
            add_nominal_twin_arms(&mut merged.mod_aliases, &nominal_quals); // R1489 — likewise
            if r1483 {
                chase_alias_chains_n(&mut merged.mod_aliases, &nominal_quals); // R1483 — the re-merge rebuilt the map from the files
            }
            install_reexport_names(&merged); // R1657 stage 2 — the re-merge rebuilt `mod_aliases`
        }
    }
    // SOUNDNESS R1711 (c) — the obligation-2 implementor index under the crate ROOT's renamed `extern crate X as Y;`. Pass A
    // keys an `impl Y::de::Error for T` written in a file other than the root with the file's own `use` map, which has not
    // seen the root's extern prelude (Pass B seeds it, `lang::seed_extern_prelude`), so the key named `Y#…` while every
    // Pass B site of the same crate (`dispatchesOn`, the `implemented` witness that mints a path reference's key) names
    // `X#…`. Two authorities spelling one trait two ways: serde_json's `de::Error::invalid_type(..)` lost its key (the
    // witness asked `serde_core#…` of an index holding `serde#…`) and its union rows published under `serde#`, an owner
    // no dependency report carries. See `rekey_root_extern_owners`.
    rekey_root_extern_owners(&mut merged);
    // SOUNDNESS R1056 — the slice / array / tuple receiver tails Pass B may type a receiver into. See
    // `lang::NONPATH_RECV_TAILS`; installed here, after the last merge and before any Pass B collection.
    {
        let mut tails: HashSet<String> = merged
            .nonpath_receivers
            .iter()
            .filter_map(|e| e.split('\u{1f}').next())
            .filter(|t2| t2.starts_with('[') || t2.starts_with('(') || t2.starts_with("str::"))
            .map(str::to_string)
            .collect();
        // R1056 (residual) — the members this crate's own `impl Tr for Vec<..>` / `for String` blocks write
        // (R598's `impl_members`: `<tr>\u{1f}<ty>\u{1f}<m>`, `*` = unreadable), which win lookup at the
        // wrapper's own step before a deref to `[T]` / `str` is tried.
        {
            for k in &merged.impl_members {
                let mut p = k.split('\u{1f}');
                let (Some(_), Some(ty), Some(m)) = (p.next(), p.next(), p.next()) else { continue };
                if (ty == "Vec" || ty == "String") && !m.starts_with('!') {
                    tails.insert(crate::lang::deref_owner_marker(ty, m));
                }
                // …and an `impl Tr for str` (a PATH self type, so not a `nonpath_receivers` entry): its unit
                // is `str::m`, the tail a `String` receiver's deref forms.
                if ty == "str" && !m.starts_with('!') && m != "*" {
                    tails.insert(format!("str::{m}"));
                }
            }
        }
        *crate::lang::nonpath_recv_cell().write().unwrap() = tails;
    }
    // SOUNDNESS R1495 — the crate's trait-object aliases (`lang::DYN_ALIASES`), from Pass A's merged facts; a leaf the
    // conflict rule withdrew (`None`) or one that names something else (`""`) is left out.
    {
        let mut leaves: HashMap<String, Vec<String>> = HashMap::new();
        let mut mods: HashSet<String> = HashSet::new();
        for (k, v) in &merged.rets {
            if let Some(n) = k.strip_prefix(crate::lang::DYN_ALIAS_PREFIX) {
                if let Some(v) = v.as_deref().filter(|v| !v.is_empty()) {
                    leaves.insert(n.to_string(), v.split(',').map(str::to_string).collect());
                }
            } else if let Some(m) = k.strip_prefix(crate::lang::LOCAL_MOD_PREFIX) {
                mods.insert(m.to_string());
            }
        }
        *crate::lang::dyn_alias_cell().write().unwrap() = (leaves, mods);
        // …and the two Pass A positions a CROSS-FILE alias cannot reach there (the per-file cache boundary, R177): a
        // struct field and a fn's return written through an alias another file declares (`r: al::B`, `-> al::B`) were
        // recorded as that alias's PATH, which the alias map then expanded to the bare wrapper (`Box`). Read from each
        // file's own facts (before the expansion) and re-filed as the written `Box<dyn Tw>` files them: the field into
        // `trait_fields` / `dyn_trait_fields` (out of `fields`), the return as the `<dyn>` sentinel. A key the merge
        // withdrew (`None`) stays withdrawn.
        if !candor_report::env::switch("CANDOR_R1495_OFF") {
            for (_, _, fd) in &decls_per_file {
                for (st, fm) in &fd.fields {
                    for (f, t) in fm {
                        let Some(l) = crate::lang::dyn_alias_leaves_str(t) else { continue };
                        if crate::lang::reach_debug() || candor_report::env::trace("CANDOR_R1495_INSTR") {
                            eprintln!("R1495FIELD\t{st}.{f}\t{t}"); // §E1 REACH COUNTER
                        }
                        if let Some(m) = merged.fields.get_mut(st) {
                            m.remove(f);
                        }
                        merged.trait_fields.entry(st.clone()).or_default().insert(f.clone(), l.clone());
                        merged.dyn_trait_fields.entry(st.clone()).or_default().insert(f.clone(), l);
                    }
                }
                // `record_return` files a dispatch-object return ONLY as the leaf-keyed sentinel and returns early, so
                // every other key the alias path was filed under (`<retqual>`, the impl-qualified return) is dropped.
                for (k, v) in &fd.rets {
                    let Some(l) = v.as_deref().and_then(crate::lang::dyn_alias_leaves_str) else { continue };
                    if !matches!(merged.rets.get(k), Some(Some(_))) {
                        continue;
                    }
                    if crate::lang::reach_debug() || candor_report::env::trace("CANDOR_R1495_INSTR") {
                        eprintln!("R1495RET\t{k}\t{}", v.as_deref().unwrap_or("")); // §E1 REACH COUNTER
                    }
                    if k.starts_with('<') {
                        merged.rets.remove(k);
                    } else {
                        merged.rets.insert(k.clone(), Some(crate::model::ret_dyn_encode(&l)));
                    }
                }
            }
        }
    }
    // SOUNDNESS R529c — JOIN THE BLOCK-LOCAL STRUCTS' FIELDS. A struct declared inside a fn body was in no
    // `fields` entry, so `h.c.go()` through it typed to nothing and the caller read PURE over a module-level
    // `Inner::go` that writes. The entries are keyed `<body-item>H` (R106's sentinel — the spelling the body's
    // own receivers type as), so they can never meet a module-level `H`: the leaf collision the row named as
    // the reason not to fix it is absent by construction. What remains is TWO block-local `H`s in the crate,
    // which share the sentinel key; a key more than one declaration claims is left out (unchanged from
    // before — the residual), never merged. Run after the R99 re-merge, which rebuilds `merged` from scratch.
    {
        let mut claims: HashMap<&str, usize> = HashMap::new();
        for (_, _, fd) in &decls_per_file {
            for k in fd.block_fields.keys() {
                *claims.entry(k.as_str()).or_default() += 1;
            }
        }
        type FieldMap = HashMap<String, String>;
        let mut joined: Vec<(String, FieldMap, Option<FieldMap>)> = Vec::new();
        for (_, _, fd) in &decls_per_file {
            for (k, m) in &fd.block_fields {
                if claims.get(k.as_str()) == Some(&1) {
                    joined.push((k.clone(), m.clone(), fd.block_field_elem.get(k).cloned()));
                }
            }
        }
        for (k, m, e) in joined {
            if candor_report::env::trace("CANDOR_R529_INSTR") {
                eprintln!("R529CJOIN\t{k}"); // §E1 REACH PROBE
            }
            merged.fields.insert(k.clone(), m);
            if let Some(e) = e {
                merged.field_elem.insert(k, e);
            }
        }
    }
    // SOUNDNESS R476 — the crate-wide half of the impl-bound-generic-field join, run BEFORE the digest
    // so the hash is taken over the index resolution actually reads (it is a pure function of the
    // merged map either way, but hashing the pre-image would be hashing something no consumer sees).
    // Must come after the R99 second merge above, which rebuilds `merged` from scratch.
    // R478 joins the ELEMENT route in the same pass and from the SAME bounds half —
    // `field_elem_trait` is `trait_fields`' parallel index and carries the same pending key space.
    // VEIN A — `merge_decls` lets two crate-anchored spellings of one return-type LEAF agree (rustix's
    // backends' `ret_owned_fd`, both the one re-exported `OwnedFd`). That is sound only when the leaf
    // names ONE local type. Where the crate declares the leaf at ≥2 module paths the spellings are
    // different types and the merge picked one: cap-std's `fs::Dir::read_dir -> fs::ReadDir` and
    // `fs_utf8::Dir::read_dir -> fs_utf8::ReadDir` typed an unrelated `cap_primitives::fs::read_dir(..)
    // .map(..)` as `fs_utf8::ReadDir::map`, which named nothing and retired the untyped hedge. Withdraw
    // exactly those, recording both candidates as an ordinary conflict would.
    {
        let mut type_quals: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for (_, _, fd) in &decls_per_file {
            for (q, _, _) in &fd.ts.types {
                type_quals.entry(q.rsplit("::").next().unwrap_or(q)).or_default().insert(q.as_str());
            }
        }
        let mut withdraw: Vec<(String, BTreeSet<String>)> = Vec::new();
        for (fn_leaf, v) in &merged.rets {
            let Some(v) = v else { continue };
            // SOUNDNESS R1196 — a declared error-type fact is a union (`model::RET_ERR`), never withdrawn.
            if !v.starts_with("crate::") || crate::model::split_amb_ret_key(fn_leaf).is_some() || crate::model::is_ret_err_key(fn_leaf) {
                continue;
            }
            let ty_leaf = v.rsplit("::").next().unwrap_or(v);
            if type_quals.get(ty_leaf).is_none_or(|q| q.len() < 2) {
                continue;
            }
            let cands: BTreeSet<String> = decls_per_file
                .iter()
                .filter_map(|(_, _, fd)| fd.rets.get(fn_leaf).cloned().flatten())
                .collect();
            if cands.len() >= 2 {
                withdraw.push((fn_leaf.clone(), cands));
            }
        }
        for (fn_leaf, cands) in withdraw {
            for c in &cands {
                crate::decls::note_amb_ret(&mut merged.rets, &fn_leaf, c);
            }
            merged.rets.insert(fn_leaf, None);
        }
    }
    crate::decls::resolve_impl_bound_fields(
        &mut merged.trait_fields,
        &mut merged.fields,
        &mut merged.field_elem_trait,
        &mut merged.field_elem,
        &mut merged.unbound_gen_fields,
    );
    // VEIN A — Pass B now reads the manifest's dependency names (`lang::glob_origin`), so a cached
    // FnInfo set is reusable only under the same dependency list: it is folded into the reuse key.
    let manifest_deps = cargo_deps(dir).0;
    let mut decl_index_hash = {
        let mut d: Vec<&str> = manifest_deps.iter().map(String::as_str).collect();
        d.sort_unstable();
        format!("{}#deps:{}", decl_index_digest(&merged), d.join(","))
    };
    // Keep only unambiguous fn-leaf -> return-type / enum-variant-payload mappings (the `None`s drop).
    #[allow(unused_mut)]
    let mut returns: ReturnIndex =
        merged.rets.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    // SOUNDNESS R1659 — a WITHDRAWN accessor position is not an ABSENT one: marker beside it (`model::withdrawn_retgen_key`).
    for (k, v) in &merged.rets {
        if v.is_none() {
            if let Some(rest) = k.strip_prefix("<implretgen>") {
                returns.insert(format!("<implretgenw>{rest}"), "1".to_string());
            }
            // SOUNDNESS R1660 — and a withdrawn impl-qualified RETURN (`<implret>`), read by R1663's guessed-hop floor.
            if let Some(rest) = k.strip_prefix(crate::model::RET_IMPL) {
                if let Some((tl, fl)) = rest.split_once('\u{1f}') {
                    returns.insert(crate::model::withdrawn_impl_ret_key(tl, fl), "1".to_string());
                }
            }
        }
    }
    // SOUNDNESS R1025 (its receiver half) — an `impl` written ON a type alias (`impl LcPtr<EVP_PKEY> { fn
    // agree }`, `type LcPtr<T> = ManagedPointer<*mut T>` in aws-lc-rs) files its (TYPE, method) facts under
    // the ALIAS's leaf, while a receiver typed through the alias — a field, a parameter, a declared return
    // — expands to the TARGET's. The unit side already bridges this (`alias_tails` below, VEIN A); the
    // DECLARED-FACT side did not, so R451's gate (`impl_declared_return`, "the corrected type must declare
    // the method") refused `self.get().agree()` over `fn get(&self) -> &LcPtr<Evp>` and the call vanished.
    // That was a pre-existing silence for a non-generic alias (`type Plain = Managed<u8>`) and became one
    // for a generic alias once R1025 records those. Mirrored under the target's leaf, only where the target
    // has no fact of its own (a supply, never an override), with `alias_tails`' own target filters.
    {
        let mut adds: Vec<(String, String)> = Vec::new();
        for (q, t) in &merged.mod_aliases {
            if t.contains(crate::decls::ALIAS_ALT_SEP) {
                continue;
            }
            let a_leaf = q.rsplit("::").next().unwrap_or(q);
            let t_body = t.strip_prefix("crate::").unwrap_or(t);
            let t_root = t_body.split("::").next().unwrap_or(t_body);
            if t_body.contains('<') || manifest_deps.contains(t_root) || matches!(t_root, "std" | "core" | "alloc") {
                continue;
            }
            let t_leaf = t_body.rsplit("::").next().unwrap_or(t_body);
            if a_leaf == t_leaf || q.starts_with(crate::decls::REDIRECT_MOD_MARK) {
                continue;
            }
            for pre in [crate::model::RET_IMPL_FN, crate::model::RET_IMPL, crate::model::RET_IMPL_SELF, crate::model::RET_IMPL_ELEM] {
                let from = format!("{pre}{a_leaf}\u{1f}");
                for (k, v) in &returns {
                    if let Some(m) = k.strip_prefix(&from) {
                        adds.push((format!("{pre}{t_leaf}\u{1f}{m}"), v.clone()));
                    }
                }
            }
        }
        adds.sort();
        adds.dedup();
        for (k, v) in adds {
            if let std::collections::hash_map::Entry::Vacant(slot) = returns.entry(k) {
                if candor_report::env::trace("CANDOR_R186_DEBUG") {
                    eprintln!("R1025ALIASFACT {}", slot.key()); // §E1 REACH COUNTER
                }
                slot.insert(v);
            }
        }
    }
    // SOUNDNESS R1605 — a crate-local type that implements one of `R1605_TRAITS` (a dependency trait whose receiver-less
    // members build a value the next call runs: sea_orm's `EntityTrait::find`/`insert`/`delete_many`/…) — whether the impl
    // is written or is the skeleton `mbe::splice_sea_orm_entities` adds for a known derive — gets, for each such member it
    // does NOT declare itself, the RETURN the trait's own declaration names in the lock-pinned source (`DepSigs::
    // trait_assoc_rets`). Filed as the declared fact `impl Type { fn m() -> R }` files (`impl_ret_key`), so every typed route
    // the crate's own declarations already feed — the path call, a `let` binding, a builder chain — reads it unchanged and
    // the next call is formed against the dependency's type (`sea_orm::query::select::Select::all`), where the classifier
    // answers. A supply, never an override: a key the crate's declarations already hold (or withdrew as ambiguous) is left
    // alone, as is any member the local type declares. No lock-pinned source: nothing is filed, the value stays untyped, and
    // the `<untyped>` marker the call leaves is disclosed in the call loop instead (`r1605_ty_traits`).
    let r1578_sigs = std::cell::RefCell::new(crate::depsig::DepSigs::default());
    // SOUNDNESS R1743 — the facades `r1743_facade_owner_keys` read to re-key chained entries answer this scan's joins, so
    // they are recorded in its `dependencySources` (SPEC §2 ⟨0.42⟩: a lock-pinned source read to answer a join is named).
    for p in &deps_idx.r1743_read {
        let mut s = r1578_sigs.borrow_mut();
        if s.facade(dir, p, p).is_some() {
            s.note_typed(p);
        }
    }
    let r1605_ty_traits: HashMap<String, Vec<(String, String)>> = {
        let mut m: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for k in &merged.impl_members {
            let mut it = k.splitn(3, '\u{1f}');
            let (Some(tr), Some(ty), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
            let Some(path) = p.strip_prefix('!') else { continue };
            let Some((root, _)) = path.split_once("::") else { continue };
            if !candor_report::env::switch("CANDOR_R1605_OFF")
                && manifest_deps.contains(root)
                && R1605_TRAITS.iter().any(|(c, t)| *c == root && *t == tr)
            {
                m.entry(ty.to_string()).or_default().push((tr.to_string(), root.to_string()));
            }
        }
        m
    };
    for (ty, trs) in &r1605_ty_traits {
        if let Some((tr, cr)) = trs.first() {
            returns.insert(crate::model::r1605_type_key(ty), format!("{cr}\u{1}{tr}"));
        }
    }
    for r in &merged.reexports {
        if r.alias != r.name && r.alias != "*" {
            if let Some((tr, cr)) = r1605_ty_traits.get(&r.name).and_then(|v| v.first()) {
                returns.entry(crate::model::r1605_type_key(&r.alias)).or_insert_with(|| format!("{cr}\u{1}{tr}"));
            }
        }
    }
    {
        let renames = cargo_deps(dir).1;
        let mut seeded: Vec<(String, String, String)> = Vec::new();
        let mut seeded_alias: Vec<(String, String, String)> = Vec::new();
        let mut tys: Vec<(&String, &Vec<(String, String)>)> = r1605_ty_traits.iter().collect();
        tys.sort();
        for (ty, trs) in tys {
            for (tr, alias) in trs {
                let real = renames.get(alias).cloned().unwrap_or_else(|| alias.clone());
                let Some((_, rets)) = r1578_sigs.borrow_mut().trait_assoc_rets(dir, alias, &real, tr) else {
                    if candor_report::env::trace("CANDOR_R1605_INSTR") {
                        eprintln!("R1605NOSRC\t{ty}\t{alias}::{tr}"); // §E1 — no lock-pinned source: the marker discloses
                    }
                    continue;
                };
                for (m, r) in rets {
                    let crate::depsig::RetTy::Dep(q) = &r.ty else { continue };
                    if r.is_async || !r.wraps.is_empty() {
                        continue;
                    }
                    let k = crate::model::impl_ret_key(ty, &m);
                    if returns.contains_key(&k)
                        || merged.rets.contains_key(&k)
                        || returns.contains_key(&crate::model::impl_self_ret_key(ty, &m))
                        || merged.rets.contains_key(&crate::model::impl_fn_key(ty, &m))
                        || !crate::model::impl_does_not_declare(&merged.impl_members, tr, ty, &m)
                    {
                        continue;
                    }
                    if candor_report::env::trace("CANDOR_R1605_INSTR") {
                        eprintln!("R1605SEED\t{ty}::{m}\t{alias}::{q}"); // §E1 REACH COUNTER
                    }
                    r1578_sigs.borrow_mut().note_typed(alias);
                    returns.insert(k, format!("{alias}::{q}"));
                    seeded.push((ty.clone(), m, format!("{alias}::{q}")));
                }
            }
        }
        // …and for a GENERIC bounded by the trait (`E::find()` under `E: EntityTrait`, `Collector::r1605_generic_member`), or
        // by a local trait whose supertraits reach it (`trait Auditable: EntityTrait`), keyed by the bound's leaf. Filed
        // whenever the crate declares the dependency; the members' returns only where the lock-pinned source was read. A
        // crate that declares a trait of the same leaf itself is left alone (its bound names that one).
        if !candor_report::env::switch("CANDOR_R1605_OFF") {
            for (cr, tl) in R1605_TRAITS {
                if !manifest_deps.contains(*cr) || merged.trait_decls.contains_key(*tl) {
                    continue;
                }
                let mut leaves: Vec<String> = vec![tl.to_string()];
                let mut changed = true;
                while changed {
                    changed = false;
                    for (l, t) in merged.trait_decls.iter() {
                        if !leaves.contains(l) && t.count == 1 && t.supertraits.iter().any(|s| leaves.contains(s)) {
                            leaves.push(l.clone());
                            changed = true;
                        }
                    }
                }
                let real = renames.get(*cr).cloned().unwrap_or_else(|| cr.to_string());
                let read = r1578_sigs.borrow_mut().trait_assoc_rets(dir, cr, &real, tl);
                let rets = read.as_ref().map(|r| r.1.clone()).unwrap_or_default();
                for l in &leaves {
                    returns.insert(crate::model::r1605_trait_key(l), cr.to_string());
                    // the members the declaration names, when it was read: a path call to any other name through the bound
                    // (`E::default()`, a supertrait's or a local trait's own member) is not this trait's value
                    if let Some((names, _)) = &read {
                        returns.insert(crate::model::r1605_trait_members_key(l), names.join(","));
                    }
                    for (m, r) in &rets {
                        let crate::depsig::RetTy::Dep(q) = &r.ty else { continue };
                        if r.is_async || !r.wraps.is_empty() || merged.trait_decls.get(l).is_some_and(|t| t.assoc.contains(m) || t.methods.contains(m)) {
                            continue;
                        }
                        if candor_report::env::trace("CANDOR_R1605_INSTR") {
                            eprintln!("R1605SEED\t<{l}>::{m}\t{cr}::{q}"); // §E1 REACH COUNTER
                        }
                        returns.insert(crate::model::r1605_trait_ret_key(l, m), format!("{cr}::{q}"));
                        seeded.push((format!("<{l}>"), m.clone(), format!("{cr}::{q}")));
                    }
                }
            }
        }
        // SOUNDNESS R1606 — for each dependency type those members return, and each type reachable from it by a step, the
        // method steps it provably takes and what each yields (`Collector::nominal_ctor_type` walks a binding's chain
        // through these and nothing else). Bounded: 32 types.
        {
            let mut work: Vec<(String, String)> = seeded
                .iter()
                .filter_map(|(_, _, v)| v.split_once("::").map(|(cr, q)| (cr.to_string(), q.to_string())))
                .collect();
            work.sort();
            work.dedup();
            let mut done: HashSet<(String, String)> = HashSet::new();
            while let Some((cr, q)) = work.pop() {
                if done.len() >= 32 || !done.insert((cr.clone(), q.clone())) {
                    continue;
                }
                let leaf = q.rsplit("::").next().unwrap_or(&q).to_string();
                let real = renames.get(&cr).cloned().unwrap_or_else(|| cr.clone());
                let steps = r1578_sigs.borrow_mut().step_map(dir, &cr, &real, &leaf);
                let enc: Vec<String> = steps.iter().map(|(m, t)| format!("{m}={cr}::{t}")).collect();
                if candor_report::env::trace("CANDOR_R1605_INSTR") {
                    eprintln!("R1606STEPS\t{cr}::{q}\t{}", enc.len()); // §E1
                }
                for (_, t) in &steps {
                    work.push((cr.clone(), t.clone()));
                }
                seeded.push((format!("<steps>{cr}::{q}"), String::new(), enc.join(",")));
                returns.insert(crate::model::r1606_steps_key(&format!("{cr}::{q}")), enc.join(","));
            }
        }
        // …and under the name a `pub use <module>::Entity as Post;` re-export gives it (sea-orm-cli's `prelude.rs`), which
        // is the leaf a `Post::find()` call is typed by. Not where the crate declares a type of that name itself.
        let declared: HashSet<String> = decls_per_file
            .iter()
            .flat_map(|(_, _, fd)| fd.ts.types.iter())
            .map(|(q, _, _)| q.rsplit("::").next().unwrap_or(q).to_string())
            .collect();
        for r in &merged.reexports {
            if r.alias == r.name || r.alias == "*" || declared.contains(&r.alias) {
                continue;
            }
            for (ty, m, v) in seeded.iter().filter(|(ty, _, _)| *ty == r.name) {
                let k = crate::model::impl_ret_key(&r.alias, m);
                if returns.contains_key(&k) || returns.contains_key(&crate::model::impl_self_ret_key(&r.alias, m)) {
                    continue;
                }
                if candor_report::env::trace("CANDOR_R1605_INSTR") {
                    eprintln!("R1605SEED\t{}::{m}\t{v}\talias-of={ty}", r.alias); // §E1 REACH COUNTER
                }
                returns.insert(k, v.clone());
                seeded_alias.push((r.alias.clone(), m.clone(), v.clone()));
            }
        }
        // The filed facts come from a DEPENDENCY's source, not from this crate's declarations, so they are not in
        // `decl_index_digest(&merged)`: fold them into the Pass B reuse key, or a warm scan would replay FnInfos typed under a
        // different lock (or none) — a cached silence where a cold scan charges.
        if !seeded.is_empty() {
            let mut all: Vec<String> = seeded.iter().chain(seeded_alias.iter()).map(|(t, m, v)| format!("{t}::{m}={v}")).collect();
            all.sort();
            decl_index_hash.push_str(&format!("#r1605:{}", all.join(",")));
        }
    }
    let mut enum_variants: EnumVariantIndex =
        merged.enum_tmp.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    // R77: same ambiguous-drop filter, for a DISPATCH-typed single-field tuple-variant payload's leaves.
    let mut enum_variant_traits: EnumVariantTraitIndex =
        merged.enum_variant_traits.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
    // R77 cross-index ambiguity guard — see `drop_cross_ambiguous_enum_leaves` for the full rationale
    // (measured on reqwest 0.13.4 in the 256-crate A/B). R90: the returned set is exactly which leaves
    // were dropped — threaded into `ElemIndexes` so the binder can disclose `Unknown` instead of silence.
    let ambiguous_enum_leaves = drop_cross_ambiguous_enum_leaves(&mut enum_variants, &mut enum_variant_traits);
    let fields = &merged.fields;
    let field_elem = &merged.field_elem;
    let field_elem_trait = &merged.field_elem_trait;
    let trait_impls = &merged.trait_impls;
    let trait_decls = &merged.trait_decls;
    // SOUNDNESS R145 — every module whose namespace an unreadable `include!` may be supplying names to.
    let opaque_scope = crate::lang::opaque_include_scope(&merged.opaque_include_modules, &merged.local_globs);
    let trait_fields = &merged.trait_fields;
    // SOUNDNESS R1246 — each trait's implementor BLOCKS with the qual their units are minted under (`model::impl_owner_key`).
    let mut impl_owners = crate::model::impl_owners_by_trait(&merged.impl_members);
    // …and each trait's own declaration qual, where it is unique (two same-leaf traits refuse, as `union_trait_qual`).
    for (leaf, quals) in &merged.trait_quals {
        if let (Some(q), 1) = (quals.iter().next(), quals.len()) {
            if let Some(v) = impl_owners.get_mut(leaf) {
                v.push(crate::model::ImplOwner { ty: crate::model::TRAIT_DECL_TY.to_string(), owner: q.clone(), ..Default::default() });
            }
        }
    }
    let traits =
        TraitIndexes { impls: trait_impls, decls: trait_decls, fields: trait_fields, dyn_fields: &merged.dyn_trait_fields,
                       foreign_impls: &merged.foreign_impls, written_quals: &merged.written_trait_quals,
                       impl_members: &merged.impl_members, unbound_gen_fields: &merged.unbound_gen_fields,
                       owners: &impl_owners };
    let lazy_statics = &merged.lazy_statics;
    let const_strings = &merged.const_strings;
    let local_macros = &merged.local_macros;

    // TRANSITIVE DROP-OWNER closure (#3, FIELD edition — R49): constructing a struct `T` also runs the drop
    // glue of any LOCAL drop-type `T` OWNS through a field — directly (`_g: Guard`), via a collection element
    // (`_v: Vec<Guard>`, carried by `field_elem`), or transitively (`_s: Session` where `Session` owns a
    // Guard). The per-call drop detection charges only a local OF the drop-type itself; a guard held as a
    // FIELD dropped silent-pure. `owned_drops[T]` = the local drop-types edged when a `T` is constructed.
    // Gated to LOCAL drop-types reached through LOCAL field types, so an external field's invisible Drop is
    // never fabricated. Computed to a fixpoint (monotone; a struct owning itself via `Box` terminates).
    let (owned_drops, owned_drops_all): (HashMap<String, BTreeSet<String>>, HashMap<String, BTreeSet<String>>) = if merged.drop_types.is_empty() {
        (HashMap::new(), HashMap::new())
    } else {
        // A field type's LEAF (`type_path` may qualify it — `inner: ffi::Deflate` — but `owned_drops`,
        // `drop_types`, and the struct keys are all LEAF-keyed, so compare by leaf or the transitive
        // owner chain breaks across modules).
        //
        // SOUNDNESS R718 — AND A BORROWED FIELD IS NOT AN OWNED ONE. `type_path` peels `&`/`&mut`/
        // `Option<&mut _>` on the way into `fields`, and `elem_type` peels them on the way into
        // `field_elem`, so `pub struct Ctx<'a> { pub g: &'a mut G }` arrives here as the bare leaf `G`
        // and every `Ctx { g }` was charged `G::drop` — a drop that runs in whoever OWNS the `G`, never
        // in this frame. `type_borrows` is the SAME test R168 added for the by-value PARAMETER half; the
        // `field_borrows` index carries its answer from the declaration, which is the only place the
        // `&` still exists. EXECUTED against a drop counter: 8 of 15 field shapes were fabricating
        // (`&T`, `&mut T`, `Option<&mut T>`, `Vec<&T>`, `&[T]`, `&Inner`, `&mut Vec<T>`, tuple `&T`) and
        // 6 must keep charging (`T`, `Option<T>`, `Box<T>`, `Vec<T>`, nested `T`, tuple `T`).
        //
        // ABSENCE READS AS OWNED, deliberately: `resolve_impl_bound_fields` joins generic-bound fields
        // into `fields` AFTER the merge and contributes no declaration to this index, and a stale cache
        // entry deserializes it empty. Both then behave exactly as they did before this row — an
        // over-charge — rather than silently dropping a real charge. That is the denylist direction.
        //
        // AND THE FACT IS PER INDEX (`FieldBorrow::leaf` for `fields`, `::elem` for `field_elem`), read
        // off the walk that produced each entry. One whole-type bool withdrew BOTH leaves of
        // `m: HashMap<&'static str, Guard>` and `t: TempFile<&'a Path>` — owned values whose drop
        // EXECUTES in the constructing frame (1 drop each, file really removed) — and `deny Fs` over
        // the unit and its caller went 1 -> 0. That was R718's fix introducing the class it closed.
        let borrows = |t: &str, field: &str, elem: bool| -> bool {
            merged
                .field_borrows
                .get(t)
                .and_then(|m| m.get(field))
                .and_then(|b| if elem { b.elem } else { b.leaf })
                .unwrap_or(false)
        };
        // §E1 REACH COUNTER, on the CHANGED branch and nowhere else: it fires only where the filter
        // WITHDRAWS a candidate that `drop_types` would have accepted, which is precisely the charge
        // this row removes. An unchanged corpus row is not evidence the code ran.
        let withdrew = |t: &str, field: &str, ty: &str| {
            if candor_report::env::trace("CANDOR_ALIAS_DEBUG")
                && merged.drop_types.contains(ty.rsplit("::").next().unwrap_or(ty))
            {
                eprintln!("R718BORROW {t}.{field} {ty}");
            }
        };
        // SOUNDNESS R1723 — A TWIN LEAF'S OWN FIELDS ARE EVERY ARM'S, NOT THE LAST ONE MERGED. `fields` / `field_elem` are
        // leaf-keyed and keep the LAST same-leaf declaration merged, so for flate2's `ffi::c::Inflate { inner: Stream<..> }`
        // beside `ffi::rust::Inflate { inner: Box<InflateState> }` the owner `mem::Decompress { inner: Inflate }` owned
        // `Stream`'s drop or nothing depending on the walk order, and once R1722 sorted the walk the answer was fixed by the
        // FILE NAMES. EXECUTED (fixture `rustagent-r1723/fx`, `wcfg`/`wfeat`): a `Wrap { inner: Inner }` overwrite whose
        // built arm drops a file-writing guard read `deny Fs` 0 in one file order and 1 in the other. A twin leaf is read
        // off the qualified index instead: the UNION of its arms (SPEC §4 ⟨0.38⟩ — a platform arm cannot be decided), less
        // the arms the default build does not compile (`QualIndexes::cfg_off`, the R140/R977 default-build clause), unless
        // that would leave none (nothing describes a build there, so every arm stays, R977). Kill switch `CANDOR_R1723_OFF`.
        let r1723_on = !candor_report::env::switch("CANDOR_R1723_OFF");
        // Inside a FILE the default build does not compile there is no default build to describe (R977), so a unit there
        // reads the closure over EVERY arm (`owned_drops_all`); everywhere else the default-build one.
        let twin_all: HashMap<String, Vec<String>> = {
            let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
            if r1723_on {
                for q in merged.qual.fields.keys() {
                    by_leaf.entry(q.rsplit("::").next().unwrap_or(q).to_string()).or_default().push(q.clone());
                }
            }
            by_leaf
                .into_iter()
                .filter(|(_, qs)| qs.len() >= 2)
                .map(|(l, mut qs)| {
                    qs.sort();
                    (l, qs)
                })
                .collect()
        };
        let twin_built: HashMap<String, Vec<String>> = twin_all
            .iter()
            .map(|(l, qs)| {
                let built: Vec<String> = qs.iter().filter(|q| !merged.qual.cfg_off.contains(*q)).cloned().collect();
                (l.clone(), if built.is_empty() { qs.clone() } else { built })
            })
            .collect();
        /// One declaration's `(fields, field_elem)` maps.
        type Arm<'m> = (Option<&'m HashMap<String, String>>, Option<&'m HashMap<String, String>>);
        let close = |twin_arms: &HashMap<String, Vec<String>>| -> HashMap<String, BTreeSet<String>> {
        let candidates = |t: &str| -> Vec<String> {
            let leaf = |ty: &String| ty.rsplit("::").next().unwrap_or(ty).to_string();
            let mut v: Vec<String> = Vec::new();
            let arms: Vec<Arm<'_>> = match twin_arms.get(t) {
                Some(qs) => qs.iter().map(|q| (merged.qual.fields.get(q), merged.qual.field_elem.get(q))).collect(),
                None => vec![(fields.get(t), field_elem.get(t))],
            };
            for (elem, own_fields, src) in arms.iter().flat_map(|(f, e)| [(false, *f, *f), (true, *f, *e)]) {
                let Some(src) = src else { continue };
                for (k, ty) in src {
                    // VEIN B — `field_elem` now records a std WRAPPER's argument (`m: Mutex<G>` -> `G`) for
                    // the typing routes. Drop glue is a different question with its own model (vein D) and
                    // this change does not move it: a wrapper-held value contributes exactly what it did
                    // before vein B, which was nothing.
                    if ty == crate::cache::QUAL_TOMB {
                        continue;
                    }
                    if elem
                        && own_fields
                            .and_then(|m| m.get(k))
                            .is_some_and(|w| crate::lang::is_value_wrapper(w.rsplit("::").next().unwrap_or(w)))
                    {
                        continue;
                    }
                    // R893 — …and an element MARKED as held by a wrapper (`Arc<Mutex<Vec<G>>>`, whose own
                    // `fields` kind is `Arc`): recorded for the typing routes, and as invisible to drop
                    // glue as it was before the mark existed.
                    if elem && crate::lang::is_wrapped(ty) {
                        continue;
                    }
                    if borrows(t, k, elem) {
                        withdrew(t, k, ty);
                    } else {
                        v.push(leaf(ty));
                    }
                }
            }
            v
        };
        let all_types: BTreeSet<String> = fields.keys().chain(field_elem.keys()).cloned().collect();
        let mut owned: HashMap<String, BTreeSet<String>> = HashMap::new();
        for t in &all_types {
            let s: BTreeSet<String> = candidates(t)
                .into_iter()
                .filter(|c| merged.drop_types.contains(c))
                .collect();
            if !s.is_empty() {
                owned.insert(t.clone(), s);
            }
        }
        loop {
            let mut changed = false;
            for t in &all_types {
                let mut add: BTreeSet<String> = BTreeSet::new();
                for c in candidates(t) {
                    if &c != t {
                        if let Some(inner) = owned.get(&c) {
                            add.extend(inner.iter().cloned());
                        }
                    }
                }
                if !add.is_empty() {
                    let e = owned.entry(t.clone()).or_default();
                    for d in add {
                        if e.insert(d) {
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        if candor_report::env::trace("CANDOR_R1723_INSTR") {
            let mut tw: Vec<&String> = twin_arms.keys().collect();
            tw.sort();
            for l in tw {
                let o: Vec<&String> = owned.get(l).map(|s| s.iter().collect()).unwrap_or_default();
                eprintln!("R1723DROP\t{l}\t{}\t{}", twin_arms[l].join(","), o.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",")); // §E1 REACH
            }
        }
        owned
        };
        (close(&twin_built), close(&twin_all))
    };
    // DROP-RELEVANT LEAVES — the set the collector gates its `<construct>` markers on. Hoisted ABOVE
    // Pass B (it used to sit beside the call loop) because the marker is now emitted at the construction
    // EXPRESSION, inside the walk, and the walk needs to know which types are worth marking: without a
    // gate every `Some(..)`/`Ok(..)`/`Vec::new()` in the tree would push a synthetic call nobody reads.
    // Both halves belong in it — a type with its own `impl Drop`, and a type that OWNS one through a
    // field (the R49 field edition, whose charge is keyed on constructing the OWNER).
    let drop_relevant: std::collections::HashSet<String> = merged
        .drop_types
        .iter()
        .cloned()
        .chain(owned_drops.keys().cloned())
        .chain(owned_drops_all.keys().cloned())
        .collect();

    // SOUNDNESS R182 — fn LEAF -> the DROP-RELEVANT candidate return types the ambiguity rule withdrew.
    // `record_return`/`merge_decls` file every withdrawn candidate under `amb_ret_key`; this reduces them
    // to the ones a charge could actually have landed on, which is the whole of the flood control. A leaf
    // whose collision is `parse -> Config` vs `parse -> usize` yields NOTHING here — neither type has a
    // local `Drop`, so the refusal cost nothing and there is nothing to disclose. Built AFTER
    // `drop_relevant` for that intersection, which is why `elems` is assembled here rather than beside
    // the other indexes.
    let ambiguous_return_leaves: HashMap<String, Vec<String>> = {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for k in merged.rets.keys() {
            let Some((fn_leaf, ty_leaf)) = crate::model::split_amb_ret_key(k) else { continue };
            // Only a leaf the index really DID withdraw: a candidate recorded during an intra-file
            // conflict that a later contributor resolved would otherwise read as ambiguous forever.
            if !matches!(merged.rets.get(fn_leaf), Some(None)) {
                continue;
            }
            if drop_relevant.contains(ty_leaf) {
                m.entry(fn_leaf.to_string()).or_default().push(ty_leaf.to_string());
            }
        }
        for v in m.values_mut() {
            v.sort();
            v.dedup();
        }
        m
    };
    // VEIN B — the (type LEAF, field) pairs the leaf-keyed field indexes cannot answer for: the leaf is
    // declared at ≥2 distinct module paths AND two of those declarations give the field DIFFERENT types.
    // A field only one twin declares is unambiguous — `x.f` on the other twin does not compile. Where two
    // twins sit in ONE file the per-file index has already merged them, so every field of that leaf
    // counts (`"<leaf>\u{1f}*"`). The ⟨0.40⟩ per-file declared-type surface records every struct by its
    // module-qualified path; a `#[cfg]` twin of ONE path is one type and does not count.
    // SOUNDNESS R1116 — every type LEAF this crate declares at two or more quals, whatever their fields.
    let mut twin_type_leaves: HashSet<String> = HashSet::new();
    // EXACT-FIRST — every type a leaf-keyed FIELD index can answer for a twin owner's field (`fields`/`field_elem` keep
    // the LAST twin merged), so a receiver typed through one may name the OTHER twin's field type. A held qual equal to
    // one of these is not a claim the exact pick may trust (EXECUTED: `fx/adv/i_field_inline*`, the pure twin's field
    // type answered for the effectful twin's `self.x`; and micromap's `Drain.iter`, found by the removal audit).
    let mut twin_field_tys: HashSet<String> = HashSet::new();
    let ambiguous_type_leaves: HashSet<String> = {
        let mut quals: HashMap<String, BTreeSet<&str>> = HashMap::new();
        let mut files_of: HashMap<String, Vec<usize>> = HashMap::new();
        for (fi, (_, _, fd)) in decls_per_file.iter().enumerate() {
            let mut here: HashMap<&str, usize> = HashMap::new();
            for (q, _, _) in &fd.ts.types {
                let leaf = q.rsplit("::").next().unwrap_or(q);
                quals.entry(leaf.to_string()).or_default().insert(q.as_str());
                *here.entry(leaf).or_default() += 1;
            }
            for (leaf, _) in here {
                files_of.entry(leaf.to_string()).or_default().push(fi);
            }
        }
        let mut out: HashSet<String> = HashSet::new();
        // SOUNDNESS R1168 — the twin leaves EVERY one of whose quals is the self type of a local `impl Future`
        // (hickory-net's two `NextRandomUdpSocket`s): awaiting a value of that leaf yields an `Output`, whichever twin
        // it is. Keyed `\u{1f}fut\u{1f}<leaf>`, a prefix no `<leaf>\u{1f}` consumer of this set can match.
        // A self TyRef is `L:<qual>` or, written with no `use`, `R:<modpath>\u{1}<written>` (typesurf's encodings).
        let future_selves: HashSet<String> = decls_per_file
            .iter()
            .flat_map(|(_, _, fd)| fd.ts.impls.iter())
            .filter(|(_, tr)| !tr.starts_with("L:") && tr.rsplit("::").next() == Some("Future"))
            .filter_map(|(sr, _)| {
                if let Some(q) = sr.strip_prefix("L:") {
                    return Some(q.to_string());
                }
                let (m, w) = sr.strip_prefix("R:")?.split_once('\u{1}')?;
                Some(if m.is_empty() { w.to_string() } else { format!("{m}::{w}") })
            })
            .collect();
        for (leaf, qs) in &quals {
            if qs.len() >= 2 && qs.iter().all(|q| future_selves.contains(*q)) {
                out.insert(format!("\u{1f}fut\u{1f}{leaf}"));
            }
        }
        for (leaf, qs) in &quals {
            if qs.len() < 2 {
                continue;
            }
            twin_type_leaves.insert(leaf.clone());
            let fis = files_of.get(leaf).cloned().unwrap_or_default();
            let same_file = fis.iter().any(|&fi| {
                decls_per_file[fi].2.ts.types.iter()
                    .filter(|(q, _, _)| q.rsplit("::").next().unwrap_or(q) == leaf.as_str())
                    .map(|(q, _, _)| q.as_str()).collect::<BTreeSet<_>>().len() >= 2
            });
            if same_file {
                out.insert(format!("{leaf}\u{1f}*"));
                for &fi in &fis {
                    // Both the file's own decls and the alias-expanded ones (R978), whichever the typer reads.
                    for fd in [Some(&decls_per_file[fi].2), expanded.get(fi).and_then(Option::as_ref)].into_iter().flatten() {
                        for idx in [fd.fields.get(leaf.as_str()), fd.field_elem.get(leaf.as_str())].into_iter().flatten() {
                            twin_field_tys.extend(idx.values().cloned());
                        }
                    }
                }
                continue;
            }
            // R978 — the alias-EXPANDED decls where the R99 second merge produced them (see `expanded`).
            let fd_of = |fi: usize| expanded.get(fi).and_then(Option::as_ref).unwrap_or(&decls_per_file[fi].2);
            let mut seen: HashMap<&str, BTreeSet<String>> = HashMap::new();
            for &fi in &fis {
                let fd = fd_of(fi);
                for idx in [fd.fields.get(leaf.as_str()), fd.field_elem.get(leaf.as_str())].into_iter().flatten() {
                    for (k, v) in idx {
                        seen.entry(k.as_str()).or_default().insert(format!("{fi}\u{1f}{v}"));
                    }
                }
            }
            for (k, vs) in seen {
                let files: BTreeSet<&str> = vs.iter().map(|x| x.split('\u{1f}').next().unwrap_or("")).collect();
                let vals: BTreeSet<&str> = vs.iter().map(|x| x.split('\u{1f}').nth(1).unwrap_or("")).collect();
                if files.len() >= 2 && vals.len() >= 2 {
                    out.insert(format!("{leaf}\u{1f}{k}"));
                    // SOUNDNESS R1652/R1657 — whether the twins disagree on this field's ELEMENT entry specifically
                    // (`CallCollector::r1657_elem_disagrees`): differing values, or one twin with an entry and one without.
                    {
                        let with: Vec<&str> = fis.iter().filter_map(|&fi| fd_of(fi).field_elem.get(leaf.as_str()).and_then(|m| m.get(k)).map(String::as_str)).collect();
                        let declaring = fis.iter().filter(|&&fi| {
                            let fd = fd_of(fi);
                            fd.fields.get(leaf.as_str()).is_some_and(|m| m.contains_key(k))
                                || fd.field_elem.get(leaf.as_str()).is_some_and(|m| m.contains_key(k))
                        }).count();
                        let distinct: BTreeSet<&str> = with.iter().copied().collect();
                        if distinct.len() >= 2 || (!with.is_empty() && with.len() < declaring) {
                            out.insert(format!("{leaf}\u{1f}{k}\u{1c}elem"));
                        }
                    }
                    twin_field_tys.extend(vals.iter().map(|v| v.to_string()));
                    // VEIN A — when every twin's type for this field is CRATE-ANCHORED and shares one LEAF
                    // (`crate::deflate::window::Window` / `crate::inflate::window::Window`), record that
                    // leaf: the receiver walk answers with it (see `CallCollector::field_twin_leaf`).
                    // Per INDEX (`fields` → `\u{1d}`, `field_elem` → `\u{1e}`), because the two hold
                    // different facts about one field (`Option<Invalidator>` vs `Invalidator`).
                    for (sep, pick) in [('\u{1d}', 0usize), ('\u{1e}', 1usize)] {
                        let tv: BTreeSet<&str> = fis.iter().filter_map(|&fi| {
                            let fd = fd_of(fi);
                            let idx = if pick == 0 { &fd.fields } else { &fd.field_elem };
                            idx.get(leaf.as_str()).and_then(|m| m.get(k)).map(String::as_str)
                        }).collect();
                        // SOUNDNESS R898 — a BARE single-segment spelling of a type this crate declares is
                        // the same crate-local claim as an anchored one. Twins spelled one each way
                        // (`use super::Connection` anchored in hyper's `client::conn::http1::upgrades`, the
                        // server twin's own bare `Connection`) were ambiguous with no twin leaf, so the
                        // leaf-keyed `field_elem` kept the LAST file's entry and the server's
                        // `self.inner.map(|c| c.into_parts())` read the CLIENT's `['Fs']` while running the
                        // server's `Env` (`deny Env` 0). PRE-EXISTING for a FILE-module twin (708ce46,
                        // `rustagent-resid/fx/r898twinf`); R898 anchoring inline modules reached it in
                        // hyper itself, and the corpus A/B is what showed it.
                        let anchored = tv.len() >= 2
                            && tv.iter().all(|v| v.starts_with("crate::") || (!v.contains("::") && quals.contains_key(*v)));
                        let leaves: BTreeSet<&str> = tv.iter().map(|v| v.rsplit("::").next().unwrap_or(v)).collect();
                        if anchored && leaves.len() == 1 {
                            if let Some(l) = leaves.iter().next() {
                                out.insert(format!("{leaf}\u{1f}{k}{sep}{l}"));
                            }
                        }
                    }
                }
            }
        }
        out
    };
    let elems = ElemIndexes { ambiguous_type_leaves: &ambiguous_type_leaves, field_elem, field_elem_trait, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits, ambiguous_enum_leaves: &ambiguous_enum_leaves, callable_statics: &merged.callable_statics, static_types: &merged.static_types, callable_aliases: &merged.callable_aliases, ambiguous_return_leaves: &ambiguous_return_leaves, macro_twins: &merged.macro_twins, field_borrows: Some(&merged.field_borrows), drop_types: Some(&merged.drop_types), twin_type_leaves: Some(&twin_type_leaves), fields_q: Some(&merged.qual) };

    // ROUND 2 PARSE (parallel): files whose decls were cached but whose FnInfos are STALE (the merged
    // decl index moved) — exactly the files a decl-changing edit invalidates. On a body-only edit this
    // set is empty; on a decl edit it is "everything else", re-parsed in parallel (degrade-to-full).
    // A file whose cached entry carried an ABORT has no `cached_fninfos` row at all (dropped above), so
    // `unwrap_or(true)` puts it here: the re-attempt is a plain stale-FnInfos re-parse, no special case.
    let need_passb: Vec<&str> = decls_per_file
        .iter()
        .map(|(rel, _, _)| rel.as_str())
        .filter(|rel| {
            !parsed_files.contains_key(*rel)
                && cached_fninfos.get(*rel).map(|(h, ..)| h != &decl_index_hash).unwrap_or(true)
        })
        .collect();
    let round2: Vec<(String, Option<ParsedFile>)> = need_passb
        .par_iter()
        .map(|rel| {
            let parsed = paths
                .iter()
                .find(|(_, r)| r == rel)
                .and_then(|(p, _)| std::fs::read_to_string(p).ok())
                // R308 — the round-2 twin of the round-1 parse above.
                .and_then(|t| {
                    let (mut f, _) = crate::lang::parse_file_2015_tolerant(&t)?;
                    // R145 — the round-2 twin of the round-1 splice. Its decls are the cached ones, which
                    // were derived from the same splice of the same bytes (the cache key covers them).
                    if t.contains("include!") {
                        let p = paths.iter().find(|(_, r)| r == rel).map(|(p, _)| p.clone())?;
                        let dir = p.parent().unwrap_or(root).to_path_buf();
                        crate::lang::splice_includes(&mut f.items, &dir, &include_env, &mut Vec::new(), &mut Vec::new());
                    }
                    if t.contains("pin_project") || t.contains("link!") {
                        crate::lang::splice_pin_project(&mut f.items); // R988, R960 — the round-2 twin
                    }
                    if t.contains("macro_rules") || (!xmacros.is_empty() && t.contains('!')) {
                        crate::mbe::splice_local_macros(&mut f.items, &xmacros); // R1004 — the round-2 twin
                    }
                    crate::mbe::splice_derived_comparisons(&mut f.items); // R1146 — the round-2 twin
                    if t.contains("DeriveEntity") {
                        crate::mbe::splice_sea_orm_entities(&mut f.items); // R1605 — the round-2 twin
                    }
                    Some(f)
                })
                .map(|file| {
                    // Resolve loc on THIS parse worker (span line/col is thread-local) — same as round 1.
                    let mut locs = Vec::new();
                    fn_locs(&file.items, rel, include_tests, &mut locs);
                    (SendFile(file), locs)
                });
            (rel.to_string(), parsed)
        })
        .collect();
    for (rel, sf) in round2 {
        if let Some((sf, locs)) = sf {
            parsed_locs.insert(rel.clone(), locs);
            parsed_files.insert(rel, sf.0);
        }
    }

    // Pass B — assemble each file's FnInfos in WALK ORDER: reuse the cached set when the decl index is
    // unchanged, else re-derive from the (now parsed) file. Either way the concatenated `fns` is exactly
    // what the old single Pass B loop produced.
    let mut fns: Vec<FnInfo> = Vec::new();
    let mut fresh_fninfos: HashMap<String, Vec<FnInfo>> = HashMap::new();
    // rel -> the abort disclosure this run recorded for it, carried into the cache write-back.
    let mut fresh_aborted: HashMap<String, String> = HashMap::new();
    // Files that must NOT be cached this run: their FnInfos are UNKNOWN (not empty), so persisting
    // anything for them would publish a number nobody derived. Dropping the entry makes the next run
    // re-attempt from scratch, which is the fail-closed direction.
    let mut no_cache: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (rel, _, _) in &decls_per_file {
        // Reuse only a set that was DERIVED. An entry whose walk aborted never reaches `cached_fninfos`
        // (see the `aborted` arm above), so there is nothing here that could replay a disclosure: a file
        // that aborted last run is re-walked, and earns its disclosure again or loses it.
        let reuse = cached_fninfos.get(rel).filter(|(h, _)| *h == decl_index_hash).map(|(_, v)| v.clone());
        if let Some(v) = reuse {
            fns.extend(v.iter().cloned());
            continue;
        }
        // Re-derive: the file needs its parse. Reaching the `else` means its DECLS came from cache (so
        // it was never in round 1) but the round-2 re-read/re-parse failed — the file changed or became
        // unreadable mid-scan. (Round 2 covers two entrances: the decl index moved, or the entry carried
        // an abort and its FnInfos were refused. The reason below names neither, because from here they
        // are the same event and only one of them is true.) It contributes no FnInfos, which under §2
        // rule 3 is a purity claim over every function in it, so disclose it exactly like a round-1
        // failure and refuse to cache anything for it.
        let Some(file) = parsed_files.get(rel) else {
            eprintln!("candor-scan: {rel} failed to re-read/parse — effects in it are NOT in this report");
            had_parse_failure = true;
            unanalyzed_units.push(candor_report::UnanalyzedUnit {
                path: rel.clone(),
                reason: "source failed to re-read/parse during re-derivation".into(),
            });
            no_cache.insert(rel.clone());
            continue;
        };
        let modpath = module_path(Path::new(rel));
        // Locs were resolved on the parse worker (spans are dead on this thread); reuse them positionally.
        let locs = parsed_locs.get(rel).map(Vec::as_slice).unwrap_or(&[]);
        let mut loc_idx = 0usize;
        // Seed the crate-ROOT re-exports under `crate::<name>` (and `crate::` + GLOB_KEY for the root glob)
        // so a `use crate::net` / `crate::net::foo` in THIS file resolves through the root re-export it can't
        // otherwise see (each file starts with a fresh `use` map). Crate-rooted only: a bare `net::foo` never
        // looks up a `crate::…` key, so a genuine external-crate call is never hijacked (see `expand`).
        let mut uses = seed_root_reexports(&merged.root_reexports);
        // R186 — …and the crate ROOT's OWN declarations, under `crate::` + `ROOT_DECL_KEY`. `expand`'s
        // glob-attribution branch reads it to refuse a `crate::<name>::…` rewrite where `<name>` is this
        // crate's own root item: an explicit declaration shadows a glob in rustc, so the attribution was
        // provably wrong, and for a TYPE path (which has no tail2 rescue) it cost the call edge outright.
        crate::lang::seed_root_decls(&merged.root_decls, &mut uses);
        // SOUNDNESS R751 — …and this file's own module path, so `expand` can turn a `self::`/`super::`
        // path into a crate-root-ABSOLUTE one instead of collapsing it and losing the module context.
        crate::lang::seed_modpath(&modpath, &mut uses);
        // VEIN A — and the manifest's dependency names, so a glob is never credited with a name that is
        // an extern crate (`use sqlx_core::prelude::*; dotenvy::var(..)` keeps `dotenvy`).
        crate::lang::seed_deps(&manifest_deps, &mut uses);
        // SOUNDNESS R1710 — …and the crate ROOT's renamed `extern crate X as Y;` bindings: the extern prelude, in scope as `Y`
        // in every module that does not declare or import `Y` itself (see `lang::seed_extern_prelude`).
        crate::lang::seed_extern_prelude(&merged.extern_aliases, &file.items, include_tests, &mut uses);
        // …and the MODULE-QUALIFIED external aliases (R99): a submodule `pub use std::process::Command`, a
        // nominal `pub type Cmd = std::process::Command`, a callable-typed `const`. Same cross-file problem
        // the root re-exports have — the declaring module is usually a DIFFERENT file — and the same answer.
        seed_mod_aliases(&merged.mod_aliases, &modpath, &mut uses);
        let mut file_fns: Vec<FnInfo> = Vec::new();
        // A PANIC IN ONE FILE MUST NOT TAKE THE RUN DOWN, and must not vanish either. `syn`/`proc-macro2`
        // can abort on input candor does not control — `getrandom` 0.3.4/0.4.2 hit proc-macro2's
        // `unreachable!("Invalid span with no related FileInfo!")` deterministically — and an unwind here
        // killed the WHOLE `--deps` tree, not the one crate. A chained consumer then proceeded with fewer
        // dependency reports than it asked for, which is exactly the blind spot the κ ledger exists to
        // name. So: contain it at the FILE, and DISCLOSE that file as unanalyzed — the ⟨0.21⟩ channel that
        // already carries "this file failed to parse", and that already makes a configured gate refuse to
        // go green over it. The panic message still reaches stderr through the default hook.
        let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // FAULT INJECTION, so the containment has a deterministic test. The real trigger needs a whole
            // crate's parse state and does not reduce to a fixture, and a containment nobody can fire is a
            // containment nobody has checked — the same reason the syscall oracle is calibrated with a
            // seeded violation rather than trusted because it reported none.
            if injected_fault_for(rel) {
                panic!("CANDOR_PANIC_ON_FILE: injected fault while walking {rel}");
            }
            let mut out: Vec<FnInfo> = Vec::new();
            let mut idx = loc_idx;
            let mut u = uses.clone();
            // R167's `skip_test_fns` -- from `rel`, the SAME argument `fn_locs` derives it from, so the
            // two lockstep walks cannot disagree about which fns they emit.
            let _cfg_off = crate::lang::CfgOffScope::enter(cfg_off_files.contains(rel)); // R977
            scan_items(&file.items, &modpath, locs, &mut idx, include_tests, fields, &returns, traits, elems, lazy_statics, const_strings, local_macros, &drop_relevant, !crate::lang::is_nonlib_target_file(rel), &mut u, &mut out);
            (out, idx, u)
        }));
        match walked {
            Ok((out, idx, u)) => {
                file_fns = out;
                loc_idx = idx;
                uses = u;
            }
            Err(_) => {
                const ABORTED: &str = "the parser aborted while walking this file";
                eprintln!("candor-scan: {rel} could not be analyzed (the parser aborted); disclosed as unanalyzed");
                had_parse_failure = true;   // a configured gate must not certify a scan with a hole in it
                unanalyzed_units.push(candor_report::UnanalyzedUnit {
                    path: rel.clone(),
                    reason: ABORTED.into(),
                });
                // …and the disclosure rides into the CACHE. Without this the entry written below is
                // `{ real content hash, current decl index, fninfos: [] }` — indistinguishable from a
                // file that genuinely has no functions — so the next `--incremental` run over identical
                // bytes reuses it, `continue`s before the walk, and discloses nothing. The panic was
                // fail-closed; the cache turned it into a reproducible false all-clear.
                fresh_aborted.insert(rel.clone(), ABORTED.to_string());
            }
        }
        let _ = loc_idx;
        fns.extend(file_fns.iter().cloned());
        fresh_fninfos.insert(rel.clone(), file_fns);
    }

    // WRITE BACK the cache (incremental only) as ONE consolidated file. Each entry persists {content_hash,
    // decls, decl_index_hash, fninfos}; the FnInfos written are the CURRENT ones (reused or freshly
    // derived) tagged with the CURRENT decl_index_hash, so the next scan's gate is exact. The map is
    // rebuilt from the current path set, so deleted/renamed files drop out automatically (no pruning pass).
    // The write is SKIPPED entirely when nothing changed — every file's decls came from cache AND already
    // recorded this decl_index_hash AND no file was added/removed — so a no-edit re-scan does zero writes.
    // Best-effort: a cache write failure never affects the report (it only costs a re-derivation later).
    //
    // ⟨0.42⟩ SOUNDNESS R1634 — **A RUN THAT CONSULTED A RESULT-MOVING SWITCH STORES NOTHING.** The entries below are the Pass A decls and
    // Pass B FnInfos this run derived, and a switch selects which logic derived them: written, they would be REPLAYED by the next
    // `--incremental` run with the variable unset, whose report would then carry the switched rows and name no switch — the
    // cache-replay hole candor-java closed for `CANDOR_REFRESH`. Every read that could have shaped an entry has already executed
    // (the derivation above is finished and what follows is the uncached classify stage), so the set is complete for this purpose.
    // READING the cache under a switch is unaffected: a replayed entry is the unswitched answer, which a switched run is allowed to
    // be (the read never executed, so nothing was consulted).
    if incremental && candor_report::env::consulted_switches().is_empty() {
        let unchanged = fresh_fninfos.is_empty()
            && prior.is_empty() // every prior entry was consumed by a current file → none deleted
            && decls_per_file.iter().all(|(rel, _, _)| disk_decl_hash.get(rel) == Some(&decl_index_hash));
        if !unchanged {
            let mut files: HashMap<String, FileCache> = HashMap::with_capacity(decls_per_file.len());
            for (rel, ch, fd) in &decls_per_file {
                if no_cache.contains(rel) {
                    continue; // nothing was derived for it — see the `no_cache` note above
                }
                let fninfos = fresh_fninfos
                    .get(rel)
                    .cloned()
                    .or_else(|| cached_fninfos.get(rel).map(|(_, v)| v.clone()))
                    .unwrap_or_default();
                // The marker travels with the FnInfos it belongs to, and it can only come from THIS run:
                // a file that aborted last run was re-walked above, so either it aborted again (and is in
                // `fresh_aborted`) or it produced a real set and the marker is gone. That is what keeps
                // an abort from surviving more than the one run that observed it — the cache can record
                // "these FnInfos were never derived", but it can never answer with a stale abort.
                let aborted = fresh_aborted.get(rel).cloned();
                files.insert(
                    rel.clone(),
                    FileCache {
                        content_hash: ch.clone(),
                        decls: fd.clone(),
                        decl_index_hash: decl_index_hash.clone(),
                        fninfos,
                        aborted,
                    },
                );
            }
            let cache = ScanCache { schema: schema.clone(), files };
            let _ = std::fs::create_dir_all(&cache_dir);
            if let Ok(bytes) = serde_json::to_vec(&cache) {
                let _ = candor_report::write_atomic(&cache_path, &bytes);
            }
        }
    }

    // The κ-coverage ledger: Cargo.toml's [dependencies] are the crate's TRUE external universe, so a
    // dep the calls actually reach whose classification never fires — and that isn't in a calibrated
    // tier — is a named blind spot (invisible, not Unknown: the curated-κ caveat). Counted here,
    // disclosed in the receipt, so the caveat is per-scan evidence instead of a doc footnote.
    let (deps, dep_renames) = cargo_deps(dir);
    // ⟨0.40⟩ the per-file `use` paths (the in-scope-trait question) and the reach probe.
    let file_uses: HashMap<&str, &[String]> =
        decls_per_file.iter().map(|(rel, _, fd)| (rel.as_str(), fd.ts.uses.as_slice())).collect();
    let r843_probe = candor_report::env::trace("CANDOR_R843_PROBE");
    // ⟨0.40⟩ this crate's own trait declarations, by module-qualified path — row 1 must not read an import of
    // one of them as a chained trait in scope (see `typesurf::in_scope_publishes`).
    let local_trait_quals: Vec<&str> = merged.trait_quals.values().flatten().map(String::as_str).collect();
    // SOUNDNESS R1202 — per file, the leaves of the traits this crate DECLARES in it (in scope there without a `use`).
    let file_local_traits: HashMap<&str, HashSet<&str>> = decls_per_file
        .iter()
        .map(|(rel, _, fd)| (rel.as_str(), fd.trait_quals.keys().map(String::as_str).collect()))
        .collect();
    let r1202_off = candor_report::env::switch("CANDOR_R1202_OFF");
    // SOUNDNESS R1202 — each unit's receiver rank (`FnInfo::recv_rank`), for the local-trait-vs-local-inherent contest.
    let r1202_rank: HashMap<String, u8> = fns.iter().filter(|f| f.recv_rank > 0).map(|f| (f.qual.clone(), f.recv_rank)).collect();
    let r1202_probe = candor_report::env::trace("CANDOR_R1202_PROBE");
    // SOUNDNESS R1207 / R1208 / R1209 — falsification switches for the three halves below, and R1207's measurement
    // switch (`CANDOR_R1207_NOFORM`: hedge without the receiver-form exemption, to price it).
    let r1207_off = candor_report::env::switch("CANDOR_R1207_OFF");
    let r1207_noform = candor_report::env::switch("CANDOR_R1207_NOFORM");
    // SOUNDNESS R1207 — the POSTURE. A scan that chains reports has declared its dependency universe, and a package it
    // imports and could not read is a hole in it: `Unknown`, as R1202 already answers a dependency receiver (whose rule
    // can only fire in a chained scan). A STANDALONE scan (nothing chained) adds NOTHING, as 0.40.12 did. Priced
    // (rustagent-v075, 5,372 standalone registry crates): the `Unknown` posture there newly hedged ~102k rows, 4.3%,
    // above the ~2.6% band. And a κ ledger entry (`invisible`) there is NOT free: it turns a pure unit, which a
    // standalone report omits, into a published `[]` + `invisible` ROW, so a consumer chaining that report HITS where it
    // missed, and the hit skips the consumer's own trait-in-scope miss rule (⟨0.40⟩ permission row 1): MEASURED
    // (rustagent-v075 A/B against 0.40.12, 1,160 chained census consumers) 160 consumer rows lost a 0.40.12 `Unknown`
    // with the standalone ledger on and 0 with it off, the only difference between the two builds (regex
    // `Regex::static_captures_len`: `deny Unknown` 1 to 0; the row is pure with the ledger build's dependency reports
    // and `Unknown` with the main-line build's, whichever of the two engines scans the consumer). That is SPEC's
    // "a mechanism that makes reports better must not make silence cheaper" (R764's shape), so the standalone ledger
    // is OFF; the standalone half of R1207 is the stated residual. `CANDOR_R1207_HEDGE=1` forces `Unknown` (and its ledger
    // entry) in a standalone scan too, to price it.
    let r1207_hedge = !deps_idx.crates.is_empty() || candor_report::env::switch("CANDOR_R1207_HEDGE");
    let r1208_off = candor_report::env::switch("CANDOR_R1208_OFF");
    let r1209_off = candor_report::env::switch("CANDOR_R1209_OFF");
    // ⟨0.40⟩ SOUNDNESS R888 — this crate's own `Deref` impls whose target is a DEPENDENCY's type.
    let foreign_derefs = {
        let files: Vec<&crate::typesurf::FileSurface> = decls_per_file.iter().map(|(_, _, fd)| &fd.ts).collect();
        let ctx = crate::typesurf::BuildCtx {
            crate_name: &crate_name, deps: &deps, renames: &dep_renames, chained: &deps_idx.surface,
            incomplete: false,
        };
        crate::typesurf::foreign_derefs(&ctx, &files)
    };
    // dep crate root -> count of FLOORED call sites into it. Floored only: the tally has to mean the
    // same thing as the crate name beside it — calls whose effects this scan could not see.
    let mut dep_seen: HashMap<String, usize> = HashMap::new();
    // fn -> the dep crates it DIRECTLY calls into where the classifier floored the call. Post-filtered to
    // the genuinely-blind crates (not calibrated, no sibling report) + propagated transitively → the
    // per-fn `invisible` honesty disclosure (the κ ledger, but attributed per function).
    let mut blind_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
    // SOUNDNESS R1605 — the units whose `Unknown` is the R1605 floor (a dependency value an `R1605_TRAITS` member produced,
    // left untyped); read by R1095's untyped-receiver floor below.
    let mut r1605_floored: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Blind crates inherited from a dep fn's `invisible` (sweep [8]): genuinely blind (the dep confirmed
    // it), but a TRANSITIVE crate the consumer never saw directly, so it is absent from `dep_seen` and
    // would be dropped by the `global_blind` filter. Collected here and unioned into global_blind below.
    let mut dep_invisible: BTreeSet<String> = BTreeSet::new();
    // Callers whose `Unknown` came across the chain join with no reason the dependency recorded. See
    // `DepSink::unknown_via_dep` and the §4 invariant in the report writer.
    let mut unknown_via_dep: BTreeSet<String> = BTreeSet::new();

    // Two name indexes for resolving a call to a local definition. `by_leaf` keys on the bare last
    // segment (`new`); `by_tail2` keys on the last TWO segments (`RequestBuilder::new`). The leaf index
    // alone catastrophically over-connects on real crates: every call to *some* `new()` would link to
    // ALL `*::new` defs (in reqwest, 181 of them), smearing one type's effect across the whole graph.
    // So a `Type::method`/`mod::fn` call matches the qualified tail (keeping `RequestBuilder::new` distinct
    // from `Body::new`) and a bare free call matches the leaf — BOTH only when the match is UNAMBIGUOUS
    // (exactly one def), under-reporting rather than fabricating. See `resolve_target` + the module doc.
    // SOUNDNESS R894 — ONE UNIT PER PUBLISHED DECLARATION QUAL. `#[cfg]` twins of a foreign declaration
    // (libc's `unix::usleep`, `pthread_once`; 28 quals in web-sys) are one name with one answer, and a
    // second unit is a DUPLICATE ROW that breaks `analyzed.count == |callgraph nodes|` (measured: libc
    // 7802 vs 7800, web-sys 20472 vs 20444; 0 duplicates before R894). And where a REAL unit carries the
    // same qual (a Rust shim under one `#[cfg]`, the declaration under the other — termios'
    // `ffi::cfmakeraw`), the declaration is folded INTO it: SPEC §4's union over the units of one qual,
    // carried by one row, so the shim's callers inherit the arm that is a foreign call.
    {
        let real: std::collections::HashSet<String> =
            fns.iter().filter(|f| !f.extern_decl).map(|f| f.qual.clone()).collect();
        let folded: std::collections::HashSet<String> =
            fns.iter().filter(|f| f.extern_decl && real.contains(&f.qual)).map(|f| f.qual.clone()).collect();
        let mut kept_decls: std::collections::HashSet<String> = std::collections::HashSet::new();
        fns.retain(|f| !f.extern_decl || (!real.contains(&f.qual) && kept_decls.insert(f.qual.clone())));
        for f in fns.iter_mut().filter(|f| folded.contains(&f.qual)) {
            f.unresolved = true;
            if !f.unresolved_why.iter().any(|w| w == "native:extern fn") {
                f.unresolved_why.push("native:extern fn".to_string());
            }
        }
    }
    // SOUNDNESS R1146 (v052 audit) — a synthetic derived-comparison unit is dropped where any hand-written unit shares
    // its `Type::method` tail (see `FnInfo::derive_synth`); the survivors are published only.
    {
        let tail2 = |q: &str| -> String {
            let mut it = q.rsplit("::");
            let m = it.next().unwrap_or("");
            let t = it.next().unwrap_or("");
            format!("{t}::{m}")
        };
        let real: std::collections::HashSet<String> =
            fns.iter().filter(|f| !f.derive_synth).map(|f| tail2(&f.qual)).collect();
        fns.retain(|f| !f.derive_synth || !real.contains(&tail2(&f.qual)));
    }
    // The surviving declaration units, for the report's `declaration` key (R894).
    let decl_quals: std::collections::HashSet<String> =
        fns.iter().filter(|f| f.extern_decl).map(|f| f.qual.clone()).collect();
    // SOUNDNESS R1675 — every module path the scan's own quals sit under (each proper prefix of a unit qual).
    let qual_mods: HashSet<String> = fns
        .iter()
        .flat_map(|f| {
            let segs: Vec<&str> = f.qual.split("::").collect();
            (1..segs.len()).map(move |i| segs[..i].join("::")).collect::<Vec<_>>()
        })
        .collect();
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
    // SOUNDNESS R1157 — macro-made primitive operator impl units by tail2 (`FnInfo::mbe_prim_op`).
    let mut by_primop: HashMap<String, Vec<String>> = HashMap::new();
    // SOUNDNESS R1180 — a MACRO-MADE nominal operator impl's units (`FnInfo::mbe_op`, R1146) whose `Type::method` tail a
    // HAND-WRITTEN unit also claims, by that tail. R1146 put them in `by_tail2` beside the hand-written claimant, which made
    // the tail contested: `resolve_target` refused it and the edge to the hand-written impl was TRADED for an `ambiguous:`
    // reason (EXECUTED, rustagent-v061/fx/v061 `na_hand`: `n - N(1)` over an effectful hand `impl Sub for N`, `deny Fs` 1
    // on the pre-R1146 binary, 0 on 0.40.11, because a pure macro-made `impl Sub<u8> for N` sat in another module). Rust
    // picks between them by the right operand's type, which this typer cannot prove (R1128), so neither may displace the
    // other: the hand-written claimant keeps every route it had before R1146, and these units are judged at each site that
    // reaches the tail — `Unknown` with the same `ambiguous:` reason VEIN A uses, where one carries something the caller
    // lacks (`veina_contested`). Never an edge (a union over distinct definitions is a fabrication — only one runs). A
    // macro-made unit with NO hand-written rival is untouched: it stays in `by_tail2` and resolves as R1146 made it.
    let mut mbe_side: HashMap<String, Vec<String>> = HashMap::new();
    let (hand_t2, hand_leaf): (HashSet<String>, HashSet<String>) = {
        let mut t: HashSet<String> = HashSet::new();
        let mut l: HashSet<String> = HashSet::new();
        for f in fns.iter().filter(|f| !f.mbe_op && !f.mbe_prim_op && !f.extern_decl && !f.reexport_alias && !f.derive_synth) {
            if let Some(t2) = tail2(&f.qual) {
                t.insert(t2);
            }
            // `by_leaf` holds no lazy-init unit (see below), so one cannot contest a leaf there either.
            if !f.qual.starts_with(LAZY_UNIT_PREFIX) {
                l.insert(f.leaf.clone());
            }
        }
        (t, l)
    };
    // SOUNDNESS R1154 — the crate ROOT's own free fns, by their exact (one-segment) qual.
    let mut by_root: HashMap<String, Vec<String>> = HashMap::new();
    // Type names with a LOCAL definition — the penultimate `Type` segment of a `Type::method` qual. A
    // receiver-typed method call resolves to a local method ONLY if its type is in here, so an external
    // `reqwest::Client::send` can't mis-link to a same-named local `Client::send` (an inverse fabrication).
    let mut local_types: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &fns {
        // SYNTHETIC lazy-init units (`<lazy>::NAME`) are resolved ONLY via the qualified `<lazy>::`
        // tail2 route a forcing site emits — they must NOT enter `by_leaf`, or a bare call to a real fn
        // sharing the static's NAME would see an ambiguous leaf and stop resolving (a spurious
        // under-report on unrelated code). Their tail2 (`<lazy>::NAME`) is unique and the forcing edge
        // always qualifies, so keeping them out of `by_leaf` loses nothing.
        // SOUNDNESS R894 — a published foreign declaration's unit is for the REPORT; in-crate calls keep
        // answering through `extern_fns`, so it joins neither local resolution index.
        // SOUNDNESS R1157 — a macro-made primitive operator impl's unit: published only, indexed for the disclosure.
        if f.mbe_prim_op {
            if let Some(t2) = tail2(&f.qual.replace(&format!("::{}", crate::model::MBE_PRIM_SEG), "")) {
                let v = by_primop.entry(t2).or_default();
                if !v.contains(&f.qual) {
                    v.push(f.qual.clone());
                }
            }
            continue;
        }
        if f.extern_decl || f.reexport_alias || f.derive_synth {
            continue;
        }
        // SOUNDNESS R1180 — a macro-made operator unit a hand-written unit's tail already claims: kept out of both
        // resolution indexes (see `mbe_side`). Its type still counts as local.
        if f.mbe_op {
            if let Some(t2) = tail2(&f.qual).filter(|t2| hand_t2.contains(t2)) {
                if let Some(ty) = t2.split("::").next() {
                    if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                        local_types.insert(ty.to_string());
                    }
                }
                let v = mbe_side.entry(t2).or_default();
                if !v.contains(&f.qual) {
                    v.push(f.qual.clone());
                }
                continue;
            }
        }
        let is_lazy_unit = f.qual.starts_with(LAZY_UNIT_PREFIX);
        if !is_lazy_unit && !(f.mbe_op && hand_leaf.contains(&f.leaf)) {
            // SOUNDNESS R222/R129 — these buckets are the input to a UNIQUENESS test, so they must hold
            // DISTINCT DEFINITIONS, not `FnInfo` entries. `fns` carries one entry per analysed UNIT and
            // several units routinely share one qual: two `#[cfg]` arms of a fn or of its module, ≥2
            // impls of one trait for one type differing only in type params (`jiff`'s
            // `ZonedArithmetic::from` ×3), an `impl Trait for [T]`/`(T,)`/`&T` whose type segment
            // `lang.rs`'s `impl_type_name` cannot form and drops (`half`'s `convert_from_f32_slice`),
            // an inherent assoc fn beside a trait method of one name, erased generic args (`typenum`'s
            // `private_pow`), and ≥2 DIFFERENT traits with a same-named method (`rustix`'s `Uid::fmt`
            // ×6). Pushing the qual per unit made ONE definition look like two, `resolve_target`'s
            // `len() == 1` filter read "ambiguous", and the call resolved to NOTHING — a lost edge on a
            // call that was never ambiguous. 11,230 functions were in that state; 20,495 of 647,698
            // analysed units (3.16%) in 884 of 1,509 corpus crates are duplicate quals.
            //
            // WHAT THIS IS NOT. It does not deduplicate `all`/`functions[]` — that was tried and
            // reverted (see the four numbered reasons at the `blind_direct` insertion below), and
            // `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`
            // (tests/cli.rs) still pins both units in the report and one violation at the gate; neither
            // claim is touched here. It also does not distinguish the units — R129's own subject, that
            // a per-qual map is a UNION over units that need not agree, is untouched and still open.
            // The two REAL definitions a key can hold (two modules each with a `Bar::go`) stay two, and
            // R190(c)'s refusal over them is unchanged: those are distinct quals sharing a tail.
            //
            // `by_leaf` DEDUPLICATES FOR THE SAME REASON, AND THE SPEC RULING IS WHY IT LANDS SEPARATELY.
            // The bare-leaf spelling does not go silent — it discloses `ambiguous:same-name local defs`
            // (the `v.len() >= 2` site below) — so this half is precision, not the cardinal sin, and it
            // WITHDRAWS an existing answer, which is the move measured and refused for R208. SPEC §4
            // (2026-09-05) settles it: "several bodies under ONE qualified name are one definition, and
            // resolve to the UNION of their effects"; `ambiguous:` is reserved for two DISTINCT
            // definitions competing for one bare name. A deduplicated bucket of size one resolves to
            // that one qual, and a report entry is keyed on the qual and merges its units (R129), so
            // what the caller inherits IS the union of the arms — a COMPLETE answer replacing a hedge,
            // not a withdrawal into silence. Conformance PART 10's `armsunion`/`armsswap` pin it with
            // arms carrying DIFFERENT effects (Fs vs Exec) in both source orders, because same-effect
            // arms cannot separate a union from a PICK, and a pick would be fabrication.
            //
            // WHAT IT COSTS, MEASURED RATHER THAN ASSERTED SAFE (SOUNDNESS R227). The hedge it
            // withdraws was, in a minority of cases, ACCIDENTALLY COVERING a silence that belongs to
            // something else. Over 1,509 registry crates, 1,167 rows lose the reason; 961 of them now
            // point at a target that carries an effect, and the rest at one this engine already
            // reported PURE. All 122 such targets were read from source: nearly all are
            // arithmetic/conversion/platform-shim twins with no effect in either arm, and 32 rows sit
            // over an arm whose effect reaches the machine by a route this engine does not model at all
            // — inline `asm!` and `core::arch` RNG intrinsics (`getrandom`'s `rndr`/`rdrand`), an
            // `extern "Rust"` logger hook (`defmt`'s `export::timestamp`), a `link`ed Win32 call
            // (`cmake`'s `fix_build_dir`), `ffi::sqlite3_threadsafe`, libc `poll`. In EVERY one of
            // those the target is absent from the report on BOTH sides of this change: the silence is
            // at the callee and pre-exists, and what moves is the cover. That is not a reason to keep
            // the hedge — a hedge that happens to sit over an unrelated hole is not a disclosure of it,
            // and it was equally absent from the thousands of QUALIFIED call sites reaching the same
            // callees — but it is why this comment does not claim the change is free. R128/R139 (a
            // macro-declared callee, zero units) and R123/R140 (a `#[cfg]`'d `use` resolved by source
            // order) are the shapes R227 predicted here, and NEITHER appears in that population; the
            // one that does is unmodelled FFI, which is a different open question.
            let vl = by_leaf.entry(f.leaf.clone()).or_default();
            if !vl.contains(&f.qual) {
                vl.push(f.qual.clone());
            }
            if !f.qual.contains("::") && f.qual == f.leaf {
                by_root.entry(f.qual.clone()).or_insert_with(|| vec![f.qual.clone()]);
            }
        }
        if let Some(t2) = tail2(&f.qual) {
            if let Some(ty) = t2.split("::").next() {
                if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                    local_types.insert(ty.to_string());
                }
            }
            let v = by_tail2.entry(t2).or_default();
            if !v.contains(&f.qual) {
                v.push(f.qual.clone());
            }
        }
    }
    // SOUNDNESS R1265 — THE BARE-CALL VIEW OF `by_leaf`. A bare call (`main(x)`: no `::`, not a method call) names a
    // free fn in scope and NEVER an associated one — Rust reaches an `impl` member or a trait default only through
    // `Self::`/`Type::`/`Trait::` or a method call. `by_leaf` keys every unit, so a free `main` beside an inherent
    // `Attrs::main(&self)` made the leaf two-way: `resolve_target` refused it and the call was traded for `ambiguous:same-name
    // local defs` — a reason that is false, over a call that names exactly one definition. EXECUTED (rustab-a/fx/amb):
    // `caller` reads `HOME` through `main(1)` and `deny Env caller` exited 0, hand-written method and (after R1245
    // expanded `methods!(main, ..)`) macro-made alike; wasm-bindgen-macro-support's `Item::macro_parse` (`return
    // main(program, f, tokens)` beside `BindgenAttrs::main`) lost Env on the corpus.
    //
    // A RESOLUTION: an associated claimant is dropped only where a NON-associated one remains, so the bucket can only
    // shrink toward the definitions a bare path can name. Where EVERY claimant is associated, the bucket is kept as it
    // was — that call names something this crate does not declare as a free fn (a glob import, a prelude fn, a
    // macro-hidden fn), and the existing answer (classifier suppressed, `ambiguous:` hedge at two or more) is left
    // exactly as it stood rather than turned into a silence. Two or more FREE claimants still hedge.
    // Used ONLY where a bare non-method call is resolved (`resolve_target`, which reads `by_leaf` for nothing else)
    // and at the bare-leaf `ambiguous:` site; `suppress_bare_leaf` and the method-call routes keep `by_leaf`.
    let by_leaf_call: HashMap<String, Vec<String>> = {
        let mut free_qual: HashSet<&str> = HashSet::new();
        for f in fns.iter().filter(|f| !f.assoc) {
            free_qual.insert(f.qual.as_str());
        }
        by_leaf
            .iter()
            .map(|(leaf, v)| {
                let free: Vec<String> = v.iter().filter(|q| free_qual.contains(q.as_str())).cloned().collect();
                if free.is_empty() || free.len() == v.len() {
                    (leaf.clone(), v.clone())
                } else {
                    if candor_report::env::trace("CANDOR_R1265_INSTR") {
                        eprintln!("R1265LEAF\t{leaf}\t{}\t{}", v.len(), free.len()); // §E1 REACH COUNTER
                    }
                    (leaf.clone(), free)
                }
            })
            .collect()
    };
    // VEIN A — AN `impl` WRITTEN ON A LOCAL TYPE ALIAS IS ALSO AN `impl` OF THE ALIAS'S TARGET.
    // `pub type TzifOwned = Tzif<…>;` + `impl TzifOwned { fn parse64(..) }` (jiff-0.2.28) keys the
    // method under the ALIAS's name, while a receiver whose type is spelled through the alias resolves to
    // the TARGET (`expand` follows a module-qualified alias, and once `use super::TzifOwned` is anchored
    // it is followable): `tzif.parse_indicators(..)` became `Tzif::parse_indicators`, matched nothing,
    // and `TzifOwned::parse64` lost a real `Log`. The target tail is ADDED only where no definition
    // already claims it — so this can supply a resolution and can never turn an existing unique one
    // into an ambiguity.
    // SOUNDNESS R986 — the crate's path-redirected modules (see `decls::REDIRECT_MOD_MARK`).
    // Ordered MOST SPECIFIC first (longest, then lexical): the consumer takes the first module a path is
    // under, and `mod_aliases` is a HashMap, so in key order a path under both `crate::backend` and
    // `crate::backend::libc::c` named either one run to run (measured on rustix-0.37.11: the published
    // binary printed both reasons across four runs of the same tree).
    let mut redirect_mods: Vec<String> = merged
        .mod_aliases
        .keys()
        .filter_map(|k| k.strip_prefix(crate::decls::REDIRECT_MOD_MARK).map(String::from))
        .collect();
    redirect_mods.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    // SOUNDNESS R1475 — each redirected module's ARMS: every module path one of its `mod` items is analysed under (a twin
    // placed at the module's own name included). A call written through the module is resolved in each arm below.
    let redirect_arms: HashMap<String, Vec<String>> = merged
        .mod_aliases
        .iter()
        .filter_map(|(k, v)| {
            let own = k.strip_prefix(crate::decls::REDIRECT_MOD_MARK)?;
            Some((own.to_string(), v.split(crate::decls::ALIAS_ALT_SEP).filter(|a| !a.is_empty()).map(String::from).collect()))
        })
        .collect();
    let mut alias_tails: Vec<(String, String)> = Vec::new();
    for (q, t) in &merged.mod_aliases {
        if t.contains(crate::decls::ALIAS_ALT_SEP) || q.starts_with(crate::decls::REDIRECT_MOD_MARK) {
            continue;
        }
        let a_leaf = q.rsplit("::").next().unwrap_or(q);
        let t_body = t.strip_prefix("crate::").unwrap_or(t);
        let t_root = t_body.split("::").next().unwrap_or(t_body);
        if t_body.contains('<') || manifest_deps.contains(t_root) || matches!(t_root, "std" | "core" | "alloc") {
            continue; // an external target is the classifier's, not an `impl` this crate wrote
        }
        let t_leaf = t_body.rsplit("::").next().unwrap_or(t_body);
        if a_leaf != t_leaf && local_types.contains(a_leaf) {
            alias_tails.push((a_leaf.to_string(), t_leaf.to_string()));
        }
    }
    alias_tails.sort();
    alias_tails.dedup();
    for (a_leaf, t_leaf) in &alias_tails {
        let adds: Vec<(String, Vec<String>)> = by_tail2
            .iter()
            .filter_map(|(k, v)| {
                let m = k.strip_prefix(&format!("{a_leaf}::"))?;
                Some((format!("{t_leaf}::{m}"), v.clone()))
            })
            .collect();
        if !adds.is_empty() {
            // The target is a type this crate declares (`pub struct Tzif<T>`), and a typed call on it is
            // admitted only for a type in `local_types` — which is built from unit quals, and an alias's
            // target that only ever receives `impl`s through the alias has none of its own.
            local_types.insert(t_leaf.clone());
        }
        for (k, v) in adds {
            if let std::collections::hash_map::Entry::Vacant(slot) = by_tail2.entry(k) {
                if candor_report::env::trace("CANDOR_R186_DEBUG") {
                    eprintln!("VEINAALIASIMPL {} -> {}", slot.key(), v.join(",")); // §E1 REACH COUNTER
                }
                slot.insert(v);
            }
        }
    }
    // SOUNDNESS R1034 (residual) — A NON-PATH IMPL'S METHODS ANSWER TO THEIR REFERENT'S RECEIVER. `impl Encode
    // for &str` (no colliding free fn) files `encode` under the module path, so `s.encode()` — typed
    // `str::encode` — and `f.tk()` on a `&Foo`/`Foo` receiver — typed `Foo::tk` — matched no tail and the
    // caller read PURE over a body that writes. The referent tail is ADDED only where no definition already
    // claims it (VEIN A's discipline above), and only where the unit it names exists, so it can supply an
    // edge and never displace a unique resolution or split one into an ambiguity. `nonpath_tails` admits the
    // tail to `resolvable` without putting `str` in `local_types`, which would re-route EVERY typed `str`
    // call in the crate. Two modules both implementing the member for one referent leave the tail with two
    // claimants, which `resolve_target` refuses and R451 (a) discloses `ambiguous:` — never a pick.
    let mut nonpath_tails: HashSet<String> = HashSet::new();
    // t2 -> the MERGED unit quals that tail names (R1034's shared case; judged post-fixpoint, see below).
    let mut nonpath_shared: HashMap<String, Vec<String>> = HashMap::new();
    // SOUNDNESS R1649 — t2 -> (unit qual, trait leaf) of an `impl Tr for &T` / `&mut T` (module-keyed, R1034) whose tail
    // `T::m` is ALREADY claimed by another impl, so the fallback below does not file it. Read at the typed call site.
    let mut r1649_ref: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let r1649_off = candor_report::env::switch("CANDOR_R1649_OFF");
    {
        let have: HashSet<&str> = fns.iter().filter(|f| !f.extern_decl && !f.reexport_alias).map(|f| f.qual.as_str()).collect();
        let mut adds: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for e in &merged.nonpath_receivers {
            let mut parts = e.split('\u{1f}');
            let (Some(t2), Some(q), Some(shared)) = (parts.next(), parts.next(), parts.next()) else { continue };
            if by_tail2.contains_key(t2) && have.contains(q) && shared == "0" && !r1649_off {
                if let Some(tr) = parts.next().filter(|t| !t.is_empty()) {
                    let v = r1649_ref.entry(t2.to_string()).or_default();
                    if !v.iter().any(|(x, _)| x == q) {
                        v.push((q.to_string(), tr.to_string()));
                    }
                }
            }
            if by_tail2.contains_key(t2) || !have.contains(q) {
                continue;
            }
            if shared == "1" {
                // The unit is a MERGE of several impls' bodies: an edge would charge a sibling's effect
                // to this receiver. Disclose at the call site instead (`nonpath_shared`).
                let v = nonpath_shared.entry(t2.to_string()).or_default();
                if !v.iter().any(|x| x == q) {
                    v.push(q.to_string());
                }
                continue;
            }
            let v = adds.entry(t2.to_string()).or_default();
            if !v.iter().any(|x| x == q) {
                v.push(q.to_string());
            }
        }
        for (t2, v) in adds {
            if candor_report::env::trace("CANDOR_R1034_INSTR") {
                eprintln!("R1034RECV\t{t2}\t{}", v.join(",")); // §E1 REACH PROBE
            }
            nonpath_tails.insert(t2.clone());
            by_tail2.insert(t2, v);
        }
    }
    // ⟨peek-scope-attribution⟩ `(trait_leaf, method_leaf) -> the in-scope fns that DIRECTLY dispatch
    // there` (see `FnInfo::dispatch`'s doc comment for why this exists and what it does NOT do — it
    // never contributes an effect, only a reachability fact). Built once, from this run's own Pass-B
    // output; consulted only by the ⟨0.29⟩ peek's out-of-scope block, far below, to test a policy's scope
    // against every in-scope function that could REACH a peeked declaration via dynamic dispatch — not
    // only the peeked declaration's own name.
    // ⟨0.39⟩ `foreign_impls` under the 2-SEGMENT TAIL of the member. Since R503 the wire value and this
    // index agree on spelling, so the tail is no longer the reconciliation it was built as — it is the
    // fallback for the two cases the exact key cannot cover: an owner that RE-EXPORTS its abstraction
    // from a module other than the one declaring it, and a dependency report written by a pre-R503
    // producer, which spelled the member by LEAF. Built once. A tail two keys share is
    // MERGED rather than dropped — the value is a list of implementor quals and the union over them is
    // what the rung publishes, which is §4 bounded CHA, not a choice between two traits.
    // SOUNDNESS R1748 — (tail key, implementor qual) -> the trait paths inside the owner that qual implements, for the tail
    // joins below that drop the module (`ser::Error` and `de::Error` share `serde_core#Error::custom`).
    let mut r1748_paths: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    let foreign_impls_by_tail2: HashMap<String, Vec<String>> = {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in merged.foreign_impls.iter() {
            if let Some((owner, member)) = k.split_once('#') {
                if let Some(t2) = tail2(member) {
                    let tk = format!("{owner}#{t2}");
                    let tp = member.rsplit_once("::").map_or(member, |x| x.0);
                    for q in v {
                        r1748_paths.entry((tk.clone(), q.clone())).or_default().insert(tp.to_string());
                    }
                    let e = m.entry(tk).or_default();
                    for q in v {
                        if !e.contains(q) {
                            e.push(q.clone());
                        }
                    }
                }
            }
        }
        // SOUNDNESS R1345 — an implementor whose trait NAME came through a dependency's glob has no full key, but its
        // tail is exactly `{glob root}#{Trait}::{member}`, so it joins here and nowhere on the wire (see
        // `lang::collect_glob_foreign_impls`). Only for a real manifest dependency, and never for a leaf this crate
        // declares a trait of (a local glob's trait is local). `CANDOR_R1345_OFF=1` disables it.
        if !candor_report::env::switch("CANDOR_R1345_OFF") {
            for g in &merged.glob_impl_foreign {
                let Some((k, q)) = g.split_once('\u{1f}') else { continue };
                let q = q.split('\u{1f}').next().unwrap_or(q); // R1715 — the third field is the glob base
                let Some((owner, member)) = k.split_once('#') else { continue };
                let tr_leaf = member.split("::").next().unwrap_or("");
                if !deps.contains(owner) || merged.trait_decls.contains_key(tr_leaf) {
                    continue;
                }
                if crate::lang::reach_debug() {
                    eprintln!("R1345GLOB\t{k}\t{q}"); // §E1 REACH COUNTER
                }
                let e = m.entry(k.to_string()).or_default();
                if !e.iter().any(|x| x == q) {
                    e.push(q.to_string());
                }
            }
        }
        m
    };
    // SOUNDNESS R1715 — THE CONSUMER'S IMPLEMENTORS UNDER THE CRATE A FACADE RE-EXPORTS FROM. An obligation-2 key is
    // formed from THIS crate's own `use` map (`lang::foreign_trait_owner_qual`), so `impl serde::de::Visitor for V` keys
    // `serde#de::Visitor::visit_u64` — while `serde` only re-exports `serde_core::de` (`pub use serde_core::{de, ..}` in
    // serde 1.0.228's root), and a producer keys the dispatch under the trait's true owner, `serde_core#…` (⟨0.39⟩,
    // obligation 1). Nothing followed the facade, so the join missed and the consumer read pure over its own implementor:
    // EXECUTED (`deny Fs` 1 on 0.41.3 -> 0) once R1710 made serde_json publish `serde_core#` instead of the alias's
    // accidental `serde#`, and on every arm for a serde_core producer key. Each implementor key whose owner is a manifest
    // dependency is resolved through that dependency's lock-pinned root re-exports (`DepSigs::facade_owners`, SPEC §2
    // ⟨0.42⟩ `dependencySources`: the reading only ADDS a join, and the package read is recorded when it answers one), and
    // filed here under the owner's 2-segment tail. ADDED at the join beside the exact / tail2 hit, never instead of it.
    // R1345's glob records resolve through their glob base. `CANDOR_R1715_OFF=1` disables it.
    // …and where the facade's source CANNOT be read (no Cargo.lock, a version not checked out), the trait's 2-segment tail
    // (`Visitor::visit_u64`) -> (the dependency this crate implements it under, the implementor qual), for the join below.
    // (owner alias, implementor qual, the trait path inside the owner — R1739's proof asks it)
    let mut facade_blind: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    let facade_t2: HashMap<String, (Vec<String>, BTreeSet<String>)> = {
        let mut m: HashMap<String, (Vec<String>, BTreeSet<String>)> = HashMap::new();
        if !candor_report::env::switch("CANDOR_R1715_OFF") {
            // (owner alias, trait path inside it, method, implementor qual)
            let mut todo: Vec<(String, String, String, String)> = Vec::new();
            for (k, v) in merged.foreign_impls.iter() {
                let Some((owner, member)) = k.split_once('#') else { continue };
                let Some((tp, meth)) = member.rsplit_once("::") else { continue };
                for q in v {
                    todo.push((owner.to_string(), tp.to_string(), meth.to_string(), q.clone()));
                }
            }
            for g in &merged.glob_impl_foreign {
                let mut it = g.split('\u{1f}');
                let (Some(k), Some(q), Some(base)) = (it.next(), it.next(), it.next()) else { continue };
                let Some((owner, member)) = k.split_once('#') else { continue };
                let Some((leaf, meth)) = member.rsplit_once("::") else { continue };
                let Some((broot, bpath)) = base.split_once("::") else {
                    todo.push((owner.to_string(), leaf.to_string(), meth.to_string(), q.to_string()));
                    continue;
                };
                if broot == owner && !merged.trait_decls.contains_key(leaf) {
                    todo.push((owner.to_string(), format!("{bpath}::{leaf}"), meth.to_string(), q.to_string()));
                }
            }
            todo.sort();
            todo.dedup();
            let mut sigs = r1578_sigs.borrow_mut();
            for (owner, tp, meth, q) in todo {
                if !deps.contains(owner.as_str()) {
                    continue;
                }
                let real = dep_renames.get(owner.as_str()).cloned().unwrap_or_else(|| owner.clone());
                if sigs.facade(dir, &owner, &real).is_none() {
                    if let Some(t2) = tail2(&format!("{tp}::{meth}")) {
                        let e = facade_blind.entry(t2).or_default();
                        if !e.iter().any(|(o, x, _)| o == &owner && x == &q) {
                            e.push((owner.clone(), q.clone(), tp.clone()));
                        }
                    }
                    continue;
                }
                for (o, rest, via) in sigs.facade_owners(dir, &owner, &real, &tp) {
                    let Some(t2) = tail2(&format!("{rest}::{meth}")) else { continue };
                    let key = format!("{o}#{t2}");
                    if crate::lang::reach_debug() {
                        eprintln!("R1715FACADE\t{owner}#{tp}::{meth}\t{key}\t{q}"); // §E1 REACH COUNTER
                    }
                    r1748_paths.entry((key.clone(), q.clone())).or_default().insert(rest.clone());
                    let e = m.entry(key).or_default();
                    if !e.0.contains(&q) {
                        e.0.push(q.clone());
                    }
                    e.1.extend(via);
                }
            }
        }
        m
    };
    // ⟨0.40⟩ SOUNDNESS R608 — THE TRAIT-PREFIX PROJECTION OF BOTH IMPLEMENTOR INDEXES.
    //
    // The consumer rule below asks java's conjunct 4 — `chaTargets(owner, name, desc).isEmpty()`,
    // i.e. "does this project implement THAT ABSTRACTION at all". java's query is spelled with a
    // `name`+`desc` off an `INVOKEINTERFACE`, which the JVM verifier guarantees names a member the
    // interface DECLARES, so asking it per-member and asking it per-interface differ only in
    // precision. **THIS ENGINE MINTS ITS KEY FROM SOURCE, WHERE NOTHING GUARANTEES THAT** — R549's
    // class is exactly a member key the owning trait does not declare (`tower_service#Service::map_err`
    // is a `ServiceExt` method, and `ServiceExt` is tower's own trait) — and a MALFORMED member is BY
    // CONSTRUCTION never a key in `foreign_impls`, so a member-keyed conjunct 4 cannot fire on it and
    // the rule charges `Unknown` for a dispatch that does not exist.
    //
    // MEASURED AT THIS BASE, six chained pairs, Σ 2,826 analyzed consumer functions, `REMOVED 0` in
    // every arm: member-keyed moves **39 rows (1.38%) on 35 direct hits**; TRAIT-PREFIX-keyed moves
    // **6 rows (0.21%) on 3**. **34 of those 35 member-keyed hits are on MALFORMED keys**, and that
    // is ground-truthed against each trait's own declaration rather than taken from a row:
    // `tower_service::Service` declares `poll_ready` and `call` (22 hits name `map_err`, `load`,
    // `make_service`, `err_into`, `then`, `map_ok`, `map`, `is_pending`, `and_then` — all
    // `ServiceExt`, which is TOWER's trait); `http_body::Body` declares `poll_frame`/`is_end_stream`/
    // `size_hint` (`as_mut`, `exact` — 3); `bytes::Buf` declares neither `map` nor
    // `unwrap_or_default` (2); `futures_core`'s `Stream`/`FusedStream`/`TryStream` declare
    // `poll_next`+`size_hint` / `is_terminated` / `try_poll_next` (7 more). The ONE well-formed hit is
    // `futures_core#TryStream::try_poll_next`. So the prefix is not a tuning knob picked to fit a
    // threshold — it is the granularity java's query actually has, and porting it per-member was the
    // mis-port. (An earlier pricing on `0c0a0fd` measured 45/6 for the same two arms; R569 landed in
    // between and moved the member-keyed figure to 39. The prefix figure did not move.)
    //
    // `{owner}#{trait qual}::{method}` -> `{owner}#{trait qual}::`. `None` for a key with no `::`:
    // no trait qual can be formed, so there is nothing to compare and the caller falls through.
    let r533_fi_prefix: HashSet<String> =
        merged.foreign_impls.keys().filter_map(|k| r533_trait_prefix(k)).collect();
    // SOUNDNESS R828 — `"{trait leaf}::{method}"` -> the units that implement it for a NON-NOMINAL self
    // type, and the set of unit quals that exist, so a recorded qual that names no unit is hedged rather
    // than trusted.
    let nonnominal: HashMap<String, Vec<String>> = {
        let mut m: HashMap<String, Vec<String>> = HashMap::new();
        for e in &merged.nonnominal_impls {
            if let Some((k, q)) = e.split_once('\u{1f}') {
                m.entry(k.to_string()).or_default().push(q.to_string());
            }
        }
        m
    };
    let unit_quals: HashSet<String> = fns.iter().map(|f| f.qual.clone()).collect();
    // SOUNDNESS R1537 (c) — CO-PATH UNITS (SPEC §2 ⟨0.42⟩): AN INHERENT METHOD AND A TRAIT'S IMPLEMENTATION OF ONE NAME ON ONE
    // TYPE ARE TWO UNITS. Both are keyed `Ty::m` (`lang::impl_type_name`), so they shared one qual and were ONE merged unit: its
    // row (printed twice, at the inherent's span) carried the union, every caller of either was charged both bodies (R1420 (c),
    // a fabrication EXECUTED on published 0.40.16), and the report spelled no unit a consumer could tell apart. Each member is
    // renamed here to the bare qual plus a disambiguator that ENDS the name — `Ty::m(Ty)` for the inherent, `Ty::m(Tr)` for
    // the trait's — so each has its own effects, row, hash and sidecar key.
    //
    // THE BARE QUAL STAYS AS AN INTERNAL UNION NODE, NEVER PUBLISHED: every index built above still names it, and every route
    // that forms an edge to it without proving which member it reaches keeps doing so, so that edge joins the UNION of the
    // members (calls `bare -> {inherent, trait}` below) — exactly what the merged unit gave, so no route can lose an effect it
    // reached before. Only a site that PROVES its member (`copath_member`) edges that member alone. The published report spells
    // an edge to the bare node as the edge to every member (`pub_calls`), and a chained consumer joins the bare path to the
    // union of the members (`deps.rs`).
    //
    // Narrow by construction: every body under the qual is a hand-written impl member (`FnInfo::impl_kind`), at least one
    // inherent and at least one of exactly ONE trait. Two traits (R1342), a module fn or a macro-made unit under the same
    // qual, a trait leaf equal to the type's, are left merged as before. Within a side, several bodies (cfg arms, generic
    // twins) stay one unit, as they were. `CANDOR_R1537C_OFF=1` disables it; `CANDOR_R1537C_INSTR` prints each split and pick.
    //
    // THE RENAME HAPPENS HERE, right after the last TARGET index (`unit_quals`; `by_tail2`/`by_leaf` above) is built from the
    // bare quals and before the first CALLER-keyed structure (`direct_dispatchers`, the R1095 `untyped_contested` sites, the
    // call loop): a caller-keyed fact recorded under the bare qual would land on the unpublished union node and leave the
    // member whose body raised it reading pure (measured on the first corpus A/B: addr2line `FrameIter::next`'s inherent
    // body lost its `dispatch:untyped receiver` hedge when the rename sat just before the call loop).
    let r1537c_instr = candor_report::env::trace("CANDOR_R1537C_INSTR");
    let copath: HashMap<String, CoPathC> = if candor_report::env::switch("CANDOR_R1537C_OFF") {
        HashMap::new()
    } else {
        copath_sets(&fns, &unit_quals)
    };
    if !copath.is_empty() {
        for f in fns.iter_mut() {
            if let Some(cp) = copath.get(f.qual.as_str()) {
                f.qual = if f.impl_kind == 1 { cp.inh.clone() } else { cp.tr.clone() };
            }
        }
        if r1537c_instr {
            for (q, cp) in &copath {
                eprintln!("R1537CSPLIT\t{q}\t{}\t{}", cp.inh, cp.tr); // §E1 REACH COUNTER
            }
        }
    }
    // VEIN C — a LOCAL type's impls of a DEPENDENCY's trait, `type leaf -> [(trait leaf, expanded trait
    // path)]`, read off R652's `!` key (the path each `impl` block wrote). Consulted only where a typed
    // method call on that type resolved to nothing local; see the inherited-dependency-default arm.
    let dep_traits_by_ty: HashMap<String, Vec<(String, String)>> = {
        let mut m: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for k in &merged.impl_members {
            let mut it = k.splitn(3, '\u{1f}');
            let (Some(tr), Some(ty), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
            let Some(path) = p.strip_prefix('!') else { continue };
            let Some((root, _)) = path.split_once("::") else { continue };
            if crate::lang::is_dependency_crate_root(root) && deps_idx.crates.contains(root) {
                m.entry(ty.to_string()).or_default().push((tr.to_string(), path.to_string()));
            }
        }
        m
    };
    // SOUNDNESS R1607 — VEIN C's map for an UNCHAINED dependency too: a local type's impls of any declared dependency's trait.
    let r1607_dep_traits_by_ty: HashMap<String, Vec<(String, String)>> = {
        let mut m: HashMap<String, Vec<(String, String)>> = HashMap::new();
        if !candor_report::env::switch("CANDOR_R1607_OFF") {
            for k in &merged.impl_members {
                let mut it = k.splitn(3, '\u{1f}');
                let (Some(tr), Some(ty), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
                let Some(path) = p.strip_prefix('!') else { continue };
                let Some((root, _)) = path.split_once("::") else { continue };
                if crate::lang::is_dependency_crate_root(root) && manifest_deps.contains(root) && !deps_idx.crates.contains(root) {
                    m.entry(ty.to_string()).or_default().push((tr.to_string(), path.to_string()));
                }
            }
        }
        m
    };
    // SOUNDNESS R1171 — `(trait leaf, self-type leaf)` -> each `impl` block's WRITTEN self path, its declared members and
    // whether a macro item blinds them (`model::impl_path_key`). Read only by the twin-default rule in the call loop.
    type WpathBlock = (String, HashSet<String>, bool); // (written self path, declared members, a macro item blinds them)
    let wpath_blocks: HashMap<(String, String), Vec<WpathBlock>> = {
        let mut m: HashMap<(String, String), Vec<WpathBlock>> = HashMap::new();
        for k in &merged.impl_members {
            let Some(rest) = k.strip_prefix("\u{1f}wpath\u{1f}") else { continue };
            let p: Vec<&str> = rest.split('\u{1f}').collect();
            let (tr, ty, w) = match p.as_slice() {
                [tr, ty, w] | [tr, ty, w, _] => (*tr, *ty, *w),
                _ => continue,
            };
            let v = m.entry((tr.to_string(), ty.to_string())).or_default();
            let i = match v.iter().position(|b| b.0 == w) {
                Some(i) => i,
                None => {
                    v.push((w.to_string(), HashSet::new(), false));
                    v.len() - 1
                }
            };
            match p.get(3) {
                Some(&"*") => v[i].2 = true,
                Some(mem) => {
                    v[i].1.insert(mem.to_string());
                }
                None => {}
            }
        }
        m
    };
    let r533_fi_prefix_t2: HashSet<String> =
        foreign_impls_by_tail2.keys().filter_map(|k| r533_trait_prefix(k)).collect();
    let mut direct_dispatchers: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    for f in &fns {
        for site in &f.dispatch {
            direct_dispatchers.entry(site.clone()).or_default().insert(f.qual.clone());
        }
    }
    // The SUBMODULE-level `pub use` alias index — the tails a definition also answers to because a module
    // re-exported it (`pub use self::platform::*` makes `imp::platform::doit` nameable as `imp::doit`).
    // A FALLBACK, never a competitor: `reexport_target` consults it only where `by_tail2` holds nothing at
    // all for the call's tail, so no resolution that worked before can be displaced or made ambiguous.
    // SOUNDNESS R190(e) — `by_reexport_refused` is the keys `reexport_aliases` declined to admit. It is
    // an out-param rather than a second return so the index's own control flow (three `continue`s, each
    // its own reason) stays where it is and is not duplicated by a second pass over the same edges.
    let mut by_reexport_refused: HashMap<String, &'static str> = HashMap::new();
    let by_reexport =
        reexport_aliases(&merged.reexports, &fns, &merged.macro_modules, &mut by_reexport_refused);
    // SOUNDNESS R1690 (R1657 stage 2) — the modules that GLOB re-export (`pub use self::rc::*;`) a module whose items sit
    // inside an unexpanded item-position macro, transitively. They can name ANY of those hidden items: objc2's
    // `ffi::objc_autoreleaseReturnValue` is declared inside `extern_c_unwind! { .. }` in `ffi/rc.rs` and reached as
    // `ffi::..` through `pub use self::rc::*`, so the call named no unit and its caller read PURE on every arm (published
    // 0.41.3 included): `Retained::autorelease_return_option`, `retain`, `drop`. The stage-2 table EXPOSED it — a wrong-
    // reason `ambiguous:` hedge on `define_class::Retained::autorelease_return` resolved to that pure-reading callee. Asked
    // only by R128's macro-opacity hedge below (never by `reexport_aliases`). `CANDOR_R1657S2_OFF` empties it.
    let r1690_glob_macro: HashSet<String> = {
        let mut m: HashSet<String> = HashSet::new();
        if !crate::lang::r1657s2_off() {
            loop {
                let before = m.len();
                for e in merged.reexports.iter().filter(|e| e.name == "*") {
                    if e.from.iter().any(|f| merged.macro_modules.contains(f) || m.contains(f)) {
                        m.insert(e.module.clone());
                    }
                }
                if m.len() == before {
                    break;
                }
            }
        }
        m
    };

    // Inverse of trait_impls (impl-TYPE leaf → the trait leaves it impls), for the trait-DEFAULT-method
    // caller fallback below: a call `t.m()` on a concrete type T that does NOT declare `m` but impls a
    // trait with a DEFAULT `m` should edge to that trait's `Trait::m` (the inherited default body — now
    // scanned, via the Item::Trait arm). Without this the caller silently under-reported (`run()` calling
    // `l.flush()` on a FileLogger that inherits `Logger::flush`'s Fs/Net — adversarial review).
    let mut type_to_traits: HashMap<String, Vec<String>> = HashMap::new();
    for (tr, types) in &merged.trait_impls {
        let tr_leaf = tr.rsplit("::").next().unwrap_or(tr).to_string();
        for ty in types {
            let ty_leaf = ty.rsplit("::").next().unwrap_or(ty).to_string();
            type_to_traits.entry(ty_leaf).or_default().push(tr_leaf.clone());
        }
    }
    // ⟨peek-scope-attribution⟩ THIS run's own `type_to_traits`, published for the ENCLOSING (primary)
    // frame to read — see `gate::PEEK_TYPE_TO_TRAITS`'s doc comment. Only meaningful, and only written,
    // when THIS scan_one call IS the peek (`peek_excluded`): the peek walks excluded files only, so its
    // `type_to_traits` answers "which trait(s) does this EXCLUDED declaration's owning type implement" —
    // exactly the fact the primary frame needs and cannot compute itself (it never parses excluded files
    // at all). A normal (non-peek) scan never touches this thread-local.
    if peek_excluded {
        crate::gate::PEEK_TYPE_TO_TRAITS.with(|c| *c.borrow_mut() = type_to_traits.clone());
    }
    // A type whose ONLY impl is an (empty / non-overriding) trait impl has NO fn unit of its own, so it
    // never entered `local_types` (built from fn quals above) — which made its typed calls un-`resolvable`
    // and GATED OUT the trait-default fallback below: `impl Logger for FileLogger {}` + `l.flush()`
    // inheriting `Logger::flush`'s effect read silent-pure (R30). Every type with a local trait impl IS
    // local — register it so the fallback (already present) can fire.
    for ty in type_to_traits.keys() {
        local_types.insert(ty.clone());
    }

    // Method leaves that name a LOCAL method definition (a `Type::method` qual whose `Type` is local).
    // A bare-leaf method CALL (`x.fastrand()`, recorded path==leaf, no `::`) whose leaf matches one of
    // these resolves to the project's OWN method, so the calibrated-crate classification of that leaf
    // (`fastrand` → Rand, `now` → Clock) must be SUPPRESSED — the local definition is authoritative. This
    // covers the case `resolve_target` deliberately leaves unresolved: a method on a receiver whose type
    // the scanner can't infer (`Mutex::lock()`'s guard, `self.state.lock()` → `MutexGuard<FastRand>`),
    // where no typed `FastRand::fastrand` sibling forms yet the leaf still names a local method. Suppress
    // on PRESENCE of a same-named local method, not on a recorded edge — under-reporting on the rare
    // ambiguous leaf beats fabricating an effect candor never observed (the precision failure). (Real tokio
    // sweep: `RngSeedGenerator::next_seed` calls `rng.fastrand()` through a lock guard → bare leaf
    // `fastrand` → Rand, propagated to ~14 fns incl `Runtime::new`.)
    let mut direct: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    // VEIN A — contested non-method path calls the written path could not anchor: (caller, claimants).
    // Judged after the fixpoint (see `veina_contested` below the first `propagate`).
    let mut veina_contested: Vec<(String, Vec<String>)> = Vec::new();
    // SOUNDNESS R1157 — (caller, `prim::method`, macro-made candidate impl units), judged after the fixpoint.
    let mut primop_contested: Vec<(String, String, Vec<String>)> = Vec::new();
    // SOUNDNESS R1540 — (caller, `StdLeaf::method`, crate impl units): a std-typed receiver's method whose name std also
    // has, beside a crate impl of that name for that std type (`STD_RECV_AMBIG_MARKER`). Judged after the fixpoint.
    let mut stdrecv_contested: Vec<(String, String, Vec<String>)> = Vec::new();
    // SOUNDNESS R1165 — (caller, dependency as spelled, its real name, local type leaf, argument position, callee rest): a
    // local value with a std-trait impl handed to a CHAINED dependency (`WITNESS_MARKER`). Judged after the fixpoint.
    let mut witness_sites: Vec<(String, String, String, String, usize, String)> = Vec::new();
    // SOUNDNESS R1186 — the same, for a local `From`/`TryFrom` impl whose SOURCE the argument names (`model::from_impl_key`):
    // (caller, dependency as spelled, its real name, source leaf, argument position, callee rest).
    let mut from_sites: Vec<(String, String, String, String, usize, String)> = Vec::new();
    // SOUNDNESS R1034 / R1056 merged-sibling hedge — (caller, merged unit quals), judged after the fixpoint.
    let mut nonpath_contested: Vec<(String, Vec<String>)> = Vec::new();
    // SOUNDNESS R1095 — (caller, method leaf, candidate units): a method called on a receiver no route typed,
    // whose name a crate-local `self` method carries. Judged after the fixpoint, below.
    // The crate-local `self` methods by leaf, owned: the R1095 judge below reads them, and SOUNDNESS R1663 adds a site to
    // that judge from the typed-call resolution (a guessed chain hop whose typed tail names no unit).
    let mut self_methods_by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    for f in &fns {
        if f.has_self && !f.extern_decl && !f.reexport_alias {
            let v = self_methods_by_leaf.entry(f.leaf.clone()).or_default();
            if !v.contains(&f.qual) {
                v.push(f.qual.clone());
            }
        }
    }
    // SOUNDNESS R1747 — (caller, leaf) sites whose std-named method was called at a non-std arity (see the collector).
    let mut r1747_nonstd: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut untyped_contested: Vec<(String, String, Vec<String>, bool)> = {
        let mut out = Vec::new();
        for f in &fns {
            for m in &f.untyped_methods {
                // SOUNDNESS R1665 — a dependency-wrapper site (`WRAP_PREFIX`) counts an `Unknown`-only candidate (narrow population).
                let (leaf, admit) = match m.strip_prefix(crate::lazy::WRAP_PREFIX) {
                    Some(l) => (l, true),
                    None => (m.as_str(), false),
                };
                let leaf = match leaf.strip_prefix(crate::lazy::NONSTD_ARITY_PREFIX) {
                    Some(l) => {
                        r1747_nonstd.insert((f.qual.clone(), l.to_string()));
                        l
                    }
                    None => leaf,
                };
                // SOUNDNESS R1692 — a site whose only typing was an `<untyped>` marker: judged as untyped exactly when the
                // marker is inert, i.e. no root is a declared dependency (the call loop's and `r1578_resolve`'s conjunct). A
                // dependency root is consumed there (`Unknown` chained, κ `invisible` unchained) and is not judged twice.
                let leaf = match leaf.strip_prefix(crate::lazy::INERT_MARK_PREFIX) {
                    Some(rest) => {
                        let Some((roots, l)) = rest.split_once('\u{1}') else { continue };
                        if candor_report::env::switch("CANDOR_R1692_OFF") || roots.split(',').any(|r| deps.contains(r)) {
                            continue;
                        }
                        if candor_report::env::trace("CANDOR_R1692_INSTR") && self_methods_by_leaf.contains_key(l) {
                            eprintln!("R1692SITE\t{}\t{roots}\t{l}", f.qual); // §E1 REACH COUNTER, on the ADDED branch only
                        }
                        l
                    }
                    None => leaf,
                };
                if let Some(c) = self_methods_by_leaf.get(leaf) {
                    out.push((f.qual.clone(), leaf.to_string(), c.clone(), admit));
                }
            }
        }
        out
    };
    // …and, per fn, the external-looking calls NOTHING handled (no classification, no local definition, no
    // dependency join): `mydep::touch()` in a scan that cannot see the manifest drops with no trace at all, so
    // a merged unit reading pure is not EVIDENCE of purity where its bodies (transitively) contain one.
    let mut unresolved_ext: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut hosts: HashMap<String, BTreeSet<String>> = HashMap::new();
    // SPEC §2 `fs` — DIRECT ONLY, deliberately, matching candor-java's `fsDirect`, candor-swift and
    // candor-ts. It must NOT be propagated over call edges: a caller reaching one callee that writes and
    // another whose kind is undetermined would inherit `["write"]` and thereby claim "writes but never
    // reads", the partial claim §2 forbids. Direct-only keeps the field a statement about calls this
    // function makes itself, where every contributing verb was seen.
    let mut fskinds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut cmds: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut paths: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut tables: HashMap<String, BTreeSet<String>> = HashMap::new();
    // Effects whose literal SURFACE is INCOMPLETE for a fn: it has a Net reach whose host is invisible to
    // the gate (a Net call with no string-literal arg — a runtime host, or a builder terminal whose host was
    // on a pure builder candor doesn't capture). The AS-EFF-008 gate treats an incomplete surface as
    // uncertifiable EVEN with other visible hosts, so a benign literal can't MASK the invisible endpoint
    // (the same gate evasion fixed in candor-java 0.5.29). Generalized from Net to Exec/Fs/Db (a masked
    // path/table alongside a benign sibling literal defeated `opaque` and silently passed `allow Fs`/
    // `allow Db`) — the establishing-allowlist predicate per effect (is_net_establishing /
    // is_cmd_naming_method / is_fs_path_arg / is_db_query_arg), matching candor-java's surfaceIncomplete.
    let mut incomplete: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    let mut calls: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut loc: HashMap<String, String> = HashMap::new();
    // SOUNDNESS R959 — A RENAMED RE-EXPORT IS A NAME A CONSUMER CALLS. `pub use inner::eff as reff;`
    // published nothing under `reff` (no row, no call-graph node), so a chained consumer's
    // `ydep::reff()` matched no unit of a COVERED crate and read PURE — ABSENT, `deny Fs` 0 — while the
    // un-renamed `pub use inner2::eff2` beside it resolved by its leaf. A unit per renamed re-export of a
    // crate-local fn, with ONE call edge to each `#[cfg]` arm's definition, publishes exactly the answer
    // the in-crate route gives a caller of `reff` (which already resolves through the `use` alias).
    // Only where the target IS a unit this scan analysed — a renamed TYPE, trait, const or an external
    // item names no fn here and gets nothing (an external one is R99's alias route) — and never over a
    // name a real fn of that module already carries.
    //
    // ADDED HERE, AFTER every in-crate resolution index (`by_leaf`, `by_tail2`, the re-export alias index,
    // `unit_quals`) is built, and kept out of all of them: a unit named `construct::mangled_name` beside
    // `pub use self::symbol::mangled as mangled_name` told the alias index the module DECLARES that name,
    // which shadowed the alias and left `construct::mangled_name()` unresolved (measured:
    // `a_renamed_and_a_super_relative_reexport_both_resolve` lost its `Exec`).
    {
        let real: HashMap<&str, &FnInfo> = fns
            .iter()
            .filter(|f| !f.extern_decl)
            .map(|f| (f.qual.as_str(), f))
            .collect();
        let mut add: Vec<FnInfo> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for r in &merged.reexports {
            if r.alias == "*" || r.alias == r.name {
                continue;
            }
            let aq = crate::decls::qualify(&r.module, &r.alias);
            if real.contains_key(aq.as_str()) || !seen.insert(aq.clone()) {
                continue;
            }
            let mut targets: Vec<&FnInfo> =
                r.from.iter().filter_map(|m| real.get(crate::decls::qualify(m, &r.name).as_str()).copied()).collect();
            // SOUNDNESS R1642 (R1657 stage 2) — a rename of a RE-EXPORT (`pub use io::w as root_w;` where `io::w` is
            // itself `pub use rw::w`) names no unit at `io::w`; the re-export table names the definition it denotes.
            if targets.is_empty() {
                if let Some(arms) = crate::lang::reexport_lookup(&aq) {
                    targets = arms
                        .split(crate::decls::ALIAS_ALT_SEP)
                        .filter_map(|a| a.strip_prefix("crate::"))
                        .filter_map(|q| real.get(q).copied())
                        .collect();
                    if !targets.is_empty() && candor_report::env::trace("CANDOR_R1657S2_INSTR") {
                        eprintln!("R1657S2R959\t{aq}\t{arms}", arms = arms.replace(crate::decls::ALIAS_ALT_SEP, "|")); // §E1 REACH
                    }
                }
            }
            let Some(first) = targets.first() else { continue };
            let mut u = (*first).clone();
            u.qual = aq;
            u.leaf = r.alias.clone();
            u.calls = targets
                .iter()
                .filter_map(|t| {
                    serde_json::from_value::<crate::model::Call>(
                        serde_json::json!({ "p": format!("crate::{}", t.qual), "l": t.leaf }),
                    )
                    .ok()
                })
                .collect();
            u.unresolved = false;
            u.unresolved_why = Vec::new();
            u.refusals = Vec::new();
            u.dispatch = Vec::new();
            u.foreign_dispatch = Vec::new();
            u.foreign_dispatch_prov = Vec::new();
            u.untyped_methods = Vec::new();
            u.has_self = false;
            u.reexport_alias = true;
            if candor_report::env::trace("CANDOR_R959_INSTR") {
                eprintln!("R959ALIAS\t{}\t{}", u.qual, first.qual);
            }
            add.push(u);
        }
        fns.extend(add);
    }
    // SOUNDNESS R1420(a) — A TRAIT DEFAULT CALLED ON A CONCRETE IMPLEMENTOR RUNS FOR THAT IMPLEMENTOR.
    // `P.go()`, where `P` inherits `Tr::go` and the default's body is `self.act()`, was edged to the default's ONE unit, whose
    // `self.act()` is a dispatch over EVERY implementor (R576(a)): the caller was charged a sibling `S::act`'s effect over a
    // program that never performs it (EXECUTED, matrix `t_provided_{local,mod}_mixP` solo; chrono's
    // `Parsed::to_datetime` → `FixedOffset.from_local_datetime(..)` charged `Local`'s Clock/Env/Fs, R1528). For each
    // implementor that inherits a default whose body dispatches on `self`, an INSTANCE of the default is minted here: the
    // default's own calls, except that of the edges its `self.m()` formed (`FnInfo::self_disp`) only the one naming THIS
    // implementor stays. The inherited-default route below edges a call whose receiver is typed as that implementor to the
    // instance instead of the default.
    //
    // NARROWING ONLY WHERE THE IMPLEMENTOR IS EXACT, else the default's union stands (over-charge, never a silence): the
    // trait leaf is declared once; the implementor's leaf is a local type declared once (no same-leaf twin, R1246) with ONE
    // block for this trait, whose written path no macro blinds; every member the default dispatches on `self` kept an edge
    // naming the implementor (a `>12`/ambiguous dispatch formed none, so there is nothing to keep); the default unit is
    // unique. Everything else on the default (direct effects, `Unknown`s, refusals, dispatch keys) is copied unchanged.
    // Never published: the report spells the edge as the default it instantiates. `CANDOR_CT3A_OFF=1` disables it.
    // SOUNDNESS R1420(a) — §E1 CENSUS: every unit qual two or more Pass-B bodies share (a merged unit), with each body's
    // location, so the identity merge's reach is MEASURED (R1420 (b)(c), R1342, the `Box<dyn X>` face) rather than guessed.
    if candor_report::env::trace("CANDOR_CT3_CENSUS") {
        let mut by_q: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for f in fns.iter().filter(|f| !f.extern_decl && !f.reexport_alias && !f.derive_synth && !f.mbe_prim_op) {
            by_q.entry(f.qual.as_str()).or_default().push(f.loc.as_str());
        }
        for (q, ls) in by_q.iter().filter(|(_, ls)| ls.len() >= 2) {
            eprintln!("CT3MERGE\t{q}\t{}", ls.join("\t"));
        }
    }
    let mut default_inst: HashMap<(String, String), String> = HashMap::new();
    if !candor_report::env::switch("CANDOR_CT3A_OFF") {
        let mut qcount: HashMap<&str, usize> = HashMap::new();
        for f in &fns {
            *qcount.entry(f.qual.as_str()).or_default() += 1;
        }
        // The implementor must be a NOMINAL type this crate DECLARES (a struct/enum/union in the per-file declared-type
        // surface). A std or primitive self type (`impl Message for bool`) is excluded: its leaf is also how a MODULE of
        // the same name is spelled (prost's macro-made `encoding::bool`, whose `bool::encode(..)` the typer reads as the
        // primitive's inherited `Message::encode`), and there the default's union was the only thing standing over a
        // callee this scan cannot see (EXECUTED on the census corpus: prost `types::bool::encode_raw` lost its `Unknown`).
        let declared: HashSet<&str> = decls_per_file
            .iter()
            .flat_map(|(_, _, fd)| fd.ts.types.iter().map(|(q, _, _)| q.rsplit("::").next().unwrap_or(q)))
            .collect();
        let mut add: Vec<FnInfo> = Vec::new();
        for (tr, impls) in trait_impls.iter() {
            let tr_leaf = tr.rsplit("::").next().unwrap_or(tr);
            if trait_decls.get(tr_leaf).is_none_or(|lt| lt.count != 1) {
                continue;
            }
            let Some(dq) = crate::model::trait_decl_qual(&impl_owners, tr_leaf) else { continue };
            let blocks_all = impl_owners.get(tr_leaf);
            for def in fns.iter().filter(|f| !f.self_disp.is_empty() && f.inst_of.is_none()) {
                let Some(d) = def.qual.strip_prefix(dq).and_then(|r| r.strip_prefix("::")) else { continue };
                if d.contains("::") || qcount.get(def.qual.as_str()) != Some(&1) {
                    continue;
                }
                let members: BTreeSet<&str> = def.self_disp.iter().map(|(_, m)| m.as_str()).collect();
                for ty in impls {
                    let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                    if twin_type_leaves.contains(ty_leaf) || !local_types.contains(ty_leaf) || !declared.contains(ty_leaf) {
                        continue;
                    }
                    let blocks: Vec<&crate::model::ImplOwner> =
                        blocks_all.map(|v| v.iter().filter(|o| &o.ty == ty).collect()).unwrap_or_default();
                    let [blk] = blocks.as_slice() else { continue };
                    if blk.declares(d) {
                        continue; // the implementor overrides `d`: its own unit runs, no default to instantiate
                    }
                    match wpath_blocks.get(&(tr_leaf.to_string(), ty_leaf.to_string())).map(Vec::as_slice) {
                        Some([w]) if !w.2 => {}
                        _ => continue,
                    }
                    let sd: HashMap<usize, &str> = def.self_disp.iter().map(|(i, m)| (*i as usize, m.as_str())).collect();
                    // `self_disp` holds INDICES into `calls`, recorded by the collector and carried through the Pass-B cache
                    // (`sdx`). They stay valid only while nothing reorders or dedups `calls` between the two — true today
                    // (decls.rs rewrites paths in place; scan.rs never mutates a cached FnInfo's `calls`). A mis-pointed
                    // index fails safe here (`kept_for != members`, no instance), but say so loudly in a debug build.
                    debug_assert!(
                        sd.iter().all(|(i, m)| def.calls.get(*i).is_some_and(|c| c.leaf == *m)),
                        "CT3: `{}`'s self_disp points at a call that is not its `self.m()` (calls reordered after collection?)",
                        def.qual
                    );
                    let mut kept_for: BTreeSet<&str> = BTreeSet::new();
                    let mut calls_i: Vec<crate::model::Call> = Vec::new();
                    for (i, c) in def.calls.iter().enumerate() {
                        match sd.get(&i) {
                            None => calls_i.push(c.clone()),
                            Some(m) => {
                                if c.path == format!("{ty}::{m}") && c.typed && c.method {
                                    kept_for.insert(m);
                                    calls_i.push(c.clone());
                                }
                            }
                        }
                    }
                    if kept_for != members {
                        continue;
                    }
                    let mut u = def.clone();
                    u.qual = format!("{}\u{1f}inst\u{1f}{ty}", def.qual);
                    u.calls = calls_i;
                    u.self_disp = Vec::new();
                    u.inst_of = Some(def.qual.clone());
                    if crate::lang::reach_debug() {
                        eprintln!("CT3INST\t{}\t{ty}", def.qual); // §E1 REACH COUNTER
                    }
                    default_inst.insert((ty_leaf.to_string(), def.qual.clone()), u.qual.clone());
                    add.push(u);
                }
            }
        }
        fns.extend(add);
    }
    // Per-fn DIRECT Unknown-origin reasons (the receipt's `unknownWhy`, spec §2). Coarse, like the lint's
    // per-trait tag: a callback we can't see through, an FFI/extern boundary, or a genuinely-unresolvable
    // bare call. Tracked so the disclosure names WHY, not just that an Unknown exists.
    // `String`, not `&'static str`: a chained dependency's own `unknownWhy` is carried VERBATIM across the
    // join (`dispatch:<owner>.<member>` has a normative detail that re-deriving would destroy), so the set
    // can no longer be only compile-time literals.
    let mut unknown_why: HashMap<String, BTreeSet<String>> = HashMap::new();
    let non_registry_names: std::collections::HashSet<String> =
        non_registry_lock_names(dir).into_iter().chain(non_registry_manifest_names(dir)).collect();
    let lock_versions = {
        // SOUNDNESS R329 — MAX over {member lock, workspace root lock}, not an else-branch. My first
        // version preferred the member's own lock whenever it was non-empty, so a stale or partial
        // member lock SHADOWED the root's and silently restored the exemption — and cargo ignores
        // member lockfiles entirely, so the root's is the only real one. Note the sibling three lines
        // above: `non_registry_names` deliberately UNIONS its lock and manifest evidence. Two adjacent
        // lookups of one question, and mine had drifted to the weaker rule.
        let mut v = deps::registry_lock_versions(dir);
        if let Some(root) = deps::find_workspace_root(std::path::Path::new(dir)) {
            for (k, rv) in deps::registry_lock_versions(&root.to_string_lossy()) {
                let take = match v.get(&k) {
                    Some(prev) => candor_classify::calibration_exceeded_raw(&rv, prev),
                    None => true,
                };
                if take {
                    v.insert(k, rv);
                }
            }
        }
        v
    };
    // The κ ledger's coverage question, ONE authority: does a call into `cr` that no rule matched DISCLOSE (`invisible`) —
    // `true` — or is the crate covered (chained, calibrated within its ceiling, reviewed pure) so its silence is read as an
    // answer? Asked by the ledger filter below and by SOUNDNESS R1578's floor at the untyped-receiver marker.
    let kappa_blind = |cr: &String| -> bool {
        let real = dep_renames.get(cr.as_str()).map(String::as_str).unwrap_or(cr.as_str());
        // COVERED = a report exists for this crate AND the staleness gate did not refuse it AND the
        // report did not declare itself incomplete AND it judged at least one unit. The second,
        // third and fourth conjuncts are the three fixes; spelled as a named local so clippy's
        // boolean simplification can't rewrite the claims into one unreadable disjunction.
        let covered = deps_idx.crates.contains(real)
            && !deps_idx.untrusted.contains(real)
            && !deps_idx.incomplete_pkgs.contains(real)
            && !deps_idx.judged_nothing_pkgs.contains(real);
        // A name Cargo.lock confirms is NOT the real registry crate loses every one of the three
        // CALIBRATED_* exemptions below outright — it is a different artifact wearing a reviewed
        // name, not the thing that was reviewed. Checked on `real` (Cargo.lock records the actual
        // package identity, not a caller-chosen manifest alias).
        let impostor = non_registry_names.contains(real);
        // `CALIBRATED_CRATES` normally exempts a crate outright — "classify has rules here" is read
        // as "an unmatched call was reviewed and found pure". `CALIBRATED_BUT_PARTIAL_CRATES` (R59,
        // SOUNDNESS.md) carves the FFI-thin syscall crates OUT of that exemption: `libc`/`nix`/
        // `rustix`'s generic fd verbs (`read`/`write`/`close`/...) are deliberately UNCLASSIFIED
        // (an ambiguous fd could be Fs/Net/Ipc — see candor-classify's table), so "no rule matched"
        // there is an honest gap, not a reviewed verdict — it must still reach the ledger and
        // disclose `invisible`, exactly like a call into an uncalibrated dependency.
        // R311 — a resolved version ABOVE the reviewed ceiling is downgraded to the same
        // treatment as `CALIBRATED_BUT_PARTIAL_CRATES`: matched calls keep their rules, unmatched
        // calls rejoin the ledger and disclose rather than reading as reviewed-pure.
        // SOUNDNESS R322 — TWO KEY SPACES, ONE FACT, and I reintroduced R312's exact shape inside
        // R311's own fix on the same day. The exemption below is keyed on `cr` (the syntactic first
        // segment a consumer writes); this lookup was keyed only on `real` (the resolved package
        // name after `dep_renames`). `CALIBRATED_CEILINGS` is populated from CALIBRATED_CRATES
        // IDENTIFIERS, so `("native_tls_crate", "0.2.18")` sat under a key `real` can never hold and
        // that ceiling could never fire — while `cr` = `native_tls_crate` still took the exemption.
        // Both keys are consulted, and EITHER saying "beyond review" is enough: the disagreement
        // between the two spaces is precisely what must not be allowed to grant silence.
        // The VERSION and the CEILING live in different key spaces and must be CROSSED, not
        // matched pairwise: under a `package =` alias the lock records the resolved name
        // (`native_tls`) while `CALIBRATED_CEILINGS` is keyed on the identifier
        // (`native_tls_crate`). Checking each key against itself finds a version with no ceiling
        // and a ceiling with no version, and concludes "not beyond review" — silently.
        let ver = lock_versions
            .get(real)
            .or_else(|| lock_versions.get(cr.as_str()));
        let beyond_review = ver.is_some_and(|v| {
            candor_classify::calibration_exceeded(real, v)
                || candor_classify::calibration_exceeded(cr.as_str(), v)
        });
        if beyond_review && candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
            // INSTRUMENTED: an unchanged corpus row is not evidence the branch ran.
            eprintln!(
                "R311CEILING {} resolved {} exceeds its reviewed ceiling",
                real,
                lock_versions.get(real).map(String::as_str).unwrap_or("?")
            );
        }
        // SOUNDNESS R1591 — every exemption below is asked of the REAL package (`real`, after `dep_renames`), never of the
        // alias the source spells. Keyed on the alias, `reqwest = { package = "zz-uncalibrated-http" }` was exempted as if
        // reviewed and an unruled `reqwest::f()` into the uncalibrated package read ABSENT — while the identical call
        // spelled under its real name disclosed `invisible`. The classifier itself is already asked under `real`; the
        // exemption now agrees with it, so `htclient = { package = "reqwest" }` reads covered exactly as `reqwest` does.
        //
        // ONE list entry is itself a rename identifier rather than a package: `native_tls_crate` — the extern alias
        // reqwest and tungstenite give `native-tls`, under which the rules are written (classify lib.rs:79/:1342). Its
        // exemption holds when the rename resolves to the package it stands for, and for no other package.
        let calibrated = |n: &str| candor_classify::CALIBRATED_CRATES.contains(&n);
        let alias_for_real = cr.as_str() != real
            && calibrated(cr.as_str())
            && R1591_CALIBRATED_ALIASES.iter().any(|(a, p)| *a == cr.as_str() && *p == real);
        let fully_calibrated = (calibrated(real) || alias_for_real)
            && !candor_classify::CALIBRATED_BUT_PARTIAL_CRATES.contains(&real)
            && !beyond_review
            && !impostor;
        !covered
            && !fully_calibrated
            && !(candor_classify::PATH_CALIBRATED_CRATES.contains(&real) && !impostor)
            && !(candor_classify::CALIBRATED_PREFIXES.iter().any(|p| real.starts_with(p)) && !impostor)
            // REVIEWED-PURE: read and found to perform no effect of its own, so its silence is an
            // answer rather than a gap. A separate list from the calibrated ones because those must
            // carry a live rule (`calibrated_crates_are_live`) and a pure crate has none — see the
            // evidence requirement on `REVIEWED_PURE_CRATES`. Same impostor carve-out: a reviewed-
            // pure verdict is about the real crate too.
            && !(candor_classify::REVIEWED_PURE_CRATES.contains(&real) && !impostor)
    };
    // SOUNDNESS R1605 — a re-exported name for an `R1605_TRAITS` implementor (`pub use post::Entity as Post`) -> its leaf.
    let r1605_aliases: HashMap<String, String> = merged
        .reexports
        .iter()
        .filter(|r| r.alias != r.name && r.alias != "*" && r1605_ty_traits.contains_key(&r.name))
        .map(|r| (r.alias.clone(), r.name.clone()))
        .collect();
    // SOUNDNESS R1715 — per (package, path after the crate): the owner keys a facade's re-exports resolve it to.
    type R1715Keys = Vec<(String, Vec<String>)>; // (owner key, packages read to reach it)
    let mut r1715_call_cache: HashMap<(String, String), R1715Keys> = HashMap::new();
    let mut r1715_blind_cache: HashMap<String, bool> = HashMap::new();
    let mut r1715_rev_cache: HashMap<String, R1715Keys> = HashMap::new();
    let mut r1748_cache: HashMap<(String, String, String), bool> = HashMap::new();
    // …and every ABSTRACTION MEMBER a chained report dispatches on, by its 2-segment tail (`Deserialize::deserialize` of
    // `serde_core#de::Deserialize::deserialize`) -> the owners it is dispatched under. A dispatched member is a trait's
    // (obligation 1 keys only those), so an inherent `Type::m` another package merely ADDS never qualifies.
    // SOUNDNESS R1739 — and, per (tail, owner), the trait paths it is dispatched under (`de::Visitor`), for the ownership proof.
    let mut r1739_tail_paths: HashMap<(String, String), BTreeSet<String>> = HashMap::new();
    let r1715_tail_owners: HashMap<String, BTreeSet<String>> = {
        let mut m: HashMap<String, BTreeSet<String>> = HashMap::new();
        for d in deps_idx.by_key.values() {
            for k in &d.dispatches_on {
                let Some((o, rest)) = k.split_once('#') else { continue };
                if let Some(t2) = tail2(rest) {
                    if let Some((tp, _)) = rest.rsplit_once("::") {
                        r1739_tail_paths.entry((t2.clone(), o.to_string())).or_default().insert(tp.to_string());
                    }
                    m.entry(t2).or_default().insert(o.to_string());
                }
            }
        }
        m
    };
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            // SOUNDNESS R485 — THE REASON COMES FROM THE SITE THAT RAISED THE FLAG, not from here.
            // This line used to read `insert("callback:unresolved call")` unconditionally, for all eight
            // of `collector.rs`'s write sites. Five of them are not callbacks: two are a trait dispatch
            // whose implementor set is absent or wider than the cross-engine bound (SPEC §4's own
            // "no impl, bounded-CHA over many impls", owner and member both in hand) and three are the
            // analyser's own name resolution finding two same-named local definitions (§4 ⟨0.24⟩
            // `ambiguous:`). The effect set is untouched — `Unknown` either way, `unresolved: true`
            // either way — but §6.2 classes `callback:` as `indirect` and both corrected kinds as
            // `dispatch`, and `deny E Unknown[dispatch]` is REASON-SCOPED: candor-java fired on the
            // zero-implementor shape where this engine passed it.
            //
            // THE FALLBACK IS FAIL-CLOSED, NOT DECORATION. If a future site sets the flag through some
            // path that records no reason, an `Unknown` with NO `unknownWhy` classes as `unresolved`
            // (gate.rs's catch-all) — which is sound but LOSES the `indirect` scoping this line has
            // always given callback holes. Re-supplying the historical reason keeps that narrowing from
            // happening silently. `mark_unresolved` is the only writer of the flag, so this is reachable
            // only through a regression, and `every_unresolved_site_records_a_reason` is the control.
            let whys = &f.unresolved_why;
            let e = unknown_why.entry(f.qual.clone()).or_default();
            if whys.is_empty() {
                e.insert("callback:unresolved call".to_string());
            } else {
                e.extend(whys.iter().cloned());
            }
        }
        // SOUNDNESS R182/R196 — A REFUSAL MUST DISCLOSE, NOT CERTIFY. Each entry is a place where name
        // resolution found more than one answer, correctly refused to guess, and then had nothing to
        // say — so the enclosing function was reported PURE over a destructor that really runs. The
        // reason kind is `ambiguous:`, SPEC §4's fifth kind, for exactly the argument the
        // `ambiguous:same-name local defs` site below sets out: one function runs and Rust resolves it
        // statically, so what failed is this analyser's name resolution, not the program's.
        for why in &f.refusals {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert(why.clone());
        }
        // DROP-GLUE (#3): local types this fn CONSTRUCTS that have a local `impl Drop`. The `Drop::drop`
        // body runs at scope exit — an implicit edge the call graph misses, so a guard that flushes/closes
        // on drop read silent-pure. Collected per-call below (a `T::*` associated-fn call where `T` has a
        // local Drop), then edged to `T::drop` after the call loop. Over-approximates toward the SOUND
        // direction (a constructed value is assumed to drop in this scope); gated to LOCAL drop types only,
        // so an external type's invisible Drop is never fabricated.
        let mut drops_here: BTreeSet<String> = BTreeSet::new();
        // SOUNDNESS R1253 — per drop leaf, the spelled twin type of each construction that put it in `drops_here`
        // (`None` = not recoverable: a parameter, an assignment, a field-owned drop, an unspelled constructor).
        let mut drop_quals: HashMap<String, BTreeSet<Option<String>>> = HashMap::new();
        // ESCAPE GATE (R49): a drop-type built here whose value can ESCAPE via the return must NOT be charged
        // — its `Drop` runs in the CALLER's scope (a constructor `Compress::new -> Compress` that builds an
        // owned `Stream` and returns the `Compress` otherwise FABRICATES the Stream's FFI-Drop; the flate2
        // miss that A/B-reverted the naive field fix). CONSERVATIVE test: the fn RETURNS a local aggregate (a
        // struct in `fields`) or a drop-type — so a value built here may be moved into what's returned. A
        // membership check (not ownership traversal), so it stays SOUND under the leaf-name COLLISIONS that
        // flate2's parallel read/write/bufread modules create (three `GzEncoder`s etc.) — precise ownership
        // can't be trusted there. It over-skips (a fn returning an aggregate won't get drop-glue even for a
        // genuinely-local guard), the SOUND direction — never a fabrication. A `-> ()`/`Result`/primitive fn
        // (the local-use case: `let _g = Guard{..}; …`) is NOT an escape, so it still charges.
        let returns_escapable = f
            .ret_idents
            .iter()
            .any(|r| fields.contains_key(r) || merged.drop_types.contains(r));
        for c in &f.calls {
            // SOUNDNESS R1578 — an UNTYPED receiver whose value an UNCHAINED dependency produced is typed from that
            // dependency's own source signature, and the call then goes down the ordinary typed-external path below
            // (classifier, κ ledger, masking) exactly as `js.m()` over a `js: &Context` parameter does. See `r1578_type`.
            let r1578_owned = r1578_type(c, &deps, &dep_renames, deps_idx, dir, &r1578_sigs, &returns)
                .or_else(|| r1605_chain_type(c, &deps, &dep_renames, deps_idx, dir, &r1578_sigs))
                .or_else(|| r1617_chain_type(c, &deps, &dep_renames, deps_idx, dir, &r1578_sigs));
            let r1578_twin = r1578_owned.as_ref().is_some_and(|(_, t)| *t);
            let c = r1578_owned.as_ref().map(|(c, _)| c).unwrap_or(c);
            // SOUNDNESS R1605 — …AND WHERE THE MEMBER'S RETURN WAS NOT FILED (no lock-pinned source to read, a member whose
            // declared return cannot be placed, a spelling the typed routes do not reach), the value is untyped and the method
            // on it formed only this crate-rooted `<untyped>` marker, which reached nothing: the caller ABSENT over a query
            // that runs. A receiver an `R1605_TRAITS` implementor's member produced is a dependency's value, so the caller
            // discloses `Unknown` with the untyped-receiver reason the chained arm uses. Never a charge.
            if c.path.starts_with(&format!("{}::", crate::model::R1605_UNTYPED)) {
                if candor_report::env::trace("CANDOR_R1605_INSTR") {
                    eprintln!("R1605FLOOR\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER (generic bound)
                }
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("dispatch:untyped cross-package receiver".to_string());
                r1605_floored.entry(f.qual.clone()).or_default().insert("r1605".to_string());
                continue;
            }
            if let Some((root, rest)) = c.path.split_once(&format!("::{UNTYPED_RECV_MARKER}::")) {
                if !deps.contains(root) && !rest.contains('<') {
                    if let Some((callee, _)) = rest.rsplit_once("::") {
                        // the marker's root is the callee's first segment (`crate`, a module, or — through a glob that
                        // `expand` leaves bare — the type itself: `SysUser::<untyped>::find::all`)
                        let full = format!("{root}::{callee}");
                        let segs: Vec<&str> = full.split("::").collect();
                        if let [.., ty, _m] = segs.as_slice() {
                            let real = r1605_aliases.get(*ty).map(String::as_str).unwrap_or(ty);
                            if r1605_ty_traits.contains_key(real) {
                                if candor_report::env::trace("CANDOR_R1605_INSTR") {
                                    eprintln!("R1605FLOOR\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                                }
                                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                unknown_why.entry(f.qual.clone()).or_default()
                                    .insert("dispatch:untyped cross-package receiver".to_string());
                                r1605_floored.entry(f.qual.clone()).or_default().insert("r1605".to_string());
                            }
                        }
                    }
                }
            }
            // SOUNDNESS R334 — THE OS ENTROPY SOURCE PASSED AS AN ARGUMENT, decided at the TOP of this
            // loop and deliberately BEFORE every `continue` below.
            //
            // Every other entropy rule keys on the CALLEE, so `StdRng::try_from_rng(&mut SysRng)` — the
            // idiomatic way to seed a userspace CSPRNG from the OS (quinn-proto `endpoint.rs:81`) —
            // matched nothing and read pure, in the OLD `OsRng` spelling as well as the new one. The
            // callee cannot be charged instead: `from_rng(&mut pcg)` is deterministic and the
            // deterministic population is the larger one (802 `from_seed`/`seed_from_u64` sites in the
            // local registry), so charging the verb would be R330's mistake one crate over. Only the
            // argument separates them.
            //
            // POSITION IS THE FIX, not the rule. Placed beside the other second effects further down,
            // this was measured as INERT: the flag reached the loop with the right path
            // (`rand::rngs::StdRng::try_from_rng`) and the body never ran, because a dependency call
            // that resolves through `deps_idx` `continue`s long before it. Correct code on a path the
            // failing case never takes — the third time in two days — and only a probe at this loop's
            // entry showed it. The fact is a property of the ARGUMENT LIST alone, so it belongs where
            // nothing can skip it.
            //
            // Unguarded by `suppress_bare_leaf`/`local_is_authoritative` (which are not yet computed
            // here) and that is correct rather than convenient: if the callee is a LOCAL function its
            // own effects propagate anyway, and handing the OS entropy source to it is worth charging
            // at the hand-off — the only point where candor can see the value at all.
            if c.entropy_arg {
                direct.entry(f.qual.clone()).or_default().insert("Rand");
            }
            // SOUNDNESS R754 — A CALL TO A FOREIGN FUNCTION THIS BODY DECLARED. The collector records a
            // bare call to a body-local `extern` / `link!` name under `EXTERN_SENTINEL` (see
            // `body_declared_externs`), so
            // this path IS the import by Rust's scoping, and it is answered here — before `resolve_target`
            // can look the bare leaf up crate-wide and find a same-named WRAPPER (windows' own shape),
            // whose self-edge is then dropped and whose `resolved_local` suppresses every disclosure.
            // Same answer the item-level `extern` block has always produced: `Unknown`, `native:extern fn`.
            if !c.is_macro && c.path.starts_with(crate::decls::EXTERN_SENTINEL) {
                if candor_report::env::trace("CANDOR_VEINE_INSTR") {
                    eprintln!("VEINE_EXTERNCALL\t{}\t{}", f.qual, c.path); // §E1 REACH PROBE
                }
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn".to_string());
                continue;
            }
            // SOUNDNESS R1165 — a local value with a std-trait impl handed to a dependency (`WITNESS_MARKER`). Recorded
            // only where the dependency is CHAINED: there its report answers the call (a hit, or a miss that §2 rule 3
            // reads as purity) and cannot say the body runs the caller's impl. Unchained, the κ ledger's `invisible`
            // already discloses the whole call and this adds nothing. Never an edge, never a ledger entry.
            if let Some((wcr, rest)) = c.path.split_once(&format!("::{WITNESS_MARKER}::")) {
                let wcr_real: &str = dep_renames.get(wcr).map(String::as_str).unwrap_or(wcr);
                // `CANDOR_R1165_OFF` is the falsification switch (the fixtures' 0.40.9 behaviour), never a user setting.
                if deps_idx.crates.contains(wcr_real) && !candor_report::env::switch("CANDOR_R1165_OFF") {
                    let mut it = rest.splitn(3, "::");
                    // SOUNDNESS R1200 — a position written `<n>m` is a METHOD-CALL site; `depsig` is told so by a `.` before the
                    // callee (`.Foo::m`), since method resolution may run a trait method out of the dependency's sight there.
                    let (l, pos_s, callee) = (it.next(), it.next(), it.next());
                    let meth = pos_s.is_some_and(|p| p.ends_with('m'));
                    let pos: Option<usize> = pos_s.and_then(|p| p.trim_end_matches('m').parse().ok());
                    let callee = callee.map(|c| if meth { format!(".{c}") } else { c.to_string() });
                    if let (Some(l), Some(pos), Some(callee)) = (l, pos, callee) {
                        // SOUNDNESS R1186 — a local conversion taking the argument is judged on its own list.
                        if let Some(src) = l.strip_prefix(crate::model::FROM_WITNESS_PREFIX) {
                            if !candor_report::env::switch("CANDOR_R1186_OFF") {
                                from_sites.push((f.qual.clone(), wcr.to_string(), wcr_real.to_string(), src.to_string(), pos, callee));
                            }
                        } else {
                            witness_sites.push((f.qual.clone(), wcr.to_string(), wcr_real.to_string(), l.to_string(), pos, callee));
                        }
                    }
                }
                continue;
            }
            // ── R105 — A `#[cfg]`-DUPLICATED ALIAS, ADJUDICATED WHERE THE LEAF IS KNOWN ────────────────
            // `decls::record_alias` keeps EVERY arm of an alias whose declaration is duplicated across
            // `#[cfg]` branches (the ordinary platform/feature shim) instead of letting the last one in
            // the file win, and `lang::alias_join` distributed this call's own trailing segments over
            // them. So the path in hand is the complete set of callees this one call site could name, and
            // the question "do the arms answer the same?" can be put to the AUTHORITY that owns it — the
            // classifier, with the real leaf attached — instead of being guessed at collection time from
            // a prefix nobody had yet appended a leaf to (§G).
            //
            //   * every arm classifies the SAME → charge that effect. This is the union, and it is the
            //     case that decides the cost: essentially every real collision in a 256-crate corpus is a
            //     portability shim (`alloc::boxed::Box` / `std::boxed::Box`, `core::sync::atomic` /
            //     `loom::sync::atomic`, `LittleEndian` / `BigEndian`) whose arms are indistinguishable to
            //     the classifier, so this arm stays silent and costs nothing.
            //   * the arms DISAGREE → `Unknown`. candor cannot know which platform or feature set the
            //     build selects, and the two things it must not do are pick one (measured: `deny Fs` went
            //     exit 1 → exit 0 on two crates differing ONLY in arm order, with the real `Fs` nowhere in
            //     the document) and charge both (a fabricated effect from the arm that is not compiled).
            //     The kind is `ambiguous:` — SPEC §4's fifth kind, already meaning "two definitions, no
            //     way to say which runs"; this is that state one resolution step earlier, so it reuses the
            //     token rather than minting one.
            //
            // NO SURFACE IS CAPTURED for either outcome, deliberately: the arms are DIFFERENT paths, so
            // their `fs`/`cmds`/`hosts` literals are different claims and publishing one arm's would be
            // the pick-by-position this fix removes. The agreed-effect arm marks itself `incomplete` so an
            // empty surface cannot read as a complete one and let `allow <E> <lit>` certify a claim the
            // engine never made.
            if c.path.contains(crate::decls::ALIAS_ALT_SEP) {
                let arms = || {
                    c.path.split(crate::decls::ALIAS_ALT_SEP).map(|alt| {
                        let written = alt.split("::").next().unwrap_or("");
                        let acr: &str = dep_renames.get(written).map(String::as_str).unwrap_or(written);
                        candor_classify::classify(acr, alt)
                            .or_else(|| scan_builder_entry_effect(acr, alt))
                    })
                };
                let first = arms().next().flatten();
                let agree = arms().all(|e| e == first);
                // Instrument the PRECONDITION, not just the output (the standing bar, and the sibling of
                // `CANDOR_TYPESURFACE_DEBUG` above): a report diff cannot show that this branch never
                // fired, and a byte-identical A/B over a corpus that never reaches it is the most
                // flattering number available.
                if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
                    eprintln!(
                        "ALIASCOLLIDE {} :: {} -> {}",
                        f.qual,
                        c.path.replace(crate::decls::ALIAS_ALT_SEP, " | "),
                        if agree { format!("AGREE {first:?}") } else { "DISAGREE".to_string() }
                    );
                }
                // SOUNDNESS R1612 — each arm's SECOND effects (`ALSO_EFFECTS`) are charged with it, and marked like
                // the primary: this branch withholds every surface, so every charged effect is `incomplete`.
                let alsos: Vec<&'static str> = c.path.split(crate::decls::ALIAS_ALT_SEP).flat_map(|alt| {
                    let written = alt.split("::").next().unwrap_or("");
                    let acr: &str = dep_renames.get(written).map(String::as_str).unwrap_or(written);
                    candor_classify::also_effects(acr, alt)
                }).collect();
                for e2 in &alsos {
                    direct.entry(f.qual.clone()).or_default().insert(e2);
                    incomplete.entry(f.qual.clone()).or_default().insert(e2);
                }
                match (agree, first) {
                    (true, Some(eff)) => {
                        direct.entry(f.qual.clone()).or_default().insert(eff);
                        incomplete.entry(f.qual.clone()).or_default().insert(eff);
                    }
                    (true, None) => {}
                    // RULED 2026-09-12 — UNION (SOUNDNESS.md "OWED TO TOM"). This arm used to answer
                    // `Unknown` + `ambiguous:cfg-duplicated alias`. That is the HEDGE, and the ruling
                    // replaced it with "a hedge replaced by a COMPLETE answer, not a withdrawal": both
                    // arms were READ, so charging both is not fabrication under SPEC §4's own definition
                    // ("asserting something the engine did not read from the code"), while withdrawing a
                    // determined effect into `{Unknown}` — which §4.0 says is not `⊤` — stops `deny Fs`
                    // firing on a call that reaches `Fs` in one configuration.
                    //
                    // THE RULING'S PREMISE WAS THAT THIS CHANGED NOTHING ("the engines already do it"),
                    // AND THAT WAS MEASURED FALSE. `main` unioned only on the DEFINITION route
                    // (`collector.rs`, R140/R287); this — the `use`-ALIAS route — still hedged, so one
                    // program answered two ways depending on whether its alias crossed a module
                    // boundary. Measured on the same fixture in both spellings: the union route made
                    // `deny Fs`, `deny Env` and `allow Fs <lit>` all fire, and the hedge route made all
                    // three SILENT. That is the R347 shape — two mechanisms answering one question
                    // differently — which the family treats as a defect, not a choice.
                    //
                    // THE SURFACE IS STILL WITHHELD, and that is not a leftover: the arms are DIFFERENT
                    // paths, so their `fs`/`cmds`/`hosts` literals are different claims and publishing
                    // one arm's would be the pick-by-position R105 removed. Marking every charged effect
                    // `incomplete` is what stops an empty surface reading as a complete one — the same
                    // treatment the agreeing arm above already gets, now applied to the disagreeing one.
                    (false, _) => {
                        for eff in arms().flatten() {
                            direct.entry(f.qual.clone()).or_default().insert(eff);
                            incomplete.entry(f.qual.clone()).or_default().insert(eff);
                        }
                    }
                }
                // …AND THE κ LEDGER STILL RUNS, once per arm. This branch `continue`s, so without it an
                // UNCLASSIFIED call through a duplicated alias would leave the blind-spot ledger silent —
                // and MEASURED, that is not hypothetical: the first cut of this fix dropped 32 rows and
                // changed 20 more across `bytes-1.12.1`, every one of them an `invisible:
                // ["extra_platforms"]` that vanished, because `loom.rs` declares
                // `core::sync::atomic::{…}` and `extra_platforms::{…}` under opposing `#[cfg]`s and the
                // whole ledger contribution went out with the `continue`. Losing a disclosure is the
                // direction that matters here, so every arm that names a declared dependency is charged,
                // which is the SAME union this file already applies to `#[cfg]` branches elsewhere (see
                // the `blind_direct` note below: a union over cfg arms is over-report noise, never the
                // cardinal sin). With this, the fix's whole-corpus diff is byte-identical.
                for alt in c.path.split(crate::decls::ALIAS_ALT_SEP) {
                    let acr = alt.split("::").next().unwrap_or("");
                    let acr_real: &str = dep_renames.get(acr).map(String::as_str).unwrap_or(acr);
                    let alt_real: std::borrow::Cow<str> = if acr_real == acr {
                        std::borrow::Cow::Borrowed(alt)
                    } else {
                        std::borrow::Cow::Owned(format!("{acr_real}{}", &alt[acr.len()..]))
                    };
                    if alt.contains("::")
                        && deps.contains(acr)
                        && candor_classify::classify(acr_real, &alt_real).is_none()
                        && scan_builder_entry_effect(acr_real, &alt_real).is_none()
                    {
                        *dep_seen.entry(acr.to_string()).or_insert(0) += 1;
                        blind_direct.entry(f.qual.clone()).or_default().insert(acr.to_string());
                    }
                    // An arm carrying the UNTYPED-RECEIVER marker would, on the ordinary path below, earn
                    // a `dispatch:` disclosure for a CHAINED dependency. It is doubly conditional (an
                    // ambiguous alias AND an untyped receiver) and fired on nothing in the corpus, but a
                    // disclosure that exists on the single-arm path and not here is exactly the kind of
                    // asymmetry that turns into a finding later, so it is mirrored rather than left out.
                    if let Some(_rest) = alt.strip_prefix(&format!("{acr}::{UNTYPED_RECV_MARKER}::")) {
                        if deps_idx.crates.contains(acr_real) {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why
                                .entry(f.qual.clone())
                                .or_default()
                                .insert("dispatch:untyped cross-package receiver".to_string());
                        }
                    }
                }
                // ── SOUNDNESS R438 — AN ARM THE CLASSIFIER CANNOT NAME IS NOT AN ARM WITH NO EFFECT ──
                // The `arms()` adjudication above asks the CLASSIFIER what each arm does, and a LOCAL
                // arm is exactly the one the classifier has no answer for — `classify("loc","loc::put")`
                // is `None` whether that function writes a file or does nothing at all. So a mixed arm
                // set came out of the match above charged with the EXTERNAL arm's effect and nothing
                // else, which is the pick this branch exists to prevent, arriving one level down.
                //
                // The local arm's effects are not the classifier's to supply; they are its own, and they
                // already propagate — through the call graph — to every caller with an EDGE to it. So
                // the fix is an edge, not an effect: resolve each unclassifiable arm the ordinary local
                // way and record the call. Whatever `loc::put` turns out to do, `probe` inherits it,
                // including transitively and including effects added to it later.
                //
                // WHY THIS MUST BE HERE AND NOT BELOW. The branch `continue`s, so an aliased path never
                // reaches the local-resolution block at `resolvable && !aliased` — the same `continue`
                // the κ ledger above had to be lifted over, for the same reason and with the same shape
                // of consequence. MEASURED before this loop existed: `probe` came back `['Fs']` with
                // `incomplete: ['Fs']` and NO `Env`, while `loc::put` sat in the same report carrying
                // `Env`. A BARE `deny Env` therefore fired and looked correct; only the FUNCTION-level
                // attribution was wrong, which is why a scoped rule is the discriminator here exactly as
                // it is for R429 and R439.
                //
                // NARROW BY CONSTRUCTION, in the direction that matters: an arm the classifier DOES name
                // is skipped (it was already charged), and an arm that does not resolve to a local
                // definition adds nothing. `resolve_target` refuses a key with competing claimants
                // rather than guessing between them, so a tail2 collision between an external arm's
                // spelling and an unrelated local one costs a missing edge, not a fabricated one — the
                // honest under-report this file takes everywhere else. The synthetic LAZY marker is not
                // a call site and is skipped by name.
                // SOUNDNESS (v053, the R1153 vein) — …AND THE CHAINED JOIN, once per arm. The ledger above records a
                // dependency arm's crate, and the coverage filter drops it when that crate is CHAINED — so a
                // `#[cfg]`-duplicated alias whose arms are dependency types (`#[cfg(unix)] use cdep4::Tool as T;
                // #[cfg(windows)] use cdep4::inner::Deep as T;` then `t.run()`) answered NOTHING chained: ABSENT,
                // `deny Fs` exit 0 over an executed write, on 0.40.7 and 0ef4c2d alike, while the single-arm
                // spelling joined. Each unclassified dependency arm now asks the chained report exactly as the
                // single-arm path does (its `cr#tail2` key, then the ⟨0.40⟩ walk's rows); every answer is ADDED, as
                // the arms are a union. A miss adds nothing, as on the single-arm path.
                for alt in c.path.split(crate::decls::ALIAS_ALT_SEP) {
                    let acr = alt.split("::").next().unwrap_or("");
                    let acr_real: &str = dep_renames.get(acr).map(String::as_str).unwrap_or(acr);
                    if !alt.contains("::") || !deps_idx.crates.contains(acr_real)
                        || candor_classify::classify(acr_real, alt).is_some()
                    {
                        continue;
                    }
                    let rel = alt.strip_prefix(&format!("{acr}::")).unwrap_or(alt);
                    let key = if rel.contains("::") {
                        tail2(rel).map(|t2| format!("{acr_real}#{t2}"))
                    } else {
                        Some(format!("{acr_real}#{rel}"))
                    };
                    let mut rows: Vec<&crate::deps::DepFn> = Vec::new();
                    if let Some(de) = key.as_ref().and_then(|k| deps_idx.by_key.get(k)) {
                        rows.push(de);
                    } else if c.method && c.typed {
                        if let Some((tpath, member)) = rel.rsplit_once("::") {
                            let start = deps_idx.surface.type_key(acr_real, tpath)
                                .unwrap_or_else(|| format!("{acr_real}#{tpath}"));
                            rows = crate::typesurf::join_through(deps_idx, &start, member, false).0;
                        }
                    }
                    if !rows.is_empty() && candor_report::env::trace("CANDOR_V053_INSTR") {
                        eprintln!("R1153ARMJOIN\t{}\t{alt}", f.qual); // §E1 REACH COUNTER
                    }
                    for de in rows {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                let mut r438_gained_edge = false;
                for alt in c.path.split(crate::decls::ALIAS_ALT_SEP) {
                    // Every synthetic marker, not just the lazy one: these are not call sites, they
                    // are encodings this file consumes elsewhere, and resolving one locally would
                    // invent an edge from a name no source text spells.
                    if alt.contains(LAZY_UNIT_PREFIX)
                        || alt.contains(DROP_MARKER)
                        || alt.contains(CONSTRUCT_MARKER)
                        || alt.contains(UNTYPED_RECV_MARKER)
                        || alt.contains(CHAIN_GUESS_MARKER)
                        || alt.contains(EXT_METHOD_MARKER)
                        || alt.contains(GLOB_CAND_MARKER)
                        || alt.contains(OP_RHS_MARKER)
                        || alt.contains(HOOK_JOIN_MARKER)
                    {
                        continue;
                    }
                    let acr = alt.split("::").next().unwrap_or("");
                    let acr_real: &str = dep_renames.get(acr).map(String::as_str).unwrap_or(acr);
                    if candor_classify::classify(acr_real, alt).is_some()
                        || scan_builder_entry_effect(acr_real, alt).is_some()
                    {
                        continue;
                    }
                    // SOUNDNESS R440 — `arm_exact_target` LAST, and only when the two above declined.
                    // It is the narrow rescue of the one ambiguity that this loop's own input creates:
                    // a portability twin whose arms share a type LEAF (`unix::X` / `windows::X`) makes
                    // the tail2 many-way, so `resolve_target` refused BOTH arms and the branch fell
                    // through to R440's no-trace state. Measured on mio: `Waker::wake` ABSENT over two
                    // `WakerInternal::wake` bodies that each write a file descriptor and each charge
                    // `Fs` in their own row.
                    let targets: Option<Vec<String>> =
                        resolve_target(alt, &c.leaf, c.method, &by_tail2, &by_leaf_call)
                            .or_else(|| reexport_target(alt, c.method, &by_tail2, &by_reexport))
                            .cloned()
                            // SOUNDNESS R1657 stage 2 — an arm the re-export table names EXACTLY that is a foreign DECLARATION
                            // unit (generator's `pub use self::asm::swap_registers` over nine `#[path]` arms, each an
                            // `extern "C" { fn swap_registers(..) }`): declarations are kept out of `by_tail2`, so before the
                            // table the call reached them only through `reexport_target`, and the first cut left it with no
                            // edge and no `native:extern fn` (RegContext::swap, Unknown -> ABSENT). Exact qual only.
                            .or_else(|| {
                                let q = alt.strip_prefix("crate::")?;
                                (!c.method && !crate::lang::r1657s2_off() && decl_quals.contains(q)).then(|| vec![q.to_string()])
                            })
                            .or_else(|| {
                                let t = crate::collector::arm_exact_target(alt, &by_tail2)?;
                                // §E1 REACH COUNTER, on the CHANGED branch.
                                if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
                                    eprintln!("R440ARM");
                                }
                                Some(vec![t.clone()])
                            });
                    // SOUNDNESS R1682 — a LOCAL arm none of the three routes placed. R1682 makes a `#[cfg]` arm set reach
                    // other modules, and its arms name the item where the arm's `use` names it — `sys::unix::TcpListener`
                    // for mio's `#[cfg(unix)] pub use self::unix::{TcpListener, ..}` — while the definition sits one module
                    // deeper (`sys::unix::tcp::TcpListener`, re-exported by `unix/mod.rs`). The tail is many-way, the exact
                    // qual names no unit, and this branch `continue`s: the call had NO trace, where before R1682 it reached
                    // the ordinary route's `ambiguous:same-name local methods` (measured, mio 0.6.23
                    // `TcpListener::accept_std`: `Unknown` on 81f7200d, `[]` on the first cut over `sys::unix`'s `accept`).
                    // So (1) the claimants of the tail that live UNDER the arm's own module are its definitions — the
                    // module re-exports them — and are edged, all of them (the union); (2) a local arm with claimants and
                    // none under its module is disclosed exactly as the ordinary route discloses it. An arm whose tail has
                    // no claimant at all names nothing local and adds nothing, as before.
                    let targets = targets.or_else(|| {
                        if crate::lang::r1682_off() {
                            return None;
                        }
                        let t2 = tail2(alt)?;
                        let cl = by_tail2.get(&t2)?;
                        let local = alt.strip_prefix("crate::").unwrap_or(alt);
                        let n = local.split("::").count();
                        if n < 3 {
                            return None;
                        }
                        let module = local.split("::").take(n - 2).collect::<Vec<_>>().join("::");
                        let under: Vec<String> =
                            cl.iter().filter(|q| q.starts_with(&format!("{module}::")) && *q != &f.qual).cloned().collect();
                        if under.is_empty() {
                            let head = local.split("::").next().unwrap_or("");
                            let local_head = !deps.contains(head)
                                && !deps_idx.crates.contains(dep_renames.get(head).map(String::as_str).unwrap_or(head))
                                && !matches!(head, "std" | "core" | "alloc");
                            if local_head && cl.iter().any(|q| q != &f.qual) {
                                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local methods".to_string());
                                if candor_report::env::trace("CANDOR_R1682_INSTR") {
                                    eprintln!("R1682ARMHEDGE\t{}\t{alt}", f.qual); // §E1 REACH COUNTER
                                }
                            }
                            return None;
                        }
                        if candor_report::env::trace("CANDOR_R1682_INSTR") {
                            eprintln!("R1682ARMSUB\t{}\t{alt}\t{}", f.qual, under.join("|")); // §E1 REACH COUNTER
                        }
                        Some(under)
                    });
                    if let Some(targets) = targets {
                        for t in &targets {
                            if t != &f.qual {
                                // The GAIN probe, distinct from the REACH probe in `decls.rs`: reaching
                                // the arm set and actually gaining an edge from it are two facts, and a
                                // fix that reaches everything and gains nothing is inert in the way that
                                // looks like success.
                                if candor_report::env::trace("CANDOR_R438_PROBE") {
                                    eprintln!("R438 EDGE {} -> {t}", f.qual);
                                }
                                r438_gained_edge = true;
                                calls.entry(f.qual.clone()).or_default().insert(t.clone());
                            }
                        }
                    }
                }
                // ── SOUNDNESS R438 — THIS BRANCH `continue`s, SO SAYING NOTHING HERE IS SAYING PURE ──
                // Every path above can charge nothing: `(true, None)` is "no arm was classifiable", and
                // the edge loop adds nothing when no arm resolves locally. The `continue` skips ALL
                // ordinary handling, so reaching the end in that state leaves the call with no trace at
                // all and its caller absent from `functions[]` — a ⟨0.21⟩ purity claim.
                //
                // THIS STATE IS MEASURED, NOT ASSUMED ABSENT. A first cut of this fix hedged it to
                // `Unknown`, and the corpus said that was the wrong fix TWICE over. First it fired on
                // every unclassifiable arm set, adding 259 rows over 1,556 crates, because the commonest
                // such set is ALL-EXTERNAL and is already disclosed — the κ ledger above records each
                // arm's crate in `invisible`, which is the ⟨0.24⟩ answer and names WHO, where `Unknown`
                // does not. Narrowed to arms the κ ledger cannot reach it still added 47 `Unknown` rows
                // to pure arithmetic (`minimal-lexical::bigint::*`, `crossbeam-epoch::atomic::*`), where
                // the "arm set" is a pair of `#[cfg(target_pointer_width)]` TYPE aliases carrying no
                // effect at all. And it prevented nothing: with the DEFINITION arm recorded (see
                // `decls.rs`), the same corpus reports REMOVED 0 with the hedge off — the silence the
                // hedge existed for was the dropped definition arm, and it is fixed at its cause.
                //
                // So the hedge is GONE and the state is instrumented instead. `CANDOR_R438_SILENT` makes
                // a real occurrence countable on a corpus rather than argued about here: the honest
                // claim is "this did not occur on 1,556 crates", and that claim needs a counter, not a
                // comment. If it ever fires, the answer is another arm route recorded at its cause — the
                // definition arm was one — and not a hedge laid over the top.
                if !r438_gained_edge
                    && arms().flatten().next().is_none()
                    && candor_report::env::trace("CANDOR_R438_SILENT")
                {
                    {
                        // Silence and UNDISCLOSED silence are two different numbers, and only the
                        // second is a finding: the κ ledger above records a dep-crate arm in
                        // `invisible`, so an all-external set still says "I could not see into X" even
                        // when this branch charges nothing. Splitting them is the difference between
                        // "960 occurrences" and a number that means something.
                        let undisc = c.path.split(crate::decls::ALIAS_ALT_SEP).any(|alt| {
                            let head = alt.split("::").next().unwrap_or("");
                            let head_real: &str =
                                dep_renames.get(head).map(String::as_str).unwrap_or(head);
                            !deps.contains(head)
                                && !deps_idx.crates.contains(head_real)
                                && !matches!(head_real, "std" | "core" | "alloc")
                        });
                        eprintln!(
                            "R438 SILENT{} {} :: {}",
                            if undisc { "-UNDISCLOSED" } else { "-disclosed" },
                            f.qual,
                            c.path
                        );
                    }
                }
                continue;
            }
            let cr = c.path.split("::").next().unwrap_or("");
            // The dependency's REAL package identity (post `package = "…"` manifest rename), hoisted
            // once per call and reused by every crate-identity-keyed lookup below (the cross-crate joins,
            // and — since ⟨renamed-dep classify() alias⟩ — the direct classifier too): `cr` is only ever
            // the alias a project's own Cargo.toml key chose (and the source text a call site actually
            // spells), which is meaningless to a rule table written against the published crate's name.
            let cr_real: &str = dep_renames.get(cr).map(String::as_str).unwrap_or(cr);
            // A DEP-LAZY MARKER (`<crate>::<lazy>::NAME`, emitted for a qualified mention of what may be a
            // dependency's lazy static) is consumed by the cross-crate join BELOW and by nothing else: it
            // is a speculative name, so letting it reach local resolution or the classifier would invent
            // edges for every qualified path in the file. Join it directly and move on; a crate the deps
            // index does not cover, or a dep with no such unit, resolves to nothing and costs nothing.
            // Cross-crate DROP GLUE: `cr::<drop>::Type` — binding a dependency's value runs its `Drop` at
            // scope exit. Joined here and nowhere else, for the same reason as the lazy marker below.
            // SOUNDNESS R1136 — a DEPENDENCY EXTENSION-TRAIT method on a std-typed receiver (`EXT_METHOD_MARKER`):
            // recorded in the κ ledger exactly as a floored qualified call into `cr` is, so the coverage filter
            // downstream decides whether it surfaces as `invisible` (a chained or calibrated crate is dropped
            // there). Nothing else: no effect, no `Unknown`.
            // SOUNDNESS R1157 — a primitive operator only a macro-made impl answers (`PRIM_OP_MARKER`): its candidates are
            // judged after the fixpoint (`primop_contested`), never linked.
            // SOUNDNESS R1540 — a std-typed receiver's method that std ALSO has by name and arity, beside a crate impl of
            // that name for that std type (`STD_RECV_AMBIG_MARKER`): which one runs is not decidable here, so the
            // candidates are judged after the fixpoint (`stdrecv_contested`), never linked.
            // SOUNDNESS R1540 — a std-typed receiver's method no std method and no crate unit answers, where unexpanded
            // macro text writes `fn <method>`: R452 (b)'s disclosure, for a receiver that arm cannot see.
            if let Some(t2) = c.path.strip_prefix(&format!("{STD_RECV_MACRO_MARKER}::")) {
                if !by_tail2.contains_key(t2) && merged.macro_hidden_fns.contains(&c.leaf) {
                    if candor_report::env::trace("CANDOR_R1540_INSTR") {
                        eprintln!("R1540MACRO\t{}\t{t2}", f.qual); // §E1 REACH COUNTER
                    }
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    unknown_why.entry(f.qual.clone()).or_default().insert("macro:module items hidden by an unexpanded macro".to_string());
                }
                continue;
            }
            if let Some(t2) = c.path.strip_prefix(&format!("{STD_RECV_AMBIG_MARKER}::")) {
                if let Some(v) = by_tail2.get(t2) {
                    let cl: Vec<String> = v.iter().filter(|q| *q != &f.qual).cloned().collect();
                    if !cl.is_empty() {
                        stdrecv_contested.push((f.qual.clone(), t2.to_string(), cl));
                    }
                }
                continue;
            }
            if let Some(t2) = c.path.strip_prefix(&format!("{PRIM_OP_MARKER}::")) {
                if let Some(v) = by_primop.get(t2) {
                    let cl: Vec<String> = v.iter().filter(|q| *q != &f.qual).cloned().collect();
                    if !cl.is_empty() {
                        primop_contested.push((f.qual.clone(), t2.to_string(), cl));
                    }
                }
                continue;
            }
            // SOUNDNESS R1153 — a GLOB CANDIDATE for a bare receiver type (`GLOB_CAND_MARKER`): chained, the
            // dependency's own `Type::leaf` entry is joined where it exists and a miss adds nothing (the glob may not
            // be the origin); unchained, the crate is recorded in the κ ledger like any floored call into it.
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{GLOB_CAND_MARKER}::")) {
                if deps_idx.crates.contains(cr_real) {
                    let key = tail2(rest).map(|t2| format!("{cr_real}#{t2}"));
                    if let Some(de) = key.as_ref().and_then(|k| deps_idx.by_key.get(k)) {
                        if candor_report::env::trace("CANDOR_V053_INSTR") {
                            eprintln!("R1153JOIN\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                        }
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                } else if deps.contains(cr) {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
                continue;
            }
            // SOUNDNESS R1164 — an operator whose right operand is `cr`'s type and whose left is not (`OP_RHS_MARKER`): the
            // impl can only be in `cr`, so `cr`'s own entry for the candidate `Self` key is joined where one exists (a
            // miss in a covered report is its purity claim), and the site is a κ ledger entry for `cr` exactly as a floored
            // qualified call into it is — the coverage filter downstream drops it for a chained, covered `cr`, and keeps
            // it as `invisible` for an unchained, stale, incomplete or judged-nothing one.
            if let Some(rest) = c
                .path
                .strip_prefix(&format!("{cr}::{OP_RHS_MARKER}::"))
                .or_else(|| c.path.strip_prefix(&format!("{cr}::{HOOK_JOIN_MARKER}::")))
            {
                if deps.contains(cr) {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{rest}")) {
                        if candor_report::env::trace("CANDOR_V055_INSTR") {
                            eprintln!("R1164JOIN\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                        }
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                continue;
            }
            // SOUNDNESS R1540 — a std-typed receiver's method a chained dependency's impl FOR THAT STD TYPE may answer
            // (`STD_RECV_JOIN_MARKER`, `cr::<stdrecvjoin>::Leaf::method`, formed only for a method std does not have by
            // name and arity): the orphan rule puts `impl cr::Tr for AtomicUsize` in `cr`, whose report files it under
            // `cr#AtomicUsize::method`, and only a trait's impl can answer such a call. A hit is joined. A miss where a
            // chained trait carrying the member is in scope is R887's disclosure (that row's typed-call arm never sees a
            // std-headed receiver, which forms no typed call) — unless that trait's union row for the member is published
            // PURE: every implementor its producer read is then pure, the receiver's impl among them (a covered report's
            // miss is its purity claim). An unchained `cr` adds nothing here: R1136's ledger entry already speaks for it.
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{STD_RECV_JOIN_MARKER}::")) {
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{rest}")) {
                        if candor_report::env::trace("CANDOR_R1540_INSTR") {
                            eprintln!("R1540JOIN\t{}\t{cr_real}#{rest}", f.qual); // §E1 REACH COUNTER
                        }
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    } else {
                        let fu = f.loc.split(':').next().and_then(|r| file_uses.get(r)).copied().unwrap_or(&[]);
                        let pure_union = |tr: &str| {
                            deps_idx.by_key.get(&format!("{tr}::{}", c.leaf)).is_some_and(|u| u.effects.is_empty() && u.invisible.is_empty())
                        };
                        if let Some(tr) = crate::typesurf::in_scope_publishes(deps_idx, fu, &dep_renames, &local_trait_quals, &c.leaf)
                            .filter(|tr| !pure_union(tr))
                        {
                            let tleaf = tr.split_once('#').map_or(tr.as_str(), |(_, q)| q).rsplit("::").next().unwrap_or("").to_string();
                            if candor_report::env::trace("CANDOR_R1540_INSTR") {
                                eprintln!("R1540SCOPE\t{}\t{cr_real}#{rest}\t{tr}", f.qual); // §E1 REACH COUNTER
                            }
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tleaf}.{}", c.leaf));
                        }
                    }
                }
                continue;
            }
            if c.path.starts_with(&format!("{cr}::{EXT_METHOD_MARKER}::")) {
                if deps.contains(cr) {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
                continue;
            }
            if let Some(ty) = c.path.strip_prefix(&format!("{cr}::{DROP_MARKER}::")) {
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{ty}::drop")) {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                continue;
            }
            // ⟨0.40⟩ SOUNDNESS R861 — A GUESSED OWNER (`cr::<chain>::<steps>::<base>::<method>`, see
            // `CHAIN_GUESS_MARKER`). The typed call beside this marker joined the chain's BASE type's
            // member on the assumption that every step returned its receiver; that join is KEPT. Here the
            // chain is re-typed from the dependency's own surface: every step answered by `returns` (or
            // `returnsProtocol`) and the final type's member reached by the walk is a RESOLUTION, joined
            // and ADDED; anything less — a step no trusted surface answers, a member the walk does not
            // reach — keeps the guess and ADDS `Unknown` (SPEC §2 ⟨0.40⟩ "every miss keeps the guess and
            // ADDS `Unknown`", which names this walk). `clone` is the one step whose type is the
            // receiver's by contract (`Clone::clone(&self) -> Self`).
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{CHAIN_GUESS_MARKER}::")) {
                // SOUNDNESS R1617(4) — a marker through an accessor step is the unchained re-typing's alone (see
                // `R1617_ACCESSOR_STEP`): R861's chained walk never saw one and does not start now.
                let accessor_marker = rest.starts_with(crate::lazy::R1617_ACCESSOR_STEP);
                if deps_idx.crates.contains(cr_real) && !accessor_marker {
                    let parsed = rest.rsplit_once("::").and_then(|(head, method)| {
                        head.split_once("::").map(|(steps, base)| (steps, base, method))
                    });
                    // Only where the guessed typed call beside this marker really ASKED the chained
                    // report: a call the builtin table classifies is answered by the rule, not by a lookup
                    // on the guessed owner (the chained join is skipped for it), so the miss rule has
                    // nothing to cover there.
                    let typed_classified = parsed.is_some_and(|(_, base, method)| {
                        let tp = format!("{cr_real}::{base}::{method}");
                        candor_classify::classify(cr_real, &tp).is_some()
                            || scan_builder_entry_effect(cr_real, &tp).is_some()
                    });
                    if let Some((_, base, method)) = parsed.filter(|(steps, _, _)| steps.is_empty() && !typed_classified) {
                        // A VALUE PATH read as a unit struct. BACKED when the manifest says `base` is a
                        // type; when `holds` says it is a VALUE, its declared type is joined (the guess
                        // beside it is kept); neither → the miss rule's hedge.
                        let ts = &deps_idx.surface;
                        let vkey = format!("{cr_real}#{base}");
                        let held: Vec<String> = ts.holds_of(&vkey).map(|t| t.iter().cloned().collect()).unwrap_or_default();
                        for t in &held {
                            let (rows, _) = crate::typesurf::join_through(deps_idx, t, method, false);
                            for de in rows {
                                apply_dep_fn(de, &f.qual, DepSink {
                                    direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds,
                                    paths: &mut paths, tables: &mut tables, incomplete: &mut incomplete,
                                    unknown_why: &mut unknown_why, blind_direct: &mut blind_direct,
                                    dep_invisible: &mut dep_invisible, unknown_via_dep: &mut unknown_via_dep,
                                });
                            }
                        }
                        let backed = !ts.distrusted.contains(cr_real)
                            && (!held.is_empty() || ts.type_key(cr_real, base).is_some());
                        if r843_probe {
                            eprintln!("R843UNIT\t{}\t{vkey}\t.{method}\tbacked={backed}", f.qual);
                        }
                        if !backed {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default()
                                .insert("dispatch:untyped cross-package receiver".to_string());
                        }
                    } else if let Some((steps, base, method)) = parsed.filter(|_| !typed_classified) {
                        let ts = &deps_idx.surface;
                        let mut cur: Vec<String> =
                            vec![ts.type_key(cr_real, base).unwrap_or_else(|| format!("{cr_real}#{base}"))];
                        let mut forced = false;
                        let mut answered = true;
                        for step in steps.split('.') {
                            if step == "clone" {
                                continue;
                            }
                            let mut next: BTreeSet<String> = BTreeSet::new();
                            forced = false;
                            for t in &cur {
                                let k = format!("{t}::{step}");
                                if let Some(r) = deps_idx.returns.get(&k) {
                                    next.insert(r.clone());
                                } else if let Some(ps) = ts.rproto.get(&k) {
                                    next.extend(ps.iter().cloned());
                                    forced = true;
                                } else {
                                    answered = false;
                                }
                            }
                            if !answered {
                                break;
                            }
                            cur = next.into_iter().collect();
                        }
                        let mut all_hit = answered;
                        if answered {
                            for t in &cur {
                                let (rows, w) = crate::typesurf::join_through(deps_idx, t, method, forced);
                                all_hit &= !w.hits.is_empty();
                                for de in rows {
                                    apply_dep_fn(de, &f.qual, DepSink {
                                        direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds,
                                        paths: &mut paths, tables: &mut tables, incomplete: &mut incomplete,
                                        unknown_why: &mut unknown_why, blind_direct: &mut blind_direct,
                                        dep_invisible: &mut dep_invisible, unknown_via_dep: &mut unknown_via_dep,
                                    });
                                }
                            }
                        }
                        if r843_probe {
                            eprintln!("R843CHAIN\t{}\t{cr_real}#{base}\t{steps}\t.{method}\tanswered={answered}\thit={all_hit}",
                                f.qual);
                        }
                        if !all_hit {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default()
                                .insert("dispatch:untyped cross-package receiver".to_string());
                        }
                    }
                } else if !deps_idx.crates.contains(cr_real) && deps.contains(cr) && !candor_report::env::switch("CANDOR_R1617H_OFF") {
                    // SOUNDNESS R1617(4) — an UNCHAINED dependency's chain `r1617_chain_type` could not re-type (its source
                    // refuses — reqwest's `pin_project!` makes the whole crate unreadable — or a step it cannot place). The
                    // guessed call beside this marker is charged by a rule keyed on the step's LEAF (`reqwest::*::send`), but
                    // its second effects are keyed on the step's TYPE (`RequestBuilder::send` + Log), which the guess does not
                    // name: `deny Log` passed over `c.get(u).send()`. Where the guessed path's primary charges, every second
                    // effect the crate's rows give that method on ANY of its types is charged too — the step's type is one of
                    // them. Never where the primary misses: that would be a charge with no rule behind the call at all.
                    let parsed = rest.rsplit_once("::").and_then(|(head, method)| {
                        head.split_once("::").map(|(steps, base)| (steps, base, method))
                    });
                    if let Some((steps, base, method)) = parsed.filter(|(st, _, _)| !st.is_empty()) {
                        let tp = format!("{cr_real}::{base}::{method}");
                        if let Some(primary) = candor_classify::classify(cr_real, &tp) {
                            let have = candor_classify::also_effects(cr_real, &tp);
                            for e2 in candor_classify::also_effects_by_method(cr_real, method) {
                                if e2 == primary || have.contains(&e2) {
                                    continue;
                                }
                                direct.entry(f.qual.clone()).or_default().insert(e2);
                                if candor_classify::also_hides_locator(e2) {
                                    incomplete.entry(f.qual.clone()).or_default().insert(e2);
                                }
                                if e2 == "Fs" {
                                    fskinds.entry(f.qual.clone()).or_default().insert("?".to_string());
                                }
                                if candor_report::env::trace("CANDOR_R1617_INSTR") {
                                    eprintln!("R1617UNION\t{}\t{e2}\t{tp}\t{steps}", f.qual); // §E1 REACH COUNTER
                                }
                            }
                        }
                    }
                }
                continue;
            }
            // COULD-NOT-FORM-A-KEY: `cr::<untyped>::method`. The receiver came from `cr` but we never
            // learned its type, so no lookup happened — and the dep report's silence is only an answer to
            // a question that was asked. Disclose `Unknown` with the existing `dispatch:` reason class
            // (SPEC §2: "a project abstraction with no visible impl"), which is what this is from the
            // consumer's side. Gated on the crate being a DECLARED dependency, so the marker is inert for
            // a local module that merely looks crate-qualified.
            //
            // Deliberately NOT `invisible`: the κ ledger correctly excludes a crate that HAS a chained
            // sibling report, which is exactly this case — so `invisible` would be filtered away and the
            // disclosure lost. That filtering is right for keyed-and-missed and wrong here; the two need
            // different spellings. See DEP-RECEIVER-TYPING-DESIGN.md.
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{UNTYPED_RECV_MARKER}::")) {
                // SOUNDNESS R1578 §E1 CENSUS — every untyped-receiver marker that reaches here (an R1578-typed one never
                // does), with whether its crate is chained and declared: the population the unchained floor would price.
                if candor_report::env::trace("CANDOR_R1578_CENSUS") {
                    eprintln!("R1578MARK\t{}\t{}\tchained={}\tdeclared={}", f.qual, c.path, deps_idx.crates.contains(cr_real), deps.contains(cr));
                }
                // SOUNDNESS R807 — THE PROBE THAT CAN SEE AN UNTYPED HANDLE. Diagnostic only; no
                // behaviour. Every masking probe (`R379MASK`, `R460*`, `R417MASK`, `UNMASKED`) lives
                // inside the CLASSIFIED arm below, so a call that was never classified emits none of
                // them — and that is R807's shape: `let s = UdpSocket::bind(..)?; s.send_to(buf, dst)`
                // left `s` untyped, the `send_to` was never formed as `UdpSocket::send_to`, and the
                // caller-chosen destination vanished with no tag at all. A 0/0/0 A/B over a fix for it
                // would read exactly like an inert one.
                //
                // It prints a method call whose receiver was PRODUCED by a factory `classify` charges
                // (`std::net::UdpSocket::bind` → Net) but whose own type was never learned, so the call
                // reached no rule. `loc=1` is the gate-relevant subset: the method is a locator-bearing
                // verb for that effect (`masks_locator`), i.e. a runtime destination a benign sibling
                // literal could certify. Placed BEFORE the chained-dep conjunct below on purpose: a
                // `std` factory is never a declared dependency, so that branch is where it went silent.
                if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                    if let Some((callee, method)) = rest.rsplit_once("::") {
                        let factory = format!("{cr_real}::{callee}");
                        if let Some(eff) = candor_classify::classify(cr_real, &factory) {
                            let loc = candor_classify::masks_locator(
                                eff, cr_real, &format!("{factory}::{method}"), method, true);
                            eprintln!("R807UNTYPED {eff} loc={} {} :: {factory} .{method}()", u8::from(loc), f.qual);
                        }
                    }
                }
                // THIRD conjunct, and it is what makes this precise rather than noisy: the dep must be
                // CHAINED. For an UNCHAINED dep the κ ledger already discloses `invisible: [cr]`, so the
                // reader is warned and a second disclosure buys nothing but false uncertainty — measured,
                // that arm fired on `let finds = dep::best_finds(); finds.first()`, where the value came
                // from a dep but `.first()` is a std Vec method. Only when the dep IS chained does the
                // ledger fall silent (covered, correctly, per §2 rule 3) — and that silence is the
                // confident purity claim this exists to prevent.
                // SOUNDNESS R856/R857 — THE κ LEDGER FOR A DEPENDENCY VALUE, which no CALL carries. The
                // factory form's provenance is itself a qualified call (`dep::build()`), so the ledger
                // below already counts the crate for it; a static or a field hop is not a call at all,
                // and with the dependency UNCHAINED the caller was ABSENT with no `invisible` either —
                // the same silence as R856, one mode over (measured: `ratescore::SHARED.ping()`,
                // `n.parent.visit()`, `t.0.visit()` all absent, no `invisible`, while every caller that
                // also made a qualified call carried `invisible: ["ratescore"]`). Recorded exactly as
                // the ledger records a floored qualified call, so the coverage filter downstream (which
                // drops a chained, calibrated or otherwise covered crate) is the one authority on whether
                // it surfaces — for a CHAINED crate it is filtered and the `Unknown` below speaks instead.
                let new_prov = rest.contains(VALUE_PROV_SEG) || rest.contains(TYPE_PROV_SEG)
                    || rest.contains(FIELD_HOP_SEG);
                if new_prov && deps.contains(cr) {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
                // SOUNDNESS R1580 — …AND THE FACTORY, METHOD AND CHAIN PROVENANCE TOO, WHEN THE DEPENDENCY IS UNCHAINED. The note
                // above leaned on the factory CALL's own ledger entry ("the ledger below already counts the crate for it"), and
                // that entry exists only while the call goes unmatched by a rule in a crate the ledger keeps: a classified factory
                // (`async_nats::connect`, a `Log` rule on `jetstream::new`) drops it, and a CLASSIFIER-COVERED crate (calibrated
                // within its ceiling, or no lockfile to exceed one) never records it. Then the method on the value reached
                // nothing and the caller read ABSENT over an executed request. Reaching here means SOUNDNESS R1578 could not type
                // the receiver from the dependency's source either, so no key was formed: the ledger counts the crate where it
                // discloses, and where the crate is covered — the ledger's silence an answer only about keys it was asked —
                // the caller discloses `Unknown` with the chained arm's reason. Never a charge.
                // ROUND 3 PRICING — a receiver R1578 placed on a STD type (or a still-wrapping `Result`/`Option`) calling a
                // method std has is std's call: no dependency body runs, so no floor.
                // ROUND 4 NARROWING — a method name std itself has is not floored at all (std's method, or a Result/Option
                // adapter on the value): the floor names only a method std does NOT have, i.e. a dependency's.
                let std_call = !new_prov && deps.contains(cr) && !deps_idx.crates.contains(cr_real)
                    && crate::std_methods::is_std_method_name(&c.leaf);
                if std_call && candor_report::env::trace("CANDOR_R1578_INSTR") {
                    eprintln!("R1580STD\t{}\t{}", f.qual, c.path); // §E1 — a floor SUPPRESSED as std's own call
                }
                // OPT-IN (round 6): on the 1,510-crate corpus a 40-site sample of this floor read S 0 / R 4 / F 36 — its
                // reach there is almost all false uncertainty — so it runs only under `CANDOR_R1580=1`. SOUNDNESS R1580
                // stays OPEN; the default behaviour is the pre-floor one.
                if !new_prov && !std_call && deps.contains(cr) && !deps_idx.crates.contains(cr_real)
                    && candor_report::env::switch_value("CANDOR_R1580", candor_report::env::Selects::AnyOf(&["1"]))
                {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                    // ROUND 5 CUT — the floor's `Unknown` speaks for a COVERED crate, where the classifier is the authority on
                    // what a formed key would mean. When the lock-pinned source shows every type that defines this method,
                    // ask the classifier of each: if none has an effect rule, a formed key would read pure whichever type the
                    // value is, so there is nothing to disclose. A method the source does not define (a third crate's trait
                    // method), or a source this reading cannot see whole, keeps the floor.
                    let classifier_pure = || -> bool {
                        let owners = r1578_sigs.borrow_mut().method_owners(dir, cr, cr_real, &c.leaf);
                        if candor_report::env::trace("CANDOR_R1578_INSTR") {
                            eprintln!("R1580OWNERS\t{}\t{}\t{:?}", f.qual, c.path, owners.as_ref().map(|o| o.len())); // §E1
                        }
                        owners.is_some_and(|o| !o.is_empty()
                            && o.iter().all(|q| candor_classify::classify(cr_real, &format!("{cr_real}::{q}::{}", c.leaf)).is_none()))
                    };
                    let cut = !candor_report::env::switch("CANDOR_R1580_CUT_OFF") && classifier_pure();
                    if cut && candor_report::env::trace("CANDOR_R1578_INSTR") {
                        eprintln!("R1580CUT\t{}\t{}", f.qual, c.path); // §E1 — the floor withheld: every owner classifies pure
                    }
                    if !kappa_blind(&cr.to_string()) && !cut {
                        let fresh = direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why.entry(f.qual.clone()).or_default()
                            .insert("dispatch:untyped cross-package receiver".to_string());
                        if candor_report::env::trace("CANDOR_R1578_INSTR") {
                            eprintln!("R1580FLOOR\t{}\t{}\tfresh={fresh}\t{}", f.qual, c.path, c.prov.as_deref().unwrap_or("-")); // §E1 REACH COUNTER
                        }
                    }
                }
                if deps_idx.crates.contains(cr_real) {
                    // SOUNDNESS R856/R857 §E1 REACH COUNTER, on the new provenance only: a receiver that
                    // is a dependency VALUE (`<value>`) or a field of one (`<type>`/`<field>`). Their
                    // callees carry an angle-bracket segment, so the `returns` lookup below misses by
                    // construction and they always reach the disclosure — the counter is what lets an
                    // A/B tell "never fired" from "fired and changed nothing".
                    if new_prov && candor_report::env::trace("CANDOR_R856_INSTR") {
                        eprintln!("R856HIT\t{}\t{cr_real}::{rest}", f.qual);
                    }
                    // ⟨0.40⟩ THE DECLARED TYPE (SPEC §2 ⟨0.40⟩; SOUNDNESS R843). A chained `holds` (a
                    // static, a field, an associated const), a `returns` or a `returnsProtocol` names what
                    // this receiver IS, so its member is joined — through the walk, so an inherited one is
                    // found — and that join is ADDED. Nothing below changes: the typed guess and the
                    // `returns` join still run, and the `dispatch:untyped cross-package receiver`
                    // disclosure still fires wherever it fired. SPEC permits withdrawing it where every
                    // walk path hits; this engine does not (CHANGELOG ⟨0.40⟩ — the removal direction).
                    // A stale copy beside a trusted one does not outrank it (§2 rule 1, TRUST LEVELS DO
                    // NOT RANK): the surface index holds only trusted copies, and the stale copy's own
                    // entries are already downgraded to `Unknown` in `by_key`.
                    {
                        if let Some((callee, method)) = rest.rsplit_once("::") {
                            if let Some((targets, forced)) =
                                crate::typesurf::receiver_targets(deps_idx, cr_real, callee)
                            {
                                for t in &targets {
                                    let (rows, w) = crate::typesurf::join_through(deps_idx, t, method, forced);
                                    if r843_probe {
                                        eprintln!("R843HOLDS\t{}\t{cr_real}::{rest}\t{t}\thits={}\tstructural={}",
                                            f.qual, w.hits.len(), w.structural);
                                    }
                                    for de in rows {
                                        apply_dep_fn(de, &f.qual, DepSink {
                                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds,
                                            paths: &mut paths, tables: &mut tables,
                                            incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                                            unknown_via_dep: &mut unknown_via_dep,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    // SOUNDNESS R856 — A DEPENDENCY VALUE PATH WHOSE LEAF IS TYPE-SHAPED NAMES ITS OWN
                    // TYPE when it is a unit struct: `ratescore::Other.ping()` is `Other::ping` on the
                    // value `Other`. That is the same reading `resolve_recv_type`'s unit-struct fallback
                    // already gives the IMPORTED spelling (`use ratescore::Other; Other.ping()`), which
                    // types and joins; the qualified spelling had no route and read ABSENT. So ask the
                    // chained report the typed key first — a RESOLUTION from the entry hashes the chain
                    // already publishes, not a new fact — and disclose only on a miss.
                    //
                    // Confined to a path whose segment BEFORE the leaf is not itself type-shaped, so
                    // the leaf is an item of a MODULE: `ratescore::Kind::A` (an enum variant, whose type
                    // is `Kind`) and `ratescore::Other::DEFAULT` (an associated const) never form a key
                    // and fall to the disclosure. A type-ident const coexisting with a same-named
                    // non-unit struct is the one mis-read left, and it is the bare spelling's today.
                    // Full key, then the 2-segment tail — the same two keys the ordinary typed join
                    // asks, so the two spellings answer alike.
                    if let Some((callee, method)) = rest.rsplit_once("::") {
                        if let Some(vp) = callee.strip_prefix(&format!("{VALUE_PROV_SEG}::")) {
                            let segs: Vec<&str> = vp.split("::").collect();
                            let leaf_ty = segs.last().copied().unwrap_or("");
                            let owner_is_module = segs.len() < 2
                                || !crate::lang::is_type_ident(segs[segs.len() - 2]);
                            if !vp.contains('<') && crate::lang::is_type_ident(leaf_ty) && owner_is_module {
                                let full = format!("{cr_real}#{vp}::{method}");
                                let t2 = format!("{cr_real}#{leaf_ty}::{method}");
                                let typed_hit =
                                    deps_idx.by_key.get(&full).or_else(|| deps_idx.by_key.get(&t2));
                                if candor_report::env::trace("CANDOR_R856_INSTR") {
                                    eprintln!("R856TYPED-{}\t{}\t{full}",
                                        if typed_hit.is_some() { "HIT" } else { "MISS" }, f.qual);
                                }
                                if let Some(de) = typed_hit {
                                    apply_dep_fn(de, &f.qual, DepSink {
                                        direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds,
                                        paths: &mut paths, tables: &mut tables,
                                        incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                                        blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                                        unknown_via_dep: &mut unknown_via_dep,
                                    });
                                    // ⟨0.40⟩ "every miss keeps the guess and ADDS `Unknown`" — and a HIT
                                    // on a GUESSED owner too. Reading the value `dep::X` as the unit
                                    // struct `X` is a naming convention; it is a fact only when a trusted
                                    // surface says `X` is a type (its `types` key) or says what the value
                                    // `X` holds (`holds`, joined above). Neither → keep the join, hedge.
                                    let ts = &deps_idx.surface;
                                    let backed = !ts.distrusted.contains(cr_real)
                                        && (ts.holds_of(&format!("{cr_real}#{vp}")).is_some()
                                            || ts.type_key(cr_real, vp).is_some());
                                    if !backed {
                                        if r843_probe {
                                            eprintln!("R843GUESS\tunit\t{}\t{full}", f.qual);
                                        }
                                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                        unknown_why.entry(f.qual.clone()).or_default()
                                            .insert("dispatch:untyped cross-package receiver".to_string());
                                    }
                                    continue;
                                }
                            }
                        }
                    }
                    // ⟨typeSurface.returns⟩ DETERMINATION BEFORE DISCLOSURE (half 2). `rest` is
                    // `<callee path>::<method>`; the method leaf is one segment and the callee path is
                    // not, so it splits from the RIGHT. If the dependency PUBLISHED what that factory
                    // returns, the receiver's type is recoverable and the real key can be formed after
                    // all — a FULL qual on both ends, answered by the full-qual key the dep index now
                    // carries.
                    let split = rest.rsplit_once("::");
                    let hit = split.and_then(|(callee, method)| {
                        let ty = deps_idx.returns.get(&format!("{cr_real}#{callee}"))?;
                        deps_idx.by_key.get(&format!("{ty}::{method}"))
                    });
                    // Instrument the PRECONDITION, not just the output: a diff cannot show that a
                    // mechanism never fired or fired on the wrong thing (standing bar item 8).
                    if candor_report::env::trace("CANDOR_TYPESURFACE_DEBUG") {
                        let (callee, method) = split.unwrap_or((rest, ""));
                        let ty = deps_idx.returns.get(&format!("{cr_real}#{callee}"));
                        eprintln!(
                            "TYPESURFACE-{} {} :: {cr_real}#{callee} -> {} :: .{method}()",
                            if hit.is_some() { "HIT " } else { "MISS" },
                            f.qual,
                            ty.map(String::as_str).unwrap_or("<no returns entry>")
                        );
                    }
                    if let Some(de) = hit {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                        continue;
                    }
                    // A MISS — on `returns` or on `by_key` — FALLS THROUGH TO THE DISCLOSURE, never to
                    // silence. This is defect 3 of the reverted attempt, which read a `by_key` miss after
                    // a `returns` hit as a keyed-and-missed and stayed quiet. `by_key` deliberately DROPS
                    // ambiguous keys ("never guess"), so a miss cannot distinguish "no such method" from
                    // "I withdrew an entry" — and that is not hypothetical: measured over three crates'
                    // dep trees, adding the full-qual key alone withdrew 1865 keys on pgman as
                    // full-qual-vs-full-qual collisions. A refusal to answer is not a purity claim.
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    // A COARSE token, like the `callback:` one above — the reason set is `&'static str`,
                    // and interpolating the crate/method here would mean leaking a string per call site.
                    unknown_why
                        .entry(f.qual.clone())
                        .or_default()
                        .insert("dispatch:untyped cross-package receiver".to_string());
                }
                continue;
            }
            if let Some(rest) = c.path.strip_prefix(&format!("{cr}::{LAZY_UNIT_PREFIX}::")) {
                if deps_idx.crates.contains(cr_real) {
                    if let Some(de) = deps_idx.by_key.get(&format!("{cr_real}#{LAZY_UNIT_PREFIX}::{rest}")) {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
                continue;
            }
            // ⟨renamed-dep classify() alias⟩ SOUNDNESS finding: `classify()`'s rule tables (and
            // `scan_builder_entry_effect`'s/`is_model_sdk_crate`'s) are written against the REAL published
            // crate — `cr` is the manifest's own alias when `[dependencies] alias = { package = "real" }`
            // renames it, so a call through the alias (`htclient::blocking::get` for `reqwest` renamed to
            // `htclient`) previously never matched any rule at all. NOT a cardinal sin — the miss still
            // fell through to the honest `invisible`-qualified floor below, never a fabricated purity claim
            // — but it silently downgraded a KNOWN, calibrated effect (`deny Net` exiting 1 on the
            // unaliased crate) to an advisory `invisible` disclosure that a `deny` gate does not act on
            // (`deny Net` exits 0 on the byte-identical renamed call). Live-reproduced against this fix's
            // pre-fix binary: a `reqwest` dependency renamed `htclient` read `inferred: []`,
            // `invisible: ["htclient"]`, `deny Net` exit 0; the SAME call unaliased read `inferred:
            // ["Net"]`, `deny Net` exit 1. rust-deep is unaffected (`cx.tcx.crate_name` resolves the
            // compiled crate's true identity independent of any local extern alias) — verified via
            // `cargo dylint` against the identical renamed-reqwest fixture, `inferred: ["Net"]` unchanged.
            // Fixed by resolving `cr_real` (already computed above, and already used by every OTHER
            // crate-identity-keyed lookup in this loop — the CANDOR_DEPS joins) before classification, and
            // rewriting the path's own leading segment too: several `classify()` rules match a full EXACT
            // path (`path == "git2::Repository::clone"`), which a resolved crate_name paired with an
            // alias-prefixed path would still never satisfy.
            let path_real: std::borrow::Cow<str> = if cr_real == cr {
                std::borrow::Cow::Borrowed(c.path.as_str())
            } else {
                std::borrow::Cow::Owned(format!("{cr_real}{}", &c.path[cr.len()..]))
            };
            // SPEC §1 ⟨0.13⟩ `Llm` model-SDK surface (candor_classify::MODEL_SDK_CRATES): a qualified call
            // into a curated model-provider client → `Llm` + `Net` (Net is never dropped — a model call IS
            // network I/O). No method gating (single-purpose clients), the analog of java's isModelSdkOwner.
            let model_sdk = c.path.contains("::") && candor_classify::is_model_sdk_crate(cr_real);
            // SOUNDNESS R330 — the ONE arity-keyed refinement, applied where the argument count is in
            // hand. `classify` charges `::hash_password` unconditionally because the PATH cannot tell
            // password-hash 0.5.x's `(password, salt)` from 0.6.x's `(password)`; this narrows that
            // charge, and only for the shape it can prove is the explicit-salt one. Direction: the
            // refinement can only REMOVE a fabricated `Rand`, never create a silence — an unrecorded
            // arity leaves the charge standing, which is why `Call::argc`'s 0 sentinel is safe.
            let explicit_salt =
                candor_classify::password_hash_explicit_salt(cr_real, &path_real, c.argc, c.method);
            let classified = candor_classify::classify(cr_real, &path_real)
                .filter(|_| !explicit_salt)
                .or_else(|| scan_builder_entry_effect(cr_real, &path_real))
                .or(if model_sdk { Some("Llm") } else { None });
            // DROP-GLUE detection, ONE route. A `Type::<construct>` marker says the collector saw this
            // body CONSTRUCT a `Type` — in ANY position, in any of the three construction spellings —
            // and that the value does not lexically escape the scope. Its `Drop::drop` therefore runs
            // at scope exit: an implicit edge the syntactic call graph never sees.
            //
            // This REPLACES a `tail2`-on-every-call heuristic that lived here, and the replacement is
            // the point rather than a refactor. That rule could only see the `Type::assoc()` spelling,
            // so `Guard(f)` — a single-segment `Expr::Call`, the newtype-guard idiom — failed its
            // `contains("::")` test and `use m::Guard; Guard(f)` presented `m` as the type. Beside it
            // sat a BINDER-keyed marker in the collector, which is where the other half of the vein
            // was. Keying both on the construction expression is what makes the positions equal, and
            // deleting the second authority is what stops them drifting apart again.
            if c.leaf == CONSTRUCT_MARKER {
                let ty = c.path.split("::").next().unwrap_or("").to_string();
                if merged.drop_types.contains(&ty) {
                    drops_here.insert(ty.clone());
                    drop_quals.entry(ty.clone()).or_default().insert(c.str_arg.clone());
                }
                // FIELD edition (R49): constructing `ty` also runs the drop glue of any local drop-type
                // it transitively OWNS through a field. The collector's lexical escape gate has already
                // refused an escaping construction; `returns_escapable` is kept here as a SECOND, purely
                // NARROWING conjunct on this half alone, so the field route can only lose charges
                // relative to what shipped — R49's A/B is not being re-opened by this change.
                if !returns_escapable {
                    // SOUNDNESS R1723 — a unit in a file the default build does not compile reads every twin arm (R977).
                    let in_off_file = f.loc.rsplitn(3, ':').nth(2).is_some_and(|file| cfg_off_files.contains(file));
                    let od = if in_off_file { &owned_drops_all } else { &owned_drops };
                    if let Some(owned) = od.get(&ty) {
                        drops_here.extend(owned.iter().cloned());
                        for o in owned {
                            drop_quals.entry(o.clone()).or_default().insert(None); // R1253 — a field's twin is not spelled
                        }
                    }
                }
                // The marker is consumed HERE and nowhere else: it names no crate, resolves to no unit,
                // and must not reach the κ ledger's call tally (an earlier cross-crate marker inflated
                // exactly that count, which the envelope test caught).
                continue;
            }
            // κ ledger: a qualified call into a declared dependency. (A bare leaf has no `::`, so it
            // can't name a crate; a local module sharing a dep's name is the rare accepted ambiguity.)
            if c.path.contains("::") && deps.contains(cr) {
                // A FLOORED dep call is a candidate per-fn blind spot (filtered to genuinely-blind
                // below). A CLASSIFIED one is not — its effect is on the record — so it is counted in
                // NEITHER the ledger nor the call tally. Coverage is a REVIEW claim, not a resolution
                // outcome: κ matching `zip::ZipArchive::new` vouches for THAT call, never for the crate,
                // so a single classified call must not clear the blind marker for every other call shape
                // into it. The vouching mechanisms are the CALIBRATED_* lists and a chained sibling
                // report — both of which someone reviewed.
                if classified.is_none() {
                    *dep_seen.entry(cr.to_string()).or_insert(0) += 1;
                    // ⟨2026-08-29 ADVERSARIAL REVIEW, finding 2 — MEASURED, ACCEPTED, NOT FIXED HERE⟩
                    // `blind_direct` (and every sibling per-fn map keyed the same way: `direct`, `hosts`,
                    // `cmds`, `paths`, `tables`, `fskinds`, `incomplete`, `unknown_why`) is keyed on the
                    // BARE qualified name `f.qual`, which two `#[cfg(...)]` branches of one same-named
                    // function SHARE — `#[cfg(target_os = "macos")] fn get_thing() { core_foundation::… }`
                    // beside `#[cfg(not(target_os = "macos"))] fn get_thing() {}` is real
                    // `crates/cargo-util/src/paths.rs` shape. Reproduced live: BOTH branches' report
                    // entries carried `"invisible": ["core_foundation"]`, one of them on the branch that
                    // provably makes no such call — this insertion charges the crate to `f.qual`, and the
                    // OTHER branch's entry later reads the SAME bucket back.
                    //
                    // PRE-EXISTING, INDEPENDENT OF ⟨caca530⟩/⟨fda08ad⟩/⟨75045f0⟩: `core_foundation` is not
                    // a `CALIBRATED_CRATES` entry, so this keying collision has always fired for an
                    // ordinary uncalibrated blind spot. ⟨75045f0⟩ only widened WHICH calls reach genuine
                    // disclosure (closing the impostor-exemption false-negative); it made this pre-existing,
                    // always-over-reporting quirk visible in more cases, it did not create the mechanism.
                    //
                    // NOT FIXED HERE, and the reason is a genuine constraint, not a schedule excuse:
                    //   1. SAFE DIRECTION. The union is still a TRUE statement about the qualified name
                    //      ("`get_thing` reaches `core_foundation` under some configuration") and a syntactic
                    //      scanner cannot know the build target — `#[cfg(unix)]`/`cfg_if!` arms are
                    //      DELIBERATELY unioned elsewhere in this file on exactly that argument (see
                    //      collector.rs's cfg_if handling). No `deny` gate is weakened: a real violation on
                    //      either branch still fires on the shared name, which is MORE conservative, never
                    //      less. This is over-report noise, never the family's cardinal sin (a silent
                    //      under-report).
                    //   2. A per-declaration-instance fix cannot be a local key rename. `direct`/
                    //      `blind_direct`/etc. are populated once per `FnInfo`'s OWN body walk (unambiguous
                    //      — this call site knows exactly which cfg branch it is in), but `calls` edges
                    //      point at callees BY BARE NAME, because a CALLER genuinely cannot know which cfg
                    //      branch of a callee will run — so `propagate`/`propagate_str`'s transitive closure
                    //      MUST keep resolving callees by bare name; that is not a bug, it is the same
                    //      unavoidable-ambiguity argument as (1) one hop further out. Disambiguating the
                    //      DIRECT maps' keys while leaving `calls`/the propagated `*acc` maps on bare names
                    //      means reconciling two key spaces at the entry-building loop, which currently
                    //      assumes one — a real, cross-cutting change to this function's core data model,
                    //      not a narrow patch.
                    //   3. THE OBVIOUS QUICK FIX IS WRONG, MEASURED: deduplicating `all` so each qual
                    //      produces one report entry was tried and reverted — it collapses two GENUINELY
                    //      DISTINCT source declarations (own `loc` each) into one, which
                    //      `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`
                    //      (candor-scan/tests/cli.rs) pins as WRONG: the report must list both units
                    //      (their own comment: "so this is a GATE de-duplication, not a lost entry"). That
                    //      test's own fixture never caught this finding because both its cfg branches
                    //      genuinely perform the identical call — the cross-contamination is invisible
                    //      whenever the branches AGREE, which is why this shipped for as long as it did.
                    //   4. BLAST RADIUS. `f.qual`/the report's `"fn"` field is the cross-engine identity
                    //      candor-query, gate baselines, `whatif`/`callers` and the other three engines'
                    //      matching convention all key on. A same-file keying fix is tractable; verifying it
                    //      doesn't disturb any of those consumers, and deciding whether java/ts/swift need
                    //      the identical treatment for their own multi-release-jar/conditional-export/
                    //      `#if os()` equivalents, is a cross-engine SPEC question (this family's own "write
                    //      the row before the port" rule) — not a one-file patch bundled into an unrelated
                    //      adversarial-review response. STATED, not silently absorbed: needs its own design
                    //      pass and a BACKLOG entry.
                    blind_direct.entry(f.qual.clone()).or_default().insert(cr.to_string());
                }
            }
            // (The CANDOR_DEPS cross-crate JOIN moved BELOW — it must run AFTER `resolved_local`/
            // `suppress_bare_leaf` are known and be gated on them, else a local fn/method/module named like
            // a covered dep crate inherits that dep's effects onto a provably-pure LOCAL path — the same
            // fabrication the classifier's `resolved_local` guard prevents, which this join
            // never had. Found by the cross-jar sweep.)
            // Resolve the call to a local definition via the precise, uniqueness-filtered `resolve_target`.
            // A receiver-typed `Type::method` call (`x.go()` inferred to `S::go`) resolves to the local
            // method ONLY when `Type` is locally defined — this recovers the common `x.method()` edge that
            // a bare leaf can't safely provide, while an external `reqwest::Client::send` is left to the
            // classifier (its type isn't local, so it can't mis-link to a same-named local `Client::send`).
            // A non-typed call uses the leaf/qualified-tail routes; std/core/alloc are the classifier's.
            let resolvable = if c.is_macro {
                // A macro is never a call to a local FUNCTION. Its (possibly crate-local) qualified path
                // must NOT resolve to a same-named local fn, or that fn's effect is fabricated onto the
                // caller (a phantom-edge fabrication — the precision failure). Its effect still flows via `classified` / κ above.
                false
            } else if c.typed {
                // A std/core/alloc-QUALIFIED typed path names STD's type, and the classifier owns it —
                // the same rule the non-typed branch below states, now stated for the typed branch too.
                // It could be left unsaid while no typed std path existed; the std I/O HANDLE receiver
                // routing creates them, and without this the routing DEFEATS ITSELF: `tail2` throws the
                // `std::process` qualifier away, so a crate that also defines its own `Command` puts
                // that bare leaf in `local_types`, `std::process::Command::spawn` MIS-LINKS to the
                // local method, and `resolved_local` then SUPPRESSES the classifier — the real spawn
                // certifies clean again, with a local type's effects fabricated in its place. MEASURED
                // on a fixture with a local `mine::Command::spawn` beside a std `Command` parameter:
                // exit 0, the cardinal sin reopened by the fix that closed it.
                //
                // This is the invariant the external case already relies on and states two comments
                // up — "an external `reqwest::Client::send` is left to the classifier (its type isn't
                // local, so it can't mis-link to a same-named local `Client::send`)". For an external
                // crate the leaf collision is unlikely; for `Command`/`File`/`Path` it is ordinary.
                // The cost is a local extension-trait impl on a std type (`impl CommandExt for
                // Command`) no longer contributing its edge: `by_tail2` keys BOTH that impl and a
                // local `struct Command`'s method as `Command::spawn`, so honouring one means
                // guessing between them — the collision the `resolved_local` guard exists to refuse.
!matches!(c.path.split("::").next().unwrap_or(""), "std" | "core" | "alloc")
                    && (tail2(&c.path)
                        .and_then(|t2| t2.split("::").next().map(str::to_string))
                        .is_some_and(|ty| local_types.contains(&ty))
                        // R1034 (residual) — a non-path impl's referent tail (`str::encode`), admitted
                        // by its exact tail only; see `nonpath_tails`.
                        || tail2(&c.path).is_some_and(|t2| nonpath_tails.contains(&t2)))
            } else {
                !matches!(cr, "std" | "core" | "alloc")
            };
            // SOUNDNESS R1578 (round 2, hole 4) — a dependency type typed from its source whose leaf the consumer ALSO declares
            // never takes the local route: `tail2` (`Context::query_account`) cannot tell the two apart, and edging the call
            // to the consumer's own `Context` charged that twin's effects to a caller that never runs it (executed on 5e45a4c,
            // `tests/r1578_holes.rs`). The classifier and κ ledger still answer it below. A local impl ON the dependency's
            // type (no local declaration of the leaf — rav1e's `impl .. for bitstream_io::BitWriter`) keeps its edge.
            let resolvable = resolvable && !r1578_twin;
            // A `Type::assoc()` whose `Type` is a NON-NOMINAL alias (`type Inner = [u8; N]`) names a type
            // with no local impl — its assoc fn is std/core's, NOT a same-named local STRUCT's. Skip the
            // local link so the array alias's `Inner::default()` doesn't inherit `struct Inner`'s
            // effectful `Default` (the sled IVec fabrication).
            let aliased = tail2(&c.path)
                .and_then(|t2| t2.split("::").next().map(str::to_string))
                .is_some_and(|ty| merged.prim_aliases.contains(&ty));
            // SOUNDNESS R223 — WHOSE crate does this qualified path name? `tail2` keeps the last TWO
            // segments only, so a call WRITTEN into a dependency (`tokio_postgres::Client::execute`) and a
            // local definition (`generic_client::Client::execute`) present the IDENTICAL key
            // `Client::execute`. The local one wins the lookup, `t == f.qual` then drops the edge as a
            // self-reference, and `resolved_local` SUPPRESSES the `Db` the classifier had — so the
            // newtype/wrapper forwarding idiom (deadpool-postgres, fs-err, cap-std) reports the very
            // methods a consumer calls as PURE. Census over 1,509 registry crates: 123 suppression sites,
            // 99 caller functions, 26 crates, and 59 of the sites leave the caller ABSENT — silent.
            //
            // Cargo.toml is the AUTHORITY on what is external — the same role `std | core | alloc` plays
            // in `resolvable` below for the toolchain's own crates. `cr` is the written head segment and
            // `deps` is keyed the same way (post `-`→`_`, pre `package = "…"` rename), which is why the κ
            // ledger above tests `deps.contains(cr)` with this same expression.
            //
            // A DEP-LOOKING HEAD IS NOT PROOF OF A DEP CALL, and this half was MEASURED, not reasoned.
            // `c.path` is not the written source text: `expand` resolves it through the file's `use` map,
            // and a GLOB import claims every name it could plausibly supply — rdkafka's
            // `use rdkafka_sys::types::*` turns `self.client.native_ptr()`, a call on rdkafka's OWN
            // `client::Client`, into `rdkafka_sys::types::client::Client::native_ptr`. `expand` also
            // STRIPS a written `crate::`/`self::`/`super::` prefix (`lang.rs`, the crate-rooted branch),
            // so an intra-crate call into a module named like a dependency (fs-err's own `mod tokio`,
            // aws-lc-rs's `mod rand`) arrives byte-identical too. What separates the two is whether the
            // local definition's OWN qual is a SUFFIX of the written path:
            // `rdkafka_sys::types::client::Client::native_ptr` ends with `client::Client::native_ptr`, so
            // the head segments are a prefix an import put there and the path names that very definition.
            // `tokio_postgres::Client::execute` does NOT end with `generic_client::Client::execute`, and
            // the only thing linking that call to the local definition is the two segments `tail2` kept.
            //
            // THE SUFFIX TEST NEEDS A THIRD SEGMENT TO MEAN ANYTHING — measured, not tidied. A local
            // definition at the crate ROOT has a two-segment qual (`TlsConnector::connect`), which is a
            // suffix of EVERY path ending in those two segments, so the test degenerates into the tail2
            // match it is supposed to corroborate: tokio-native-tls' `native_tls::TlsConnector::connect`
            // walked straight through it and kept its `Net` suppressed. A suffix therefore only counts as
            // evidence when the definition contributes a segment `tail2` had thrown away — an exact match
            // aside, which needs no corroboration because it IS the path.
            let extern_dep_qualified = c.path.contains("::")
                && deps.contains(cr)
                && !tail2(&c.path)
                    .and_then(|t2| by_tail2.get(&t2))
                    .is_some_and(|v| {
                        v.iter().any(|q| {
                            q == &c.path
                                || (q.matches("::").count() >= 2
                                    && c.path.ends_with(&format!("::{q}")))
                        })
                    });
            // Did this call resolve to a LOCAL definition (free fn, method, or a unique trait-default)?
            // If so the local def is AUTHORITATIVE and its effects flow through the `calls` edge — the
            // crate/FFI classifier MUST NOT also fire, or a pure local fn whose NAME collides with an FFI
            // tier (`sqlite3_step`/`git_clone`/`curl_*`/`SSL_*`) or a whole-crate rule (`getrandom`/
            // `fastrand`) inherits that crate's effect: FABRICATION on a provably-pure path, transitively
            // poisoning every caller (a fabrication — the precision failure the syntactic floor must never commit). The
            // bare-leaf-METHOD suppression below was the special case of this; this covers the general
            // case (free fns and qualified `Type::method` calls the bare-leaf guard missed).
            let mut resolved_local = false;
            // VEIN C — set when a typed call on a local type reaches a DEFAULT it inherits from a
            // DEPENDENCY's trait; the chained join below then answers that member instead of `c.path`.
            let mut dep_default_path: Option<String> = None;
            // SOUNDNESS R1171 — A TRAIT DEFAULT INHERITED BY ONE OF TWO SAME-LEAF IMPLEMENTORS. Units are keyed by the self
            // type's LEAF, so `impl Shut for hdep::server::Connection` (inherits `Shut::dflt`) and `impl Shut for
            // auto::Connection` (overrides it) in one module share `Connection::`: `Connection::dflt` is the override's
            // unit, every route above resolves `<either Connection>.dflt()` to it, and the default the inheriting type
            // runs was never edged (EXECUTED, rustagent-v057/fx/r1169 `via_dflt` and rustagent-v058/fx/r1171b: `Fs`
            // ran, `['Env']` read, chained and unchained; also two LOCAL twins and a free fn's parameter).
            //
            // ADDITIVE ONLY: whatever this call resolved to is kept, so the override's edge rides beside (the
            // over-charge direction), and nothing is withheld on a typing claim. The default is added where the
            // blocks of that trait for that leaf DISAGREE about the member (one declares it, one does not), which is
            // the collision itself: with no twin, a resolved `L::m` is the type's own, and the existing fallback
            // already reaches the default when nothing resolves. A QUALIFIED callee type that is exactly a declaring
            // block's written path and no inheriting block's is that override, and adds nothing; a bare or
            // differently spelled one cannot be told apart, and gets the default (over-charge, never a silence).
            if !c.is_macro {
                let segs: Vec<&str> = c.path.split("::").collect();
                if segs.len() >= 2 {
                    let (ty_leaf, m) = (segs[segs.len() - 2], segs[segs.len() - 1]);
                    let written = crate::model::norm_type_path(&segs[..segs.len() - 1].join("::"));
                    for tr in type_to_traits.get(ty_leaf).into_iter().flatten() {
                        let Some(blocks) = wpath_blocks.get(&(tr.clone(), ty_leaf.to_string())) else { continue };
                        let declares = |b: &WpathBlock| b.2 || b.1.contains(m);
                        let inherits: Vec<&String> = blocks.iter().filter(|b| !declares(b)).map(|b| &b.0).collect();
                        if inherits.is_empty() || !blocks.iter().any(declares) {
                            continue;
                        }
                        let is_override = written.contains("::")
                            && !inherits.iter().any(|w| **w == written)
                            && blocks.iter().any(|b| declares(b) && b.0 == written);
                        if is_override || !trait_decls.get(tr).is_some_and(|lt| lt.methods.contains(m) || lt.assoc.contains(m)) {
                            continue;
                        }
                        if let Some([t]) = by_tail2.get(&format!("{tr}::{m}")).map(Vec::as_slice) {
                            if t != &f.qual {
                                if candor_report::env::trace("CANDOR_R1171_INSTR") {
                                    eprintln!("R1171TWINDFLT\t{}\t{}\t{t}", f.qual, c.path); // §E1 REACH COUNTER, ADDED branch only
                                }
                                calls.entry(f.qual.clone()).or_default().insert(t.clone());
                            }
                        }
                    }
                }
            }
            if resolvable && !aliased {
                let targets = resolve_target(&c.path, &c.leaf, c.method, &by_tail2, &by_leaf_call)
                    .or_else(|| reexport_target(&c.path, c.method, &by_tail2, &by_reexport))
                    // SOUNDNESS R1154 (and the crate-root import it reduces to) — `use crate::write_it;`
                    // then `write_it()` keeps the `use`-map value `crate::write_it`, whose one-segment
                    // remainder has no tail2 key and is not a bare leaf, so the call named NOTHING: every
                    // caller importing a crate-ROOT free fn into a module (`use crate::f`, `use super::f`
                    // from a direct child, a bin's `use mylib::f`) was ABSENT over the fn's effect
                    // (executed). A surviving `crate::` head is crate-root ABSOLUTE (it only comes from a
                    // `use` value, see `macro_hidden_owner`), so `crate::f` names the root's own `f` and
                    // nothing else: the lookup is the EXACT qual, never a leaf guess.
                    .or_else(|| {
                        let r = c.path.strip_prefix("crate::").filter(|r| !r.contains("::") && !c.method)?;
                        // A root name ALSO declared in an `extern` block is a cfg pair (wasm-bindgen's `externs!`: the
                        // extern on wasm32, a panicking stub elsewhere): resolving to the stub would withdraw the §4
                        // `native:extern fn` disclosure below (measured: `JsValue::is_none` went ABSENT). Left to it.
                        if merged.extern_fns.contains(r) {
                            return None;
                        }
                        let hit = by_root.get(r);
                        if hit.is_some() && candor_report::env::trace("CANDOR_R1154_INSTR") {
                            eprintln!("R1154ROOT\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                        }
                        hit
                    })
                    // SOUNDNESS R1154 — a path headed by this package's OWN library name (`mylib::f()` from a bin, or
                    // the `use mylib::f` value): the head names this crate's root, so the rest is resolved as a
                    // crate path. The written path is untouched, so the classifier still reads it (a calibrated
                    // crate's own bin — clap's `stdio-fixture`, duct's `child_grandchild` — keeps its rule's answer).
                    .or_else(|| {
                        let lib = self_lib.as_deref()?;
                        let rest = c.path.strip_prefix(lib)?.strip_prefix("::")?;
                        let hit = if rest.contains("::") {
                            resolve_target(rest, &c.leaf, c.method, &by_tail2, &by_leaf_call)
                        } else if !c.method && !merged.extern_fns.contains(rest) {
                            by_root.get(rest)
                        } else {
                            None
                        };
                        if hit.is_some() && candor_report::env::trace("CANDOR_R1154_INSTR") {
                            eprintln!("R1154SELFLIB\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                        }
                        hit
                    });
                if let Some(targets) = targets {
                    resolved_local = true;
                    // SOUNDNESS R1649 — A TAIL TWO IMPLS CLAIM, ONE OF THEM ON `&T`. `impl Exec for &mut Conn` is keyed at module
                    // level (R1034) and its `Conn::prep` fallback is not filed when `impl AnyBackend for Conn` already claims
                    // that tail, so a typed `conn.prep(q)` resolved to `AnyBackend::prep` whatever the call site imported.
                    // sqlx-postgres `listener`'s `self.connection().await?.prepare_with(..)` (only `Executor` imported) ran
                    // `<&mut PgConnection as Executor>::prepare_with` and read `AnyConnectionBackend`'s (rustc's MIR;
                    // EXECUTED one-variable fixture rustagent-r474/fx/r1649: the write ran, the row was ABSENT, and the pure
                    // twin read the other impl's `Fs`). Rust picks by the trait in scope at the call, so the edge MOVES from the
                    // claimant to the `&T` impl only where that is determined: the `&T` impl's trait is CERTAINLY in scope (one
                    // written `use` names its leaf, and it is not a leaf two of this crate's traits share), the claimant is the one
                    // trait impl of `T::m`, of a DIFFERENT trait, and that trait may NOT be in scope (no glob, no `use` naming it,
                    // not declared in the file): two in-scope candidates at one probe step would not compile. Residual, stated:
                    // a claimant trait in scope under a re-exported alias, met by an OWNED receiver at an earlier probe step.
                    // `CANDOR_R1649_OFF` restores the old reading; `CANDOR_R1649_INSTR` counts both branches.
                    let mut withdrawn: Option<String> = None;
                    if c.method && c.dot && c.typed && !c.is_macro && !r1649_ref.is_empty() {
                        let segs: Vec<&str> = c.path.split("::").collect();
                        if segs.len() >= 2 {
                            let (ty_leaf, m) = (segs[segs.len() - 2], segs[segs.len() - 1]);
                            if let Some(refs) = r1649_ref.get(&format!("{ty_leaf}::{m}")) {
                                let file = f.loc.split(':').next().unwrap_or("");
                                let fu = file_uses.get(file).copied().unwrap_or(&[]);
                                // A glob of ANY root may bring either trait; a written `use` naming the leaf brings it.
                                let any_glob = fu.iter().any(|u| u.ends_with('*'));
                                let declared_here = file_local_traits.get(file);
                                let named = |tr: &str| fu.iter().filter(|u| u.rsplit("::").next() == Some(tr)).count();
                                let maybe = |tr: &str| any_glob || named(tr) > 0 || declared_here.is_some_and(|d| d.contains(tr));
                                // CERTAIN: one written `use` names the leaf, and it is not a leaf two of this crate's own traits share.
                                let imported_once = |tr: &str| {
                                    named(tr) == 1 && merged.trait_quals.get(tr).is_none_or(|v| v.len() == 1)
                                };
                                // Only where rustc's choice is DETERMINED by scope does the resolution move: the claimant is the one
                                // trait impl of `T::m`, its trait may NOT be in scope, and a `&T` impl's (different) trait IS — named by
                                // exactly one written `use`. Everywhere else the old resolution stands untouched: a first cut that
                                // EDGED every `&T` impl whose trait merely might be in scope added 400 edges rustc's MIR does not make
                                // (rustagent-r474/ab1649/mc.tsv: a leaf two types share, `serde_json`'s two `Serializer`s; a generic
                                // receiver typed by its bound's name; a builder-chain receiver guess, tempfile `self.as_file_mut()`).
                                if targets.len() == 1 {
                                    let claim = &targets[0];
                                    let traits: Vec<&str> = merged
                                        .impl_members
                                        .iter()
                                        .filter_map(|k| {
                                            let mut p = k.split('\u{1f}');
                                            let (Some(tr), Some(ty), Some(mm), None) = (p.next(), p.next(), p.next(), p.next()) else {
                                                return None;
                                            };
                                            (ty == ty_leaf && mm == m).then_some(tr)
                                        })
                                        .collect();
                                    let one_unit = fns.iter().filter(|g| &g.qual == claim).count() == 1;
                                    if let [t] = traits.as_slice() {
                                        let winners: Vec<&String> = refs
                                            .iter()
                                            .filter(|(q, tr)| tr != t && imported_once(tr) && q != &f.qual)
                                            .map(|(q, _)| q)
                                            .collect();
                                        if !winners.is_empty() && !maybe(t) && one_unit {
                                            if candor_report::env::trace("CANDOR_R1649_INSTR") {
                                                eprintln!("R1649WITHDRAW\t{}\t{}\t{claim}", f.qual, c.path); // §E1 REACH COUNTER
                                            }
                                            withdrawn = Some(claim.clone());
                                            for q in winners {
                                                if candor_report::env::trace("CANDOR_R1649_INSTR") {
                                                    eprintln!("R1649EDGE\t{}\t{}\t{q}", f.qual, c.path); // §E1 REACH COUNTER
                                                }
                                                calls.entry(f.qual.clone()).or_default().insert(q.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    for t in targets {
                        if t != &f.qual && withdrawn.as_ref() != Some(t) {
                            // SOUNDNESS R1537 (c) — a co-path set's member, where this call site proves which one runs.
                            let t = match copath.get(t.as_str()).and_then(|cp| copath_member(cp, c, &f.qual)) {
                                Some(m) => {
                                    if r1537c_instr {
                                        eprintln!("R1537CPICK\t{}\t{}\t{m}", f.qual, c.path); // §E1 REACH COUNTER
                                    }
                                    m
                                }
                                None => {
                                    if r1537c_instr && copath.contains_key(t.as_str()) {
                                        eprintln!("R1537CUNION\t{}\t{}\t{t}\t{:?}", f.qual, c.path, c.cside); // §E1 REACH COUNTER
                                    }
                                    t
                                }
                            };
                            if t != &f.qual {
                                calls.entry(f.qual.clone()).or_default().insert(t.clone());
                            }
                        }
                    }
                } else if let Some(t) = c
                    .exact_ty
                    .as_deref()
                    .and_then(|q| {
                        let t = exact_ty_target(q, &c.path, &by_tail2)?;
                        let w = crate::collector::exact_ty_writer(q);
                        let g = ExactGuard {
                            use_bound: &merged.use_bound,
                            redirect_mods: &redirect_mods,
                            macro_modules: &merged.macro_modules,
                            twin_field_tys: &twin_field_tys,
                        };
                        exact_ty_module_settled(t, w, &g, &f.qual, &c.path)
                            .then_some(t)
                    })
                {
                    if candor_report::env::trace("CANDOR_EXACTTY_INSTR") {
                        eprintln!("EXACTTY\tPICK\t{}\t{}\t{t}", f.qual, c.path); // §E1 REACH COUNTER, the ADDED branch only
                    }
                    resolved_local = true;
                    if t != &f.qual {
                        calls.entry(f.qual.clone()).or_default().insert(t.clone());
                    }
                } else if let Some(t) = arm_exact_target(&c.path, &by_tail2).filter(|t| {
                    // SOUNDNESS R1675 — a claimant that accounts for only a TAIL of the written path, where the head the source
                    // wrote before it names no module of this scan. polling 3.x: `self.poller: sys::Poller`, `sys` bound by
                    // `use kqueue as sys` inside `cfg_if!` (hidden from the scan), formed `sys::Poller::wait_deadline`, and the
                    // ranked pass settled it on the ROOT `Poller::wait_deadline` (the public wrapper, a cycle) — never the
                    // platform arm that performs the syscall. The written head says the callee is elsewhere; the scan cannot
                    // say where, so the tail keeps its refusal and `ambiguous:` hedge. Kill switch `CANDOR_R1675_OFF`.
                    if candor_report::env::switch("CANDOR_R1675_OFF") {
                        return true;
                    }
                    let mut rel = c.path.as_str();
                    while let Some(r) = rel.strip_prefix("crate::").or_else(|| rel.strip_prefix("self::")).or_else(|| rel.strip_prefix("super::")) {
                        rel = r;
                    }
                    match rel.strip_suffix(t.as_str()).and_then(|h| h.strip_suffix("::")) {
                        Some(head)
                            if !head.is_empty()
                                && !t.ends_with(&format!("::{rel}"))
                                && !qual_mods.contains(head)
                                // an EXTERNAL head (`proc_macro::Span::start`, a dependency's path) is not this rule's: the
                                // claimant was never the callee, and the hedge would be a false `ambiguous:` (measured: 3,200
                                // refusals over 83 entries, proc-macro2's `proc_macro::*` wrappers the bulk of them).
                                && !head.split("::").next().is_some_and(|r| {
                                    manifest_deps.contains(r) || matches!(r, "std" | "core" | "alloc" | "proc_macro" | "test")
                                }) =>
                        {
                            if candor_report::env::trace("CANDOR_R1675_INSTR") {
                                eprintln!("R1675REFUSE\t{}\t{}\t{t}", f.qual, c.path); // §E1 REACH COUNTER
                            }
                            false
                        }
                        _ => true,
                    }
                }) {
                    // SOUNDNESS R452 (a) — THE WRITTEN PATH NAMES ONE OF THE CLAIMANTS, so the tail's
                    // ambiguity is this INDEX's, not the program's. `resolve_target` keys on the
                    // 2-segment tail and refuses a tail several definitions claim, because linking a
                    // many-way tail fabricates one type's effect onto the other's caller. That refusal is
                    // right — and it was throwing away the source text that settles it. `arm_exact_target`
                    // takes a claimant ONLY where the path the collector resolved IS that claimant's qual,
                    // or one is the other's `::`-suffix, and REFUSES on a second match exactly as
                    // `resolve_target` does; it can therefore only ever pick a definition the source
                    // spelled out, and it is strictly MORE precise than the tail it rescues.
                    //
                    // It is not new code and not a new warrant: R440 added it for the arm loop below,
                    // where the several claimants ARE a portability twin's arms, and it has keyed that
                    // resolution since. What is new is that the GENERAL site consults it too. Measured
                    // over 250 registry crates, on typed method calls alone: of 856 caller functions
                    // holding a silently-dropped ambiguous-tail method call, 216 (25%) have a written path
                    // that names exactly one claimant — those become an EDGE here rather than the hedge
                    // below, which is the better answer for every one of them.
                    resolved_local = true;
                    // SOUNDNESS R1657 stage 1b × R1537 (c) — a co-path claimant here is proved to one member by the same rule
                    // `resolve_target`'s branch applies. Once `self` carries its origin-qualified type, a twin's `self.m()`
                    // reaches this branch instead of being refused, and without the rule the trait member delegating to the
                    // inherent one (`impl Ld for Font { fn name(&self) { self.name() } }`) edged the union, itself included.
                    let t = match copath.get(t.as_str()).and_then(|cp| copath_member(cp, c, &f.qual)) {
                        Some(m) if !candor_report::env::switch("CANDOR_R1657B_OFF") => m,
                        _ => t,
                    };
                    if t != &f.qual {
                        calls.entry(f.qual.clone()).or_default().insert(t.clone());
                    }
                    if candor_report::env::trace("CANDOR_R452_INSTR") {
                        eprintln!("R452HIT\tARM\t{}\t{}", f.qual, c.path);
                    }
                } else if c.method && c.typed {
                    // No `T::leaf` resolved (T doesn't declare `leaf`). If T impls EXACTLY ONE trait whose
                    // DEFAULT `leaf` body exists (a `Trait::leaf` FnInfo), the call inherits it — edge there.
                    // COLLISION-SAFE: zero or >1 distinct candidate → skip (the honest under-report; never
                    // guess between traits — the keying-collision discipline that keeps this from FABRICATING
                    // a wrong trait's effect onto the caller).
                    if let Some(t_type) = tail2(&c.path).and_then(|t2| t2.split("::").next().map(str::to_string)) {
                        if let Some(trs) = type_to_traits.get(&t_type) {
                            let mut hits: Vec<&String> = Vec::new();
                            for tr_leaf in trs {
                                if let Some(ts) = by_tail2.get(&format!("{tr_leaf}::{}", c.leaf)) {
                                    for t in ts {
                                        if !hits.contains(&t) {
                                            hits.push(t);
                                        }
                                    }
                                }
                            }
                            // SOUNDNESS R1246 — "T does not declare `leaf`" is what licenses this route, and a REFUSED tail
                            // is not that fact: `T::leaf` with two or more claimants means some type of this LEAF does
                            // declare it, and `resolve_target` declined to pick which. Taking the default then CERTIFIED the
                            // call (`resolved_local`) and R452's `ambiguous:` hedge below never ran — two same-leaf
                            // implementors of `Th` overriding its pure default read PURE over a file write (EXECUTED, vein
                            // fixtures f1d/f3d). The default edge stays (a twin may really inherit it, R1171); only the
                            // certification is withheld, so the call is disclosed as the refusal it is.
                            // `CANDOR_R1246_OFF=1` restores the certification.
                            let refused_tail = tail2(&c.path).and_then(|t2| by_tail2.get(&t2)).is_some_and(|v| v.len() >= 2)
                                && !candor_report::env::switch("CANDOR_R1246_OFF");
                            if hits.len() == 1 && hits[0] != &f.qual {
                                if refused_tail {
                                    if crate::lang::reach_debug() {
                                        eprintln!("R1246DFLT\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                                    }
                                } else {
                                    resolved_local = true;
                                }
                                // SOUNDNESS R1420(a) — the receiver is typed as `t_type`: its instance of the default, where
                                // one was minted (above), instead of the default's union over every implementor. Not on a
                                // refused tail (a same-leaf twin may be the receiver; no instance exists for one anyway).
                                let target = default_inst
                                    .get(&(t_type.clone(), hits[0].clone()))
                                    .filter(|_| !refused_tail)
                                    .unwrap_or(hits[0]);
                                if target != hits[0] && crate::lang::reach_debug() {
                                    eprintln!("CT3EDGE\t{}\t{}\t{target}\t{}", f.qual, c.path, f.loc); // §E1 REACH COUNTER (+ the caller's loc, for the edge audit)
                                }
                                calls.entry(f.qual.clone()).or_default().insert(target.clone());
                            }
                            // SOUNDNESS R1245 (the REFUSED half) — T's impl of a trait that declares `leaf` holds a
                            // macro item `mbe` could not expand (another crate's macro, an unreadable matcher), so
                            // the block is OPAQUE (`model::impl_opaque_key`) and may define `leaf` itself: the
                            // default edge above, or no edge at all, is a reading of THIS ENGINE's blindness, not of
                            // the program. Executed: `impl Report for Reports { mlib::reports_impl!(fn summarize(
                            // &self)); }` with `self.report.summarize()` bound to the trait's PURE default while
                            // the macro's body wrote a file. A same-crate macro is EXPANDED instead (`mbe::Ctx::
                            // splice_impl_items`), which leaves no opaque item behind, so this fires only where the
                            // resolution was refused. Exact, not a narrowing: a trait impl can define only members
                            // its trait declares, so the trait-declares-`leaf` conjunct excludes nothing that could
                            // run. ADDITIVE: the default edge (if any) stays beside the disclosure.
                            let opaque_tr = trs.iter().find(|tr_leaf| {
                                merged.impl_members.contains(&crate::model::impl_opaque_key(tr_leaf, &t_type))
                                    && !merged.impl_members.contains(&crate::model::impl_member_key(tr_leaf, &t_type, &c.leaf))
                                    && trait_decls.get(*tr_leaf).is_some_and(|lt| lt.methods.contains(&c.leaf))
                            });
                            if let Some(tr_leaf) = opaque_tr {
                                if candor_report::env::trace("CANDOR_R1245_INSTR") {
                                    eprintln!("R1245OPAQUE\t{}\t{}\t{tr_leaf}", f.qual, c.path); // §E1 REACH COUNTER
                                }
                                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                unknown_why
                                    .entry(f.qual.clone())
                                    .or_default()
                                    .insert("macro:impl members hidden by an unexpanded macro".to_string());
                            }
                        }
                        // TRAIT-REQUIREMENT dispatch from a trait DEFAULT body: inside `Store::save_all`,
                        // `self` types as the TRAIT `Store` (decls.rs types Self as the trait), so
                        // `self.persist()` is `Store::persist` — a REQUIREMENT with no default body, hence no
                        // `Store::persist` unit for `resolve_target` to find, and `type_to_traits` keys on IMPL
                        // types not the trait. CHA `persist` over Store's IMPLS and edge to each impl's method:
                        // the bounded-CHA analog of the swift protocol-extension→conformer-witness dispatch (R32's
                        // rust sibling — the effectful `impl Store for Db { fn persist }` was reachable ONLY
                        // through the default and read silent-pure). Gated to a LOCAL TRAIT that declares `leaf`
                        // (a struct-named receiver never hijacks) and bounded ≤12 impls (a wider open-world
                        // fan-out is an honest miss, never a guess). Only fires when nothing resolved locally.
                        if !resolved_local
                            && trait_decls.get(&t_type).is_some_and(|lt| lt.methods.contains(&c.leaf))
                        {
                            if let Some(impls) = trait_impls.get(&t_type) {
                                let mut hits: Vec<String> = Vec::new();
                                for imp in impls {
                                    let imp_leaf = imp.rsplit("::").next().unwrap_or(imp);
                                    if let Some(ts) = by_tail2.get(&format!("{imp_leaf}::{}", c.leaf)) {
                                        for t in ts {
                                            if !hits.contains(t) {
                                                hits.push(t.clone());
                                            }
                                        }
                                    }
                                }
                                if !hits.is_empty() && hits.len() <= 12 {
                                    resolved_local = true;
                                    for t in &hits {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
                                    }
                                } else if hits.len() > 12 {
                                    // SOUNDNESS R576(b) — THE WIDE ARM MUST DISCLOSE, NOT FALL SILENT.
                                    // This `if` had no `else`, so a trait whose implementor set is wider
                                    // than the cross-engine bound produced no edge AND no reason, and the
                                    // enclosing fn left `functions[]` ALTOGETHER — an affirmative purity
                                    // claim under SPEC §2 rule 3 over a body that really dispatches.
                                    // Measured: a `self.sink()` inside a trait DEFAULT method resolves to
                                    // every implementor at 3 impls and vanishes at 15.
                                    //
                                    // §G — ASK THE AUTHORITY, and the authority already exists: Pass A's
                                    // `dispatch_calls_for_trait_method` ends `_ => mark_unresolved(
                                    // format!("dispatch:{tr}.{leaf}"))` with the comment ">12, or no impl
                                    // visible: honest indeterminacy". Same reason string here so the two
                                    // passes answer one question one way; §4's normative dotted detail,
                                    // so `deny E Unknown[dispatch]` scopes to it as it does in candor-java.
                                    // The comment this site DID carry — "a wider open-world fan-out is an
                                    // honest miss, never a guess" — is §K exactly: an honest miss is
                                    // `Unknown`, and what it actually produced was silence.
                                    //
                                    // `resolved_local` is deliberately NOT set. Pass A's sibling returns
                                    // `true` ("do nothing further"), but here that would suppress the
                                    // auto-deref fallback below, which can still resolve this call
                                    // precisely — losing an edge to buy a disclosure is the wrong
                                    // direction. Disclosing alongside a later resolution over-reports at
                                    // worst; ⟨0.35⟩ permits completing OR disclosing, never silence.
                                    //
                                    // NOT extended to `hits.is_empty()`, the OTHER half of the sibling's
                                    // `_` arm — and that is a MEASUREMENT, not the flood argument. The
                                    // `R576EMPTY` probe below counts it: over 400 registry crates it fires
                                    // **0 times** against `R576WIDE`'s 27, because this site is reached only
                                    // when the trait is in `trait_decls` AND declares the leaf, by which
                                    // point an impl set with no matching method in `by_tail2` is not a shape
                                    // real code produces. So widening here would buy nothing; the probe stays
                                    // so the next person re-measures instead of re-arguing.
                                    if candor_report::env::trace("CANDOR_R576_INSTR") {
                                        eprintln!("R576WIDE");
                                    }
                                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                    unknown_why
                                        .entry(f.qual.clone())
                                        .or_default()
                                        .insert(format!("dispatch:{t_type}.{}", c.leaf));
                                } else if hits.is_empty()
                                    && candor_report::env::trace("CANDOR_R576_INSTR")
                                {
                                    // MEASUREMENT ONLY — this arm changes no report. It counts the other
                                    // half of Pass A's `_` arm ("no impl visible") so the decision to
                                    // widen or not is made on a corpus number rather than on the flood
                                    // argument alone. Behind the same env var as the wide arm so one
                                    // corpus pass prices both. "CHANGED 0 is not evidence until REACH is
                                    // measured" — and the converse: a widening is not safe until the
                                    // thing it would widen INTO has been counted.
                                    eprintln!("R576EMPTY");
                                }
                            }
                        }
                        // VEIN C — …AND A DEFAULT INHERITED FROM A DEPENDENCY'S TRAIT. `impl dep::Def for
                        // Mine {}` with `m.def()` reached no local unit (the default body lives in the
                        // dependency) and the rule above looks only for a LOCAL `Trait::leaf`, so the caller
                        // left `functions[]` while the program ran the dependency's default — the UFCS
                        // spelling `Def::def(m)` of the same call was already answered by the chained join.
                        // Ask the chained report for that member, exactly as the UFCS spelling does, and
                        // only where this type's impl block does NOT declare the member (else `Mine::def`
                        // is a unit and was resolved above). A key the report does not answer adds nothing.
                        if !resolved_local {
                            if let Some(dts) = dep_traits_by_ty.get(&t_type) {
                                for (tr_leaf, tpath) in dts {
                                    if !crate::model::impl_does_not_declare(&merged.impl_members, tr_leaf, &t_type, &c.leaf) {
                                        continue;
                                    }
                                    let Some((root, rest)) = tpath.split_once("::") else { continue };
                                    let full = format!("{root}#{rest}::{}", c.leaf);
                                    let t2 = tail2(&format!("{rest}::{}", c.leaf)).map(|t| format!("{root}#{t}"));
                                    let hit = deps_idx.by_key.get(&full)
                                        .or_else(|| t2.as_ref().and_then(|k| deps_idx.by_key.get(k)));
                                    if hit.is_some() && dep_default_path.is_none() {
                                        if candor_report::env::trace("CANDOR_VEINC_INSTR") {
                                            eprintln!("VEINC_DEPDEFAULT\t{}\t{full}", f.qual);
                                        }
                                        // Joined below by the SAME chained join the UFCS spelling
                                        // reaches, so the member's `dispatchesOn` is followed too.
                                        dep_default_path = Some(format!("{tpath}::{}", c.leaf));
                                    }
                                }
                            }
                        }
                        // SOUNDNESS R1607 — …AND WHEN THAT DEPENDENCY IS NOT CHAINED. `am.insert(db)` on a derive-generated
                        // `ActiveModel` (or a hand-written `impl sea_orm::ActiveModelTrait for X {}`) formed
                        // `crate::post::ActiveModel::insert`, which no local unit answers, and VEIN C asks only a chained
                        // report — so the INSERT the trait's provided body runs read ABSENT (EXECUTED). The call is put to the
                        // classifier under the trait's own spelling, `<dep>::<Trait>::<m>`, for each dependency trait the type
                        // implements without declaring `m`. A classifier answer is a rule written for that member, but a
                        // path-shaped rule can match a name the trait does not have, so the answer is only CHARGED when the
                        // trait's declaration in the Cargo.lock-pinned source names `m` (the owner's ⟨0.42⟩ rule: lock-pinned,
                        // adding only, recorded in `dependencySources`); with no such source the caller discloses `Unknown`
                        // (`dispatch:<Trait>.<m>`) instead. A declaration that provably lacks `m` adds nothing.
                        if !resolved_local && dep_default_path.is_none() && c.method && c.dot && c.typed && !c.is_macro {
                            if let Some(dts) = r1607_dep_traits_by_ty.get(&t_type) {
                                let root = c.path.split("::").next().unwrap_or("");
                                if !deps.contains(root) {
                                    for (tr_leaf, tpath) in dts {
                                        if !crate::model::impl_does_not_declare(&merged.impl_members, tr_leaf, &t_type, &c.leaf) {
                                            continue;
                                        }
                                        let Some((dcr, _)) = tpath.split_once("::") else { continue };
                                        let dreal = dep_renames.get(dcr).map(String::as_str).unwrap_or(dcr);
                                        let tp = format!("{dreal}::{}", tpath.split_once("::").map(|x| x.1).unwrap_or(""));
                                        let Some(eff) = candor_classify::classify(dreal, &format!("{tp}::{}", c.leaf)) else { continue };
                                        let declares = r1578_sigs.borrow_mut().trait_declares_locked(dir, dcr, dreal, tr_leaf, &c.leaf);
                                        if candor_report::env::trace("CANDOR_R1607_INSTR") {
                                            eprintln!("R1607\t{}\t{}\t{tp}::{}\t{eff}\tdeclares={declares:?}", f.qual, c.path, c.leaf); // §E1
                                        }
                                        match declares {
                                            Some(true) => {
                                                r1578_sigs.borrow_mut().note_typed(dcr);
                                                direct.entry(f.qual.clone()).or_default().insert(eff);
                                                // SOUNDNESS R1612 — and the rule's second effects, as on the main path.
                                                for e2 in candor_classify::also_effects(dreal, &format!("{tp}::{}", c.leaf)) {
                                                    direct.entry(f.qual.clone()).or_default().insert(e2);
                                                    if candor_classify::also_hides_locator(e2) {
                                                        incomplete.entry(f.qual.clone()).or_default().insert(e2);
                                                    }
                                                }
                                            }
                                            Some(false) => {}
                                            None => {
                                                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                                unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tr_leaf}.{}", c.leaf));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // AUTO-DEREF fallback (last, after inherent + trait-default — Rust's resolution
                        // order): a custom `impl Deref for t_type { type Target = U }` makes `recv.leaf()`
                        // dispatch to `U::leaf`. Chase the Deref chain (bounded) and edge to the first
                        // `U::leaf` that resolves — the user-Deref analog of the Box/Arc/Rc peel (a newtype
                        // `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find). `.clone()`
                        // is guarded at the typed-call emit, so no pointee-clone fabrication recurs.
                        // SOUNDNESS R1630 (residual) — the receiver's type is a crate-local `impl<T> Deref for W<T> { type
                        // Target = T }` wrapper and `W` declares no `leaf`: the call auto-derefs to the wrapper's INSTANTIATION,
                        // which no declaration at this site writes (the collector answers only a FIELD receiver whose written
                        // arguments say it, `CallCollector::deref_param_recv`). tokio's `if let Some(mut driver) =
                        // self.shared.driver.try_lock() { driver.shutdown(handle) }` over `LockGuard<'_, T>` formed
                        // `LockGuard::shutdown` and read PURE over the I/O driver's shutdown. The chase below can only
                        // follow a CONCRETE target, so this is the generic-payload disclosure, at the site that lost it.
                        if !resolved_local
                            && !candor_report::env::switch("CANDOR_R1630_OFF")
                            && matches!(merged.rets.get(&crate::model::deref_param_key(&t_type)), Some(Some(p)) if p.parse::<usize>().is_ok())
                            // …and some crate impl declares a member of that name, so the instantiation CAN be a local type
                            // whose member runs (tokio's `Driver::shutdown`). A name no crate impl declares is a std or
                            // dependency member (time's `Spanned<&[u8]>` `.eq_ignore_ascii_case(..)`, 97 rows a crate): the
                            // pre-existing reading stands for it, as for any untyped foreign receiver.
                            && merged.rets.contains_key(&crate::model::any_method_key(&c.leaf))
                            && !matches!(c.leaf.as_str(), "deref" | "deref_mut" | "drop" | "clone" | "clone_from" | "clone_into" | "fmt" | "eq"
                                | "ne" | "cmp" | "partial_cmp" | "lt" | "le" | "gt" | "ge" | "max" | "min" | "clamp" | "hash" | "default"
                                | "into" | "try_into" | "from" | "try_from" | "borrow" | "borrow_mut" | "to_owned" | "to_string" | "type_id")
                        {
                            if candor_report::env::trace("CANDOR_R1630_INSTR") {
                                eprintln!("R1630HEDGE\t{}\t{t_type}.{}", f.qual, c.leaf); // §E1 REACH COUNTER
                            }
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default()
                                .insert(format!("dispatch:generic payload receiver of `{}`", c.leaf));
                        }
                        if !resolved_local {
                            let mut cur = t_type.clone();
                            let mut hops = 0;
                            while let Some(target) = merged.deref_target.get(&cur).cloned() {
                                if hops >= 8 { break; }
                                hops += 1;
                                if let Some(ts) = resolve_target(&format!("{target}::{}", c.leaf), &c.leaf, false, &by_tail2, &by_leaf_call) {
                                    resolved_local = true;
                                    for t in ts {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
                                    }
                                    break;
                                }
                                // ⟨0.40⟩ SOUNDNESS R888 — AFTER the local attempt above: the target is a CHAINED
                                // dependency's type, which no local unit answered: ask the dependency through the walk (its supertraits,
                                // its own `Deref`). A chain that ends OPEN on the dependency's side — an
                                // unkeyed or kind-only type — discloses (SPEC §2 ⟨0.40⟩ Rust permission, row 3).
                                if let Some(k) = foreign_derefs.get(&cur) {
                                    let (rows, w) = crate::typesurf::join_through(deps_idx, k, &c.leaf, false);
                                    if r843_probe {
                                        eprintln!("R843OWNDEREF\t{}\t{cur}->{k}::{}\thits={}\topen={}",
                                            f.qual, c.leaf, w.hits.len(), w.deref_open);
                                    }
                                    for de in rows {
                                        apply_dep_fn(de, &f.qual, DepSink {
                                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds,
                                            paths: &mut paths, tables: &mut tables, incomplete: &mut incomplete,
                                            unknown_why: &mut unknown_why, blind_direct: &mut blind_direct,
                                            dep_invisible: &mut dep_invisible, unknown_via_dep: &mut unknown_via_dep,
                                        });
                                    }
                                    if w.hits.is_empty() && w.deref_open {
                                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                        unknown_why.entry(f.qual.clone()).or_default()
                                            .insert(format!("dispatch:{cur}.{}", c.leaf));
                                    }
                                    break;
                                }
                                cur = target;
                            }
                        }
                    }
                }
            }
            // BLANKET-impl fallback (R45): a `x.leaf()` that resolved to NO concrete / trait-default / deref
            // target may be a blanket-impl method (`impl<T> Ext for T { fn ext }`), whose body qual is
            // `<param>::leaf` (`T::ext`) — a keyed lookup on the receiver's type can't find it. If `leaf` is a
            // UNIQUE blanket method (not the "" ambiguity sentinel), edge to the blanket body. SOUND: the
            // blanket provides `leaf` for EVERY type (a bounded blanket only for conforming types — and a
            // call that COMPILES meets the bound), and it fires only when nothing concrete resolved. Gated to
            // a TYPED receiver (a known LOCAL type): the type's OWN `leaf` (a local FnInfo) would resolve
            // first, so the blanket never overrides an inherent method — no fabrication. A bare/untyped
            // receiver is left an honest under-report (candor can't confirm the blanket applies vs a shadow).
            // CRUCIAL — also suppress the blanket when the receiver TYPE declares `leaf` AMBIGUOUSLY: if
            // `by_tail2["{recv}::{leaf}"]` exists but has >1 target, `resolve_target` returned None (its
            // uniqueness filter) and `resolved_local` is false, yet the type DOES have an inherent `leaf`
            // that shadows the blanket — edging to the blanket would FABRICATE its effect onto a call that
            // runs the (unresolvably-ambiguous) inherent method (review [2]). Fire only when the receiver
            // type has NO local `leaf` at all.
            if !resolved_local && c.method && c.typed && !c.is_macro {
                let recv_has_inherent = tail2(&c.path)
                    .and_then(|t2| t2.split("::").next().map(str::to_string))
                    .is_some_and(|ty| by_tail2.contains_key(&format!("{ty}::{}", c.leaf)));
                if !recv_has_inherent {
                    if let Some(bty) = merged.blanket_methods.get(&c.leaf) {
                        if !bty.is_empty() {
                            if let Some(ts) = by_tail2.get(&format!("{bty}::{}", c.leaf)) {
                                if ts.len() == 1 && ts[0] != f.qual {
                                    resolved_local = true;
                                    calls.entry(f.qual.clone()).or_default().insert(ts[0].clone());
                                }
                            }
                        }
                    }
                }
            }
            // A BARE-LEAF method call (`self.fastrand()` → path == leaf, no `::`) carries no crate
            // qualifier, so its `classify` consults the bare leaf against the calibrated crate/verb rules
            // (`fastrand` → Rand, `now` → Clock, …). When that leaf names a LOCAL method definition
            // (`local_method_leaves`), the call resolves to the project's OWN method — the local definition
            // is AUTHORITATIVE — so a local method merely NAMED like a calibrated crate (tokio's pure
            // `FastRand::fastrand` xorshift) must NOT inherit the crate's effect. Suppress the bare-leaf
            // classification; the effect (if any) flows from the resolved target through propagation. The
            // external-crate classification of a bare leaf still applies when NO local method shares the name
            // (a genuine `fastrand::u32` dependency call). Qualified calls keep their type-precise rule.
            // A BARE leaf (no `::`) naming ANY local definition (`by_leaf` keys every local fn/method by
            // leaf) is the project's OWN — the local def is authoritative. Suppress the bare-leaf
            // classifier (and the dep-join below). This covers the bare-leaf METHOD case AND the bare-leaf
            // FREE-FN case the old `c.method && local_method_leaves` guard missed: a pure local free fn
            // whose leaf is AMBIGUOUS (≥2 local defs, e.g. a free `git_clone` + a trait method `git_clone`)
            // defeats `resolve_target`'s uniqueness filter (→ `resolved_local=false`), so the FFI/crate
            // classifier fired unsuppressed and fabricated the effect (the precision failure). A bare leaf with no
            // local def (a genuine prelude/extern call) still classifies; a `use`-imported call is
            // qualified (`::`) and keeps its type-precise rule.
            let suppress_bare_leaf = !c.path.contains("::") && by_leaf.contains_key(&c.leaf);
            // CANDOR_DEPS cross-crate JOIN (spec §2), GATED: an UNCLASSIFIED qualified call into a crate a
            // sibling report covers inherits that fn's recorded effects + literal surfaces — UNLESS the call
            // resolved to a local target or names a local bare leaf (then the local is authoritative; the
            // join would fabricate). Joined unambiguous-tail2-first, then unambiguous leaf, like resolve_target.
            // A renamed dep joins under its real package name (`cr_real`, hoisted above).
            let mut dep_join_hit = false;
            let mut dep_join_unknown = false;
            // The path the join asks with: the written one, or the dependency trait member a typed call
            // inherits (see `dep_default_path`). Shadowed for this block only.
            let (jpath, cr, cr_real): (&str, &str, &str) = match dep_default_path.as_deref() {
                Some(p) => {
                    let r = p.split("::").next().unwrap_or("");
                    (p, r, dep_renames.get(r).map(String::as_str).unwrap_or(r))
                }
                None => (c.path.as_str(), cr, cr_real),
            };
            // SOUNDNESS R1152 — A LOCAL RESOLUTION THAT IS ONLY A TAIL2 COLLISION DOES NOT SPEAK FOR THE
            // DEPENDENCY. `use cdep3::Tool; fn dep_tool(t: &Tool) { t.run() }` beside an unrelated
            // `mod local { struct Tool; impl Tool { fn run(&self) {} } }` forms `cdep3::Tool::run`, whose
            // tail `Tool::run` the local method alone claims, so `resolve_target` linked it and
            // `resolved_local` closed the join: chained, `dep_tool` was ABSENT over the dependency's real
            // `Fs` and `deny Fs` exited 0 (executed). R223 already withdrew that local edge's AUTHORITY
            // over the classifier (`extern_dep_qualified`); the join is the other half of the same
            // authority and was left gated on the bare `resolved_local`. The local edge is KEPT — this
            // only ADDS the dependency's answer beside it (a union, the R223 direction), so a call whose
            // head merely looks like a dependency costs at most an over-report, never a silence.
            let r1152_union = resolved_local && extern_dep_qualified && dep_default_path.is_none();
            if r1152_union && candor_report::env::trace("CANDOR_R1152_INSTR") && deps_idx.crates.contains(cr_real) {
                eprintln!("R1152UNION\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
            }
            // SOUNDNESS R1349 — A KEY ANOTHER CHAINED PACKAGE ANSWERS, WHEN ITS OWNER IS NOT CHAINED. Obligation 2 puts a
            // package's union for a FOREIGN abstraction under the OWNER's key (aws-smithy-types publishes
            // `serde#ser::Serializer::serialize_some`), and `load_dep_reports` deliberately does not count that entry as
            // coverage of `serde`. This join was gated on coverage alone, so a consumer dispatching DIRECTLY on its own
            // generic (`fn run<S: serde::Serializer>(s: S) { s.serialize_some(&Mine) }`) chained over aws-smithy-types
            // and not serde never asked the key at all and read `[]` — while the program ran aws's implementor (EXECUTED
            // fx/cpaws c_aws). §4 ⟨0.39⟩'s headline is "every implementor visible to the consumer — its own and any
            // chained report's"; the owner being absent does not make aws's implementor invisible. The key is asked
            // when the call roots at a real dependency and SOME chained report answers it; with the owner uncovered,
            // only a union entry can (any other entry under `serde#` would have covered `serde`). It is a CONTRIBUTOR
            // only: `dep_join_hit` stays false, so every disclosure the uncovered owner already draws (its `invisible`
            // ledger line, R452/R128) still fires, and nothing the call had is withdrawn. `CANDOR_R1349_OFF=1`.
            let covered_owner = deps_idx.crates.contains(cr_real);
            let r1349_union_only = !covered_owner
                && !candor_report::env::switch("CANDOR_R1349_OFF")
                && deps.contains(cr_real)
                && jpath.contains("::")
                && {
                    let rel = jpath.strip_prefix(&format!("{cr}::")).unwrap_or(jpath);
                    rel.contains("::") && tail2(rel).is_some_and(|t2| deps_idx.by_key.contains_key(&format!("{cr_real}#{t2}")))
                };
            if r1349_union_only && crate::lang::reach_debug() {
                eprintln!("R1349UNION\t{}\t{}", f.qual, jpath); // §E1 REACH COUNTER
            }
            // SOUNDNESS R1715 — A CALL WRITTEN THROUGH A FACADE OF THE OWNER. `serde::de::Deserialize::deserialize(..)` (or
            // `<T as de::Deserialize>::deserialize`) is serde_core's trait member, re-exported by serde's root (`pub use
            // serde_core::{de, ..}`); every chained entry for it — serde_json's `interfaceUnion` row with its implementors —
            // is keyed `serde_core#…`, and `serde#Deserialize::deserialize` asked only the facade's own report. Before R1710
            // serde_json's alias spelling published that union under `serde#` and the join held by accident; after it the
            // consumer read `Unknown` with no edge and lost the `Fs` its own `Error::invalid_type` performs (EXECUTED
            // e_facade). The path is resolved through the facade's lock-pinned root re-exports (`DepSigs::facade_owners`) and
            // every entry the OWNER's key finds is UNIONED into the hit (`DepFn::union_with`, the entry-collision rule). With
            // the facade itself uncovered the resolved hit is a CONTRIBUTOR only, as R1349's is: the facade's `invisible`
            // still fires. `CANDOR_R1715_OFF=1` disables it.
            let r1715_extra: Vec<(&crate::deps::DepFn, Vec<String>)> = if jpath.contains("::")
                && deps.contains(cr_real)
                && !candor_report::env::switch("CANDOR_R1715_OFF")
            {
                let rel = jpath.strip_prefix(&format!("{cr}::")).unwrap_or(jpath);
                // R1740 — keyed by the receiver's associated-type provenance too: one `Serializer::end` path is a map's
                // `.end()` in one body and a struct's in another.
                let ck = (cr_real.to_string(), format!("{rel}\u{1f}{}", c.assoc_via.as_deref().unwrap_or("")));
                let keys = r1715_call_cache.entry(ck).or_insert_with(|| {
                    let mut out: Vec<(String, Vec<String>)> = Vec::new();
                    let mut sigs = r1578_sigs.borrow_mut();
                    let mut cands = sigs.facade_owners(dir, cr, cr_real, rel);
                    // the package's own path too, for the associated-type rescue below (never joined as a facade hop)
                    cands.push((cr_real.to_string(), rel.to_string(), Vec::new()));
                    for (o, rest, via) in cands {
                        let own = o == cr_real && via.is_empty();
                        let k = if rest.contains("::") { tail2(&rest).map(|t2| format!("{o}#{t2}")) } else { Some(format!("{o}#{rest}")) };
                        let Some(k) = k else { continue };
                        if !own {
                            out.push((k.clone(), via.clone()));
                        }
                        // SOUNDNESS R1715 (F2) — a member the named trait does not declare but one of its ASSOCIATED TYPES'
                        // bounds does: the receiver was the associated type's value (`S::SerializeMap`), and the consumer
                        // carried `S`'s bound onto it. The lock-pinned owner source names the trait that runs; join it.
                        if deps_idx.by_key.contains_key(&k) {
                            continue;
                        }
                        let Some((tp, m)) = rest.rsplit_once("::") else { continue };
                        let leaf = tp.rsplit("::").next().unwrap_or(tp);
                        for a in sigs.assoc_member_traits_locked(dir, &o, &o, leaf, m, c.assoc_via.as_deref()) {
                            if crate::lang::reach_debug() {
                                eprintln!("R1715ASSOC\t{cr}::{rel}\t{o}#{a}::{m}"); // §E1 REACH COUNTER
                            }
                            let mut v = via.clone();
                            v.push(o.clone());
                            out.push((format!("{o}#{a}::{m}"), v));
                        }
                    }
                    out
                });
                keys.iter().filter_map(|(k, via)| deps_idx.by_key.get(k).map(|d| (d, via.clone()))).collect()
            } else {
                Vec::new()
            };
            if !r1715_extra.is_empty() && crate::lang::reach_debug() {
                eprintln!("R1715CALL\t{}\t{}\t{}", f.qual, jpath, r1715_extra.len()); // §E1 REACH COUNTER
            }
            // R1715 (F4) — the same call where the dependency's source CANNOT be read (no Cargo.lock, a version not checked
            // out): whether it re-exports the member is unknowable, and a chained report DISPATCHING on that member's 2-segment
            // tail under ANOTHER owner (`serde_core#de::Deserialize::deserialize` for `serde::de::Deserialize::deserialize(..)`)
            // names exactly the abstraction such a re-export would have joined. That owner's entries are UNIONED into the call
            // as the facade's would have been (EXECUTED e_facade, no lockfile: ABSENT, `deny Fs` 0 without it); an entry that
            // is pure adds nothing. A readable source that does not re-export it proves the call is the facade's own.
            let mut r1715_extra = r1715_extra;
            if r1715_extra.is_empty() && jpath.contains("::") && deps.contains(cr_real) && !candor_report::env::switch("CANDOR_R1715_OFF") {
                let rel = jpath.strip_prefix(&format!("{cr}::")).unwrap_or(jpath);
                if let Some(t2) = rel.contains("::").then(|| tail2(rel)).flatten() {
                    let blind = r1715_blind_cache.entry(cr_real.to_string())
                        .or_insert_with(|| r1578_sigs.borrow_mut().facade(dir, cr, cr_real).is_none());
                    // …only where the call is otherwise UNANSWERED: an entry the package itself publishes under that tail is
                    // its own answer (serde_core's `Serializer::serialize_map` for serde_json's own calls), and a union over
                    // another owner's same-named member there would be a guess where nothing was lost.
                    // SOUNDNESS R1739 (the call side) — …and not another owner's entry when the TREE proves the two traits
                    // distinct (`DepSigs::distinct_in_tree`; pinned or path sources only, never an unpinned registry checkout): `otherkit::Visitor::visit_u64(
                    // &Calm, 1)` over a lock-less `otherkit` that declares its own `Visitor` was unioned with serde_core's
                    // `de::Visitor::visit_u64` entry and read `Unknown` (`dispatch:de::Visitor.visit_u64`) over a pure local
                    // implementor (EXECUTED `t5_otherkit_ufcs`; 0.42.0 `[]`). Only removes; see `DepSigs::distinct_in_tree`.
                    let dtp = rel.rsplit_once("::").map_or(rel, |x| x.0);
                    if *blind && !deps_idx.by_key.contains_key(&format!("{cr_real}#{t2}")) {
                        for o in r1715_tail_owners.get(&t2).into_iter().flatten().filter(|o| o.as_str() != cr_real) {
                            if !candor_report::env::switch("CANDOR_R1739_OFF") && r1739_tail_paths.get(&(t2.clone(), o.clone())).is_some_and(|ps| {
                                let mut sigs = r1578_sigs.borrow_mut();
                                let oalias = dep_renames.iter().find(|(_, r)| r.as_str() == o.as_str()).map(|(a, _)| a.as_str()).unwrap_or(o.as_str());
                                !ps.is_empty() && ps.iter().all(|otp| sigs.distinct_in_tree(dir, cr, cr_real, dtp, oalias, o, otp))
                            }) {
                                if crate::lang::reach_debug() {
                                    eprintln!("R1739OWNCALL\t{}\t{jpath}\t{o}", f.qual); // §E1 REACH COUNTER
                                }
                                continue;
                            }
                            if let Some(d) = deps_idx.by_key.get(&format!("{o}#{t2}")) {
                                r1715_extra.push((d, Vec::new()));
                                if crate::lang::reach_debug() {
                                    eprintln!("R1715BLINDCALL\t{}\t{jpath}\t{o}", f.qual); // §E1 REACH COUNTER
                                }
                            }
                        }
                    }
                }
            }
            if classified.is_none() && (!resolved_local || dep_default_path.is_some() || r1152_union) && !suppress_bare_leaf
                && jpath.contains("::") && (covered_owner || r1349_union_only || !r1715_extra.is_empty())
            {
                let rel = jpath.strip_prefix(&format!("{cr}::")).unwrap_or(jpath);
                let key = if rel.contains("::") {
                    tail2(rel).map(|t2| format!("{cr_real}#{t2}"))
                } else {
                    Some(format!("{cr_real}#{rel}"))
                };
                let mut hit = key.as_ref().and_then(|k| deps_idx.by_key.get(k));
                // SOUNDNESS R1637 — A PATH THROUGH A SUBMODULE RE-EXPORT. `rustix::io::write` is defined at
                // `io::read_write::write` and re-exported by `pub use read_write::*;` in `io`; the producer keys its unit by
                // leaf, tail2 (`read_write::write`) and qual, so the consumer's `rustix#io::write` matched nothing and a
                // COVERED owner's miss reads as purity: polling 3.3.1/3.4.0's `poll::Poller::notify` (whose `Notify::notify`
                // writes the wake-up pipe) read `[Log]` chained, and the minimal `pd::io::w()` over `mod io { mod rw { pub fn
                // w() { fs::read(..) } } pub use rw::w; }` read PURE chained against `invisible:[pd]` unchained (EXECUTED
                // fixture `fx/pc`, `deny Fs` 0, published 0.41.3 the same). A crate-root re-export only ever worked because
                // its tail2 IS the leaf key. On the miss: the producer's units of that LEAF (`DepIndex::members`) — exactly
                // one, and the path names it (a re-export of anything else would be a name the producer does not define,
                // which stays as before); several, and which one the path re-exports is not decidable from the report, so
                // the call discloses rather than picking (`ambiguous:`). A free-fn path call only. `CANDOR_R1637_OFF=1`.
                // A free fn reached through a MODULE: the segment before the leaf is a module name (lower-case by Rust's
                // naming rule), never a type — `Pz::eq`, `Dp::bitor_assign` are an operator's or an associated fn's path, whose
                // pure unit is simply absent from `functions[]` (executed controls `z_pz`, `cc_deppure`).
                let through_module = rel.rsplit("::").nth(1).and_then(|m| m.chars().next()).is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_');
                if hit.is_none() && !c.method && !c.is_macro && through_module && covered_owner && !candor_report::env::switch("CANDOR_R1637_OFF") {
                    // Only MODULE owners (every segment lower-case): a `pub use` re-exports a free fn, never an inherent
                    // method, so a type's same-leaf method (`cargo::Builder::build` beside dylint_internal's pure free fn
                    // `cargo::build`) is neither a join target nor a reason to disclose — the first cut joined it (a
                    // fabrication the chained census found; fixture chain/r1637 `c_mk_free_zq`).
                    let module_owner = |o: &String| {
                        o.split("::").all(|s| s.chars().next().is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_'))
                    };
                    let all: Vec<&String> = deps_idx.members.get(&c.leaf).into_iter().flatten()
                        .filter(|(k, o)| k == cr_real && module_owner(o)).map(|(_, o)| o).collect();
                    // `members` is read off `functions[]`, which lists only units that HAVE effects: a pure unit of the same
                    // leaf is invisible here, so "exactly one owner" is not "the only definition". The join is taken only for
                    // the one effectful unit BENEATH the written module (`io::read_write` for `io::write`), where a `pub use`
                    // re-exports from far more often than from elsewhere; any other miss with an effectful unit of that leaf
                    // in the producer DISCLOSES, never picks (`pd::jo::w()` over a pure `jo::rw::w` beside an effectful
                    // `io::rw::w` would otherwise be charged the sibling's `Fs`). No effectful unit of the leaf: the miss stays
                    // as it was, which is the right answer when the re-exported unit is itself pure.
                    let module = rel.rsplit_once("::").map_or("", |x| x.0);
                    let under: Vec<&String> = all.iter().copied()
                        .filter(|o| o.as_str() == module || o.starts_with(&format!("{module}::"))).collect();
                    if candor_report::env::trace("CANDOR_R1637_INSTR") && !all.is_empty() {
                        eprintln!("R1637REEXP\t{}\t{jpath}\t{}\t{}", f.qual, under.len(), all.len()); // §E1 REACH COUNTER
                    }
                    match (under.as_slice(), all.is_empty()) {
                        ([one], _) => hit = deps_idx.by_key.get(&format!("{cr_real}#{one}::{}", c.leaf)),
                        (_, true) => {}
                        _ => {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default()
                                .insert(format!("ambiguous:same-name units in {cr_real} under a re-exported path"));
                        }
                    }
                }
                // R1715 — the owner's entries, unioned into whatever the facade's own key found.
                let r1715_owned: Option<crate::deps::DepFn> = if r1715_extra.is_empty() {
                    None
                } else {
                    let mut d = hit.cloned().unwrap_or_default();
                    for (e, via) in &r1715_extra {
                        d.union_with(e);
                        for a in via {
                            r1578_sigs.borrow_mut().note_typed(a);
                        }
                    }
                    Some(d)
                };
                if r1715_owned.is_some() {
                    hit = r1715_owned.as_ref();
                }
                if let Some(de) = hit {
                    dep_join_hit = covered_owner; // R1349 — a contributor is not a resolution of the call
                    // SOUNDNESS R501 — WHAT THE JOIN ACTUALLY ANSWERED, kept because the disclosure gate
                    // below needs to tell "this call is now resolved" from "this call is now resolved TO
                    // AN UNKNOWN". `de.effects` is the joined entry's own answer and nothing else, so
                    // this flag is a function of the dependency's report, not of accumulation order.
                    dep_join_unknown = covered_owner && de.effects.contains("Unknown");
                    apply_dep_fn(de, &f.qual, DepSink {
                        direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                        tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                        blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                    });
                    // (No coverage marking here. A crate whose sibling report we joined is already
                    // covered by the `deps_idx.crates` arm of the ledger filter below — that arm is the
                    // reviewed claim, and it holds whether or not any single join happened to fire.)
                    //
                    // ⟨0.39⟩ SPEC §4 obligation 3 — THE CHAINED DISPATCH UNION. The row we just joined may
                    // say it DISPATCHES on an abstraction member (`dispatchesOn`), and in the measured
                    // instance (SOUNDNESS R475, live on `ratatui`) that is the only thing standing between
                    // this consumer and a real effect: `app_size` calls `iface::term_size`, never spells the
                    // dispatch itself, and `term_size`'s own row is PURE because the only implementor
                    // `iface` can see is pure. The effectful implementor is in a THIRD package, and it
                    // published its union under the OWNING crate's key — so the key is formed from the
                    // dispatching crate, not from whoever implements it, and `by_key`'s entry-collision
                    // UNION (ENTRY-COLLISION-DECISION.md) is what combines the contributors. That union
                    // already exists; this adds a contributor to it, not a resolution rule.
                    //
                    // A MISS ADDS NOTHING — deliberately, and it is the c3_pure_only control in conformance
                    // PART 92. An engine that hedges on "a dispatch occurred" rather than on "an implementor
                    // is invisible" charges every consumer of every dispatching library for effects nobody
                    // implements, which is the fabrication direction §4 forbids and the cost model this rung
                    // was priced against explicitly rules out. The dependency's own row has already said
                    // whatever it honestly can (its `Unknown` at zero implementors, its purity otherwise).
                    let mut pending: Vec<String> = de.dispatches_on.iter().cloned().collect();
                    let mut seen_members: std::collections::HashSet<String> =
                        pending.iter().cloned().collect();
                    // R1324 — the members a UNION carried (§4 ⟨0.40⟩ union clause), whose miss is disclosed below.
                    let mut from_union: std::collections::HashSet<String> =
                        de.union_dispatches_on.iter().cloned().collect();
                    while let Some(member) = pending.pop() {
                        let mut answered = false;
                        // R503/R504 — THE VALUE IS THE KEY. ⟨0.39⟩ pins `dispatchesOn` to the owning
                        // package's entry-key spelling, so a conforming producer hands over something
                        // that is already joinable and the consumer prefixes NOTHING. That is not a
                        // nicety: under R504 a MIDDLE package names a member owned by ITS dependency, so
                        // prefixing the dispatching crate would form a key naming the wrong owner.
                        //
                        // A value with no `#` is a pre-⟨0.39⟩-spelling producer (candor-scan ≤0.38.4
                        // published the trait LEAF), and it is read the way that producer meant it —
                        // under the dispatching crate. Kept because a consumer is chained onto whatever
                        // reports are on disk, and dropping those values would convert an older
                        // dependency's disclosure into silence at the exact moment this engine upgrades.
                        let ukey = if member.contains('#') {
                            member.clone()
                        } else {
                            format!("{cr_real}#{member}")
                        };
                        // …and the 2-SEGMENT TAIL of whatever that came to, for the one case a fully
                        // qualified key cannot reach on its own: an abstraction the owner RE-EXPORTS from
                        // a different module than it declares it in, where the implementor's `use` path
                        // and the owner's declaration path disagree. `load_dep_reports` publishes every
                        // entry under its tail2 as well, so this is asking an index that already exists;
                        // it can only ADD a contributor, never withdraw one.
                        let (ukey_owner, ukey_member) =
                            ukey.split_once('#').unwrap_or((cr_real, &ukey));
                        let ukey_t2 = tail2(ukey_member)
                            .map(|t2| format!("{ukey_owner}#{t2}"))
                            .unwrap_or_else(|| ukey.clone());
                        if let Some(ude) =
                            deps_idx.by_key.get(&ukey).or_else(|| deps_idx.by_key.get(&ukey_t2))
                        {
                            apply_dep_fn(ude, &f.qual, DepSink {
                                direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                                tables: &mut tables, incomplete: &mut incomplete,
                                unknown_why: &mut unknown_why, blind_direct: &mut blind_direct,
                                dep_invisible: &mut dep_invisible, unknown_via_dep: &mut unknown_via_dep,
                            });
                            // A union entry may itself dispatch onward; bounded by `seen_members`, which
                            // also makes a cyclic publication terminate.
                            answered = true;
                            for m2 in &ude.dispatches_on {
                                if seen_members.insert(m2.clone()) {
                                    if ude.union_dispatches_on.contains(m2) {
                                        from_union.insert(m2.clone());
                                    }
                                    pending.push(m2.clone());
                                }
                            }
                        }
                        // …AND THE CONSUMER'S OWN VISIBLE IMPLEMENTORS, which §4 names first: "its own
                        // visible implementors with every chained entry carrying that key". This crate may
                        // implement the dependency's abstraction itself, and that body is LOCAL — so it
                        // joins as an ordinary call EDGE and its effects flow through the same fixpoint
                        // every other local call uses, rather than being re-derived here.
                        // ONE SPELLING, since R503 — `dispatchesOn` and `foreign_impls` are now keyed
                        // the same way (the ⟨0.23⟩ qualified path in the OWNING crate), so `ukey` asks
                        // the local index in exactly the spelling it is built with. This used to be TWO
                        // spellings reconciled here, and the reconciliation held only because
                        // `load_dep_reports` publishes a tail2 key for every entry; the LOCAL index has
                        // no such fallback, so a consumer implementing the dependency's abstraction
                        // ITSELF was found only when the trait sat at the owner's crate ROOT — true of a
                        // fixture, false of `ratatui_core::backend::Backend`. `ukey_t2` stays as the
                        // re-export / older-producer fallback, and it can only ADD a contributor.
                        // SOUNDNESS R529 — …AND AN IMPLEMENTOR THIS CRATE SUPPLIES FROM INSIDE A
                        // BLOCK, which `foreign_impls` cannot hold. Pass A never walks into a fn body,
                        // so `fn register() -> Box<dyn dep::Backend> { struct L; impl dep::Backend for L
                        // { fn size(&self) { …net… } } … }` contributes NO key here and the join below
                        // adds nothing — while the dependency's own row is honestly pure (its only
                        // visible implementor is). The consumer then reads `inferred: []` with no
                        // `Unknown` and no `invisible`: the ⟨0.39⟩ toggle this rung exists to close,
                        // reached by an implementor the engine cannot name rather than by one it cannot
                        // see. There is no unit to edge to, so the honest answer is the disclosure.
                        if merged.nested_impl_foreign.contains(&ukey)
                            || merged.nested_impl_foreign.contains(&ukey_t2)
                        {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(&ukey));
                            answered = true;
                            if candor_report::env::trace("CANDOR_R529_INSTR") {
                                eprintln!("R529HIT\tCHAIN\t{}\t{ukey}", f.qual); // §E1 REACH PROBE
                            }
                        }
                        // SOUNDNESS R1715 — …and the implementors this crate wrote through a FACADE of the owner (see
                        // `facade_t2`): `impl serde::de::Visitor` answers `serde_core#de::Visitor::visit_u64`. Unioned with
                        // the hit above (a crate may spell one trait both ways), never instead of it.
                        let facade_hit = facade_t2.get(&ukey_t2);
                        if let Some((_, via)) = facade_hit {
                            for a in via {
                                r1578_sigs.borrow_mut().note_typed(a);
                            }
                        }
                        let direct_exact = merged.foreign_impls.get(&ukey);
                        let direct_hit = direct_exact.or_else(|| foreign_impls_by_tail2.get(&ukey_t2));
                        // R1715 — …and the other direction: a producer that keyed the member under the FACADE (`serde#de::Error::
                        // custom`, written `serde::de::Error::custom` in that producer, or any pre-R1710 serde_json report) and
                        // this crate implementing it under the owner (`impl serde_core::de::Error`). The producer's key is resolved
                        // through the facade's lock-pinned re-exports and the owner's tail asked of this crate's index.
                        let rev_hit: Vec<(String, &Vec<String>)> = if ukey_owner != crate_name
                            && deps.contains(ukey_owner)
                            && !candor_report::env::switch("CANDOR_R1715_OFF")
                        {
                            let rk = r1715_rev_cache.entry(ukey.clone()).or_insert_with(|| {
                                let Some((tp, meth)) = ukey_member.rsplit_once("::") else { return Vec::new() };
                                let real = dep_renames.get(ukey_owner).cloned().unwrap_or_else(|| ukey_owner.to_string());
                                r1578_sigs.borrow_mut().facade_owners(dir, ukey_owner, &real, tp).into_iter()
                                    .filter_map(|(o, rest, via)| tail2(&format!("{rest}::{meth}")).map(|t2| (format!("{o}#{t2}"), via)))
                                    .collect()
                            });
                            let mut v = Vec::new();
                            if crate::lang::reach_debug() {
                                eprintln!("R1715REV\t{}\t{ukey}\t{:?}", f.qual, rk); // §E1 REACH COUNTER
                            }
                            for (k, via) in rk.iter() {
                                if let Some(q) = foreign_impls_by_tail2.get(k) {
                                    v.push((k.clone(), q));
                                    for a in via {
                                        r1578_sigs.borrow_mut().note_typed(a);
                                    }
                                }
                            }
                            v
                        } else {
                            Vec::new()
                        };
                        // R1715 (F4) — no facade answered, and this crate implements a member of that very tail under a
                        // DEPENDENCY whose source could not be read (`facade_blind`): that implementor may be the owner's trait
                        // through a re-export this scan cannot see. It is JOINED as one more bounded-CHA candidate (§4: the
                        // union over every visible implementor) rather than hedged: an effectful implementor's effect then
                        // reaches the caller exactly where the join was lost (EXECUTED: the 14 lock-less cells that read `deny
                        // Fs` 0), and a pure one adds nothing, so a site that lost no effect is not touched. A source that CAN be
                        // read and does not re-export it proves the two traits distinct, and nothing is added.
                        let blind_hit: Vec<&String> = if facade_hit.is_none() && rev_hit.is_empty() && !candor_report::env::switch("CANDOR_R1715_OFF") {
                            let tl = ukey_member.rsplit("::").take(2).collect::<Vec<_>>();
                            if tl.len() == 2 {
                                let t2l = format!("{}::{}", tl[1], tl[0]);
                                // SOUNDNESS R1739 — not an implementor of a trait the TREE proves distinct from the dispatched one
                                // (`DepSigs::distinct_in_tree`: the implementor's package is a path dependency, or lock-pinned,
                                // and declares the trait itself; the dispatch owner's pinned source declares its own, or it is not
                                // a path package and so cannot re-export a path package's trait). `impl otherkit::Visitor for
                                // Other` (a file-deleting `visit_u64`, `otherkit` a path dependency) joined serde_json's
                                // `serde_core#de::Visitor::visit_u64` dispatch by tail alone (EXECUTED `t2_otherkit_loud`). Never
                                // from a registry checkout the lock does not pin (Fable F2/F3).
                                facade_blind.get(&t2l).map(|v| v.iter().filter(|(o, q, dtp)| {
                                    if o == ukey_owner {
                                        return false;
                                    }
                                    let distinct = !candor_report::env::switch("CANDOR_R1739_OFF") && ukey_member.rsplit_once("::").is_some_and(|(otp, _)| {
                                        let dreal = dep_renames.get(o.as_str()).map(String::as_str).unwrap_or(o.as_str());
                                        let oalias = dep_renames.iter().find(|(_, r)| r.as_str() == ukey_owner).map(|(a, _)| a.as_str()).unwrap_or(ukey_owner);
                                        r1578_sigs.borrow_mut().distinct_in_tree(dir, o, dreal, dtp, oalias, ukey_owner, otp)
                                    });
                                    if distinct {
                                        if crate::lang::reach_debug() {
                                            eprintln!("R1739OWN\t{}\t{ukey}\t{o}\t{q}", f.qual); // §E1 REACH COUNTER
                                        }
                                        return false;
                                    }
                                    true
                                }).map(|(_, q, _)| q).collect())
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        };
                        if !blind_hit.is_empty() && crate::lang::reach_debug() {
                            eprintln!("R1715BLIND\t{}\t{ukey}\t{}", f.qual, blind_hit.len()); // §E1 REACH COUNTER
                        }
                        if direct_hit.is_some() || facade_hit.is_some() || !rev_hit.is_empty() || !blind_hit.is_empty() {
                            // SOUNDNESS R1748 — a candidate reached through a TAIL key (`{owner}#{Leaf}::{member}`: the
                            // exact-key miss fallback, a facade resolution, the reverse facade join) whose own trait path in
                            // that owner differs from the dispatched one is dropped when the owner's TREE source (lock-pinned,
                            // or a path dependency) declares BOTH as its own items: `impl serde_core::ser::Error for MyErr`
                            // was joined at serde_json's `serde_core#de::Error::custom` dispatch by `serde_core#Error::custom`
                            // and charged a deserializing caller a file deletion `MyErr::custom` performs (EXECUTED
                            // `t4_ser_error_core`, introduced by R1710's owner key; `t4_ser_error_facade` the same since
                            // 0.42.0). Any doubt keeps the candidate. `CANDOR_R1748_OFF=1`.
                            let dtp = ukey_member.rsplit_once("::").map_or(ukey_member, |x| x.0);
                            let r1748_off = candor_report::env::switch("CANDOR_R1748_OFF");
                            let mut r1748_drop = |tk: &str, q: &str| -> bool {
                                if r1748_off {
                                    return false;
                                }
                                let Some(ps) = r1748_paths.get(&(tk.to_string(), q.to_string())) else { return false };
                                let Some((o, _)) = tk.split_once('#') else { return false };
                                if ps.is_empty() || ps.contains(dtp) || o != ukey_owner {
                                    return false;
                                }
                                let oalias = dep_renames.iter().find(|(_, r)| r.as_str() == o).map(|(a, _)| a.as_str()).unwrap_or(o);
                                let dmem = ukey_member.rsplit("::").next().unwrap_or("");
                                let drop = ps.iter().all(|p| {
                                    *r1748_cache.entry((o.to_string(), p.clone(), ukey_member.to_string())).or_insert_with(|| {
                                        r1578_sigs.borrow_mut().distinct_paths_in_owner(dir, oalias, o, p, dtp, dmem)
                                    })
                                });
                                if drop && crate::lang::reach_debug() {
                                    eprintln!("R1748DROP\t{}\t{ukey}\t{tk}\t{q}", f.qual); // §E1 REACH COUNTER
                                }
                                drop
                            };
                            let mut impl_quals: Vec<&String> = Vec::new();
                            for q in direct_hit.into_iter().flatten() {
                                if direct_exact.is_some() || !r1748_drop(&ukey_t2, q) {
                                    impl_quals.push(q);
                                }
                            }
                            for q in facade_hit.into_iter().flat_map(|f| f.0.iter()) {
                                if !r1748_drop(&ukey_t2, q) {
                                    impl_quals.push(q);
                                }
                            }
                            for (k, qs) in rev_hit.iter() {
                                for q in qs.iter() {
                                    if !r1748_drop(k, q) {
                                        impl_quals.push(q);
                                    }
                                }
                            }
                            impl_quals.extend(blind_hit.iter().copied());
                            answered = true;
                            for cand in impl_quals {
                                if let Some(ts) = by_tail2.get(cand) {
                                    // Unambiguous only — `resolve_target`'s never-guess rule. Two units
                                    // under one tail would charge one implementor's effects to the other.
                                    if ts.len() == 1 && ts[0] != f.qual {
                                        calls.entry(f.qual.clone()).or_default().insert(ts[0].clone());
                                    }
                                }
                            }
                        }
                        // SOUNDNESS R1324 — THE ZERO-IMPLEMENTOR BUCKET. A member a UNION carried that NOTHING
                        // answers here — no chained entry under its key, no implementor in this crate — is a
                        // dispatch whose implementor this consumer cannot see. The producer used to disclose it
                        // with the R1324 hedge on the union; carrying the key instead (§4 ⟨0.40⟩ rule 1) must not
                        // turn that disclosure into silence (rule 3: the hedge is right for what cannot be
                        // completed, and here it is the CONSUMER that cannot complete it). Measured in PART 102's
                        // `*_none` arms. A member an answering entry carries (owner chained, its union pure) is
                        // ⟨0.39⟩'s model working and stays pure. A ROW's miss is untouched (PART 92 `c3_pure_only`).
                        if !answered && from_union.contains(&member) && ukey_owner != crate_name
                            && !candor_report::env::switch("CANDOR_R1324_NOMISS")
                        {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(&ukey));
                            if crate::lang::reach_debug() {
                                eprintln!("R1324MISS\t{}\t{ukey}", f.qual); // §E1 REACH COUNTER
                            }
                        }
                    }
                }
                // ⟨0.40⟩ THE WALK, for a typed method call whose own key MISSED (SPEC §2 ⟨0.40⟩ "the walk
                // reads absence as 'may be inherited'"). The receiver's type is read from THIS crate's
                // source, but the member may be one the dependency's type INHERITS — a trait default
                // (`impl Grand4 for Mid4 {}`, R858's concrete twin) — or one a chained package ADDS to a
                // type it does not own (`impl PBase for base::Tok`, PART 95 `r8_adds`, SILENT before
                // this). Every key the walk reaches is ADDED; nothing is withdrawn.
                //
                // WHERE THE WALK FINDS NOTHING — SPEC §2 ⟨0.40⟩'s LANGUAGE-SCOPED PERMISSION FOR RUST, for a
                // receiver typed from this crate's own source. A Rust type's members come from three places
                // only: its inherent impls, a trait in scope at the call, and its `Deref` target. So the
                // miss is the producer's purity claim (§2 rule 3) unless one of these says otherwise:
                //   1. a chained TRAIT carrying the member is in scope in this file (imported, glob-imported,
                //      re-exported, `as _`) — `adds` is never complete (PART 95 `o10_adds_partial`): ADD
                //      `Unknown`;
                //   2. the receiver's key carries `deref` — FOLLOWED inside `join_through`, and whatever it
                //      reaches is charged (PART 95 `r16_deref`);
                //   3. the `Deref` chain is OPEN — the receiver, or a type the chain reached, has no key or a
                //      KIND-ONLY one: ADD `Unknown` (PART 95 `o15_deref_macro`).
                // A closed, `Deref`-free key with no in-scope trait keeps the purity reading (PART 95
                // `c8_closed_pure`). Supertype paths that end structurally are NOT a hedge here: a method
                // a supertrait provides is callable only with that trait in scope, which is row 1.
                if hit.is_none() && c.method && c.typed && dep_default_path.is_none() {
                    if let Some((tpath, member)) = rel.rsplit_once("::") {
                        let ts = &deps_idx.surface;
                        let start = ts.type_key(cr_real, tpath).unwrap_or_else(|| format!("{cr_real}#{tpath}"));
                        let (rows, w) = crate::typesurf::join_through(deps_idx, &start, member, false);
                        let in_scope = if w.hits.is_empty() && w.deref_open {
                            Some("<deref chain open>".to_string())
                        } else if w.hits.is_empty() {
                            let fu = f.loc.split(':').next().and_then(|r| file_uses.get(r)).copied().unwrap_or(&[]);
                            crate::typesurf::in_scope_publishes(deps_idx, fu, &dep_renames, &local_trait_quals, member)
                        } else {
                            None
                        };
                        if r843_probe && (!w.hits.is_empty() || in_scope.is_some()) {
                            eprintln!("R843WALK\t{}\t{start}::{member}\thits={}\tstructural={}\tinscope={}",
                                f.qual, w.hits.len(), w.structural, in_scope.as_deref().unwrap_or("-"));
                        }
                        // MEASUREMENT ONLY: every walk that missed STRUCTURALLY (a kind-only or unkeyed node) —
                        // the population SPEC §2 ⟨0.40⟩'s default rule would hedge — tagged with which of the
                        // permission's rows, if any, still discloses it (`deref` = row 3, `scope` = row 1,
                        // `pure` = the purity reading the permission allows).
                        if r843_probe && w.hits.is_empty() && w.structural {
                            let why = if w.deref_open { "deref" } else if in_scope.is_some() { "scope" } else { "pure" };
                            eprintln!("R843SMISS\t{}\t{start}::{member}\t{why}", f.qual);
                        }
                        for de in rows {
                            apply_dep_fn(de, &f.qual, DepSink {
                                direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                                tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                                blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                                unknown_via_dep: &mut unknown_via_dep,
                            });
                        }
                        if in_scope.is_some() {
                            let leaf = tpath.rsplit("::").next().unwrap_or(tpath);
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{leaf}.{member}"));
                        }
                    }
                }
            }
            // SOUNDNESS R1202 — A TRAIT IN SCOPE MAY BEAT THE INHERENT METHOD A TYPED CALL RESOLVED TO. Rust's method probe
            // walks the receiver by value, then `&`, then `&mut`, and takes the FIRST step at which ANY candidate matches;
            // an inherent method wins only against a trait method found at the SAME step. So `use tgo::Go; let b =
            // ydep::Bar; b.go(1)` runs a blanket `impl<T> Go for T`'s `go(self)` over `Bar::go(&self)` (EXECUTED,
            // rustagent-v072/fx/r1202: `Fs` ran, `['Env']` read, `deny Fs` and `deny Unknown` both 0, on 0.40.12 too), and a
            // trait's `&self` method beats an inherent `&mut self` one. The site does not carry the receiver's
            // reference-ness, so every trait in scope that may carry the member is a candidate callee, UNIONED with the
            // resolution already made (which is kept: the inherent does run when the receiver is a reference):
            //   * a CHAINED trait publishing the member: its DEFAULT body, or an impl for the receiver's type in the trait's
            //     crate, is JOINED (a resolution); with neither on the wire a MISS discloses and a HIT keeps its answer;
            //   * a trait THIS CRATE declares with a BLANKET impl, in scope and declaring the member, is EDGED (its default,
            //     its blanket body) unless both receiver ranks are known and the inherent's step comes first; where
            //     neither unit exists the call discloses;
            //   * a dependency receiver beside a `use` of a package with NO report (a glob, or an UpperCamelCase leaf)
            //     DISCLOSES: that package's members cannot be read. Asked for a DEPENDENCY's receiver only, where this
            //     scan already chains; a local receiver beside an unchained import is the residual, stated (every
            //     UpperCamelCase import of every unchained crate would hedge every typed call in its file).
            // A trait the receiver's own type already implements is not a competitor: the call's key IS that impl. Only a
            // method-call EXPRESSION (`Call::dot`) is asked — a path call runs no probe.
            // `CANDOR_R1202_OFF` is the falsification switch.
            if !r1202_off && c.dot && c.method && c.typed && !c.is_macro && dep_default_path.is_none() && !suppress_bare_leaf {
                let segs: Vec<&str> = c.path.split("::").collect();
                let dep_recv = classified.is_none()
                    && (!resolved_local || r1152_union)
                    && jpath.contains("::")
                    && deps_idx.crates.contains(cr_real);
                let local_recv = resolved_local && !dep_recv && segs.len() >= 2;
                if dep_recv || local_recv {
                    let member = c.leaf.as_str();
                    let file = f.loc.split(':').next().unwrap_or("");
                    let fu = file_uses.get(file).copied().unwrap_or(&[]);
                    let st = crate::typesurf::in_scope_traits(deps_idx, fu, &dep_renames, &local_trait_quals, &deps, member);
                    // the receiver type: its chained key (dependency) or its leaf (local)
                    let (recv_key, ty_leaf) = if dep_recv {
                        let rel = jpath.strip_prefix(&format!("{cr}::")).unwrap_or(jpath);
                        let tpath = rel.rsplit_once("::").map_or("", |(t, _)| t);
                        let k = deps_idx.surface.type_key(cr_real, tpath).unwrap_or_else(|| format!("{cr_real}#{tpath}"));
                        (Some(k), tpath.rsplit("::").next().unwrap_or(tpath).to_string())
                    } else {
                        (None, segs[segs.len() - 2].to_string())
                    };
                    // a dependency type's supertraits (closure): a trait among them is the type's own impl, already the key
                    // (the receiver key itself too: a receiver typed as a TRAIT dispatches on that trait already)
                    let mut supers: HashSet<String> = recv_key.iter().cloned().collect();
                    if let Some(k) = &recv_key {
                        let mut todo = vec![k.clone()];
                        while let Some(t) = todo.pop() {
                            for s in deps_idx.surface.types.get(&t).and_then(|i| i.supers.as_ref()).into_iter().flatten() {
                                if supers.insert(s.clone()) {
                                    todo.push(s.clone());
                                }
                            }
                        }
                    }
                    let own_traits = type_to_traits.get(&ty_leaf);
                    // a receiver typed as a trait (`S: Serializer`, `&dyn Array`) is a dispatch on that trait, not an
                    // inherent resolution; nothing here is a competitor of it
                    let recv_is_trait = recv_key.as_ref().is_some_and(|k| deps_idx.surface.kind(k) == Some(crate::typesurf::KIND_PROTOCOL))
                        || (!dep_recv && trait_decls.contains_key(&ty_leaf));
                    for key in st.chained.iter().filter(|_| !recv_is_trait) {
                        let tr_leaf = key.split_once('#').map_or(key.as_str(), |(_, q)| q).rsplit("::").next().unwrap_or("");
                        if supers.contains(key) || own_traits.is_some_and(|v| v.iter().any(|t| t == tr_leaf)) {
                            continue;
                        }
                        // NOT the trait's ⟨0.39⟩ union: that unions EVERY implementor, and where the receiver's type
                        // implements the trait itself (its impl is the key that already answered) it charged the other
                        // implementors' effects — measured, `StdRng::fill_bytes` read `Rand` through `RngCore`'s union.
                        // What can run INSTEAD of the resolved method is the trait's DEFAULT body (a blanket impl that does
                        // not override it) or an impl for the receiver's own type written in the trait's crate (`impl Go2
                        // for ydep::Bar` in `tgo2`). Where neither body is on the wire: a MISS discloses; a HIT (the
                        // receiver's own key answered, or a local unit resolved) keeps its resolution — the residual is a
                        // blanket impl that OVERRIDES the member, whose body is keyed by a type parameter, stated.
                        let tpkg = key.split_once('#').map_or("", |(p, _)| p);
                        // an impl for the receiver's own type in the trait's crate (an `adds` override among them) is the
                        // body that runs for it; only where there is none can the DEFAULT run
                        let mut bodies: Vec<String> = Vec::new();
                        if dep_recv && Some(tpkg) != recv_key.as_ref().and_then(|k| k.split_once('#')).map(|(p, _)| p) {
                            if let Some(k) = crate::typesurf::present(deps_idx, &format!("{tpkg}#{ty_leaf}::{member}"), true) {
                                bodies.push(k);
                            }
                        }
                        if bodies.is_empty() {
                            if let Some(k) = crate::typesurf::present(deps_idx, &format!("{key}::{member}"), true) {
                                bodies.push(k);
                            }
                        }
                        let hit = !dep_recv || dep_join_hit;
                        // SOUNDNESS R1209/R1210 — NEITHER BODY, ON A HIT: what can still run is a BLANKET impl that
                        // OVERRIDES the member (`impl<T> OGo for T { fn go(self) .. }`). Its body is on the wire only
                        // under the impl's type PARAMETER (`o::T::go`), which no receiver key names, and in the
                        // trait's `interfaceUnion` row, which unions it. A hit never covers it: a dependency
                        // receiver's key is that TYPE's own unit (an impl in the type's crate shares it, a blanket
                        // in the trait's crate does not), and a local receiver's "hit" is only that this crate
                        // resolved the inherent. EXECUTED (rustagent-v075/fx/r1207): q11_odep and q11_olty write a
                        // file and read `['Env']` with every crate chained, `deny Fs` and `deny Unknown` both 0. The
                        // union is joined: it is the trait's every implementor in its crate, so it may charge an
                        // implementor that is not the receiver's (the over-charge direction), but it holds every
                        // body that CAN run here. A miss keeps its disclosure below, unchanged.
                        //
                        // ONLY FOR A TRAIT THIS FILE NAMES FROM ITS OWN PACKAGE (a `use` whose head is the trait's
                        // package, by its leaf or by a glob of that package). `in_scope_traits` matches a `use` LEAF or
                        // ANY chained glob against EVERY chained package, so `use std::task::Context` admits rustls's
                        // `crypto::hash::Context` and `use proto::ReadError::*` admits tokio's private
                        // `multi_thread::overflow::Overflow`. Where such a false candidate had no body, R1202 did
                        // nothing; joining its UNION there charged every implementor: MEASURED (rustagent-v075 A/B,
                        // 1,160 chained census consumers) quinn `ReadToEnd::poll` gained `Log` through `self.buffer.push`
                        // on a local struct, sqlx `TcpStream::poll_shutdown` through `Pin::new(self)`. A false
                        // candidate kept out here is exactly R1202's answer, so this can only narrow the new join.
                        let named_here = fu.iter().any(|u| {
                            let head = u.split("::").next().unwrap_or("");
                            let head = dep_renames.get(head).map_or(head, String::as_str).replace('-', "_");
                            head == tpkg.replace('-', "_") && (u.ends_with('*') || u.rsplit("::").next() == Some(tr_leaf))
                        });
                        // and NOT where the receiver's own form is known and IS the inherent method's receiver: the
                        // inherent matches at the probe's first step and nothing comes before it (R1207's exemption,
                        // exact). MEASURED: async-nats `Connection::easy_write_and_flush` (`self.poll_write(cx)` in a
                        // `&mut self` fn, over an inherent `poll_write(&mut self, ..)`) gained `Log` from tokio's union.
                        let inh_first = c.recv_form != 0 && {
                            let inh: Vec<u8> = by_tail2.get(&format!("{ty_leaf}::{member}")).into_iter().flatten()
                                .filter_map(|q| r1202_rank.get(q).copied()).collect();
                            !inh.is_empty() && inh.iter().all(|&i| i == c.recv_form)
                        };
                        let mut union_body = false;
                        if bodies.is_empty() && hit && !r1209_off && named_here && !inh_first {
                            if let Some(k) = crate::typesurf::present(deps_idx, &format!("{key}::{member}"), false) {
                                bodies.push(k);
                                union_body = true;
                            }
                        }
                        if r1202_probe {
                            eprintln!("R1202\t{}\t{}\tchained\t{key}\tbodies={}{}\t{}", f.qual, c.path, bodies.len(),
                                if hit { "/hit" } else { "/miss" }, if dep_recv { "dep" } else { "local" });
                        }
                        if bodies.is_empty() && !hit {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tr_leaf}.{member}"));
                        }
                        for k in &bodies {
                            if let Some(de) = crate::typesurf::row(deps_idx, k, !union_body) {
                                apply_dep_fn(de, &f.qual, DepSink {
                                    direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                                    tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                                    blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                                    unknown_via_dep: &mut unknown_via_dep,
                                });
                            }
                        }
                    }
                    if dep_recv && !st.unread.is_empty() {
                        if r1202_probe {
                            eprintln!("R1202\t{}\t{}\tunread\t{}\t-\tdep", f.qual, c.path, st.unread.join(","));
                        }
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{ty_leaf}.{member}"));
                    }
                    // each receiver rank this crate records for `<leaf>::<member>` (a leaf two types share yields both)
                    let ranks_of = |k: &str| -> Vec<u8> {
                        by_tail2.get(k).into_iter().flatten().filter_map(|q| r1202_rank.get(q).copied()).collect()
                    };
                    // SOUNDNESS R1207 — A LOCAL RECEIVER BESIDE A TRAIT THIS SCAN CANNOT READ. `use tgo::Go; let b =
                    // crate::LTy; b.go(1)` with `tgo` unchained runs tgo's blanket `go(self)` over `LTy::go(&self)`
                    // (EXECUTED, rustagent-v075/fx/r1207: seven receiver spellings write a file and read `['Env']`,
                    // `deny Fs` and `deny Unknown` both 0). A foreign trait reaches a LOCAL type only through a blanket
                    // impl in its crate (an impl here is `own_traits`, already the key), and that crate's members are
                    // not on the wire, so the call DISCLOSES, as the dependency receiver above does. One exemption,
                    // exact: where the receiver's own form is known (`Call::recv_form`: `self`, a parameter, `&x`) and
                    // the inherent method's receiver IS that form, the inherent matches at the probe's first step and
                    // no trait method can come before it (`self.go()` inside `fn f(&self)` over `go(&self)`). An
                    // unread leaf this type already implements is the key, not a competitor; a glob is never one.
                    if local_recv && !dep_recv && !recv_is_trait && !r1207_off {
                        let unread: Vec<&String> = st.unread.iter().filter(|u| {
                            let leaf = u.rsplit("::").next().unwrap_or("");
                            leaf == "*" || !own_traits.is_some_and(|v| v.iter().any(|t| t == leaf))
                        }).collect();
                        if !unread.is_empty() {
                            let inh = ranks_of(&format!("{ty_leaf}::{member}"));
                            let first_step = !r1207_noform && c.recv_form != 0
                                && !inh.is_empty() && inh.iter().all(|&i| i == c.recv_form);
                            if r1202_probe {
                                eprintln!("R1207\t{}\t{}\tunread\t{}\tform={}/inh={:?}/{}\tlocal", f.qual, c.path,
                                    unread.iter().map(|u| u.as_str()).collect::<Vec<_>>().join(","), c.recv_form, inh,
                                    if first_step { "exempt" } else { "hedge" });
                            }
                            if !first_step && r1207_hedge {
                                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                                unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{ty_leaf}.{member}"));
                                // and the κ ledger entry beside it: the call may land in each unread package (never
                                // alone — see the POSTURE above for why a ledger-only row is not free)
                                for u in &unread {
                                    let head = u.split("::").next().unwrap_or("");
                                    if deps.contains(head) {
                                        *dep_seen.entry(head.to_string()).or_insert(0) += 1;
                                        blind_direct.entry(f.qual.clone()).or_default().insert(head.to_string());
                                    }
                                }
                            }
                        }
                    }
                    // this crate's own traits declaring the member, in scope in this file
                    let leaves: HashSet<&str> = fu.iter().filter_map(|u| u.rsplit("::").next()).collect();
                    let local_glob = fu.iter().any(|u| {
                        let head = u.split("::").next().unwrap_or("");
                        u.ends_with('*') && !deps.contains(head) && !deps_idx.crates.contains(head)
                            && !matches!(head, "std" | "core" | "alloc")
                    });
                    let declared_here = file_local_traits.get(file);
                    for (tl, lt) in trait_decls.iter().filter(|_| !recv_is_trait) {
                        if !lt.methods.contains(member) || own_traits.is_some_and(|v| v.iter().any(|t| t == tl)) {
                            continue;
                        }
                        // the trait can apply to the receiver only through a BLANKET impl (its own impls for the
                        // receiver's type are `own_traits`): an `impl<T> Tr for T`, recorded by Pass A.
                        let blanket_impl = merged.blanket_methods.contains_key(&crate::model::blanket_trait_key(tl));
                        // SOUNDNESS R1208 — or for every REFERENCE (`impl<T> Tr for &T` / `&mut T`): "1" / "2", "" both
                        let ref_blanket = if r1208_off { None } else {
                            merged.blanket_methods.get(&crate::model::blanket_ref_trait_key(tl)).map(|v| match v.as_str() {
                                "1" => 1u8,
                                "2" => 2,
                                _ => 3,
                            })
                        };
                        if !blanket_impl && ref_blanket.is_none() {
                            continue;
                        }
                        if !(local_glob || leaves.contains(tl.as_str()) || declared_here.is_some_and(|d| d.contains(tl.as_str()))) {
                            continue;
                        }
                        let blanket = merged.blanket_methods.get(member).filter(|b| !b.is_empty()).map(|b| format!("{b}::{member}"));
                        // BOTH bodies local, so both RANKS are known: on a value receiver the trait wins only at an
                        // EARLIER step than the inherent (`self` before `&self` before `&mut self`); at the same or a
                        // later step the inherent wins (Rust prefers it within a step). A REFERENCE receiver can let a
                        // blanket impl for `&T`/`&mut T` win at a later rank; the site does not carry the receiver's
                        // reference-ness, so that half is the residual, stated.
                        if local_recv {
                            let inh = ranks_of(&format!("{ty_leaf}::{member}"));
                            let mut tr = ranks_of(&format!("{tl}::{member}"));
                            if tr.is_empty() {
                                tr = blanket.as_deref().map(ranks_of).unwrap_or_default();
                            }
                            if tr.is_empty() && !r1208_off {
                                // a member with no default body: its declared rank (Pass A, `model::trait_rank_key`)
                                if let Some(r) = merged.blanket_methods.get(&crate::model::trait_rank_key(tl, member))
                                    .and_then(|v| v.parse::<u8>().ok()).filter(|r| *r > 0)
                                {
                                    tr.push(r);
                                }
                            }
                            if r1208_off {
                                // the R1202 rule: a VALUE receiver of a plain blanket, by the lowest rank on each side
                                if let (Some(i), Some(t)) = (inh.iter().min(), tr.iter().min()) {
                                    if t >= i {
                                        continue;
                                    }
                                }
                            } else if !inh.is_empty() && !tr.is_empty() {
                                // SOUNDNESS R1208 — the R1202 rule (`t >= i` → the inherent wins) holds only for a VALUE
                                // receiver of a PLAIN blanket. On `&mut x` a trait `&mut self` method beats an inherent
                                // `&self` one; on `&x` of a `Copy` type a trait `&self` method beats an inherent `self`
                                // one; an impl for `&T` / `&mut T` is matched one autoref step later than its rank says.
                                // EXECUTED (rustagent-v075/fx/r1207: q8a/q8b/q8c/q8d/q8f each write a file and read
                                // `['Env']`, `deny Fs` 0). So the probe is run (`model::trait_may_win`) over the receiver's
                                // own form where the site knows it, and over every form where it does not; and over
                                // every pair of ranks the two names carry.
                                let sf: Vec<u8> = match (blanket_impl, ref_blanket) {
                                    (true, None) => vec![0],
                                    (false, Some(r)) => vec![r],
                                    (_, Some(r)) => vec![0, r],
                                    (false, None) => vec![],
                                };
                                let wins = inh.iter().any(|&i| tr.iter().any(|&t| sf.iter().any(|&f|
                                    crate::model::trait_may_win(i, t, f, c.recv_form))));
                                if !wins {
                                    continue;
                                }
                            }
                        }
                        let mut edged = false;
                        for k in std::iter::once(format!("{tl}::{member}")).chain(blanket) {
                            for t in by_tail2.get(&k).into_iter().flatten() {
                                if t != &f.qual {
                                    calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                    edged = true;
                                }
                            }
                        }
                        if r1202_probe {
                            eprintln!("R1202\t{}\t{}\tlocal\t{tl}\tedged={edged}\t{}", f.qual, c.path, if dep_recv { "dep" } else { "local" });
                        }
                        if !edged {
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tl}.{member}"));
                        }
                    }
                }
            }
            // ⟨0.40⟩ SOUNDNESS R887 — THE PERMISSION'S FIRST ROW ON A RECEIVER THAT IS NOT A DEPENDENCY'S
            // TYPE. `use dep::Bl; x.bl()` with `x: &u8` reaches the dependency's blanket `impl<T: ?Sized> Bl
            // for T`, but the receiver types as `u8`, so no chained key was ever asked and the call read
            // SILENT (`['Net']`, executed `Env`). A typed method call on a std, primitive or local type that
            // nothing answered — not classified, not resolved locally — can only be supplied by a trait in
            // scope; where a chained TRAIT carrying the member is in scope here, ADD `Unknown` (SPEC §2
            // ⟨0.40⟩: "joined through ⟨0.39⟩'s union, or the consumer MUST ADD `Unknown`"; disclosing keeps
            // every new answer an `Unknown`, never a charge that an inherent method may shadow). An UNTYPED
            // receiver is not asked: a bare-leaf twin rides beside every typed call, so it cannot tell a
            // resolved local method from a missing one.
            if c.method && c.typed && classified.is_none() && !resolved_local && !suppress_bare_leaf
                && !deps_idx.crates.contains(cr_real)
            {
                let fu = f.loc.split(':').next().and_then(|r| file_uses.get(r)).copied().unwrap_or(&[]);
                if let Some(tr) = crate::typesurf::in_scope_publishes(deps_idx, fu, &dep_renames, &local_trait_quals, &c.leaf) {
                    let leaf = tr.split_once('#').map_or(tr.as_str(), |(_, q)| q).rsplit("::").next().unwrap_or("").to_string();
                    if r843_probe {
                        eprintln!("R843SCOPE\t{}\t{}\t{tr}", f.qual, c.path);
                    }
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{leaf}.{}", c.leaf));
                }
            }
            // SOUNDNESS R223 — WHICH resolution is allowed to silence the classifier. `resolved_local`
            // stays exactly as computed: the edge is KEPT, so this change can only ADD a charge and can
            // never withdraw one — the direction matters, because the first two cuts of this fix made the
            // path un-`resolvable` instead and dropped 161 (then 43) concrete effects across the corpus,
            // among them tungstenite's `Rand` through an extension-trait `impl IntoClientRequest for
            // http::Uri` and tokio-websockets' `Rand` through a `pub use` re-export. Both are local
            // definitions a consumer really reaches, and a fix for a silent under-report that creates
            // silent under-reports elsewhere is not a fix.
            //
            // What is withdrawn is only the AUTHORITY to speak for the dependency. `resolved_local`
            // suppresses the classifier because the local definition is authoritative — true when the
            // call names that definition, false when the only thing tying them together is the two
            // segments `tail2` kept (`extern_dep_qualified` above). The result is a UNION: the local
            // edge's effects flow through propagation AND the dependency's classified effect is charged,
            // which is the sound over-approximation. The precision cost is a call whose head merely LOOKS
            // like a dependency inheriting that crate's rule — over-report, on the record in `inferred`,
            // never the cardinal sin.
            let local_is_authoritative = resolved_local && !extern_dep_qualified;
            if let Some(eff) = classified.filter(|_| !suppress_bare_leaf && !local_is_authoritative) {
                direct.entry(f.qual.clone()).or_default().insert(eff);
                // SPEC §2 `fs` — refine an Fs we just PROVED with the direction its verb implies. A verb
                // the table does not recognise contributes nothing, so the field stays absent rather than
                // half-claimed.
                if eff == "Fs" {
                    // A verb revealing no direction records the POISON marker "?" rather than nothing.
                    // Abstaining would let a caller inherit a neighbour's ["write"] and claim "writes but
                    // never reads" over a reach whose kind was never determined — the partial claim §2
                    // forbids. Suppressed at emit; it never reaches the wire.
                    let kinds = candor_classify::fs_kind(&c.path);
                    let e = fskinds.entry(f.qual.clone()).or_default();
                    if kinds.is_empty() { e.insert("?".to_string()); }
                    else { for k in kinds { e.insert((*k).to_string()); } }
                }
                // §1 ⟨0.13⟩ model-SDK dispatch is `Llm` + `Net`, added UNCONDITIONALLY — a model-SDK
                // crate call IS a model dispatch AND is network I/O, regardless of what `classified`
                // carried. Insert BOTH: a model-SDK crate whose call ALSO resolves via `classify` to
                // `Net` (e.g. `aws_sdk_bedrockruntime::…::send`) short-circuits the `.or(Some("Llm"))`
                // fallback, so `eff == "Net"` and the `Llm` would be DROPPED (a gate evasion — the
                // model surface silently vanishes behind the plain Net) unless we add it here. Matches
                // the deep engine (src/lib.rs) and candor-java, which both add Llm+Net unconditionally.
                if model_sdk && !suppress_bare_leaf && !local_is_authoritative {
                    let d = direct.entry(f.qual.clone()).or_default();
                    d.insert("Llm");
                    d.insert("Net");
                }
                // SOUNDNESS R1612 (was R338's trust-store hook) — the effects the call performs BESIDE its
                // primary, from the one table every charging site asks (`candor_classify::ALSO_EFFECTS`). Charged,
                // never substituted; a second effect with a locator surface is marked `incomplete`, because the
                // call's literal is the PRIMARY's locator, and an Fs one records the direction its row declares (the
                // executed window's), or the poison marker `?` when the row declares none. The Llm block above is the same shape for the model-SDK surface.
                if !suppress_bare_leaf && !local_is_authoritative && !explicit_salt {
                    for e2 in candor_classify::also_effects(cr_real, &path_real) {
                        direct.entry(f.qual.clone()).or_default().insert(e2);
                        if candor_classify::also_hides_locator(e2) {
                            incomplete.entry(f.qual.clone()).or_default().insert(e2);
                        }
                        if e2 == "Fs" {
                            let e = fskinds.entry(f.qual.clone()).or_default();
                            let kinds = candor_classify::also_fs_kind(cr_real, &path_real);
                            if kinds.is_empty() { e.insert("?".to_string()); }
                            else { for k in kinds { e.insert(k.to_string()); } }
                        }
                        if candor_report::env::trace("CANDOR_ALSO_DEBUG") {
                            eprintln!("R1612ALSO {e2} {} :: {path_real}", f.qual);
                        }
                    }
                }
                // A host-ESTABLISHING Net / program-NAMING Exec call with NO captured literal → the endpoint
                // is invisible to the gate (a runtime value). Mark the surface incomplete so a benign captured
                // literal can't certify it (the masking evasion). Establishing-allowlist via the SHARED
                // predicate (is_net_establishing / is_cmd_naming_method) — same as the deep engine — so a
                // USE-verb (`stream.write()`) whose host was fixed at `connect` never false-positives.
                // ⟨0.29⟩ …or a two-path Fs op whose SECOND path is a runtime value: `str_arg` covers
                // position 0, and `copy`/`rename`/`symlink*` write to position 1.
                if c.path_lits_partial && eff == "Fs" {
                    incomplete.entry(f.qual.clone()).or_default().insert("Fs");
                }
                // ⟨0.29⟩ …and when it IS a literal, PUBLISH it. `str_arg` carries position 0 only, so a
                // `copy("/tmp/lit", "/tmp/dst")` published half its surface and — both positions being
                // literal — called it COMPLETE. `allow Fs /tmp/lit` then answered exit 0 over a write to
                // `/tmp/dst`. Marking it incomplete would be sound but needlessly blind: candor-java and
                // candor-swift publish both, and the destination is right there in the source.
                if let (Some(p2), "Fs") = (&c.path_lit2, eff) {
                    paths.entry(f.qual.clone()).or_default().insert(p2.clone());
                }
                if c.str_arg.is_none() {
                    // SOUNDNESS R806/R808 — every arm below asks ONE shared authority,
                    // `candor_classify::masks_locator`, which the deep engine asks too. The per-arm
                    // comments record why each disjunct inside it exists; the disjunctions themselves no
                    // longer live here, because two copies of them is how R460 reached this engine and
                    // not the other one.
                    let masks = candor_classify::masks_locator(eff, cr_real, &c.path, &c.leaf, c.method)
                        // SOUNDNESS R806 — the Db arm's scan-ONLY half, derived from the scan-only table
                        // that charges it. `scan_builder_entry_effect` charges `sqlx::query_with` /
                        // `query_as` / `query_scalar` / `diesel::sql_query` `Db` at the ENTRY because this
                        // engine cannot type the chain to its terminal; every one of those entries takes
                        // the SQL text at argument 0, and `is_db_query_arg` held only `query` of them. The
                        // deep engine never charges an entry (it types the terminal), which is why this
                        // disjunct lives here and not in the shared authority.
                        || (eff == "Db" && scan_builder_entry_effect(cr_real, &path_real) == Some("Db"));
                    // R806 REACH PROBE — the table-derived arms only, asked of the SAME function the guard
                    // asks, so the A/B's reach counts exactly the calls this row's fix newly masks.
                    if masks
                        && candor_report::env::trace("CANDOR_MASK_DEBUG")
                        && (candor_classify::masks_locator_by_table(eff, cr_real, &c.path, &c.leaf)
                            || (eff == "Db"
                                && !candor_classify::is_db_query_arg(&c.leaf)
                                && scan_builder_entry_effect(cr_real, &path_real) == Some("Db")))
                        && !candor_classify::masks_locator(eff, "", &c.path, &c.leaf, c.method)
                    {
                        eprintln!("R806MASK {eff} {} :: {} :: {}", f.qual, c.path, c.leaf);
                    }
                    if eff == "Net" && masks {
                        // R379 — instrument the TRIGGER, not the outcome. Which verb marked this
                        // surface incomplete is the whole question when pricing the denylist.
                        if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                            eprintln!("R379MASK {} :: {}", f.qual, c.leaf);
                        }
                        incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    } else if eff == "Exec"
                        && masks
                        // (was: is_cmd_naming_method(&c.leaf)
                            // SOUNDNESS R460 / SPEC ⟨0.37⟩ — …OR a METHOD whose locator is its RECEIVER.
                            // `cmd.spawn()` runs the program the `Command` it is invoked ON was built
                            // with; when that construction is not in this function, nothing here names
                            // the program, and the naming-method arm above has nothing to fire on. So a
                            // benign sibling `Command::new("git")` certified a caller-supplied spawn —
                            // AS-EFF-008's masked-literal evasion, the same one R414 closed for the Fs
                            // stat, by another spelling. The DETERMINED case does not arrive here: the
                            // collector resolves a receiver whose chain is rooted at `Command::new(lit)`
                            // into `str_arg`, so `Command::new("git").status()` is certifiable exactly as
                            // before and only an unnamed receiver reaches this branch.
                            //  || (c.method && is_exec_receiver_locator(&c.path)) — now inside `masks_locator`)
                    {
                        // R379's rule — instrument the TRIGGER, not the outcome. Which verb masked the
                        // surface is the question when pricing a denylist, and this arm IS a denylist.
                        //
                        // The two arms are marked SEPARATELY on purpose: the naming arm is pre-existing
                        // and fires on every crate that builds a command at all, so one shared marker
                        // would report its traffic as this fix's REACH. Measured: 402 mask lines over the
                        // registry, of which only the `R460MASK` half is new.
                        if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                            // The two tags share NO PREFIX, and that is deliberate: `corpus-ab.py`
                            // counts a mark by substring, so `R460MASK` / `R460MASKNAMING` reported the
                            // old arm's traffic as this fix's REACH — 409 instead of 217, measured,
                            // before the rename.
                            let tag = if candor_classify::is_cmd_naming_method(&c.leaf) {
                                "R460NAMING" // the pre-existing arm
                            } else {
                                "R460RECV" // the receiver-locator arm this row adds
                            };
                            eprintln!("{tag} {} :: {} (method={})", f.qual, c.path, c.method);
                        }
                        incomplete.entry(f.qual.clone()).or_default().insert("Exec");
                    } else if eff == "Fs"
                        && masks
                        // (was: !c.method && is_fs_path_arg(&c.leaf)
                            // SOUNDNESS R383 — …OR a METHOD whose path is an ARGUMENT rather than the
                            // receiver. `!c.method` alone let `OpenOptions::new().open(runtime_path)`
                            // through unmasked, so a benign sibling literal certified it and
                            // `allow Fs <lit>` exited 0 over a caller-supplied path.
                            //  || (c.method && is_fs_path_arg_method(&c.path))
                            // SOUNDNESS R414 / SPEC ⟨0.37⟩ — …OR a method whose locator is its RECEIVER.
                            // `p.exists()` stats the path it is invoked on; the argument spelling of the
                            // same reach (`fs::metadata(p)`) was already marked, so a silent receiver form
                            // is AS-EFF-008's masked-literal evasion by another spelling. The DETERMINED
                            // case does not arrive here at all: the collector resolves a determined
                            // receiver into `str_arg`, so `Path::new("/lit").exists()` is captured and
                            // published rather than marked, and only an indeterminate receiver reaches
                            // this branch. That split is the rung's over-charge control (arm a4local).
                            //  || (c.method && is_fs_receiver_locator(&c.path)) — now inside `masks_locator`)
                    {
                        // R379's rule — instrument the TRIGGER, not the outcome — applied to the Fs
                        // twin, which had no probe. R417 needed one: the `Dir` arm masks by DEFAULT on
                        // a directory-capability receiver, and "how often does that fire, and on what"
                        // is the question that prices the decision. Without it an A/B showing no change
                        // cannot distinguish a safely-narrow rule from a corpus that never reached it.
                        if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                            eprintln!("R417MASK {} :: {} (method={})", f.qual, c.path, c.method);
                        }
                        // A path-NAMING Fs call (`fs::write(p,…)`/`File::open(p)` — a free fn / constructor,
                        // `method=false`) with NO captured path literal → the path is a runtime value,
                        // invisible to the gate. Mark Fs incomplete so a benign sibling literal can't certify
                        // the masked path (`allow Fs` fails closed). The `!c.method` gate excludes the
                        // path-stat METHODS (`p.metadata()`/`p.exists()`) whose path is the RECEIVER, not an
                        // arg — same establishing-allowlist discipline as Net/Exec (matches candor-java).
                        incomplete.entry(f.qual.clone()).or_default().insert("Fs");
                    } else if eff == "Db" && masks {
                        // A SQL-QUERY-bearing Db call (`con.execute(sql,…)`/`query`/`prepare`) with NO captured
                        // query literal → the table is a runtime value, invisible to the gate. Mark Db
                        // incomplete so a benign sibling literal can't certify the masked table. The allowlist
                        // excludes build-then-execute terminals (`fetch_all`/`load`/`all`) and lifecycle ops
                        // (`connect`/`open`/`begin`) whose query is built structurally (no maskable string).
                        incomplete.entry(f.qual.clone()).or_default().insert("Db");
                    } else if candor_report::env::trace("CANDOR_MASK_DEBUG")
                        // R817 — an accept is marked just below, outside this arm; not "let through".
                        && !(eff == "Net" && candor_classify::is_net_accepting(&path_real, &c.leaf))
                    {
                        // THE INVERSE PROBE (R379/R381 follow-on, diagnostic only — no behaviour).
                        // R379 was found because a verb `classify` calls Net was absent from the
                        // masking ALLOWLIST, and `allow Net <benign literal>` then certified a runtime
                        // endpoint. R381 is the same defect in candor-swift. Both rows left the same
                        // note: the Fs, Exec and Db analogues answer the identical question for their
                        // effects and NEITHER HAS BEEN CHECKED.
                        //
                        // This prints every call that carries an allowlisted effect, has NO captured
                        // literal, and was NOT marked incomplete — i.e. exactly the population a
                        // missing establishing verb hides in. It is the R379 trigger sweep inverted:
                        // that one asked "what do we mask", this asks "what do we let through".
                        eprintln!("UNMASKED {} :: {} :: {} :: path={} method={} argc={}",
                                  eff, f.qual, c.leaf, c.path, c.method, c.argc);
                    }
                }
                // ⟨0.29⟩ A BIND/LISTEN ADDRESS IS LOCAL — it never enters `hosts`, the DESTINATION
                // surface `allow Net` gates on (§2). MEASURED before this: `UdpSocket::bind("0.0.0.0:0")`
                // put `0.0.0.0:0` in `hosts` and, being a captured literal, made the surface look
                // complete — so `allow Net 0.0.0.0` answered `policy ✓` at exit 0 over a `send_to` to a
                // RUNTIME endpoint. A local listen address certifying a remote send.
                //
                // WITHHOLDING THE LITERAL IS THE WHOLE FIX, and the first attempt got that wrong: it also
                // marked the surface `incomplete` unconditionally, on the assumption that an EMPTY host
                // surface would otherwise certify. **Measured, that assumption is false** — a `Net`
                // function with no captured host fails closed on its own ("performs Net with no visible
                // literal", four-way). The extra hedge bought no soundness and cost real precision: a
                // review fixture of an ordinary client — `bind("0.0.0.0:0")` beside a visible
                // `connect("api.example.com:443")` — could not be certified by `allow Net api.example.com`
                // even though its destination is right there. `UdpSocket` has no constructor but `bind`,
                // so that is every UDP client. Withhold the literal; add no hedge.
                //
                // ⟨0.40⟩ SOUNDNESS R817 — …and an ACCEPT is not a bind. `accept`/`incoming` sat on the
                // binding list, so they were withheld and marked nothing, exactly like a use-verb: beside a
                // benign `connect("ok.example:80")`, `allow Net ok.example` certified a server writing to
                // whoever connected. The peer of an accept is chosen by the connecting side, so no literal
                // can name it: mark `Net` incomplete whether or not a literal was captured (a server
                // bootstrap's literal is its LISTEN address, withheld like a bind's). The shared predicate
                // is asked by the deep engine too (src/lib.rs).
                if eff == "Net" && candor_classify::is_net_accepting(&path_real, &c.leaf) {
                    if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                        eprintln!("R817ACCEPT {} :: {}", f.qual, path_real);
                    }
                    incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    continue;
                }
                if eff == "Net" && candor_classify::is_net_binding(&c.leaf) {
                    // ⟨0.40⟩ SOUNDNESS R949 — …EXCEPT the name a runtime STRING makes it resolve. The
                    // collector sets `path_lits_partial` on a `bind` whose argument 0 is a provably
                    // string-typed runtime value: std resolves it through `impl ToSocketAddrs for str`
                    // (getaddrinfo), a Net reach whose locator nobody can see, so a benign sibling literal
                    // must not certify it. A `SocketAddr` or untyped argument marks nothing.
                    if c.path_lits_partial {
                        if candor_report::env::trace("CANDOR_MASK_DEBUG") {
                            eprintln!("R949NAME {} :: {}", f.qual, path_real);
                        }
                        incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    }
                    continue;
                }
                if let Some(s) = &c.str_arg {
                    match eff {
                        // `Net` and `Llm` share the host surface: `Llm` ⟨0.13⟩ rides the Net host literal
                        // (`allow Llm <host>` certifies against the SAME captured host), so a model-SDK
                        // call reaching `Llm` via the `.or(Some("Llm"))` fallback (classify returned None)
                        // with an endpoint literal MUST capture that host — else `allow Llm <host>` has no
                        // literal to certify and fails closed. Identical capture logic to `Net`.
                        "Net" | "Llm" => {
                            let h = host_part(s);
                            // A DOTLESS host is NOT a certifiable Net allowlist literal — java/ts/swift
                            // `hostLiteral` REJECT dotless hosts (`localhost:8080` is a bare label, not a
                            // routable name), so rust must NOT capture one into the host surface either
                            // (a cross-engine divergence — rust over-captured `localhost:8080` where the
                            // siblings dropped it). The ONE exception is the port-based Ollama refinement:
                            // a dotless `:11434` still adds `Llm` (the model signal is the PORT, not the
                            // host name) but is likewise never captured. So:
                            //   • dotless `:11434`  → add `Llm`, no host capture (Ollama, matches java);
                            //   • dotless otherwise → nothing captured (Net effect stays; no literal — a
                            //     dotless host has no gate-certifiable surface, matching the siblings);
                            //   • dotted host       → model-host refinement + capture (rides `allow Net`
                            //     / `allow Llm <host>`), incl. `127.0.0.1:11434` which IS dotted.
                            if h.contains('.') {
                                // §1 ⟨0.13⟩ host-literal refinement: a known model host adds `Llm` (Net kept).
                                if candor_classify::is_model_host(&h) {
                                    direct.entry(f.qual.clone()).or_default().insert("Llm");
                                }
                                hosts.entry(f.qual.clone()).or_default().insert(h);
                            } else if h.rsplit_once(':').is_some_and(|(_, p)| p == "11434") {
                                // §1 ⟨0.13⟩ Ollama dotless-host (`localhost:11434`): refine to `Llm` WITHOUT
                                // capturing the bare host (`allow Llm localhost` has no literal to certify —
                                // fails closed; `deny Llm` still catches it). Matches java's dotless branch.
                                direct.entry(f.qual.clone()).or_default().insert("Llm");
                            }
                            // else: a plain dotless host (`localhost:8080`) — Net effect already recorded;
                            // no host captured (matches sibling `hostLiteral`; `allow Net` can't certify it).
                        }
                        "Exec" => {
                            // Capture the program head + refine the cliff (spec §4 ⟨0.5⟩) ONLY at a
                            // program-NAMING call (`new`/`cmd`), an ALLOWLIST — not "any method except a
                            // known modifier". A whole-crate-Exec crate (portable_pty/duct) classifies
                            // EVERY method as Exec, so a denylist leaked non-naming methods (a getter
                            // `get_env("psql")` reads back a KEY, not a program) → fabricated Db + polluted
                            // the `cmds` surface (a false `allow Exec` match). Method = the path's last segment.
                            if candor_classify::is_cmd_naming_method(c.path.rsplit("::").next().unwrap_or("")) {
                                cmds.entry(f.qual.clone()).or_default().insert(s.clone());
                                direct.entry(f.qual.clone()).or_default()
                                    .extend(candor_classify::classify_command_head(s).iter().copied());
                            }
                        }
                        "Fs" => { paths.entry(f.qual.clone()).or_default().insert(s.clone()); }
                        // Table-position identifiers in a SQL string literal — the Db literal
                        // surface (feeds `allow Db …`); a dynamically-built query yields nothing.
                        "Db" => { tables.entry(f.qual.clone()).or_default().extend(candor_classify::tables_in_sql(s)); }
                        _ => {}
                    }
                }
            }
            // §4 HONESTY — FFI BOUNDARY: a call that fell through EVERY resolution route above (not
            // classified, not a local def, not a dep-report join) AND names a fn declared in an `extern`
            // block is the canonical unknowable boundary — its body is in another language, so the effect
            // (Fs/Net/Exec/…) is unknowable. DISCLOSE Unknown — the same honest signal an unresolved
            // callback gets — instead of silent-pure. A safe wrapper `unsafe { system(cmd) }` otherwise
            // read pure (the `extern` block was never collected, so the call was a bare leaf resolving to
            // nothing → pure). NEVER fires when a LOCAL def of the same name exists (`suppress_bare_leaf`
            // / `resolved_local` win — the local is authoritative, no fabrication).
            //
            // (The general "any unresolvable bare call → Unknown" disclosure was PROTOTYPED and REJECTED:
            // it floods on a real corpus — closure-param invocations (`func(x)`), macro-DEFINED local
            // helpers absent from `by_leaf`, and cfg-gated platform fns all read as bare-unresolved, so it
            // charged ~80 pure tokio fns Unknown for ~0 genuine signal beyond this FFI case. See the task
            // report's residual note. The extern case below is the precise, non-flooding subset.)
            // SOUNDNESS R501 — A DEP JOIN THAT ANSWERED `Unknown` HAS NOT HANDLED THE CALL, AND THE
            // DISCLOSURES BELOW ARE WHAT SAY WHY. `dep_join_hit` used to sit in this disjunction
            // unqualified, so a call that ALSO met one of the three §4 disclosure conditions below lost
            // its `unknownWhy` the moment a sibling report covered the crate — the row kept `Unknown`
            // and `unresolved: true` and shed the reason, which is a REASON-CLASS withdrawal, not a
            // silence: `ambiguous:` classes `dispatch` (§6.2) and an unreasoned `Unknown` classes
            // `unresolved`, so `deny E Unknown[dispatch]` stopped firing on a row `deny E Unknown` still
            // caught. Measured on bson-2.15.0: `de::serde::Bson::deserialize` and
            // `de::serde::BsonVisitor::visit_some` carry `ambiguous:same-name local methods` scanned
            // alone and carry NO reason with serde chained — same binary, same crate, the dependency
            // reports the only variable.
            //
            // THE QUALIFICATION IS WHERE THE WHOLE COST SITS, and it was measured rather than reasoned.
            // Ungating on `dep_join_hit` outright restores the reason on 130 call sites and ALSO charges
            // `Unknown` to 22 that the join answered CONCRETELY — reqwest's `Response::status` /
            // `::headers`, hyper's `ClientTask::poll`, aws-config's `AssumeRoleProviderBuilder::build`.
            // Those 22 are the direction §4.0 forbids: the join keyed on `crate#tail2`, i.e. on the crate
            // head that `tail2` threw away and that produced the local "ambiguity" in the first place, so
            // it is answering on STRICTLY MORE of the written path than the collision that would hedge
            // it — and `(E, ∅)` and `(∅, {r})` are incomparable, so replacing a determined effect with a
            // hedge WITHDRAWS `E` rather than adding disclosure. So the join is treated as handling the
            // call exactly when its own answer was not itself `Unknown`.
            let already_handled = classified.is_some() || resolved_local || suppress_bare_leaf
                || (dep_join_hit && !dep_join_unknown);
            // SOUNDNESS R1034 / R1056 merged-hedge guard — an unhandled call whose expanded path is rooted in a
            // lowercase segment that is not std/core/alloc/the crate itself names some OTHER crate's code (or a
            // module this scan could not place): the body behind it was not read.
            if !already_handled && !c.is_macro {
                let root = c.path.split("::").next().unwrap_or("");
                if c.path.contains("::")
                    && root.chars().next().is_some_and(|ch| ch.is_ascii_lowercase())
                    && !matches!(root, "std" | "core" | "alloc" | "crate" | "self" | "super")
                {
                    unresolved_ext.entry(f.qual.clone()).or_default().insert(root.to_string());
                }
            }
            // SOUNDNESS R1034 (residual) — a typed call whose tail names a non-path impl MERGED with a sibling
            // impl under one module-level qual (`nonpath_shared`): which body runs is the receiver's, and
            // the unit cannot say which. Disclosed as the name-resolution ambiguity it is.
            //
            // JUDGED AFTER THE FIXPOINT, NOT HERE. The hedge exists because an edge into the merged unit would
            // charge a SIBLING impl's effect to this receiver. Where the merged unit is GENUINELY PURE — its
            // propagated effects are empty (no `Unknown` either) and it reaches no unanalysed dependency — no
            // body in it can do anything, whichever one runs, so the hedge discloses nothing true: the x11rb
            // `[T]`/tuple `serialize_into` family (which R1056's deref/re-slice routes made reachable from ~700
            // callers) is every one of them writing into a `Vec`. Recorded here, decided below
            // (`nonpath_contested`), where the merged unit's propagated answer exists.
            if !already_handled && !c.is_macro && c.method && c.typed {
                if let Some(units) = tail2(&c.path).and_then(|t2| nonpath_shared.get(&t2)) {
                    nonpath_contested.push((f.qual.clone(), units.clone()));
                    if candor_report::env::trace("CANDOR_R1034_INSTR") {
                        eprintln!("R1034SHARED\t{}\t{}", f.qual, c.path); // §E1 REACH PROBE
                    }
                }
            }
            // ── SOUNDNESS R986 — A CALL THROUGH A PATH-REDIRECTED MODULE THAT RESOLVED TO NOTHING ──────
            // rustix `#[cfg_attr(libc, path = "backend/libc/mod.rs")] mod backend;`: the crate writes
            // `crate::backend::c::timerfd_create`, this scanner placed the file at `backend::libc::c`, and
            // the call — FFI through `c.rs`'s `pub(crate) use libc::*` — formed no edge and no disclosure.
            // Bounded to calls nothing above handled, under a module the crate itself redirects; name
            // resolution failed, which is §4's `ambiguous:` kind.
            if !c.is_macro {
                let body = c.path.strip_prefix("crate::").unwrap_or(&c.path);
                if let Some(m) = redirect_mods.iter().find(|m| {
                    body.strip_prefix(m.as_str()).is_some_and(|r| r.starts_with("::"))
                }) {
                    // ── SOUNDNESS R1475 — THE UNION OF THE MODULE'S ARMS ─────────────────────────────────────────────
                    // The module is analysed in every arm (`redirect_arms`), so the call is resolved in every arm: an
                    // arm whose module holds the item EXACTLY (`<arm>::<rest>` is a unit's qual) is edged, beside whatever
                    // the written path resolved to. Before, a twin placed at the module's own name — a dead `imp.rs` or an
                    // inline `mod imp {}` — answered alone: `imp::go()` resolved to the twin's pure body and the redirected
                    // arm's write was never edged (EXECUTED `fx1475/p_file_*`, `p_inline_*`: `deny Fs` and `deny Unknown`
                    // both 0). Where the call resolved to that twin ALONE — no redirected arm holds the item at its path (a
                    // re-export, a glob, a macro) — the redirected arms are unread and the call is DISCLOSED as R986
                    // discloses one that resolved nowhere. An arm missing an item that another redirected arm holds is the
                    // ordinary `#[cfg]` split (a unix-only `advise`), not an unread arm, and is not hedged (measured on the
                    // first corpus A/B: hedging every miss read 129 `ambiguous:` rows in syn's `FoldHelper::lift`, whose
                    // impl is keyed by its self type, and memmap2's unix-only methods). `CANDOR_R1475_OFF=1` restores R986.
                    let rest = &body[m.len() + 2..];
                    let arms = redirect_arms.get(m).map(Vec::as_slice).unwrap_or(&[]);
                    // A path ALREADY under an arm's placed module (`os::x86::bit::test`, written inside the `#[path =
                    // "os/x86.rs"] mod os;` file the scanner places at `os::x86`) is not a call through the module's own
                    // name: it names the arm it sits in. Not rewritten, not hedged here (measured on the first corpus
                    // A/B: core_detect, every call inside its redirected `os` file read `ambiguous:` beside its own edge).
                    let in_arm = arms.iter().any(|a| a != m && body.strip_prefix(a.as_str()).is_some_and(|r| r.starts_with("::")));
                    let r1475 = !candor_report::env::switch("CANDOR_R1475_OFF") && !in_arm;
                    // ── SOUNDNESS R1484 — A CALLER INSIDE ONE ARM NAMES ITS OWN ARM ─────────────────────────────────
                    // `crate::backend::conv::ret_c_int` written in rustix's `backend/libc/process/syscalls.rs` was edged
                    // to the libc AND the linux_raw `conv` — but the caller compiles only where `mod backend` was loaded
                    // from the libc file, and in that build `crate::backend` IS the libc arm. Exclusivity is not decided by
                    // evaluating `#[cfg]` predicates (they overlap: `unix` and `target_os = "macos"`): the arms are
                    // targets of `mod` items of ONE name at ONE parent, and a module name is declared once per build
                    // (E0428; one item takes one `path`), so no build compiles two of them. What must be PROVEN is that the
                    // caller sits ONLY inside its arm: `r1484_own_arm` requires every module from the arm down to the
                    // caller to be placed by exactly one `mod` item (a file `#[path]`-included or `mod`-declared a second
                    // time compiles where `crate::backend` is the OTHER arm — EXECUTED `fx1484/k_shared_*`), and refuses
                    // the whole crate when a macro may place a module unseen. The sibling arms are dropped only when the
                    // own arm holds the item EXACTLY; a miss (a re-export of the other arm's file, `k_reexport_other`)
                    // keeps the union. The disclosure below is untouched. `CANDOR_R1484_OFF=1` restores R1475.
                    let own_arm = if r1475 && !candor_report::env::switch("CANDOR_R1484_OFF") {
                        r1484_own_arm(&f.qual, &f.loc, m, arms, &merged.mod_places).filter(|a| {
                            let q = format!("{a}::{rest}");
                            tail2(&q).and_then(|t2| by_tail2.get(&t2)).is_some_and(|v| v.contains(&q))
                        })
                    } else {
                        None
                    };
                    if candor_report::env::trace("CANDOR_R1484_INSTR") && r1475 {
                        let cross: Vec<&String> = arms.iter().filter(|a| *a != m && Some(*a) != own_arm.as_ref()).collect();
                        eprintln!("R1484ARM\t{}\t{}\t{}\t{}", f.qual, c.path, own_arm.as_deref().unwrap_or("-"),
                            cross.iter().map(|a| a.as_str()).collect::<Vec<_>>().join(",")); // §E1
                    }
                    let (mut tried, mut hits) = (0usize, 0usize);
                    for arm in arms.iter().filter(|a| *a != m && r1475) {
                        tried += 1;
                        let q = format!("{arm}::{rest}");
                        let hit = tail2(&q).and_then(|t2| by_tail2.get(&t2)).is_some_and(|v| v.contains(&q));
                        if hit {
                            hits += 1;
                            // `tried`/`hits` still count every arm: R1475's `twin_only` disclosure below is not this
                            // row's to move (R1484 removes EDGES only; measured — narrowing the count withdrew 80 hedges).
                            if q != f.qual && own_arm.as_ref().is_none_or(|o| o == arm) {
                                calls.entry(f.qual.clone()).or_default().insert(q.clone());
                            }
                        }
                        if candor_report::env::trace("CANDOR_R1475_INSTR") {
                            eprintln!("R1475ARM\t{}\t{}\t{q}\t{}", f.qual, c.path, if hit { "edge" } else { "unread" }); // §E1
                        }
                    }
                    let own_q = format!("{m}::{rest}");
                    let twin_only = tried > 0
                        && hits == 0
                        && resolved_local
                        && tail2(&own_q).and_then(|t2| by_tail2.get(&t2)).is_some_and(|v| v.contains(&own_q));
                    // ── SOUNDNESS R1500 — A CALLER PROVEN INSIDE THE TWIN IS NOT ASKING ABOUT THE OTHER ARMS ─────────────
                    // `twin_only` discloses the redirected arms as unread where the call resolved to the own-name twin
                    // alone. That is a true disclosure for a caller that could compile beside a redirected arm, and a
                    // false `Unknown` for one R1484's placement proof puts INSIDE the twin: in every build that compiles
                    // the caller, `crate::<m>` is the twin (one name, one `mod` item per build), and the twin holds the
                    // item exactly, so the redirected arms are not candidates for this call. rustix's `backend::libc::c`
                    // (`#[cfg_attr(windows, path = "winsock_c.rs")] mod c;`) hedged its own `preadv` this way. Dropped only
                    // where `r1484_own_arm` returns the twin itself — the same proof: every module from the twin to the
                    // caller placed by exactly one `mod` item, no macro that may place a module. A twin file compiled a
                    // second time (`#[path]`, a second `mod`, the other arm including it, a macro) keeps the hedge (EXECUTED,
                    // `rustagent-ownname/fx` `k_*`: each WROTE through the other arm's re-export, and the hedge is the
                    // only charge). `CANDOR_R1500_OFF=1` restores R1475's hedge here.
                    let own_twin = twin_only
                        && own_arm.as_deref() == Some(m.as_str())
                        && !candor_report::env::switch("CANDOR_R1500_OFF");
                    if twin_only && candor_report::env::trace("CANDOR_R1500_INSTR") {
                        eprintln!("R1500TWIN\t{}\t{}\t{}", f.qual, c.path, if own_twin { "drop" } else { "keep" }); // §E1
                    }
                    if !already_handled || (twin_only && !own_twin) {
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why
                            .entry(f.qual.clone())
                            .or_default()
                            .insert(format!("ambiguous:path-redirected module crate::{m}"));
                        if crate::lang::reach_debug() {
                            eprintln!("R986REDIR\t{}\t{}", f.qual, c.path);
                        }
                    }
                }
            }
            // ── §4 HONESTY — SOUNDNESS R452: A TYPED METHOD CALL THAT RESOLVED TO NO UNIT ──────────
            // A receiver-typed `Type::method` call that reached no local definition was dropped with NO
            // edge, NO `Unknown`, NO `unresolved` and NO `unknownWhy` — an affirmative §4 purity claim
            // over a body the engine never read. Measured: `bare(s: &Sel) { s.deregister(3) }` over a
            // `Sel::deregister` that really spawns a process is ABSENT from `functions[]` and passes
            // `deny Exec` while naming only a decoy.
            //
            // EVERY DISCLOSURE ABOVE EXCLUDES THIS SHAPE, AND NOT BY OVERSIGHT. The `ambiguous:` bare-leaf
            // hedge is `!c.method` by construction; R190(e) and R128 are `!c.method` too; R184 needs an
            // `aliased` receiver. R128's gate is the deeper reason: it asks `macro_hidden_owner` whether
            // the call path's OWNING MODULE is one whose items a macro hid — and a receiver-typed method
            // call is formed from the receiver's TYPE, so its path is `Type::method` and carries no
            // module for that question to be asked of. **So "the engine already has the vocabulary, this
            // is only wiring" is half true**: the vocabulary is there, the EVIDENCE R128 runs on is not
            // available in this call shape, which is why (b) below needs an index keyed on the TYPE.
            //
            // TWO EVIDENCED CONDITIONS, NOT A GENERAL HEDGE. Hedging every unresolved typed method call
            // is the sound over-approximation and it is a flood: measured over 250 registry crates,
            // 14,409 caller functions across 230 of them — `str::trim`, `Vec::push`, `HashMap::get`,
            // `Option::map` and every other std receiver, which this engine documents as an honest miss
            // and must stay one. Each condition below NARROWS that over-approximation on a named,
            // measured fact, which is the denylist direction; neither is an allowlist of shapes
            // permitted to hedge.
            // SOUNDNESS R1657 stage 1b (R1674) — A GUESSED HOP THAT NAMES THE CALLER ITSELF. `self.forward().reset_cache(c)` inside
            // `Regex::reset_cache`, with `forward` DECLARED `-> &DFA` (regex-automata 0.4 `hybrid::regex`): the walk typed the tail on
            // the receiver's own type, `Regex::reset_cache` — the caller — and a self-edge adds nothing, so the real callee
            // (`DFA::reset_cache`, `Log`) was neither edged nor disclosed. Before 1b an `ambiguous:` hedge on the contested
            // `Regex::forward` happened to cover the site; once `self` is typed that step resolves and the silence is exposed (MIR
            // checker COVER-MISS). The site goes to the R1095 judge as an untyped receiver of its method, as R1663's miss does.
            if !c.is_macro && c.method && c.typed && !candor_report::env::switch("CANDOR_R1657B_OFF") {
                if let Some(t2) = tail2(&c.path) {
                    let guessed = f.untyped_methods.iter().any(|m| {
                        m.strip_prefix(crate::lazy::HOP_GUESS_PREFIX) == Some(t2.as_str())
                            || m.strip_prefix(crate::lazy::GUESS_TAIL_PREFIX) == Some(t2.as_str())
                    });
                    // A receiver typed through a transparent accessor the type itself declares (`self.get_mut().read_buf(buf)`,
                    // `get_mut -> &mut T`, tokio-io's `FramedWrite`) is flagged by the collector too (`GUESS_TAIL_PREFIX`).
                    // Settled on the caller by the single claimant, or by the exact pass among twins (time 0.3's
                    // `self.date().monday_based_week()` on `crate::offset_date_time::OffsetDateTime`).
                    if guessed
                        && (by_tail2.get(&t2).is_some_and(|v| v.iter().all(|q| q == &f.qual))
                            || arm_exact_target(&c.path, &by_tail2).is_some_and(|t| t == &f.qual))
                    {
                        if let Some(cands) = self_methods_by_leaf.get(c.leaf.as_str()) {
                            if candor_report::env::trace("CANDOR_R1657_INSTR") {
                                eprintln!("R1657SELFHOP\t{}\t{t2}\tguessed={guessed}", f.qual); // §E1 REACH COUNTER
                            }
                            untyped_contested.push((f.qual.clone(), c.leaf.clone(), cands.clone(), true));
                        }
                        // …and where a step's DECLARED return is another type (R1663's `HOP_GUESS_PREFIX`: arc-swap 1.x
                        // `self.load().serialize(s)` with `load -> Guard<T, S>`, `Guard: Deref<Target = T>`, `T: Serialize`), the
                        // callee is that type's (or its deref target's) member, which may be the CALLER's type parameter: the judge
                        // above finds no crate-local candidate with an effect and would leave the site silent. Disclosed here.
                        if f.untyped_methods.iter().any(|m| m.strip_prefix(crate::lazy::HOP_GUESS_PREFIX) == Some(t2.as_str())) {
                            if candor_report::env::trace("CANDOR_R1657_INSTR") {
                                eprintln!("R1657SELFHOPMOVED\t{}\t{t2}", f.qual); // §E1 REACH COUNTER
                            }
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why.entry(f.qual.clone()).or_default().insert(format!(
                                "dispatch:untyped receiver of `{}` (a chained step is declared to return another type)",
                                c.leaf
                            ));
                        }
                    }
                }
            }
            if !c.is_macro && c.method && c.typed && !already_handled {
                if let Some(t2) = tail2(&c.path) {
                    let ty = t2.split("::").next().unwrap_or("").to_string();
                    let claimants = by_tail2.get(&t2).map(|v| v.len()).unwrap_or(0);
                    // SOUNDNESS R1664 — THE RECEIVER'S TYPE IS A RENAME RE-EXPORT (`pub(crate) use segment::HashMap as
                    // SegmentedHashMap`) and no unit answers the tail. Nothing indexes a renamed TYPE (`reexport_aliases` folds
                    // fns only), so a call on it formed `SegmentedHashMap::get` and vanished: no edge, no `Unknown`, no reason
                    // (EXECUTED, lane fixture `f64`: all three spellings of the owner read ABSENT over a write; moka's
                    // `Inner::admit`, whose `type CacheStore = crate::cht::SegmentedHashMap<..>` parameter reads `inferred []`
                    // with no `unknownWhy`). The miss is disclosed; the RESOLUTION (map the alias to its single target) is stage 2's.
                    // Only an alias no struct/enum of the crate is itself named, and only where the renamed item is type-shaped
                    // (a unit struct has no `fields` entry, so the declaration index cannot say it is one). Kill switch
                    // `CANDOR_R1664_OFF`.
                    if claimants == 0 && (resolvable || c.path.starts_with("crate::")) && !aliased && !candor_report::env::switch("CANDOR_R1664_OFF")
                        && !merged.fields.contains_key(&ty)
                        && merged.reexports.iter().any(|r| {
                            r.name != "*" && r.alias == ty && r.name != r.alias && crate::lang::is_type_ident(&r.name)
                        })
                    {
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why
                            .entry(f.qual.clone())
                            .or_default()
                            .insert(format!("unresolved:renamed re-export type `{ty}` names no unit"));
                        if candor_report::env::trace("CANDOR_R1664_INSTR") {
                            eprintln!("R1664HIT\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                        }
                    }
                    // SOUNDNESS R1663 — a typed call FORMED BY A GUESSED CHAIN HOP (`x.sel().go()`, `sel` declared to return
                    // another type: the walk typed `go` on `x`'s own type) that names no unit is a call on a receiver no route
                    // typed. Handed to the R1095 judge below, which discloses where a crate-local `self` method of that name
                    // carries a concrete effect. Only the miss: where `Base::go` exists the typed edge is already formed.
                    if claimants == 0
                        && f.untyped_methods.iter().any(|m| m.strip_prefix(crate::lazy::HOP_GUESS_PREFIX) == Some(t2.as_str()))
                    {
                        if let Some(cands) = self_methods_by_leaf.get(c.leaf.as_str()) {
                            if candor_report::env::trace("CANDOR_R1663_INSTR") {
                                eprintln!("R1663MISS\t{}\t{t2}", f.qual); // §E1 REACH COUNTER
                            }
                            untyped_contested.push((f.qual.clone(), c.leaf.clone(), cands.clone(), true));
                        }
                    }
                    // (a) THE CRATE DECLARES `Type::method` MORE THAN ONCE. `resolve_target` keys on the
                    // 2-segment tail and refuses a tail several DISTINCT definitions claim, because
                    // linking a many-way tail fabricates one type's effect onto the other's caller. The
                    // refusal is right and the silence after it is not: candor SAW the definitions and
                    // declined to choose, which is what `ambiguous:` means. `arm_exact_target` has
                    // already run above, so what reaches here is the residue the WRITTEN PATH could not
                    // settle either — genuinely unresolvable, not merely unresolved-so-far.
                    //
                    // This is the spelling that produced [[R451]]'s 170 removals: mio-0.8.11 declares
                    // `Selector::deregister` in epoll.rs, kqueue.rs AND poll.rs, so a corrected receiver
                    // moved the call onto a tail with three claimants and it vanished. R451's second
                    // gate exists to route around that, and this is the condition for relaxing it.
                    //
                    // `resolvable && !aliased` is the same gate the resolution block above ran under, so
                    // this fires on exactly the population `resolve_target` refused — never on a call
                    // into a dependency, and never where R184 is already disclosing the alias collision.
                    if claimants >= 2 && resolvable && !aliased {
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why
                            .entry(f.qual.clone())
                            .or_default()
                            .insert("ambiguous:same-name local methods".to_string());
                        if candor_report::env::trace("CANDOR_R452_INSTR") {
                            eprintln!("R452HIT\tAMBIG\t{}\t{}", f.qual, c.path);
                        }
                        // SOUNDNESS R501 reach marker — fires only on the population the R501
                        // qualification newly ADMITS (a dep join whose own answer was `Unknown`).
                        if dep_join_hit && candor_report::env::trace("CANDOR_R501_INSTR") {
                            eprintln!("R501MARK\tAMBIG\t{}\t{}", f.qual, c.path);
                        }
                    } else if claimants == 0
                        && (merged.macro_hidden_fns.contains(&c.leaf)
                            // An ALIAS declared only in macro text names a hidden target: whatever the member, it is that
                            // target's (tokio 1.18's `cfg_time! { type TimeDriver = Either<..> }`, `self.inner.park()`).
                            || (merged.macro_hidden_types.contains(&format!("{}{ty}", crate::lang::MACRO_DECL_ALIAS))
                                && !merged.fields.contains_key(&ty)
                                && !candor_report::env::switch("CANDOR_R1636_OFF")))
                        && (merged.macro_hidden_types.contains(&ty)
                            // SOUNDNESS R1636 — …or the receiver's TYPE is declared only inside unexpanded macro text (tokio
                            // 1.45's `cfg_io_driver! { enum IoStack { .. } }`, 1.18's `cfg_time! { type TimeDriver = .. }`): no
                            // declaration of it was read anywhere. The TYPE evidence is the new half; the METHOD-NAME half
                            // stays R452's (`fn <leaf>` written in unexpanded macro text): measured, the type half alone hedged
                            // 2,871 rows across the corpus, syn's `ast_struct!` types dominating, where the member is a derive's or
                            // a readable trait impl's. Not a struct candor read (`fields` holds every one it saw).
                            || (merged.macro_hidden_types.contains(&format!("{}{ty}", crate::lang::MACRO_DECL_TYPE))
                                && !merged.fields.contains_key(&ty)
                                && !candor_report::env::switch("CANDOR_R1636_OFF")))
                    {
                        // (b) THE RECEIVER'S TYPE IS ONE THIS CRATE DECLARES INSIDE A MACRO-HIDDEN
                        // MODULE, and no `Type::method` unit exists. `collect_decls` skips an
                        // item-position macro invocation, so an `impl` written inside `cfg_os_poll!{..}`
                        // contributes no unit at all — and the absence of `Sel::deregister` from
                        // `by_tail2` is then a fact about THIS ENGINE'S READING, not about the program.
                        // That is precisely R128's argument, keyed on the TYPE because this call shape
                        // carries no module (see above).
                        //
                        // WHAT KEEPS IT OFF THE FLOOD, AND BOTH HALVES WERE MEASURED. The type must be
                        // one candor read a DECLARATION for, in a module it could not read in FULL — so a
                        // std or dependency receiver can never match, which is what the 14,409-caller
                        // general hedge could not say. That alone is NOT enough: it condemns every type
                        // in the module, and over 250 registry crates it hedged 581 caller functions of
                        // which the sample is nearly all `BigDecimal::unwrap` / `SmallIndex::expect` /
                        // `Unstructured::collect` / `Vec::add` — std combinators on a receiver this
                        // engine typed wrongly, where nothing was hidden. So the METHOD NAME must also
                        // appear as `fn <name>` inside unexpanded macro text: 581 callers -> 95, keeping
                        // aho-corasick's `StateID::as_usize`/`as_u32`, `PatternID::as_usize` (declared by
                        // `index_type_impls!`) and bitflags' `Flag::bits`. The residual cost is a name
                        // collision — some macro in the crate declares a `fn` of the same name — which is
                        // over-disclosure, bounded twice over.
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why
                            .entry(f.qual.clone())
                            .or_default()
                            .insert("macro:module items hidden by an unexpanded macro".to_string());
                        if candor_report::env::trace("CANDOR_R452_INSTR") {
                            eprintln!("R452HIT\tMACRO\t{}\t{}", f.qual, c.path);
                        }
                        if dep_join_hit && candor_report::env::trace("CANDOR_R501_INSTR") {
                            eprintln!("R501MARK\tMACRO\t{}\t{}", f.qual, c.path);
                        }
                    }
                }
            }
            // SOUNDNESS R1004 (trait-impl residual) — R452 (b)'s condition, for the PATH spelling. A
            // `macro_rules!`-generated TRAIT impl stays unexpanded (`mbe::Ctx::splice`), so `Y::tm(&y)` —
            // an associated-fn path call on a type this crate declares beside a hidden item, naming a `fn`
            // written inside unexpanded macro text — has no claimant and read PURE, while `y.tm()` already
            // disclosed `macro:` through (b). Same two conjuncts, same evidence; only the call shape differs.
            if !c.is_macro && !c.method && !already_handled && c.path.contains("::") {
                if let Some(t2) = tail2(&c.path) {
                    let ty = t2.split("::").next().unwrap_or("").to_string();
                    if !by_tail2.contains_key(&t2)
                        && merged.macro_hidden_types.contains(&ty)
                        && merged.macro_hidden_fns.contains(&c.leaf)
                    {
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why
                            .entry(f.qual.clone())
                            .or_default()
                            .insert("macro:module items hidden by an unexpanded macro".to_string());
                        if candor_report::env::trace("CANDOR_R1004_INSTR") {
                            eprintln!("R1004PATH\t{}\t{}", f.qual, c.path); // §E1 REACH PROBE
                        }
                    }
                }
            }
            if !c.is_macro && !already_handled && merged.extern_fns.contains(&c.leaf) {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn".to_string()); // FFI is a native boundary — canonical `native:` (SPEC §4 ⟨0.7⟩)
                if dep_join_hit && candor_report::env::trace("CANDOR_R501_INSTR") {
                    eprintln!("R501MARK\tNATIVE\t{}\t{}", f.qual, c.path);
                }
            }
            // §4 HONESTY — AMBIGUOUS LOCAL: a BARE leaf naming TWO-OR-MORE local defs (`tail2`/leaf
            // collision: a free `tail2` + a `Type::tail2` method, or two `Type::method`s) defeats
            // `resolve_target`'s uniqueness filter (resolved_local=false) AND is suppressed from the
            // classifier/dep-join (`suppress_bare_leaf` — the local is authoritative). Today that leaves
            // NO edge and NO disclosure: the callee's effects vanish (silent-pure over a real local call).
            // DISCLOSE Unknown instead — we can't pick WHICH local def runs, so its effects are unknown,
            // not absent. PRECISELY scoped (≥2 local defs of this bare leaf) so it can't flood like the
            // rejected "any unresolvable bare call → Unknown": a closure-param call / macro-helper isn't in
            // `by_leaf`, and a UNIQUE leaf resolves through `resolve_target` (never reaches here).
            // EXCLUDE method calls (`x.run()`): an unqualified method call already resolves to NOTHING by
            // design (the `method` flag — linking it to a same-named def would guess/fabricate), and a
            // same-named method is the COMMON case (`run`/`get`/`handle` across many types), so firing here
            // floods every such call with Unknown. This disclosure is for genuinely-bare FREE calls (the M1
            // case): `run()` with ≥2 free `run` defs, where the silent drop really is a lost local edge.
            //
            // ── WHY THE KIND IS `ambiguous:` AND STAYS THAT WAY (measured, 2026-07-27) ────────────────
            // ⟨0.24⟩ `ambiguous:` IS NOW THE FIFTH KIND IN SPEC §4's closed vocabulary. It was outside it
            // when this engine started emitting it, and the argument below is the one that got it in —
            // kept, because it is the record of what the kind buys. NONE OF THE OTHER FOUR CAN EXPRESS
            // THIS STATE:
            //   • NOT `dispatch:` — that kind is reserved for unresolved member dispatch WITH a resolvable
            //     owner type, and its detail is NORMATIVE `<owner>.<member>`. A bare free call has no owner,
            //     so the detail cannot be formed, and PART 10 rejects a dot-free `dispatch:` outright. It is
            //     also not dispatch in the first place: exactly ONE function runs and Rust resolves it
            //     statically — what failed is this ANALYSER's name resolution, not the program's.
            //   • NOT `callback:` — an unresolved HIGHER-ORDER invocation over a function VALUE. This is a
            //     named call. And `callback:` is not the residual bucket: §6.2 reaches the residual by the
            //     ABSENCE of a reason, and `f2309a5` is the record of what reaching for it costs.
            //   • NOT `native:` / `reflect:` — no foreign boundary, no metadata-driven invocation.
            // SPEC §6.2's reason-class table had ALWAYS named `ambiguous*` and ruled its class `dispatch`,
            // so the spec blessed the prefix in one section and omitted it from the closed set in another
            // — an asymmetry that survives because a CONSUMER never complains about a token it can
            // classify, only a PRODUCER is non-conforming. ⟨0.24⟩ closed it in §4's favour of the
            // producer. THIS ENGINE HOLDS THE VOCABULARY ONCE: as the raw `kind:detail` string it emits,
            // read back only through `ReasonClass::classify`'s prefix table. There is no second, typed
            // kind enum here to drift out of step with that table — which is the failure the ⟨0.24⟩
            // "AN ENGINE HOLDS THIS VOCABULARY TWICE" paragraph records against the JVM engine, where a
            // correct string classifier concealed a typed `Kind` enum that lacked the kind entirely. If a
            // typed kind ever lands in this engine, `ambiguous` goes in it at the same commit.
            //
            // AND THE RENAME IS NOT FREE — the counterfactual was BUILT and RUN, not argued. With
            // `ambiguous*` reclassified to `indirect` (one line in `ReasonClass::classify`, both binaries
            // kept by content hash), `deny E Unknown[dispatch]` goes from firing on **58 of 200 crates.io
            // crates to 0 of 200**, and from exit 1 to exit 0 on pgman, ebman and candor-rust alike. That
            // is not the narrowing candor-ts measured for its malformed `dispatch:` strings (`5ba301c`,
            // where every reclassified reason named NOTHING); it deletes the rule in this engine, because
            // every OTHER `dispatch:` rust emits — 20 in a 1062-report census, all
            // `dispatch:untyped cross-package receiver` — requires a chained dependency to exist at all.
            // Pinned by `the_ambiguous_reason_kind_and_its_class_are_pinned`; conformance PART 10 now
            // scans a purpose-built fixture so the kind is VISIBLE there instead of silently absent.
            // That measurement is also what §4 ⟨0.24⟩ cites for admitting the kind rather than deleting
            // it, so the number and the vocabulary now stand or fall together.
            //
            // ── WHAT THE ⟨0.35⟩ `by_leaf` DEDUP CHANGED HERE (SOUNDNESS R222, SPEC §4 2026-09-05) ────
            // `v.len() >= 2` now counts DISTINCT QUALS, so this fires on a genuine ambiguity — two
            // separately-written definitions competing for one bare name (two modules each exporting a
            // `helper`; a free fn beside a same-named method) — and no longer on a `#[cfg]` arm set,
            // which SPEC §4 rules is ONE definition resolving to the UNION of its arms. Those calls now
            // resolve through `resolve_target` above and never reach this branch. THE NUMBERS ABOVE ARE
            // PRE-DEDUP AND ARE LEFT AS THE HISTORICAL MEASUREMENT THEY WERE: the 8,710-of-19,607
            // census and the 58/200 counterfactual were both taken while the cfg shape was hedged here,
            // and the population this branch sees is now smaller by that shape. They are not re-derived
            // — an unmeasured number here would be worse than a dated one. What is measured: over 1,509
            // registry crates the dedup withdraws this reason from 1,167 rows and adds a concrete effect
            // to 128 functions, losing none. AND IT DOES NOT MAKE THE KIND RARE — counted, not assumed:
            // over the same corpus this reason goes 24,226 → 22,826 entries and 682 → 632 crates, so
            // the 58/200 counterfactual's subject is still overwhelmingly present and the rename is
            // still a SPEC rung rather than an edit here.
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && suppress_bare_leaf
                && !c.path.contains("::")
                && by_leaf_call.get(&c.leaf).is_some_and(|v| v.len() >= 2)
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local defs".to_string());
            }
            // ── §4 HONESTY — SOUNDNESS R750: THE CONTEST THAT INCLUDES THE CALLER ITSELF ──────────
            // A qualified, non-method call whose 2-segment tail is claimed by two-or-more DISTINCT quals,
            // ONE OF WHICH IS THE CALLER'S OWN, was refused by `resolve_target` and then went SILENT: no
            // edge, no `Unknown`, no reason, and the caller ABSENT from `functions[]` — an affirmative §4
            // purity claim over a body the engine declined to read.
            //
            // THE SILENCE IT CLOSES, in one checkable sentence: `impl Tr for Ty { fn m(..) { Ty::m(self) }}`
            // — a trait impl forwarding to its type's inherent method of the same name — made the forwarder
            // vanish, and `deadpool-postgres`' `generic_client::Transaction::prepare_cached` was exactly
            // that shape over a real `Db`.
            //
            // WHY A HEDGE HERE AND A RESOLUTION THERE, because the same lane answered both ways and the
            // difference is evidence, not taste. Where the written path NAMES one claimant — an exact qual
            // under a `crate::` root, or more of the written segments than any rival — `arm_exact_target`
            // RESOLVES it (R748(i)/(ii)) and that is strictly better than disclosing. What reaches here is
            // the residue with no such evidence: the tail is genuinely contested, or the choice needs an
            // inherent-vs-trait fact this index does not hold. For that residue the honest answer is
            // `Unknown` with a reason, never a pick and never silence.
            //
            // NOT the general R190(c) hedge, which this repo priced at 4.88-7.02% of analysed units and
            // DECLINED — see the note immediately below, which stands. The self-claimant condition is what
            // makes this affordable, and it is a fact rather than a size filter: the caller is one of the
            // contestants, so the contest is partly an artefact of the caller's own existence.
            //
            // REASON STRING REUSED, NOT MINTED (§G): `ambiguous:same-name local defs` already names this
            // kind one site up, and `ReasonClass::classify` reads it through the one prefix table. A second
            // spelling for one question is the drift §F1-3 describes.
            if !c.is_macro && !c.method && !already_handled && c.path.contains("::") {
                if let Some(t2) = tail2(&c.path) {
                    if let Some(v) = by_tail2.get(&t2) {
                        let distinct: std::collections::BTreeSet<&String> = v.iter().collect();
                        if distinct.len() >= 2 && !distinct.contains(&f.qual) {
                            // VEIN A — the residue: a contested non-method path that `arm_exact_target`
                            // could not settle even with the written path anchored to its module. Recorded
                            // for the post-fixpoint judgement below rather than hedged here; see there.
                            let cl: Vec<String> = distinct.iter().map(|s| s.to_string()).collect();
                            if candor_report::env::trace("CANDOR_VEINA_RESIDUE") {
                                eprintln!("VEINARESIDUE\t{}\t{}\t{}", f.qual, c.path, cl.join(","));
                            }
                            veina_contested.push((f.qual.clone(), cl));
                        }
                        if distinct.len() >= 2 && distinct.contains(&f.qual) {
                            if candor_report::env::trace("CANDOR_R186_DEBUG") {
                                eprintln!("R750HEDGE\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER
                            }
                            direct.entry(f.qual.clone()).or_default().insert("Unknown");
                            unknown_why
                                .entry(f.qual.clone())
                                .or_default()
                                .insert("ambiguous:same-name local defs".to_string());
                        }
                    }
                }
            }
            // SOUNDNESS R190(c) — THE QUALIFIED-TAIL SPELLING OF THE SAME REFUSAL IS *NOT* CLOSED HERE,
            // AND THE REASON IS A MEASUREMENT. The block above hedges a BARE leaf naming two-or-more local
            // defs; a QUALIFIED call whose 2-segment tail names two-or-more does not, so
            //
            //     #[cfg(unix)]    mod sys { pub fn sz() -> u32 { Command::new("stty")… } }
            //     #[cfg(windows)] mod sys { pub fn sz() -> u32 { fs::read("x")… } }
            //     pub fn go_def() -> u32 { sys::sz() }
            //
            // still leaves `go_def` ABSENT from `functions[]` while both `sys::sz` rows carry ['Exec','Fs']
            // (`panel-fixes-2/cfgmod`). It is a real silence and it stays open, PRICED rather than assumed:
            //
            //   hedging every ambiguous tail2      45,472 fns / 674 of 1,509 crates  =  7.02% of analyzed
            //   …counting DISTINCT quals only      34,242 fns / 445 crates           =  5.29%
            //   …and only where the WRITTEN path
            //     fails to select one candidate    31,634 fns / 428 crates           =  4.88%
            //
            // against 0.85% for the whole rest of this family. `collector.rs`'s own note records that
            // hedging every untyped receiver was rejected at an 8-25% false-uncertainty flood; this is
            // that band, and no narrowing above brings it out.
            //
            // WHAT THE FLOOD IS MADE OF, read off the hit log rather than guessed — and it is two things
            // that are not this row's subject:
            //   * `u32::from` / `u16::from` / `u8::from` — 2,658 of x11rb-protocol's 3,419 hits. A crate
            //     with several `impl From<_> for u32` blocks keys them all under one tail, and every
            //     primitive conversion in the crate then reads ambiguous.
            //   * a type name two modules share (`xproto::QueryVersionRequest::serialize` beside
            //     `present::…`). The CALL is written with its module and names one definition; `tail2`
            //     throws that qualifier away. That is a resolution gap in the KEY WIDTH, not a contest
            //     between definitions — and closing it by RESOLVING (a wider key) would add edges rather
            //     than `Unknown`s, which is a different change with a different failure direction.
            // Filed with those numbers so the next attempt starts from the mechanism split, not from here.
            // §4 HONESTY — SOUNDNESS R184: THE ALIAS/NOMINAL LEAF COLLISION. `prim_aliases` is a
            // crate-wide set of BARE LEAVES (`cache.rs`'s merge says so: "set union — order-independent"),
            // so a `type H = fn(&str)` in one module makes EVERY `H::method` call in the crate skip local
            // resolution — including the calls that mean an unrelated `struct H` one module over. The
            // skip is right: it is what stops sled's `type Inner = [u8; CUTOFF]` inheriting a same-named
            // `struct Inner`'s effectful `Default`. The SILENCE is not: measured, `b::S::go_vec`/
            // `go_idx`/`go_direct` all ABSENT while the one-variable control `b::T::go_vec` (leaf `K`,
            // colliding with nothing) reads ['Fs'] over the identical `fs::write`.
            //
            // Fires only where the alias skip actually COST something — the tail names a definition this
            // scan read and would otherwise have edged to. Where the leaf is an alias and nothing else
            // (the sled case's own `Inner::default()` has no local `impl Default` unit to find) the
            // condition is false and nothing is emitted. Direction: over-disclosure, and the sled control
            // keeps working because `Unknown` is not `Clock`/`Env` — the fabrication it exists to stop is
            // a CONCRETE effect, which this cannot produce.
            if !c.is_macro && classified.is_none() && !resolved_local && aliased && resolvable
                && tail2(&c.path).is_some_and(|t2| by_tail2.contains_key(&t2))
            {
                if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
                    eprintln!("R184DISCLOSE {}", c.path); // §E1 HIT COUNTER
                }
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:type alias and nominal type share a leaf".to_string());
            }
            // §4 HONESTY — SOUNDNESS R190(e): A RE-EXPORT KEY THE ALIAS INDEX REFUSED TO ADMIT.
            // `reexport_aliases` declines a key three ways — the fan-out cap, a key two modules claim,
            // and R169's macro-hidden-contest guard — and each decline is correct (it will not guess
            // which definition a name means). Each also left the call resolving to NOTHING, with the
            // caller reported PURE. Measured: thirteen `#[cfg(unix)] pub use crate::gN::*` arms in one
            // module make `imp::pick()` ABSENT, while the identical THREE-arm control reads ['Exec'] —
            // the silence appears at 13 and nowhere below it, which is why no fixture found it before.
            //
            // Gated to exactly the calls `reexport_target` could have answered — non-method, qualified,
            // and no `by_tail2` definition — so a name that resolves some other way is untouched. The
            // reason travels from the index, so an A/B can attribute the three causes separately.
            // Direction: over-disclosure; the admitted key set is unchanged.
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && !aliased
                && !tail2(&c.path).is_some_and(|t2| by_tail2.contains_key(&t2))
            {
                if let Some(why) = tail2(&c.path).and_then(|t2| by_reexport_refused.get(&t2)) {
                    if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
                        eprintln!("R190EDISCLOSE {} {}", c.path, why); // §E1 HIT COUNTER
                    }
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    unknown_why.entry(f.qual.clone()).or_default().insert((*why).to_string());
                }
            }
            // §4 HONESTY — SOUNDNESS R128, A MACRO-HIDDEN TARGET: a crate-local FREE call
            // (`crate::<module>::<name>`) that resolved to nothing, whose owning MODULE is one whose item
            // list candor could not read in full because an unexpanded item-position macro sits in it.
            //
            // THE DEFECT. `collect_decls` skips item-position macro invocations (its `Item::Macro` arm
            // takes only a `macro_rules!` DEFINITION), so a `pub fn` or a `pub(crate) use` declared by
            // `cfg_rt! { .. }` / `foo!();` / `include!("gen.rs")` contributes NO unit and NO re-export
            // edge. The call then matched no `by_tail2` key, no alias, no classifier rule and no dep
            // report — and fell out of the loop with the caller reading PURE. Three shapes, each compiled
            // and RUN spawning a real process, each leaving the caller ABSENT from `functions[]`:
            //   * `mod m { macro_rules! r { () => { pub(crate) use crate::real::f; } } r!(); }`
            //     — tokio's `cfg_rt! { pub(crate) use crate::runtime::spawn_blocking; }`, src/blocking.rs.
            //   * `mod m { defit!(); }`, the macro declaring the `pub fn` itself. The WORST of the three:
            //     the target has no report row either, so even blanket `deny Exec` exited 0.
            //   * `mod m { include!("gen.rs"); }` — the build-script/`OUT_DIR` convention.
            //
            // THE HEDGE IS EVIDENCE-BASED, NOT A NAME HEURISTIC, and that is what keeps it off the flood
            // the general rule causes. Measured over the 1489-crate registry corpus: "any unresolved
            // crate-rooted call → Unknown" hits **66,196 call sites in 741 of 1489 crates** — half the
            // corpus, which is not a disclosure, it is noise. Requiring the owning module to be one whose
            // items were DEMONSTRABLY unreadable narrows that to **1,691 sites in 122 crates**, and those
            // are real: `crate::runtime::context::with_current`, `crate::task::coop::cooperative`,
            // `crate::util::trace::async_op` are all tokio functions declared inside `cfg_*!` bodies that
            // candor genuinely cannot see.
            //
            // THE BOUNDARY, STATED RATHER THAN IMPLIED — three things this deliberately does NOT cover:
            //   1. The ASSOCIATED-fn form (`crate::a::T::assoc`, module = path minus TWO segments). Also
            //      measured: **17,533 further sites in 278 crates**, and the sample is dominated by enum
            //      VARIANT construction (`crate::sync::mpsc::error::TrySendError::Closed`) and by std
            //      combinators mis-keyed onto a local type (`crate::fs::OpenOptions::map`/`expect`/`into`)
            //      — neither of which is macro-hidden at all. `a` having a macro says nothing about
            //      whether `T::assoc` was hidden, so that evidence is too weak to charge on. A macro that
            //      declares a whole `impl` is therefore still an under-report here.
            //   2. `self::`/`super::`-rooted paths — RELATIVE, and this loop has no current-module to
            //      resolve them against (see `macro_hidden_owner`).
            //   3. A macro-hidden target reached through a BARE leaf rather than a `crate::` path: with no
            //      module in the written path there is no module to check, and `by_leaf` cannot see the
            //      hidden def either.
            // Each is an under-report, which is the direction this engine prefers to guessing — but none
            // of them is "considered and safe", so none is written as one.
            //
            // TWO FURTHER GUARDS, AND BOTH CAME FROM AUDITING THE FIRST CUT'S OWN A/B RATHER THAN FROM
            // DESIGN. "The module has a macro in it" is NECESSARY evidence and it is not SUFFICIENT: it
            // condemns a whole module for one unexpandable item, and the first cut charged 794 of its
            // 1330 corpus hits on targets candor could see perfectly well. Ground-truthed from SOURCE,
            // never from candor's own report:
            //   * `tail2` MUST NAME NOTHING. `resolve_target` also returns None for an AMBIGUOUS key —
            //     two units sharing a 2-segment tail — and that is a DIFFERENT state: candor saw the
            //     definitions and declined to choose. redis's `crate::types::from_redis_value` is a plain
            //     `pub fn` at `types.rs:2437` and IS in `by_tail2`; the first cut hedged 67 calls to it
            //     because `types.rs` also carries `itoa_based_to_redis_impl!(..)`. R128 is a target that
            //     names NOTHING, so the key's ABSENCE is the trigger, not the resolution's failure. (An
            //     ambiguous local target silently losing its edge is a real, SEPARATE gap — the existing
            //     `ambiguous:same-name local defs` disclosure covers only its BARE-leaf spelling. Not
            //     fixed here, and not claimed to be.)
            //   * A LEAF NAMING A LOCAL TYPE IS A CONSTRUCTOR, AND CONSTRUCTION IS PURE. `by_tail2`
            //     indexes FUNCTIONS, so a tuple-struct call `Grouped(expr)` is absent from it by
            //     construction, never because a macro hid something. diesel's
            //     `pub struct Grouped<T>(pub(crate) T)` is at `grouped.rs:9`; the first cut hedged 85
            //     calls to it because the module also carries `impl_selectable_expression!(Grouped<T>)`.
            //     Same for nom's `pub enum Err` (165) and syn's `token::Paren`/`Brace`/`Bracket`. Implicit
            //     DROP GLUE is not lost by this guard — it travels the separate `drop_types` route, keyed
            //     on the constructed type rather than on this call.
            // Each guard NARROWS a sound over-approximation, so each is a DENYLIST — refuse the hedge on a
            // named, evidenced condition — never an allowlist of shapes permitted to hedge. The directions
            // they fail in: a macro-declared free `fn` whose name collides with a local type, and a
            // macro-hidden target whose 2-segment tail collides with a same-named module's visible unit.
            // Both are misses, both are rare, and neither is asserted to be impossible.
            //   * AND THE FULL QUAL MUST NAME NOTHING EITHER, not just the 2-segment tail. A CRATE-ROOT
            //     free fn has a ONE-segment qual (`logger`), so it has no `tail2` key at all — its
            //     absence from `by_tail2` is a fact about the INDEX, never about the source. The first
            //     guarded cut charged 160 of its 472 hits (34%, 27 crates) on exactly that reading, and
            //     every one checked was a plainly-visible root fn: `log`'s `pub fn logger()`
            //     (lib.rs:1607), criterion's `black_box` (:157), rusqlite's `str_to_cstring` (:304),
            //     zstd-safe's `parse_code` (:95), palette's `clamp` (:450), chacha20's `quarter_round`
            //     (:321) — all hedged because their lib.rs also carries a `compile_error!`/`doctest!` at
            //     item position. Ask `by_leaf` whether ANY unit's qual IS the module-relative target, and
            //     the root case answers itself.
            let rel = c.path.strip_prefix("crate::").unwrap_or(&c.path);
            let r128_fired = !c.is_macro
                && !c.method
                && !already_handled
                && !tail2(&c.path).is_some_and(|t2| by_tail2.contains_key(&t2))
                && !by_leaf.get(&c.leaf).is_some_and(|v| v.iter().any(|q| q == rel))
                && !local_types.contains(&c.leaf)
                && (macro_hidden_owner(&c.path, 1, &merged.macro_modules) || {
                    let hit = macro_hidden_owner(&c.path, 1, &r1690_glob_macro);
                    if hit && candor_report::env::trace("CANDOR_R1657S2_INSTR") {
                        eprintln!("R1690GLOBMACRO\t{}\t{}", f.qual, c.path); // §E1 REACH COUNTER, ADDED branch only
                    }
                    hit
                });
            if r128_fired {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                // KIND: `macro:` — SOUNDNESS R270, and the comment this replaces conceded the point
                // itself: "§4's own text describes `ambiguous:` around the two-same-named-defs case
                // rather than the zero-readable-defs one, so this REUSES a kind rather than fitting it
                // exactly." This IS the zero-readable-defs case — the module's items are hidden inside a
                // macro this engine declined to expand, so there are not two definitions competing, there
                // are none the engine can read. §4 ⟨0.25⟩'s clarification names the test: where an engine
                // cannot resolve, "what it discloses there is a LIMIT OF ITS OWN RESOLUTION rather than
                // an ambiguity in the program". `macro:` is off-vocabulary, tolerated under §2
                // forward-compatibility, round-trips verbatim and reaches `unresolved` through §6.2's
                // conservative catch-all — which is the class this state belongs in and `dispatch` is not.
                //
                // R257 re-kinded this condition at the two lines it was reported for and stopped there;
                // this is the sweep. MEASURED over 1,509 registry crates, BOTH directions, because
                // measuring one is how R257 got the cost backwards: `deny Unknown[dispatch]` loses this
                // spelling on exactly ONE crate (tiff-0.11.3), while `deny Unknown[unresolved]` GAINS
                // 29 crates that pass today while holding a real macro-opacity hole — reqwest x3, nix x4,
                // wasm-bindgen x4, openssl, flate2 x2, isahc x2, diesel, encoding_rs, parking_lot_core,
                // windows-core, lapin and more.
                unknown_why.entry(f.qual.clone()).or_default()
                    .insert("macro:module items hidden by an unexpanded macro".to_string());
                if dep_join_hit && candor_report::env::trace("CANDOR_R501_INSTR") {
                    eprintln!("R501MARK\tR128\t{}\t{}", f.qual, c.path);
                }
                if candor_report::env::trace("CANDOR_R128_INSTR") {
                    eprintln!("R128HIT\t{}\t{}\tt2present={}\tleafty={}\tleaffn={}", f.qual, c.path,
                        tail2(&c.path).is_some_and(|t| by_tail2.contains_key(&t)),
                        local_types.contains(&c.leaf),
                        by_leaf.contains_key(&c.leaf));
                }
            }
            // §4 HONESTY — SOUNDNESS R145, A NAME AN UNREADABLE `include!` COULD BE SUPPLYING. A free call
            // that nothing resolved — no local unit, no classifier rule, no dependency report — whose
            // callee is looked up in a module holding an `include!` this scan could not read
            // (`env!("OUT_DIR")`, a missing file), or in a module that glob-imports one. Rust resolves a
            // bare name against the CALLER's module, and a qualified one against the module its path
            // names, and in both cases the included text is part of that namespace; candor has not seen
            // it, so the absence of a definition is a fact about this engine's reading, not the program.
            // EXECUTED ground truth (build.rs writes the file into OUT_DIR, the program runs it): six
            // spellings — bare in the including module, bare in a sibling via `use m::*`, `m::f`,
            // `crate::m::f`, a root-level include — were ABSENT and gate-silent; only `use m::f` was
            // caught, and by R128.
            //
            // WHY THIS DOES NOT FLOOD where "any unresolved bare call hedges" would: the module must be
            // one with an UNREADABLE include — evidence about THIS crate, the R128 discipline — and the
            // R128 denylist carries over (a local type's constructor is not a call into hidden code).
            // The std prelude's free functions and variant constructors are excluded by name: a bare
            // `Some(x)`/`drop(x)` resolves to the prelude unless the hidden text shadows it, which no
            // generated binding does. The boundary, stated: a METHOD call on a value of a type the
            // hidden text declares, and an associated call `T::f()` on such a type, are not covered —
            // neither path carries a module to ask about (R128's boundaries 1 and 3 again, for the
            // types this time), and both stay misses.
            if !r128_fired
                && !c.is_macro
                && !c.method
                && !c.typed
                && !already_handled
                && !opaque_scope.is_empty()
                // a sentinel-rebound name (R106 `<body-item>`, R754 `<body-extern>`) is the BODY's own
                // declaration, never a name the module's namespace supplies
                && !c.path.starts_with('<')
                && !local_types.contains(&c.leaf)
                && !matches!(c.leaf.as_str(), "Some" | "Ok" | "Err" | "drop" | "size_of" | "size_of_val" | "align_of" | "align_of_val")
                && opaque_scope_names(&f.qual, &c.path, &local_types, trait_decls, &opaque_scope)
            {
                if candor_report::env::trace("CANDOR_VEINE_INSTR") {
                    eprintln!("VEINE_OPAQUECALL\t{}\t{}", f.qual, c.path); // §E1 REACH PROBE
                }
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default()
                    .insert("macro:items hidden by an include! this scan could not read".to_string());
            }
        }
        // DROP-GLUE EDGE (#3): for each LOCAL drop type this fn constructed, add the implicit scope-exit
        // edge to its `T::drop` body — but ONLY when that body is a UNIQUE local def (in `by_tail2` with
        // exactly one target), the same uniqueness discipline `resolve_target` uses. The drop body's
        // effects then propagate to `f` like any other callee (a flushing/closing guard stops reading
        // silent-pure). Self-edges are skipped (a `Drop::drop` that constructs its own type).
        for ty in &drops_here {
            if let Some(targets) = by_tail2.get(&format!("{ty}::drop")) {
                if targets.len() == 1 && targets[0] != f.qual {
                    calls.entry(f.qual.clone()).or_default().insert(targets[0].clone());
                } else if targets.len() >= 2 && !candor_report::env::switch("CANDOR_R1253_OFF") {
                    // RESOLVED where every construction of the leaf in this body spelled its type and that type is the
                    // written self type of an `impl Drop` block whose unit is a claimant (`model::ImplOwner::written`):
                    // the destructor that runs is that block's, and only that. Anything less — one construction this
                    // engine could not spell, a spelled type no block was recorded for — discloses, below.
                    let quals = drop_quals.get(ty);
                    let picked: Option<BTreeSet<String>> = quals.filter(|qs| !qs.is_empty()).and_then(|qs| {
                        let mut out = BTreeSet::new();
                        for q in qs {
                            let q = q.as_ref()?;
                            let hit: Vec<String> = impl_owners.get("Drop").into_iter().flatten()
                                .filter(|o| &o.written == q && o.declares("drop"))
                                .map(|o| format!("{}::drop", o.owner))
                                .filter(|u| targets.contains(u))
                                .collect();
                            if hit.is_empty() {
                                return None;
                            }
                            out.extend(hit);
                        }
                        Some(out)
                    });
                    if let Some(picked) = picked {
                        if crate::lang::reach_debug() {
                            eprintln!("R1253PICK\t{}\t{ty}\t{picked:?}", f.qual); // §E1 REACH COUNTER
                        }
                        for t in picked {
                            if t != f.qual {
                                calls.entry(f.qual.clone()).or_default().insert(t);
                            }
                        }
                        continue;
                    }
                    // SOUNDNESS R1253 — TWO SAME-LEAF TYPES WITH THEIR OWN `Drop` (`m1::Guard`, `m2::Guard`): the marker
                    // carries the LEAF, so which destructor runs is not recoverable here, and edging every claimant
                    // would charge one type's destructor to the other's frame (a fabrication). The refusal was right; the
                    // silence after it was not — `let _g = m2::Guard;` read PURE over a destructor that writes a file
                    // (EXECUTED, vein fixture f7: `deny Fs mk` exit 0). It is the R452(a) condition on the implicit call:
                    // the crate declares `Guard::drop` more than once and declined to choose. `CANDOR_R1253_OFF=1`
                    // restores the silence.
                    if crate::lang::reach_debug() {
                        eprintln!("R1253DROP\t{}\t{ty}\t{}", f.qual, targets.len()); // §E1 REACH COUNTER
                    }
                    direct.entry(f.qual.clone()).or_default().insert("Unknown");
                    unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local methods".to_string());
                }
            }
        }
        // ── §4 HONESTY — SOUNDNESS R529: A DISPATCH WHOSE IMPLEMENTOR LIVES INSIDE A BLOCK ─────────
        // Pass A's decl walks recurse through `Item::Mod` and nothing else, so an `impl Trait for Type`
        // written inside a fn body is in NO index: not `trait_impls`, not `foreign_impls`, not
        // `trait_quals`. Pass B is the opposite — `rebind_self` (R175) exists because the COLLECTOR does
        // walk into bodies — so the impl's effects are charged to the ENCLOSING function by syntactic
        // containment and its own `Type::method` is never minted as a unit.
        //
        // THE TWO HALVES DISAGREE IN THE CERTIFYING DIRECTION. The CHA universe holds every implementor
        // except the one it cannot name, so a `dyn Trait` dispatch whose other visible implementor is
        // PURE resolves to that pure body and the row claims purity — `inferred: []`, no `Unknown`, no
        // `invisible`, `deny Net` exit 0 over a body-local implementor that opens a TcpStream. It is
        // ⟨0.39⟩'s own toggle ("adding a pure implementation REMOVES a disclosure") reached one spelling
        // over, and the ZERO-implementor case is not affected at all: that already reads `Unknown` here.
        //
        // A HEDGE, NOT AN IMPLEMENTOR. The body-local method has no unit, so putting it in `trait_impls`
        // would edge to nothing (R452's "typed call that resolved to NO UNIT", which that row
        // deliberately does not hedge in general) and would also move the ≤12 bound and the ambiguity
        // count. What licenses the disclosure is the narrow, named fact that THIS crate implements THIS
        // member in a position Pass A cannot read — the same evidence shape as R452's
        // `macro_hidden_types`/`macro_hidden_fns` gate, not a blanket hedge on every dispatch.
        for (tr, meth) in &f.dispatch {
            // SOUNDNESS R828 — a NON-NOMINAL implementor (`impl T5 for (u8, u8)`) is RESOLVED: its
            // method is a unit, so this dispatch edges to it like the bounded CHA edges to a nominal
            // one. A recorded qual that names no unit cannot be trusted, and is hedged instead.
            if let Some(qs) = nonnominal.get(&format!("{tr}::{meth}")) {
                for q in qs {
                    if unit_quals.contains(q) {
                        if q != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(q.clone());
                        }
                    } else {
                        direct.entry(f.qual.clone()).or_default().insert("Unknown");
                        unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tr}.{meth}"));
                    }
                }
            }
            if merged.nested_impl_members.contains(&format!("{tr}::{meth}"))
                || merged.nested_impl_members.contains(&format!("{tr}::*")) // R828 — derive / `fn $m`
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                // §4's normative `dispatch:<owner>.<member>` — the owner type IS resolvable (it is the
                // trait), only the concrete body is not, which is verbatim the state this kind names.
                unknown_why.entry(f.qual.clone()).or_default().insert(format!("dispatch:{tr}.{meth}"));
                if candor_report::env::trace("CANDOR_R529_INSTR") {
                    eprintln!("R529HIT\tLOCAL\t{}\t{tr}::{meth}", f.qual); // §E1 REACH PROBE
                }
            }
        }
        // …and the same for an abstraction this crate does NOT own but DOES implement body-locally,
        // where the dispatch is spelled in this crate (`fn go(b: &dyn dep::Backend) { b.size() }`). The
        // chained spelling of this — where the dispatch lives in the DEPENDENCY and reaches here only
        // through `dispatchesOn` — is handled at the dep join above, not here.
        // SOUNDNESS R981 — a dispatch through a FOREIGN abstraction (`self.raw.next()` with `Raw:
        // futures_core::Stream`) that THIS CRATE ALSO IMPLEMENTS. The target body may be one of this
        // crate's own impls (mongodb's `RawBatchCursor::poll_next`, which does the network I/O), and no
        // report of the dependency can ever contain it: the row read `[]` + `invisible:[futures_core]`,
        // and chaining `futures_core` could not change that. The receiver is a monomorphized bound, so no
        // bounded-CHA edge is sound (R551's erasure carve-out) — this is the disclosure §4 names for an
        // owner that resolves and a body that does not: `dispatch:<trait>.<member>`. The conjunct is the
        // TRAIT being implemented here (any member), not the member, because an extension method
        // (`next`) reaches the trait's required members (`poll_next`), which is what the crate implements.
        for member in &f.foreign_dispatch {
            let Some((owner, mem)) = member.split_once('#') else { continue };
            let Some((tq, leaf)) = mem.rsplit_once("::") else { continue };
            // ONLY a member that DRIVES the abstraction's required methods — the required members
            // themselves and the extension methods that are defined by calling them (`StreamExt::next`
            // polls `poll_next`). For those, "the body may be one of this crate's implementors" is true
            // by construction, so `dispatch:<trait>.<member>` is a TRUE reason. Any other leaf reaching
            // here is the builder-chain walk carrying a bound onto something the trait does not own —
            // `.next().await.is_some()`, or a concrete implementor's INHERENT `has_next` typed as the
            // bound — and naming it a dispatch would publish a false reason (measured on mongodb).
            if !crate::lang::is_driving_async_member(leaf) {
                continue;
            }
            let prefix = format!("{owner}#{tq}::");
            let implementors: Vec<&String> = merged
                .foreign_impls
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .flat_map(|(_, v)| v.iter())
                .collect();
            if !implementors.is_empty() {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(member));
                if crate::lang::reach_debug() {
                    eprintln!("R981HIT\t{}\t{member}", f.qual); // §E1 REACH PROBE
                }
            }
        }
        for member in &f.foreign_dispatch {
            if merged.nested_impl_foreign.contains(member) {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(member));
                if candor_report::env::trace("CANDOR_R529_INSTR") {
                    eprintln!("R529HIT\tFOREIGN\t{}\t{member}", f.qual); // §E1 REACH PROBE
                }
            }
        }
        // ── ⟨0.40⟩ SOUNDNESS R608 — A CHAINED ABSTRACTION WITH AN EMPTY IMPLEMENTOR UNION ────────
        // `fn go(h: &dyn dep::Handler) { h.handle() }` where NOTHING — not the dependency, not this
        // crate — implements `Handler`. The engine KNOWS this is a dispatch (it publishes
        // `dispatchesOn` for exactly this key) and then judges the caller PURE: `inferred: []`,
        // `unresolved: false`, no `unknownWhy`, and `deny Net Unknown` exits 0 over it. That is the
        // silent purity claim in its barest form — no sibling implementor, no body-local trick, no
        // lambda, just a dispatch nobody answers — and it is what ⟨0.40⟩ forbids.
        //
        // FOUR CONJUNCTS, and each one is the thing that keeps this from being a flood:
        //   1. the owner is a REAL Cargo.toml dependency (`deps`), checked against the manifest and
        //      never against the spelling — a local module named like a crate also parses as one;
        //   2. the owner is CHAINED (`deps_idx.crates`), so this run actually HAS that crate's report
        //      and "the union is empty" is a reading of a document rather than of an absence. An
        //      UNCHAINED dependency is the ordinary honest miss and is already disclosed by
        //      `invisible`/coverage, which is a different channel with a different verdict;
        //   3. NOTHING on the wire answers the key — neither the fully-qualified member nor its
        //      2-segment tail is in `deps_idx.by_key`. Since the producer half of this rung publishes
        //      a PURE-ONLY union entry (the two `silence = purity` drops below are gone), an absent
        //      key now means "no implementor", not "implementors exist and are all pure"; before that
        //      change this conjunct conflated the two and PART 92's `c3_pure_only` is the control that
        //      says the all-pure case must stay pure;
        //   4. and THIS crate supplies no implementor of that abstraction either — java's conjunct 4,
        //      at the granularity java's own `chaTargets(owner, name, desc)` query has. See
        //      `r533_fi_prefix` above for why per-member is the WRONG port and what it measured.
        // Plus SPEC §4's conventionally-pure leaf set, which `r533_exempt_leaf` documents.
        //
        // WHY IT IS AN `Unknown` AND NOT AN `invisible`: the crate IS covered — we read its report —
        // so nothing here is uncovered. What is unknown is the TARGET BODY, and §4's normative detail
        // for that is `dispatch:<owner>.<member>`, which `r529_reason` is the single spelling of.
        for member in &f.foreign_dispatch {
            let Some((owner, mem)) = member.split_once('#') else { continue };
            if !deps.contains(owner) { continue }
            if !deps_idx.crates.contains(owner) { continue }
            let t2 = tail2(mem).map(|t| format!("{owner}#{t}")).unwrap_or_else(|| member.clone());
            if deps_idx.by_key.contains_key(member) || deps_idx.by_key.contains_key(&t2) { continue }
            // ⟨0.40⟩ THE WALK over the dependency's own supertrait edges — `&dyn dep::PSub` calling a
            // member `PBase` declares (PART 95 `r5_refined`, R858). What it reaches is ADDED; the R608
            // disclosure below is unchanged (a walk hit does not withdraw it).
            {
                if let Some((tq, m)) = mem.rsplit_once("::") {
                    let start = deps_idx.surface.type_key(owner, tq).unwrap_or_else(|| format!("{owner}#{tq}"));
                    let (rows, w) = crate::typesurf::join_through(deps_idx, &start, m, true);
                    if r843_probe && !w.hits.is_empty() {
                        eprintln!("R843DYN\t{}\t{start}::{m}\thits={}", f.qual, w.hits.len());
                    }
                    for de in rows {
                        apply_dep_fn(de, &f.qual, DepSink {
                            direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                            tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                            blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                        });
                    }
                }
            }
            if r533_exempt_leaf(mem) { continue }
            if r533_trait_prefix(member).is_some_and(|p| r533_fi_prefix.contains(&p))
                || r533_trait_prefix(&t2).is_some_and(|p| r533_fi_prefix_t2.contains(&p)) { continue }
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(member));
            if candor_report::env::trace("CANDOR_R533_INSTR") {
                eprintln!("R533HIT\t{}\t{member}", f.qual); // §E1 REACH PROBE
            }
        }
        // ── ⟨0.40⟩ SOUNDNESS R693 — CONJUNCT 4 ASSUMES AN EDGE THAT THE ERASURE CARVE-OUT WITHHELD ──
        //
        // R690, a CARDINAL SIN published in 0.39.2. `impl dep::Sink for Mine { .. TcpStream::connect .. }`
        // beside `fn f<T: dep::Sink>(r: &T) { r.emit() }` reads `inferred: []`, no `unknownWhy`, no
        // `unresolved` — and `deny Net f::` AND `deny Net Unknown f::` BOTH exit 0 over a real socket in
        // the same crate. The byte-identical body over a LOCAL trait is charged `Net`. Three spellings:
        // `<T: dep::Tr>`, `impl dep::Tr`, and `v.iter().for_each(dep::Tr::m)`.
        //
        // THE MECHANISM IS TWO CORRECT RULES MEETING, and neither is wrong on its own:
        //   · `collector.rs`'s R4 ERASURE carve-out refuses bounded CHA over local implementors of a
        //     FOREIGN trait unless the receiver is spelled `dyn`. A `T: Trait` bound / `impl Trait`
        //     parameter is monomorphized BY THE CALLER, so this crate's impls are not its witnesses;
        //     CHA-ing them fabricates (measured: 32 fresh `Unknown`s on serde_json). That stands.
        //   · R608's conjunct 4 stops the dispatch hedge when THIS crate supplies an implementor of the
        //     abstraction — on the premise that the caller therefore already HAS the answer, through the
        //     CHA edge. That premise is exactly what the carve-out above withholds.
        // So the two spellings that get no edge also get no hedge, and the crate's own `interfaceUnion`
        // row for the member (`mxdep#Sink::emit  inferred: ['Net']`, published in the SAME report, and
        // named by this fn's own `dispatchesOn`) is neither unioned in nor disclosed. R692's rule, one
        // spelling over: a local answer must UNION with a chained row, never preempt it.
        //
        // WHAT THE INVERSION PROVES, and it is why this is a defect rather than a documented residual:
        // with NO local implementor the same `<T: dep::Sink>` body reads `['Unknown']` /
        // `dispatch:Sink.emit` (R608's own arm). SUPPLYING the implementor DELETES that disclosure. One
        // variable, measured both ways.
        //
        // `Unknown` AND NOT THE UNION. Charging the member's union here would restore the `Net` — and
        // would be the serde_json fabrication by a second door, because the witness really is the
        // caller's. §4's answer for a dispatch whose body is not known is the hedge, which is what the
        // zero-implementor arm already publishes; this makes the two agree.
        //
        // FIVE CONJUNCTS, each one the reason this is not a flood:
        //   1-2. the owner is a REAL, CHAINED Cargo dependency — the sibling rule's conjuncts 1 and 2,
        //        same `deps` / `deps_idx.crates` reads, for the same reasons. An UNCHAINED dep is the
        //        ordinary coverage miss and `invisible` is its channel; a second channel for one fact is
        //        the ⟨0.34⟩ drift, and a NAMED residual here (`invisible` arms no policy form) rather
        //        than a second spelling of it.
        //   3. the member is not in SPEC §4's conventionally-pure leaf set (`r533_exempt_leaf`).
        //   4. THE EXACT MEMBER — not its trait prefix — is a key in this crate's own obligation-2
        //      implementor index. That is the opposite keying from conjunct 4 next door, deliberately:
        //      the prefix is right THERE because a malformed key (R549: `tower_service#Service::map_err`
        //      names a `ServiceExt` member) must not be hedged, and it is right HERE for the same
        //      reason — a malformed member is BY CONSTRUCTION never a key in `foreign_impls`, so this
        //      rule cannot fire on one. `r608_conjunct_4_is_asked_of_the_abstraction_not_of_the_member`
        //      is that control and it stays green: its `impl` block writes `a`, so `…Handler::b` is a
        //      prefix hit and not a member hit, and nothing is hedged.
        //   5. and THIS FUNCTION REACHES NONE of that member's local implementor units. This is the
        //      fact conjunct 4 assumes rather than checks, and checking it is the whole fix: a `dyn`
        //      receiver DOES get the CHA edge (`calls: ['Mine::emit']`) and is left alone, charged with
        //      the concrete `Net` it already had. The rule therefore fires exactly where an edge is
        //      absent, which is where the report was silent.
        //
        // DIRECTION IT FAILS IN: over-disclosure. It can only ADD `Unknown`; there is no path here that
        // removes an effect or a hedge, so a wrong firing costs a false `Unknown` (loud) and never a
        // false purity claim (silent). The ONE residual is conjunct 5's suffix match — the implementor
        // index records `{type leaf}::{method}` while `calls` holds full quals, so a same-leaf type in
        // another module implementing the same foreign member satisfies the test spuriously and
        // WITHHOLDS the hedge. That is the silent direction, it is narrow, and it is stated rather than
        // discovered: an exact-only match would instead hedge every module-nested implementor.
        for member in &f.foreign_dispatch {
            let Some((owner, mem)) = member.split_once('#') else { continue };
            if !deps.contains(owner) { continue }                 // 1
            if !deps_idx.crates.contains(owner) { continue }      // 2
            if r533_exempt_leaf(mem) { continue }                 // 3
            let t2 = tail2(mem).map(|t| format!("{owner}#{t}")).unwrap_or_else(|| member.clone());
            // 4 — the crate's own obligation-2 index, member-exact. `Vec<"{ty}::{method}">`.
            let Some(impls) = merged.foreign_impls.get(member)
                .or_else(|| foreign_impls_by_tail2.get(&t2)) else { continue };
            // 5 — did an edge to any of them actually land on this fn?
            let reached = calls.get(&f.qual).is_some_and(|ts| {
                impls.iter().any(|q| ts.iter().any(|t| t == q || t.ends_with(&format!("::{q}"))))
            });
            if reached { continue }
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            // ONE SPELLING of §4's normative dotted detail — `r529_reason`, the same function the
            // sibling rule and the R529 hedges call. A second `format!` here is the ⟨0.34⟩ drift.
            unknown_why.entry(f.qual.clone()).or_default().insert(r529_reason(member));
            if candor_report::env::trace("CANDOR_R693_INSTR") {
                eprintln!("R693HIT\t{}\t{member}", f.qual); // §E1 REACH PROBE
            }
        }
    }

    // `all` DELIBERATELY KEEPS ONE ENTRY PER `FnInfo`, DUPLICATES INCLUDED — see the doc note on
    // `blind_direct`'s population loop above for why a genuine per-cfg-branch keying fix belongs there,
    // not here, and why deduplicating THIS list is the wrong repair (it was tried and reverted: it
    // silently drops a report entry, which `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`
    // (candor-scan/tests/cli.rs) pins as a REQUIRED shape — "the report itself still lists both units,
    // so this is a GATE de-duplication, not a lost entry").
    // SOUNDNESS R1180 — every site that reaches a tail a macro-made operator unit shares with a hand-written one (`mbe_side`):
    // a call whose written tail is that tail (the hand-written claimant's own body recursing through the operator
    // included, whose self-edge is never formed), and every edge any route formed to a unit under it (a trait-dispatch or
    // alias route spells the call differently). Judged with VEIN A's rule after the fixpoint.
    if !mbe_side.is_empty() {
        let instr = candor_report::env::trace("CANDOR_R1180_INSTR");
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut push = |caller: &str, t2: &str, why: &str, out: &mut Vec<(String, Vec<String>)>| {
            if let Some(side) = mbe_side.get(t2) {
                if seen.insert((caller.to_string(), t2.to_string())) {
                    let cl: Vec<String> = side.iter().filter(|q| q.as_str() != caller).cloned().collect();
                    if !cl.is_empty() {
                        if instr {
                            eprintln!("R1180SITE\t{why}\t{caller}\t{t2}"); // §E1 REACH COUNTER
                        }
                        out.push((caller.to_string(), cl));
                    }
                }
            }
        };
        for f in &fns {
            for c in f.calls.iter().filter(|c| !c.is_macro && c.path.contains("::")) {
                if let Some(t2) = tail2(&c.path) {
                    push(&f.qual, &t2, "call", &mut veina_contested);
                }
            }
        }
        for (caller, ts) in &calls {
            for t in ts {
                if let Some(t2) = tail2(t) {
                    push(caller, &t2, "edge", &mut veina_contested);
                }
            }
        }
    }
    // SOUNDNESS R1537 (c) — the bare qual of each split set is the UNION of its members: an internal node every unproven edge
    // still reaches (`copath`, above), never published.
    let mut all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    for (bare, cp) in &copath {
        calls.entry(bare.clone()).or_default().extend([cp.inh.clone(), cp.tr.clone()]);
        all.push(bare.clone());
    }
    // SOUNDNESS R1537 (c) — A MEMBER THAT CALLS ITS OWN NAME REACHES ITS SIBLING. The commonest shape of a co-path set is one
    // member DELEGATING to the other: `pub fn sqrt(&self) -> Self { Roots::sqrt(self) }` (num-bigint-dig), `impl Loader for
    // Font { fn full_name(&self) -> String { self.full_name() } }` (font-kit). While merged, that call was the unit's own
    // self-edge and the body it reached was the unit itself, so no route ever had to resolve it — and several do not:
    // a foreign trait's UFCS path forms no edge to the local impl, a same-leaf twin type refuses `self`'s typing. Split, those
    // members read PURE over the sibling body they run (MEASURED on the first corpus A/B: num-bigint-dig `BigUint::sqrt`
    // lost its `Unknown`, font-kit `Loader::full_name` its `Log`). So a member whose body holds ANY call of the set's name
    // edges every sibling — the merge's answer, kept for exactly the bodies that can delegate. Over-charge where the call
    // was really recursion or another type's method; never a drop.
    if !copath.is_empty() {
        let leaf_of = |q: &str| q.rsplit("::").next().unwrap_or(q).to_string();
        for f in &fns {
            let Some((bare, cp)) = crate::model::copath_bare(&f.qual).and_then(|b| copath.get_key_value(b)) else { continue };
            let m = leaf_of(bare);
            if f.calls.iter().any(|c| !c.is_macro && c.leaf == m) {
                let sib = if f.qual == cp.inh { &cp.tr } else { &cp.inh };
                if r1537c_instr {
                    eprintln!("R1537CSIB\t{}\t{sib}", f.qual); // §E1 REACH COUNTER
                }
                calls.entry(f.qual.clone()).or_default().insert(sib.clone());
            }
        }
    }
    let mut inferred = propagate(&direct, &calls, &all);
    // ── §4 HONESTY — VEIN A: THE CONTEST NO WRITTEN PATH SETTLES, DISCLOSED WHERE IT COULD MATTER ──────
    // After anchoring every written path to its module (`lang::expand`), a non-method path call whose
    // two-segment tail several DISTINCT local definitions claim, and which `arm_exact_target` still
    // cannot settle, was left with no edge, no `Unknown` and no reason — R830's residue, R190(c)'s
    // qualified-tail spelling. Hedging all of it is the R190(c) hedge, priced at 4.88-7.02% and DECLINED,
    // and that ruling stands: over 1,276 registry entries 24,855 such sites remain, in 243 entries,
    // dominated by bindgen and per-arch twins whose claimants are all pure.
    //
    // THE SILENCE IS ONLY A SILENCE WHERE A CLAIMANT CARRIES SOMETHING THE CALLER DOES NOT. The program
    // runs exactly one claimant (rustc resolves the path statically), so if every claimant's transitive
    // effects are already in the caller's own set, nothing the call could reach is missing from the
    // caller and no gate over it can be wrong. Where one claimant carries an effect — or an `Unknown` —
    // the caller lacks, the caller cannot say which runs, so it discloses: `Unknown` with the
    // `ambiguous:same-name local defs` reason the R750 hedge above already uses for the same refusal
    // (§G — one spelling). Judged on the fixpoint, and re-run to a fixpoint, because a hedge raises the
    // caller and may raise another contested caller above it. Monotone: it only ever adds `Unknown`.
    //
    // NOT A RESOLUTION and never a pick: no edge is added, because a union over distinct definitions is
    // a fabrication (only one runs). Measured reach: 151 sites / 116 callers in 28 entries carry a
    // claimant effect the caller lacks (`resid/eff.py` in the lane's scratch); the rest are untouched.
    // R1034 / R1056 — the merged-sibling hedge, judged here. A merged unit's "is anything there" includes
    // the dependencies it reaches without a report (`invisible`): those bodies were not read, so it is not
    // genuinely pure and the hedge stays. Edges never change in this loop, so one walk of `blind_direct`
    // answers for every round.
    // SOUNDNESS R1165 — what each witness site's callee can run, from the dependency's source signature (`None` = any).
    let witness_allowed: Vec<Option<BTreeSet<String>>> = {
        let mut sigs = crate::depsig::DepSigs::default();
        let off = candor_report::env::switch("CANDOR_R1165_NO_NARROW");
        witness_sites
            .iter()
            .map(|(_, alias, real, _, pos, callee)| if off { None } else { sigs.allowed(dir, alias, real, callee, *pos) })
            .collect()
    };
    if candor_report::env::trace("CANDOR_R1165_INSTR") {
        for ((caller, alias, _, l, pos, callee), a) in witness_sites.iter().zip(&witness_allowed) {
            eprintln!("R1165SITE\t{caller}\t{alias}::{callee}\t{pos}\t{l}\t{a:?}"); // §E1 REACH COUNTER
        }
    }
    // SOUNDNESS R1186 — the crate's conversions by SOURCE leaf: (trait, target unit type, target may be this crate's own
    // type), from Pass A's `<fromimpl>` facts; and, per conversion site, which targets the dependency's callee can convert
    // the argument INTO (`depsig::DepSigs::conversion_targets`; `None` = any).
    let from_impls: HashMap<String, Vec<(String, String, bool)>> = {
        let mut m: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();
        if !from_sites.is_empty() {
            for k in returns.keys() {
                let Some(rest) = k.strip_prefix(crate::model::FROM_IMPL_PREFIX) else { continue };
                let parts: Vec<&str> = rest.split('\u{1f}').collect();
                if let [src, tr, tgt, bare] = parts[..] {
                    let local = bare == "l" && returns.contains_key(&crate::model::nominal_key(tgt));
                    m.entry(src.to_string()).or_default().push((tr.to_string(), tgt.to_string(), local));
                }
            }
            for v in m.values_mut() {
                v.sort();
                v.dedup();
            }
        }
        m
    };
    let from_targets: Vec<crate::depsig::ConvTargets> = {
        let mut sigs = crate::depsig::DepSigs::default();
        let off = candor_report::env::switch("CANDOR_R1186_NO_NARROW");
        from_sites
            .iter()
            .map(|(_, alias, real, _, pos, callee)| if off { None } else { sigs.conversion_targets(dir, alias, real, callee, *pos) })
            .collect()
    };
    if candor_report::env::trace("CANDOR_R1186_INSTR") {
        for ((caller, alias, _, l, pos, callee), a) in from_sites.iter().zip(&from_targets) {
            eprintln!("R1186SITE\t{caller}\t{alias}::{callee}\t{pos}\t{l}\t{:?}\t{a:?}", from_impls.get(l)); // §E1 REACH COUNTER
        }
    }
    let nonpath_blind = if nonpath_contested.is_empty() {
        HashMap::new()
    } else {
        propagate_str(&blind_direct, &calls, &all)
    };
    // The withdrawal needs purity ESTABLISHED, which needs the dependency universe: a scan with no manifest
    // (a single file) cannot tell a dependency call from nothing, so it keeps every merged-sibling hedge, as
    // 0.40.2 did. With a manifest, a merged unit that (transitively) makes a call nothing handled keeps it too.
    let manifest_seen = std::path::Path::new(dir).join("Cargo.toml").is_file();
    let nonpath_unres = if nonpath_contested.is_empty() {
        HashMap::new()
    } else {
        propagate_str(&unresolved_ext, &calls, &all)
    };
    let r1605_reach = if r1605_floored.is_empty() || untyped_contested.is_empty() {
        HashMap::new()
    } else {
        propagate_str(&r1605_floored, &calls, &all)
    };
    for _round in 0..8 {
        let mut hedged = false;
        for (caller, units) in &nonpath_contested {
            let impure = !manifest_seen || units.iter().any(|u| {
                inferred.get(u).is_some_and(|e| !e.is_empty())
                    || nonpath_unres.get(u).is_some_and(|b| !b.is_empty())
                    || nonpath_blind.get(u).is_some_and(|b| !b.is_empty())
                    || !inferred.contains_key(u) && direct.get(u).is_some_and(|d| !d.is_empty())
            });
            if !impure {
                if _round == 0 && candor_report::env::trace("CANDOR_R1034_INSTR") {
                    eprintln!("R1034MERGEPURE\t{caller}\t{}", units.join(",")); // §E1 — the hedge WITHDRAWN
                }
                continue;
            }
            let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
            let fresh_why = unknown_why
                .entry(caller.clone())
                .or_default()
                .insert("ambiguous:same-name local methods".to_string());
            if fresh || fresh_why {
                hedged = true;
                if candor_report::env::trace("CANDOR_R1034_INSTR") {
                    eprintln!("R1034MERGEHEDGE\t{caller}"); // §E1 REACH PROBE — the hedge KEPT
                }
            }
        }
        // SOUNDNESS R1095 — THE UNTYPED-RECEIVER FLOOR. `x.go()` where no route typed `x` (an iterator's
        // `Item`, an `if let Some(x) = Some(W(1))` payload, a `.collect()` into `Vec<_>`, an element fixed only
        // by inference — R1083) formed only the bare-leaf twin, which by design resolves to nothing: the
        // caller read ABSENT over a write (executed, scratchpad `rustagent-v045/fx4`). Typing it exactly is
        // the precision layer (R1023/R1056/R1069/R1080 each added a route); this is the floor under all of
        // them. A crate-local `self` method of that name could be the body that runs, so where ANY such
        // candidate carries a concrete propagated effect the caller discloses `Unknown` naming them — never an
        // edge, since only one runs and a union would charge the others' effects. A name only std (or a
        // dependency) defines has no candidate here and is untouched; a candidate set with no concrete effect
        // discloses nothing (see the measurement below for why `Unknown`-only candidates are excluded). The
        // detail is dot-free (§4: no owner could be formed), with any `.` in a qual replaced.
        for (caller, leaf, cands, admit_unknown) in &untyped_contested {
            // A CONCRETELY effectful candidate only. Measured over a quarter of the 1,916-crate registry corpus
            // (480 entries): any not-genuinely-pure candidate hedged 8,871 callers (5.95% of all rows newly
            // `Unknown` on the full corpus — above the declined band), dominated by std-trait names whose crate
            // impls are themselves `Unknown`-only (`hash`, `clone`, `as_ref`, `to_tokens` over a generic
            // `Hasher`/`TokenStream`); a concrete-effect candidate hedges 672. The residual is stated: an
            // untyped call to a crate method whose ONLY effect is `Unknown` stays undisclosed at the caller.
            // SOUNDNESS R1095 residual, narrowed (rustagent-0415) — …EXCEPT a member of a LOCAL trait with a BLANKET
            // impl (`impl<T, Conn> RunQueryDsl<Conn> for T {}`, diesel 2.3.10). Its default body is callable on EVERY
            // receiver with the trait in scope, so for an untyped receiver it is not one guess among std-trait twins but
            // the body that runs unless the real type shadows it — and the declined form's cost came from the std-trait
            // names, which no local blanket trait declares. Measured silence: diesel's
            // `pg::metadata_lookup::lookup_type` runs `.first(conn)` (a SQL round trip through the caller's
            // connection) on a query-builder chain no route types, its only candidate `RunQueryDsl::first` is
            // `Unknown`-only, and the caller read ABSENT; `T::lookup_type` above it lost its only cover when R1265
            // removed a false `ambiguous:` hedge. `CANDOR_R1095B_OFF=1` restores the concrete-only floor.
            let blanket_unknown = |u: &String| {
                !candor_report::env::switch("CANDOR_R1095B_OFF")
                    && inferred.get(u).is_some_and(|e| e.contains("Unknown"))
                    && u.rsplit("::").nth(1).is_some_and(|tr| merged.blanket_methods.contains_key(&crate::model::blanket_trait_key(tr)))
            };
            // SOUNDNESS R1605 — …and a candidate whose `Unknown` is (transitively) the R1605 floor: an untyped query on a
            // dependency's entity value, which before the builder fabrication was removed read as a concrete `Db` and
            // kept this caller hedged. Without this, the removal turned a hedged caller into an absent one (EXECUTED shape:
            // a generic `E::delete_by_id(id).exec(db)` with no lock-pinned source, reached through an untyped field).
            let r1605_unknown = |u: &String| r1605_reach.get(u).is_some_and(|b| !b.is_empty());
            let mut impure: Vec<&String> = cands
                .iter()
                .filter(|u| {
                    inferred.get(*u).is_some_and(|e| e.iter().any(|x| *x != "Unknown"))
                        // SOUNDNESS R1663 — a GUESSED HOP's miss is a narrow population (a typed call that named no unit), so a
                        // candidate that is not genuinely pure counts, `Unknown`-only included: moka's `Cache::insert_with_hash`
                        // is hedged all the way down and was the only candidate.
                        || (*admit_unknown && inferred.get(*u).is_some_and(|e| !e.is_empty()))
                        || blanket_unknown(u)
                        || r1605_unknown(u)
                })
                .collect();
            // SOUNDNESS R1666 — THE RESIDUAL THE FLOOR STATED, CLOSED FOR A NAME STD DOES NOT HAVE. A candidate whose ONLY effect is
            // `Unknown` (basic-toml's `Error::fix_offset`, hedged through a callback) was excluded because the std-trait names
            // (`hash`, `clone`, `as_ref`, `to_tokens`) dominated the 5.95% measurement. Those are std names; a method std has no
            // member of that name for is not one of them. So an `Unknown`-only candidate counts where `leaf` is no std method
            // name (`std_methods`, generated from rust-src). The untyped call to a crate method that is hedged all the way down
            // read ABSENT at the caller (EXECUTED, lane fixture `f66`). Kill switch `CANDOR_R1666_OFF`.
            if impure.is_empty() {
                let unknown_only: Vec<&String> =
                    cands.iter().filter(|u| inferred.get(*u).is_some_and(|e| !e.is_empty() && e.iter().all(|x| *x == "Unknown"))).collect();
                if !unknown_only.is_empty() && !*admit_unknown {
                    let std_name = crate::std_methods::is_std_method_name(leaf)
                        && !r1747_nonstd.contains(&(caller.clone(), leaf.clone()));
                    let cb = unknown_only.iter().any(|u| unknown_why.get(*u).is_some_and(|w| w.iter().any(|x| x.starts_with("callback"))));
                    if candor_report::env::trace("CANDOR_R1666_INSTR") && !direct.get(caller).is_some_and(|d| d.contains("Unknown")) {
                        eprintln!("R1666ALL\t{caller}\t{leaf}"); // §E1 REACH COUNTER: every Unknown-only site (the declined form)
                        if !std_name {
                            eprintln!("R1666NOSTD\t{caller}\t{leaf}");
                            if cb {
                                eprintln!("R1666CB\t{caller}\t{leaf}");
                            }
                        }
                    }
                    if !std_name && !candor_report::env::switch("CANDOR_R1666_OFF") {
                        impure = unknown_only;
                    }
                }
            }
            if impure.is_empty() {
                continue;
            }
            let shown: Vec<&str> = impure.iter().take(3).map(|s| s.as_str()).collect();
            let more = if impure.len() > 3 { format!(", +{}", impure.len() - 3) } else { String::new() };
            let why = format!("dispatch:untyped receiver of `{leaf}` ({}{more})", shown.join(", ")).replace('.', "_");
            let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
            let fresh_why = unknown_why.entry(caller.clone()).or_default().insert(why);
            if fresh || fresh_why {
                hedged = true;
                if candor_report::env::trace("CANDOR_R1095_INSTR") && fresh {
                    eprintln!("R1095HEDGE\t{caller}\t{leaf}"); // §E1 REACH PROBE — a caller newly Unknown
                }
            }
        }
        // SOUNDNESS R1157 — a primitive operator whose only crate impl is macro-made: which body runs depends on the
        // right operand's type, which this engine cannot prove, so where a candidate carries a CONCRETE effect the
        // caller does not already have, the caller discloses `Unknown` (`ambiguous:`, two definitions — std's and the
        // crate's — with no way to say which runs). An `Unknown`-only candidate is not hedged on (the R1095 rule).
        // SOUNDNESS R1165 — a local value with a std-trait impl handed to a CHAINED dependency. The dependency's generic
        // body may run the impl (`a == b` over `T: PartialEq`), and its report cannot say so: the body's row is pure
        // (SPEC §4, R1126) and ⟨0.39⟩ `dispatchesOn` has no spelling for a std-owned member. Chained, the caller read
        // ABSENT over an executed write while the unchained scan carried `invisible: [dep]` — chaining deleted the only
        // disclosure (SPEC :4385). So where one of the type's impl members for a `STD_WITNESS_TRAITS` trait carries a
        // CONCRETE effect the caller does not already have, the caller discloses `Unknown` with the dotted
        // `dispatch:<Trait>.<member>` reason (std's trait declares that member). A DISCLOSURE, never a charge: nothing
        // here knows the dependency's body runs the member, and charging it would fabricate wherever it does not.
        // An `Unknown`-only member is not hedged on (the R1095 rule, measured there); that is the stated residual.
        //
        // NARROWED by the dependency's own source signature where the consumer can read it (`depsig`): only the std
        // traits the callee's parameter at that position is bounded by can run, and a concrete parameter, a bound the
        // body never touches, or a constructor runs none. Every doubt there is "any trait", so the narrowing can only
        // withhold a disclosure the signature PROVES inert.
        for ((caller, _alias, _real, l, _pos, _callee), allowed) in witness_sites.iter().zip(&witness_allowed) {
            let empty = BTreeSet::new();
            let have = inferred.get(caller).unwrap_or(&empty);
            for (tr, members) in crate::model::STD_WITNESS_TRAITS {
                if !trait_impls.get(*tr).is_some_and(|v| v.iter().any(|x| x == l)) {
                    continue;
                }
                if allowed.as_ref().is_some_and(|a| !a.contains(*tr)) {
                    continue;
                }
                for m in *members {
                    // R1180 — a macro-made impl's units a hand-written rival keeps out of `by_tail2` count here too.
                    let key = format!("{l}::{m}");
                    let units: Vec<&String> = by_tail2.get(&key).into_iter().flatten().chain(mbe_side.get(&key).into_iter().flatten()).collect();
                    if units.is_empty() {
                        continue;
                    }
                    let hot = units.iter().any(|u| {
                        *u != caller && inferred.get(*u).is_some_and(|e| e.iter().any(|x| *x != "Unknown" && !have.contains(x)))
                    });
                    if !hot {
                        continue;
                    }
                    let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
                    let fresh_why = unknown_why.entry(caller.clone()).or_default().insert(format!("dispatch:{tr}.{m}"));
                    if fresh || fresh_why {
                        hedged = true;
                        if candor_report::env::trace("CANDOR_R1165_INSTR") {
                            eprintln!("R1165HEDGE\t{caller}\t{tr}.{m}\t{l}"); // §E1 REACH COUNTER
                        }
                    }
                }
            }
        }
        // SOUNDNESS R1186 — a local `impl From<S> for U` (or `TryFrom`) handed an `S` through a CHAINED dependency's generic
        // body (`fn f<T: Into<String>>(t: T) { t.into() }`). R1165's check above asks what the argument's OWN type
        // implements, and a conversion's member is keyed under its TARGET (`String::from`), so it never saw this one:
        // chained, the caller read ABSENT over an executed write while unchained it carried `invisible: [dep]` (EXECUTED,
        // rustagent-v063/fx/v063). Same rule, same reason shape: where the conversion's unit carries a CONCRETE effect
        // the caller lacks, the caller discloses `Unknown` with `dispatch:From.from` / `dispatch:TryFrom.try_from`.
        // A DISCLOSURE, never a charge — nothing here proves the body converts, and every same-target impl of the crate
        // shares the one unit (`String::from` is the union of the crate's `From<_> for String` bodies), so even the
        // effect set is not this impl's alone.
        //
        // NARROWED by what the callee can convert the value INTO (`depsig`): a `TryInto`/`TryFrom` bound runs a `From`
        // impl too (std's `TryFrom` blanket goes through `Into`), an `Into`/`From` bound never runs a `TryFrom` one. A
        // dependency cannot name a type THIS crate declares, so a conversion into one runs only where the callee's
        // target is generic; and two DIFFERENT std targets (`Into<String>` vs `impl From<S> for u64`) cannot meet. Every
        // other concrete target is kept, since a dependency's alias can rename the type the consumer wrote.
        for ((caller, _alias, _real, src, _pos, _callee), targets) in from_sites.iter().zip(&from_targets) {
            let empty = BTreeSet::new();
            let have = inferred.get(caller).unwrap_or(&empty);
            // SOUNDNESS R1196 — an UNTYPED argument (`model::FROM_UNTYPED_SRC`) may be the source of any of the crate's
            // conversions. SOUNDNESS R1197(a) — judged against a callee `depsig` could NOT read too ("any"): R1196 withheld
            // that case as 97.6% of untyped sites, priced before R1192 made `depsig` read per item; measured after it, the
            // disclosure costs 558 rows newly `Unknown` of 1,461,057 analysed chained units (0.038%), so an untyped
            // argument to an unreadable callee discloses where an effectful local conversion could meet it. Only a callee
            // `depsig` READ and found no conversion at that position (`Some(empty)`) is skipped.
            let untyped = src == crate::model::FROM_UNTYPED_SRC;
            if untyped
                && (targets.as_ref().is_some_and(|t| t.is_empty())
                    || targets.is_none() && candor_report::env::switch("CANDOR_R1197A_OFF"))
            {
                continue;
            }
            let mut convs: Vec<&(String, String, bool)> = if untyped {
                let mut v: Vec<&(String, String, bool)> = from_impls.values().flatten().collect();
                v.sort();
                v.dedup();
                v
            } else {
                from_impls.get(src).into_iter().flatten().collect()
            };
            if src != "*" && !untyped {
                convs.extend(from_impls.get("*").into_iter().flatten());
            }
            for (tr, tgt, local) in convs {
                let runs = match targets {
                    None => true,
                    Some(ts) => ts.iter().any(|(kind, t)| {
                        (kind == tr || tr == "From" && kind == "TryFrom")
                            && (t == "*"
                                || t == tgt
                                || !*local
                                    && !(crate::model::STD_CONVERSION_TARGETS.contains(&t.as_str())
                                        && crate::model::STD_CONVERSION_TARGETS.contains(&tgt.as_str())))
                    }),
                };
                if !runs {
                    continue;
                }
                let m = if tr == "TryFrom" { "try_from" } else { "from" };
                let key = format!("{tgt}::{m}");
                let units: Vec<&String> = by_tail2.get(&key).into_iter().flatten().chain(mbe_side.get(&key).into_iter().flatten()).collect();
                let hot = units.iter().any(|u| {
                    *u != caller && inferred.get(*u).is_some_and(|e| e.iter().any(|x| *x != "Unknown" && !have.contains(x)))
                });
                if !hot {
                    continue;
                }
                let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
                let fresh_why = unknown_why.entry(caller.clone()).or_default().insert(format!("dispatch:{tr}.{m}"));
                if fresh || fresh_why {
                    hedged = true;
                    if candor_report::env::trace("CANDOR_R1186_INSTR") {
                        eprintln!("R1186HEDGE\t{caller}\t{tr}.{m}\t{src}\t{tgt}\t{fresh}"); // §E1 REACH COUNTER
                    }
                }
            }
        }
        // SOUNDNESS R1540 — a std-typed receiver's method that std also has, beside a crate impl of that name for that std
        // type: std's inherent method wins where that type has one, the crate's impl runs where it does not, and the
        // std-method table does not say which type has it. Where a candidate carries a CONCRETE effect the caller lacks,
        // the caller discloses `ambiguous:` (two definitions, no way to say which runs). The R1095 rule: an
        // `Unknown`-only candidate is not hedged on.
        for (caller, label, cands) in &stdrecv_contested {
            let empty = BTreeSet::new();
            let have = inferred.get(caller).unwrap_or(&empty);
            let impure = cands.iter().any(|u| {
                inferred.get(u).is_some_and(|e| e.iter().any(|x| *x != "Unknown" && !have.contains(x)))
            });
            if !impure {
                continue;
            }
            let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
            let fresh_why = unknown_why
                .entry(caller.clone())
                .or_default()
                .insert(format!("ambiguous:std method or crate impl on a std receiver ({})", label.replace('.', "_")));
            if fresh || fresh_why {
                hedged = true;
                if candor_report::env::trace("CANDOR_R1540_INSTR") {
                    eprintln!("R1540HEDGE\t{caller}\t{label}"); // §E1 REACH COUNTER
                }
            }
        }
        for (caller, label, cands) in &primop_contested {
            let empty = BTreeSet::new();
            let have = inferred.get(caller).unwrap_or(&empty);
            let impure = cands.iter().any(|u| {
                inferred.get(u).is_some_and(|e| e.iter().any(|x| *x != "Unknown" && !have.contains(x)))
            });
            if !impure {
                continue;
            }
            let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
            let fresh_why = unknown_why
                .entry(caller.clone())
                .or_default()
                .insert(format!("ambiguous:operator on a primitive with a macro-made impl ({})", label.replace('.', "_")));
            if fresh || fresh_why {
                hedged = true;
                if candor_report::env::trace("CANDOR_V053_INSTR") {
                    eprintln!("R1157HEDGE\t{caller}\t{label}"); // §E1 REACH COUNTER
                }
            }
        }
        for (caller, claimants) in &veina_contested {
            let empty = BTreeSet::new();
            let have = inferred.get(caller).unwrap_or(&empty);
            // SOUNDNESS R1131 (v051 audit) — `Unknown` is not one fact but one per REASON CLASS, and a reason-scoped
            // gate (`deny Unknown[ambiguous]`) reads the class. A caller already `Unknown` for ANOTHER reason (a
            // `macro:` hidden by-value impl, curve25519 `coset4`) still loses this contest's true `ambiguous:`
            // hit unless a claimant's `Unknown` counts as missing while the caller lacks that class. So it does:
            // the two reasons are UNIONED, never one replacing the other. (`Unknown` itself is unchanged; only
            // the reason list grows.)
            let has_ambig = unknown_why.get(caller).is_some_and(|w| w.contains("ambiguous:same-name local defs"));
            let missing = claimants.iter().any(|c| {
                c != caller
                    && inferred.get(c).is_some_and(|ce| {
                        ce.iter().any(|e| !have.contains(e) || (*e == "Unknown" && !has_ambig))
                    })
            });
            if missing && !direct.get(caller).is_some_and(|d| d.contains("Unknown"))
                || missing && !unknown_why.get(caller).is_some_and(|w| w.contains("ambiguous:same-name local defs"))
            {
                let fresh = direct.entry(caller.clone()).or_default().insert("Unknown");
                let fresh_why = unknown_why
                    .entry(caller.clone())
                    .or_default()
                    .insert("ambiguous:same-name local defs".to_string());
                if fresh || fresh_why {
                    hedged = true;
                    if candor_report::env::trace("CANDOR_R186_DEBUG") {
                        eprintln!("VEINAHEDGE\t{caller}"); // §E1 REACH COUNTER
                    }
                }
            }
        }
        if !hedged {
            break;
        }
        inferred = propagate(&direct, &calls, &all);
    }
    let hostsacc = propagate_str(&hosts, &calls, &all);
    // `fs` kinds propagate like the literal surfaces; the "?" poison propagates WITH them, which is what
    // makes a caller of an undetermined-kind function inherit the suppression rather than a half-answer.
    let fskindsacc = propagate_str(&fskinds, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);
    let tablesacc = propagate_str(&tables, &calls, &all);
    let incompleteacc = propagate(&incomplete, &calls, &all); // transitive masking-incompleteness
    let blind_acc = propagate_str(&blind_direct, &calls, &all); // transitive per-fn blind reach
    // SOUNDNESS R503 — ONE SPELLING FOR ONE ABSTRACTION, on the wire.
    //
    // SPEC §4 ⟨0.39⟩ pins BOTH obligation 2's key and `dispatchesOn`'s value to the ⟨0.23⟩ rule: fully
    // qualified in the OWNING package's namespace, the namespace that package's entry hashes use — and it
    // names `ratatui_core#Backend::size` explicitly as the under-qualified spelling it forbids. This
    // engine had produced BOTH: the FOREIGN union entry is keyed `{owner}#{backend::Backend}::{size}`
    // (from the implementing file's `use` map) while the LOCAL union entry and every `dispatchesOn` value
    // were keyed by trait LEAF, because every trait index in this engine is leaf-keyed. So one member had
    // two names in one report and a consumer could join only one of them without a per-engine rule.
    //
    // `trait_quals` (R503, Pass A) is the missing source. A leaf declared TWICE carries two quals and this
    // returns `None`: the ambiguous leaf is refused, exactly as `LocalTrait::count > 1` refuses it for CHA
    // — never a guess between two traits. A leaf with NO recorded qual (a trait this crate does not
    // declare, reached through a leaf-keyed index) also returns `None` rather than inventing an owner.
    /// SOUNDNESS R529 — `"{owner}#{trait qual}::{method}"` -> §4's normative dotted
    /// `dispatch:<owner>.<member>` detail. One spelling, one place: the three sites that hedge a
    /// block-nested foreign implementor must not each form it (SPEC §4 ⟨0.24⟩'s "an engine holds this
    /// vocabulary twice" is the failure this avoids, and R490 is this family's own instance of it).
    /// ⟨0.40⟩ SOUNDNESS R608 — `"{owner}#{trait qual}::{method}"` -> `"{owner}#{trait qual}::"`, i.e.
    /// the ABSTRACTION the member belongs to. `None` for a key with no `::`, because no trait qual can
    /// be formed from it and a comparison against the projection would be a comparison against nothing.
    fn r533_trait_prefix(key: &str) -> Option<String> {
        key.rsplit_once("::").map(|(head, _)| format!("{head}::"))
    }

    /// ⟨0.40⟩ SOUNDNESS R608 — SPEC §4's conventionally-pure leaf set, in rust spelling. java's
    /// `isObjectProtocolExempt` exempts four Object-protocol members — `toString()`, `hashCode()`,
    /// `equals(Object)`, `compareTo(Object)`. Rust spells those same four facts across more trait
    /// methods: `Debug`/`Display` both declare `fmt`; `PartialEq` declares `eq` AND `ne`; `Hash`
    /// declares `hash`; `Ord` declares `cmp` and `PartialOrd` `partial_cmp`; `to_string` is `Display`'s
    /// blanket method and the literal `toString` analogue. `clone` is in the set because THIS ENGINE
    /// ALREADY treats it as conventionally pure at the resolution site (`collector.rs` skips typed
    /// resolution for `leaf == "clone"`), so leaving it out here would be one fact held in two
    /// spellings — the ⟨0.34⟩ drift shape.
    ///
    /// **DELIBERATELY EXCLUDED: java's function-object arm** (`kotlin FunctionN.invoke`,
    /// `scala FunctionN.apply`, `groovy Closure.call`). Rust's `Fn`/`FnMut`/`FnOnce` are not spelled by
    /// member name at a call site, and `call` IS a real, effectful trait method in this ecosystem
    /// (`tower_service#Service::call`) — exempting that leaf would suppress a genuine dispatch, which
    /// is the opposite of what a conventionally-pure set is for. Keyed on the LEAF alone, mirroring
    /// java, whose test is `(name, desc)` and never the owner.
    ///
    /// **MEASURED INERT ON THE PRICING CORPUS — it is not credited with any reduction.** With the
    /// trait-prefix conjunct in place, the six chained pairs' reports are BYTE-IDENTICAL with and
    /// without this skip (compared file by file, not by a diffstat). It was then calibrated on the arm
    /// where it CAN fire, because a skip that has never removed anything has not been shown to be a
    /// skip: on the member-keyed conjunct it removes exactly **7** hits, every one
    /// `tower_service#Service::clone` (41 -> 34). So this is a guard against a shape the corpus did
    /// not contain, not a cost reducer — crediting it with any part of the 39 -> 6 would be crediting
    /// it with the trait-prefix conjunct's work.
    ///
    /// **SOUNDNESS R630 — NOW KEYED ON THE (TRAIT, MEMBER) PAIR, NOT THE LEAF.** java's test is
    /// `(name, desc)` on an `INVOKEINTERFACE` of `java.lang.Object`'s protocol, and those names are
    /// RESERVED there. In rust none of these leaves is: any trait may declare `hash`, `fmt`, `cmp`,
    /// `eq` or `to_string` as an ordinary, effectful requirement. Keyed on the leaf alone, a
    /// dependency's `trait Digest { fn hash(&self) -> String; }` dispatched through `&dyn dep::Digest`
    /// read PURE with no implementor anywhere (`deny Net Unknown` exit 0), and the R693 sibling —
    /// `<T: dep::Shower>` over this crate's own `fmt` that opens a socket — read pure too; a test
    /// pinned that as the expected answer. The member's key carries its trait qual
    /// (`{owner}#{trait qual}::{method}`), so the exemption now asks for the protocol TRAIT as well:
    /// `Debug::fmt` is the conventional member, `Digest::hash` is not. A DENYLIST of known-conventional
    /// abstractions in place of an allowlist of bare names — the direction
    /// [[candor-denylist-over-allowlist]] requires for narrowing a sound over-approximation, because an
    /// unrecognised pair now keeps the hedge.
    ///
    /// THE RESIDUAL, STATED: the trait is matched by its LEAF, because a dependency that re-exports
    /// `core::fmt::Debug` is keyed `dep#Debug::fmt` and nothing at this site can see the re-export. So a
    /// dependency's OWN trait that is literally named `Hash` and declares an effectful `hash` is still
    /// exempted. Measured cost of the narrowing is in the commit; the previous wording's "measured
    /// inert" applies to the pricing corpus, which is also what makes this cheap.
    fn r533_exempt_leaf(mem: &str) -> bool {
        let mut it = mem.rsplit("::");
        let leaf = it.next().unwrap_or(mem);
        let tr = it.next().unwrap_or("");
        match tr {
            "Debug" | "Display" | "LowerHex" | "UpperHex" | "Octal" | "Binary" | "Pointer" | "LowerExp"
            | "UpperExp" => leaf == "fmt",
            "PartialEq" => matches!(leaf, "eq" | "ne"),
            "Hash" => leaf == "hash",
            "Ord" => leaf == "cmp",
            "PartialOrd" => leaf == "partial_cmp",
            "Clone" => leaf == "clone",
            "ToString" => leaf == "to_string",
            _ => false,
        }
    }

    fn r529_reason(member: &str) -> String {
        let detail = member.split_once('#').map(|(_, m)| m).unwrap_or(member);
        match detail.rsplit_once("::") {
            Some((owner, meth)) => format!("dispatch:{owner}.{meth}"),
            // A member with no `::` cannot form the NORMATIVE dotted detail PART 10 requires, so it does
            // not claim the kind: `ambiguous:` is what §4 names for "no owner could be formed at all".
            None => format!("ambiguous:block-nested implementor of `{detail}`"),
        }
    }

    /// The ⟨0.23⟩ DECLARATION QUAL of an unambiguous local trait leaf — `Backend` declared in
    /// `pub mod backend` is `backend::Backend`. ONE AUTHORITY, because R652's evidence test compares
    /// an impl's expanded trait path against exactly this string and `union_member_key` mints the wire
    /// key from it: two spellings of "where is this trait declared" is the ⟨0.34⟩ drift again.
    fn union_trait_qual<'q>(
        trait_quals: &'q HashMap<String, std::collections::BTreeSet<String>>,
        trait_leaf: &str,
    ) -> Option<&'q String> {
        let quals = trait_quals.get(trait_leaf)?;
        let mut it = quals.iter();
        let one = it.next()?;
        if it.next().is_some() {
            return None; // two declarations share this leaf — refuse, never pick
        }
        Some(one)
    }

    fn union_member_key(
        trait_quals: &HashMap<String, std::collections::BTreeSet<String>>,
        crate_name: &str,
        trait_leaf: &str,
        method: &str,
    ) -> Option<String> {
        let one = union_trait_qual(trait_quals, trait_leaf)?;
        // REACH PROBE — same channel and same reason as `R504HIT`. This one fires wherever the wire
        // spelling is DECIDED, which is the branch R503 changed.
        if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
            eprintln!("R503HIT {one}");
        }
        Some(format!("{crate_name}#{one}::{method}"))
    }

    /// SOUNDNESS R597/R598 — DOES THE SUPPRESSED UNION KNOW ANYTHING THE ROW ALREADY AT `hash` DOES NOT?
    ///
    /// A union entry used to be emitted *only* where no real row claimed its hash, so the union was
    /// DISCARDED wherever a body row existed. Two different facts want that one key and neither contains
    /// the other, which is why the discard loses:
    ///
    ///   · the trait member has a DEFAULT BODY, so `crate#Trait::m` is already an analysed unit — the
    ///     default body — while this union is over the IMPLEMENTORS' OVERRIDES (R597);
    ///   · or an implementor TYPE's leaf equals the trait's, so `impl Child for std::process::Child`
    ///     publishes `pty#Child::wait` and collides with the trait member key of the same name (R598).
    ///
    /// Returns `(effects_add, invisible_add)`. `Unknown` on the existing row COVERS any effect, so it is
    /// not a loss; `invisible` is checked separately because §2 makes `inferred: []` WITH a non-empty
    /// `invisible` explicitly not a purity claim, and without it the two channels mask each other.
    ///
    /// **BOTH ELEMENTS DRIVE EMISSION.** The second one used to be measured and gated off pending R598
    /// (a beside-union row carries its effects too, and R598 could put a FABRICATED one there); R598 is
    /// closed, so the coverage leg emits — see the block at the local call site.
    fn union_adds_over_row(
        entries: &[ReportEntry],
        hash: &str,
        inf_u: &std::collections::BTreeSet<&'static str>,
        blind_u: &std::collections::BTreeSet<String>,
    ) -> (bool, bool) {
        // No row at this hash cannot happen — `existing` is built from `entries` and `entries` only
        // grows — but the safe answer if it ever did is EMIT, never withhold.
        let Some(real) = entries.iter().find(|e| e.hash == hash) else { return (true, true) };
        let covered = real.inferred.iter().any(|e| e == "Unknown");
        (
            !covered && inf_u.iter().any(|e| !real.inferred.iter().any(|r| r == e)),
            blind_u.iter().any(|c| !real.invisible.iter().any(|r| r == c)),
        )
    }

    // ⟨0.39⟩ SPEC §4 obligation 1 — `dispatchesOn`, TRANSITIVELY. The raw material is `FnInfo::dispatch`
    // (`CallCollector::dispatch_sites`), which records the (trait leaf, method leaf) pairs a body dispatches
    // on through a LOCAL bounded-CHA-eligible receiver, regardless of how many implementors are visible.
    // That field was built for the ⟨0.29⟩ peek and is explicitly NOT part of the wire schema; this rung is
    // the second reader, and it needs the same fact one step further on — the member a CALLER reaches
    // through a dispatching callee, since in the measured instance (R475) the consumer calls
    // `iface::term_size` and never spells the dispatch itself.
    //
    // R503 — THE VALUE IS A WHOLE ENTRY KEY (`iface#backend::Backend::size`), not a suffix a consumer
    // prefixes. This paragraph said the opposite until R503, and the opposite was wrong twice over: the
    // leaf is the under-qualified spelling §4 ⟨0.39⟩ explicitly forbids, and prefixing the ROW's own
    // package is only correct while the dispatching package is also the OWNING one — which R504 is
    // precisely the case where it is not. Propagated with the same fixpoint every other transitive fact
    // uses.
    let dispatch_direct: HashMap<String, BTreeSet<String>> = {
        let mut m: HashMap<String, BTreeSet<String>> = HashMap::new();
        for f in &fns {
            for (tr, meth) in &f.dispatch {
                if let Some(member) = union_member_key(&merged.trait_quals, &crate_name, tr, meth) {
                    m.entry(f.qual.clone()).or_default().insert(member);
                }
            }
            // R504 — a FOREIGN abstraction's member is already in wire form and already keyed under the
            // crate that OWNS it, which is the whole point: the dispatching crate is not the owner, so
            // no amount of prefixing here could produce the right key.
            //
            // PROVENANCE IS CHECKED AGAINST THE MANIFEST HERE, exactly as obligation 2's key is, and for
            // the same reason: the collector's gate is `is_dependency_crate_root`, which admits anything
            // that is not std/`crate`/`self`/`super` — a LOCAL module spelled like a crate passes it, and
            // so does a `use std::io::{self, Write}` whose `io::Write` never expands. Measured on the
            // 1,599-crate corpus BEFORE this filter existed: nine rows published `io#Write::write_all`,
            // naming a package that does not exist. `deps` is Cargo.toml's real dependency set and is
            // only known here. A key no consumer could ever join is not harmless — it is a name collision
            // waiting for a real crate called `io`.
            for member in &f.foreign_dispatch {
                if member.split_once('#').is_some_and(|(owner, _)| deps.contains(owner)) {
                    m.entry(f.qual.clone()).or_default().insert(member.clone());
                }
            }
        }
        m
    };
    // SOUNDNESS R1420(a) — A DEFAULT'S INSTANCE NARROWS EFFECTS, NEVER JOIN KEYS. The instance keeps the `dispatchesOn` keys
    // its default's union reached: a key is what lets a chained consumer join an implementor this scan cannot see, and a key
    // that arrived only through a sibling implementor may still be the one the true path needs where that path forms no
    // edge of its own (EXECUTED on the census A/B: zerovec `ZeroMap2d::check_invariants` lost `VarULE::from_bytes_unchecked`,
    // which its generic `self.keys0.zvl_is_ascending()` really reaches but never edges). So for this propagation only, each
    // instance also points at the default it instantiates. Over-disclosure at a concrete caller, exactly as before.
    let dispatch_acc = {
        let mut calls_keys = calls.clone();
        for f in fns.iter() {
            if let Some(d) = &f.inst_of {
                calls_keys.entry(f.qual.clone()).or_default().insert(d.clone());
            }
        }
        propagate_str(&dispatch_direct, &calls_keys, &all)
    };
    // Reason-scoped Unknown (REASON-SCOPED-UNKNOWN-DESIGN.md): the Unknown reason CLASS must travel the
    // call graph the same way the Unknown EFFECT does, so `deny E Unknown[reflect]` at a caller inheriting
    // Unknown from a reflect-caused callee still fires. Classify each fn's DIRECT unknown_why tokens to
    // class tokens, then propagate transitively (mirrors the java gate's reasonClassAcc). Report unchanged.
    let mut reason_class_direct: HashMap<String, BTreeSet<String>> = unknown_why
        .iter()
        .map(|(f, whys)| {
            let classes = whys
                .iter()
                .map(|w| candor_classify::policy::ReasonClass::classify(w).token().to_string())
                .collect();
            (f.clone(), classes)
        })
        .collect();
    // …AND THE REASONLESS CHAINED `Unknown` CONTRIBUTES ITS CLASS HERE, rather than being inferred from
    // the ABSENCE of one. `f2309a5` correctly stopped inventing a `callback:` reason for a dep that
    // declared `Unknown` and recorded no reason — §6.2 already answers that case ("a function whose
    // `Unknown` carries no recorded reason is treated as `unresolved`") and §4's closed kind vocabulary
    // has no member that projects there. But it left `unknown_via_dep` feeding only the writer's
    // `debug_assert`, so the class reached the gate ONLY through the fallback below, which is per
    // FUNCTION and fires when the whole class set is absent or empty. **Any other reason on the same
    // function swallowed it**: `both() { dep::murky(); dep::mute(); }` — one dep Unknown carrying
    // `dispatch:` and one carrying nothing — classified `dispatch` alone, and
    // `deny E Unknown[unresolved]` went from exit 1 to exit 0 as the SECOND call was added. So the
    // by-absence fallback worked everywhere except where two reasons meet on one function, which is
    // where a gate needs it. Adding a reason must never REMOVE a class.
    //
    // NO TOKEN IS INVENTED: this writes a §6.2 CLASS (`unresolved`, a member of that section's own
    // closed set), never a §4 kind, and it is confined to this gate-side map — `unknown_why`, and with
    // it the report, is untouched, so the ⟨0.7⟩ field still carries only reasons somebody observed.
    // The RESIDUAL that leaves is the format's: a report cannot say "Unknown, no reason" alongside a
    // reason it does have, so a SECOND-hop consumer chaining this report re-derives `dispatch` alone.
    // That needs a §4/§6.2 rung (see the work queue) and is not patchable with a string here.
    //
    // ⟨0.24⟩ THE RESIDUAL IS NOW MEASURED, AND IT IS A LIVE GATE-vs-DISCLOSURE SPLIT — the exact thing
    // §6.2 forbids ("THE GATE AND THE DISCLOSURE MUST APPLY THE SAME RULE"). pgman over a 270-report dep
    // tree, every report marked §2.1 stale: 36 functions reach this contribution and **12 of them also
    // carry a reason of their own**, which grows to **18 of the 77** functions whose gate-side class set
    // includes `unresolved`. Recomputing that set from the REPORT — which is what `unverified --class`
    // and `blindspots --class` do — recovers only 59: the 18 come back classed `dispatch` alone, so the
    // gate bites them and the disclosure standing beside it does not name them. With TRUSTED reports the
    // split is 0, on both targets measured; it is reachable only through reports the build cannot verify.
    //
    // ⟨0.40⟩ CLOSED (SOUNDNESS R1243): `load_dep_reports` now attaches `dep:<hash>` / `dep-stale:<pkg>` per ENTRY
    // when the dependency classified nothing, so the reason reaches `unknown_why` — and the REPORT — through
    // `apply_dep_fn` like any other, and `gate --report` recomputes the same class set. The loop below stays
    // as a safety net for a reasonless `DepFn` built anywhere else. History follows.
    // WHAT WOULD CLOSE IT is exactly what §4 ⟨0.24⟩ registered for the purpose — `dep-stale:<pkg>` (and
    // `dep:<hash>`), permanent kinds now, attached per dependency ENTRY the way candor-swift attaches
    // them, so the reason travels IN the report and the two recomputations cannot disagree. It is not
    // taken here for one reason and it is not a spec one: the SHIPPED conformance PART 10 still holds a
    // four-kind `CANON` plus two named migration kinds, and `dep-stale` is in neither, so emitting it
    // scores a hard DIVERGE today. The rung landed in SPEC and not yet in the harness that checks it.
    for q in &unknown_via_dep {
        reason_class_direct
            .entry(q.clone())
            .or_default()
            .insert(candor_classify::policy::ReasonClass::Unresolved.token().to_string());
    }
    let reason_class_acc = propagate_str(&reason_class_direct, &calls, &all);
    // The genuinely-blind dep crates (the per-scan κ "unlisted" set): seen, never classified, not
    // dep-report-covered, not calibrated. A fn's `invisible` = its transitive blind reach ∩ this set.
    // ⟨0.15 staged⟩ Computed ONCE, with the call counts, as the κ-coverage LEDGER: the same list (same
    // names, same counts, same order) feeds the envelope's `coverage` field (spec §2), the stderr
    // disclosure below, and the --gate-json advisory note — three surfaces that can never disagree
    // because they share this one computation. Sorted by call count (desc) then name, the stderr
    // line's long-standing order. (A crate with a loaded sibling report is COVERED even when no join
    // fired: the report omits pure functions, so join-less calls are its honest purity claim — the
    // opposite of invisible. A RENAMED dep is covered under its real package name.)
    // This filter deliberately does NOT consult a "was ever classified" set. That was an UNVOUCHED
    // proxy standing in front of the reviewed ones: one classified call cleared the blind marker for
    // every other call shape into the same crate, so adding an unrelated call elsewhere in the program
    // silently converted a disclosed blind spot into a purity claim. `blind_direct` already holds the
    // per-call-site datum and it already propagates, so the per-fn truth was present all along.
    // AN UNTRUSTED REPORT MUST NOT GRANT COVERAGE. §2.1 downgrades a stale report's EFFECTS to
    // `Unknown` — and the exemption below would, on the authority of that same refused report, turn
    // every function it does not mention into a confident purity claim (§2 rule 3 makes an absent entry
    // one) with the `invisible` disclosure dropped. The join still fires (it is keyed on `crates`, so
    // the entries that ARE there still charge `Unknown`); only the claim that the report's SILENCE is
    // informative is withdrawn. candor-ts `651c9f9` is the same defect one repo over.
    //
    // ⟨0.21⟩ NEITHER DOES A REPORT THAT DECLARES ITSELF INCOMPLETE — the same door with a different key,
    // read one step earlier: staleness asks whether to believe what a report SAYS, completeness asks
    // whether its SILENCE means anything, and a report naming source it could not analyze has answered
    // that. Its ENTRIES are untouched (they came from source it did read), so the change is strictly
    // additive: an answered key still answers, an unanswered one falls back to the hedge. See
    // `DepIndex::incomplete_pkgs`. THIS IS THE ONLY PLACE COVERAGE IS CONSUMED, so one conjunct here is
    // the whole gate. candor-java's sibling needed two, because coverage was anchored twice there and
    // gating one was a no-op wearing a fix's clothes; the anchor count is a per-engine fact and this one
    // is 1. NOT "read nowhere else" — `dbab8be` said that and it is wrong: `load_dep_reports` reads both
    // sets again for its two stderr disclosures. CONSUMED nowhere else is the claim, and it is no longer
    // just a claim: `coverage_has_exactly_one_anchor_and_exactly_one_consumer` enumerates the writes and
    // the consumers out of the source and fails on a second of either (four mutants, four named rows).
    //
    // ⟨0.24⟩ AND NEITHER DOES A REPORT THAT JUDGED NOTHING — `analyzed.count: 0`, the THIRD answer to
    // "may this report's silence speak?" and the last one the wire can currently express. Same treatment
    // as incompleteness (chained, not covered, entries untouched), and the same single conjunct, because
    // this ledger is still the only place coverage is consumed. See `DepIndex::judged_nothing_pkgs`.
    // Cargo.lock identity check for the CALIBRATED_* exemptions below: a name CONFIRMED path/git/
    // workspace-sourced (not the real registry crate `classify`'s rules were reviewed against) never
    // gets the exemption, regardless of which of the three lists it happens to string-match — see
    // `non_registry_lock_names`'s doc for the reproduced silent-drop this closes.
    //
    // UNION with the manifest-based check: Cargo.lock is routinely absent from a library tree — an
    // adversarial review found that with no lockfile, `non_registry_lock_names` alone returns empty, so
    // a `path`/`git` impostor sharing a CALIBRATED_CRATES name reverted to the pre-⟨caca530⟩ silent
    // exemption. Cargo.toml is never absent, and a name squatting a calibrated crate almost always
    // declares its own `path =`/`git =` right there — see `non_registry_manifest_names`'s doc for the
    // shape this covers and the narrower residual (a `[patch]` table, or `.cargo/config.toml` source
    // replacement) it does not.
    // (`non_registry_names` is computed above the call loop — SOUNDNESS R1578's floor asks the same coverage question there.)
    // SOUNDNESS R311 — which VERSION of each registry dependency is actually present. The calibration
    // exemption below converts "no rule matched" into "reviewed, and pure"; that is evidence about
    // source someone READ, so it must not be granted to a release published after the reading. No
    // Cargo.lock (a library tree) leaves this empty, and an absent version keeps today's exemption —
    // stated rather than hidden, because the alternative is disclosing on every lockless scan.
    // …and a WORKSPACE MEMBER has no `Cargo.lock` of its own — the root holds it. Falling back to the
    // root is the difference between a ceiling that applies to a member crate and one that silently does
    // not; `find_workspace_root` already exists for exactly this and was simply not consulted here. The
    // previous comment stated the lockless case as an accepted residual, which is the
    // documented-limitation-is-not-measured shape: it read as considered, and that is what stopped it
    // being fixed. Failing to find a version keeps today's exemption, so this can only ADD containment.
    // (`lock_versions` is computed above the call loop — SOUNDNESS R1578's floor asks the same coverage question there.)
    let mut coverage_ledger: Vec<(String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| kappa_blind(cr))
        .map(|(cr, n)| (cr.clone(), *n))
        .collect();
    coverage_ledger.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let global_blind: std::collections::HashSet<String> =
        coverage_ledger.iter().map(|(cr, _)| cr.clone()).collect();
    // A dep-inherited invisible crate is genuinely blind (the dep's own scan confirmed it) but transitive,
    // so it never appears in `dep_seen` — keep it so the consumer's `invisible` survives the filter ([8]).
    let global_blind: std::collections::HashSet<String> =
        global_blind.into_iter().chain(dep_invisible).collect();

    // ⟨0.20⟩ Net destination-class (NET-DESTINATION-CLASS-DESIGN.md): the config `net-partner` hosts, read
    // here (before the entries) so the report's per-fn `netClass` carries known-partner — the SAME set the
    // gate resolves from `.candor/config`. Empty when no config declares partners (telemetry-only asserts).
    // ⟨0.31⟩ the declared partners that actually MOVED a classification in this run — see the envelope key.
    let mut partners_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let net_partners = candor_classify::policy::discover_config_text(std::path::Path::new(dir))
        .map(|t| candor_classify::policy::parse_net_partners(&t))
        .unwrap_or_default();

    let mut entries: Vec<ReportEntry> = Vec::new();
    let mut cg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // SOUNDNESS R1420(a) — a default's INSTANCE is not a unit of the report: it is spelled as the default it instantiates.
    let inst_name: HashMap<&str, &str> =
        fns.iter().filter_map(|f| f.inst_of.as_deref().map(|d| (f.qual.as_str(), d))).collect();
    // SOUNDNESS R1650 — §3.1 ROUTE EQUALITY: `scan --policy` must gate exactly the units the report WRITES. `all` still holds
    // the instances (they are analysed, and a caller's effects flow through them), but a policy, a baseline ratchet and the
    // unverified-holes note are evaluated over `all_pub`: an instance is no unit of the report, so a violation row naming
    // one carried a hash no report contains and made `gate --report` disagree with the scan that wrote it. Dropping it loses
    // nothing: the instance keeps a SUBSET of its default's edges (`calls_i` is filtered from `def.calls`) and its name only
    // extends the default's last segment, so every effect rule that fires on the instance also fires on the default (executed:
    // soundness/ fixtures `r1650_*`). The debug assertion below states that subset so a change that breaks it fails loudly.
    // SOUNDNESS R1537 (c) — nor is a co-path set's bare union node: its members are.
    let all_pub: Vec<String> =
        all.iter().filter(|q| !inst_name.contains_key(q.as_str()) && !copath.contains_key(q.as_str())).cloned().collect();
    debug_assert!(
        fns.iter().filter_map(|f| f.inst_of.as_deref().map(|d| (f.qual.as_str(), d))).all(|(i, d)| {
            let (a, b) = (inferred.get(i), inferred.get(d));
            a.is_none_or(|a| b.is_some_and(|b| a.is_subset(b)))
        }),
        "R1650: a default-instance unit performs an effect its default does not — dropping it from the gate would under-report"
    );
    let pub_calls = |q: &str| -> Vec<String> {
        // SOUNDNESS R1537 (c) — an edge to a co-path set's bare union node is published as an edge to EVERY member.
        let mut v: Vec<String> = calls
            .get(q)
            .map(|cs| {
                cs.iter()
                    .flat_map(|c| match copath.get(c.as_str()) {
                        Some(cp) => vec![cp.inh.clone(), cp.tr.clone()],
                        None => vec![inst_name.get(c.as_str()).map_or(c.clone(), |d| d.to_string())],
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v.dedup();
        v
    };
    for q in &all {
        if inst_name.contains_key(q.as_str()) || copath.contains_key(q.as_str()) {
            continue;
        }
        // SPEC §2.2: the sidecar records EVERY analyzed function — including a LEAF with no local
        // callees, as an empty list. Omitting leaves made an uncalled FFI-only fn (nix `unistd::pipe`)
        // invisible to `whatif`/`callers` ("no function matching") even though it's in the report;
        // an always-present key also lets a consumer distinguish "no callers" from "no such function".
        cg.insert(q.clone(), pub_calls(q));
        let inf = inferred.get(q).cloned().unwrap_or_default();
        // Keep a pure fn if it has a BLIND reach — so the honesty disclosure survives on exactly the
        // `inferred: []` fns that need it (else `invisible` would be dropped with the pure entry).
        let has_blind = blind_acc.get(q).is_some_and(|s| s.iter().any(|c| global_blind.contains(c)));
        // ⟨0.39⟩ …AND KEEP A PURE FN THAT DISPATCHES. §2 rule 3 omits pure functions and §2 chaining rule 3
        // makes that absence a POSITIVE purity claim — which is the whole defect: a dispatching function
        // whose only visible implementor happens to be pure vanished from the report, so ADDING A PURE
        // IMPLEMENTATION TO A LIBRARY DELETED A DISCLOSURE FROM EVERY CONSUMER OF IT. §4 ⟨0.39⟩ makes this
        // one deliberate exception: absence keeps its meaning, but a dispatching row is no longer absent.
        let dispatches: Vec<String> =
            dispatch_acc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        if inf.is_empty() && !has_blind && dispatches.is_empty() {
            continue;
        }
        // THE MARKER MUST TRAVEL WITH THE THING IT DESCRIBES. SPEC §4: `unknownWhy` is REQUIRED when a fn
        // introduces `Unknown` DIRECTLY (a source) and absent when the `Unknown` is purely inherited — so
        // the invariant is exactly "`direct` carries Unknown ⇒ `unknownWhy` is non-empty", checked at the
        // one place every entry is built rather than argued site by site. It is a `debug_assert`: it holds
        // in every test and over 21 980 entries of real corpus, and a marker gap must never take down a
        // user's scan in release — the release-mode consequence of a gap is a tolerated Unknown, not a
        // crash. Reason: the chained-dep join used to write `Unknown` into `direct` with no reason at all,
        // and nothing in the writer objected.
        //
        // `unknown_via_dep` IS THE ONE EXEMPTION, and it is §4's own definition rather than a hole punched
        // in the rule. §4 defines a source as a unit "whose own body has the unresolvable call"; a
        // consumer of a chained dependency has no such call — its body calls a known function whose
        // REPORT says Unknown, so its Unknown is INHERITED, across a report boundary instead of a
        // call-graph edge. The join writes into `direct` only because the callee is not a unit in this
        // report, which is an implementation fact and not a claim about whose body holds the hole. The
        // invariant as written forced the boundary case to name one of the four §4 kinds, none of which
        // projects to `unresolved` (§6.2) — so honouring it produced a FABRICATED class, which is how
        // `callback:chained dependency declared Unknown without a reason` came to exist. Every in-scan
        // path is still held to the rule, which is the class of gap this assertion was written to catch.
        debug_assert!(
            !direct.get(q).is_some_and(|d| d.contains("Unknown"))
                || unknown_why.contains_key(q)
                || unknown_via_dep.contains(q),
            "`{q}` introduces Unknown DIRECTLY but carries no `unknownWhy` (SPEC §4). Its reason class is \
             lost, so a `deny E Unknown[class]` gate silently tolerates it — whichever new path put the \
             Unknown into `direct` must charge its reason beside it."
        );
        entries.push(ReportEntry {
            func: q.clone(),
            loc: loc.get(q).cloned().unwrap_or_default(),
            inferred: inf.iter().map(|s| s.to_string()).collect(),
            direct: direct.get(q).map(|d| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default(),
            // ⟨0.26⟩ NONE, not empty. The stable scanner runs no §5 capability-reconciliation pass, and
            // an empty set would CLAIM one ran: `undeclared: []` reads as "this function performs no
            // undeclared effect". Absent means not computed. (The deep lint DOES run the pass and emits
            // `Some(...)` — one type, two honest answers.)
            declared: None,
            undeclared: None,
            overdeclared: None,
            // Honest blind-spot signal: this function (transitively) reached a callable the scan couldn't
            // see through. Mirrors the lint's `unresolved = has Unknown`, so the receipt's unresolved
            // count is truthful for the stable backend too — not a hardcoded 0.
            unresolved: inf.contains("Unknown"),
            // The cross-crate join key (spec §2): `crate#qual`, derivable by any consumer from its
            // own syntactic view of the call — what CANDOR_DEPS chaining matches against.
            hash: format!("{crate_name}#{q}"),
            // SPEC §2 `fs`: the read/write kinds this fn's OWN Fs calls revealed. Was hardcoded empty —
            // the field existed in the wire model and nothing ever wrote to it, which is worse than
            // absent because the struct implied support.
            // SPEC §2 `fs`: the read/write kinds reachable from this fn. Kinds TRAVEL the call graph —
            // a caller that transitively only writes IS a writer — but a partial answer must not: if any
            // contributing Fs had no determined kind, the "?" poison is present and the WHOLE field is
            // suppressed. Matches candor-java's FS_UNKNOWN discipline; pinned by conformance PART 31.
            fs: fskindsacc.get(q)
                .filter(|s| !s.contains("?"))
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default(),
            hosts: hostsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            cmds: cmdsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            paths: pathsacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            tables: tablesacc.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            calls: pub_calls(q),
            // DIRECTLY-introduced Unknown origins (candor-spec §2 `unknownWhy`): an unresolved callback /
            // fn-pointer call, an FFI/extern boundary, or a genuinely-unresolvable bare call. Coarser than
            // the lint's per-trait tag — by design — but now names WHICH boundary, not just "callback".
            unknown_why: unknown_why.get(q).map(|s| s.iter().cloned().collect()).unwrap_or_default(),
            // candor-spec §2 `entryPoint`: syntactically we can only spot `main` (the program root). The
            // lint also flags `#[no_mangle]`; the scanner can't see attributes, so it under-marks — the
            // sound direction for an optional reachability hint.
            entry_point: q.rsplit("::").next() == Some("main"),
            // Per-fn honesty: the genuinely-blind crates this fn transitively reaches. `inferred` is a
            // LOWER BOUND when this is non-empty.
            invisible: blind_acc
                .get(q)
                .map(|s| s.iter().filter(|c| global_blind.contains(*c)).cloned().collect())
                .unwrap_or_default(),
            // Masking-incomplete effects — carried so a CANDOR_DEPS consumer inherits the incompleteness
            // across the crate boundary (sweep [30]); the gate already fails closed locally on it.
            incomplete: incompleteacc.get(q).map(|s| s.iter().map(|e| e.to_string()).collect()).unwrap_or_default(),
            // ⟨0.20⟩ Net destination-class: the classes present in this fn's transitive Net surface. Exact
            // host-literal match for the visible hosts; fail-closed unknown-host when the Net surface is masked
            // (`incomplete` has Net) OR carries no visible host (a runtime endpoint). Empty when no Net.
            net_class: if inf.contains("Net") {
                // ⟨0.31⟩ record WHICH declared partner participated, at the point the class is decided.
                // `partner_for` is the function `net_dest_class` itself asks, so the disclosure and the
                // decision cannot use different rules — the reverted first attempt re-matched against a
                // normaliser that keeps the port and came back silently empty on every real run.
                for h in hostsacc.get(q).into_iter().flatten() {
                    if let Some(p) = candor_classify::partner_for(h, &net_partners) {
                        partners_used.insert(p);
                    }
                }
                crate::gate::net_classes_of(q, &hostsacc, &incompleteacc, &net_partners)
            } else {
                Vec::new()
            },
            interface_union: false,
            declaration: decl_quals.contains(q),
            dispatches_on: dispatches,
        });
    }
    // ⟨0.23⟩/⟨0.39⟩ INTERFACE-UNION entries — the candor-ts/swift `interfaceUnion` analog. A
    // cross-crate consumer calling a trait method on a `&dyn Trait` whose Trait is imported from HERE keys
    // the chain lookup on `crate#Trait::method` (tail2), which has no body → no entry → the call reads pure.
    // Emit a synthetic entry = the UNION over local impls of that method's effects (inferred + invisible),
    // reusing `trait_impls`/`trait_decls` (the CHA universe in-crate dispatch already uses). Sound
    // over-approximation; a `Trait::method` a consumer never resolves is harmless.
    //
    // ⟨0.39⟩ NO LONGER GATED. This rode behind `CANDOR_WORKSPACE_CHAIN` while §2's ⟨0.23⟩ paragraph read
    // "gated/opt-in until a floor rung pins it"; §4 ⟨0.39⟩ is that rung and makes the entry REQUIRED, its
    // absence a non-conformance. The gate is not incidental to the defect — it is WHY the silent-purity
    // toggle survived in default scans, which is the one thing a default scan must not let happen.
    // ⟨0.40⟩ SOUNDNESS R1268 — A UNION ENTRY CARRYING `Unknown` CARRIES ITS CONTRIBUTORS' REASONS. SPEC §3.1 (the
    // 2026-10-20 interfaceUnion clarification): its `unknownWhy` MUST be non-empty and MUST project to EXACTLY the
    // union of its contributors' TRANSITIVE reason classes, as this producer's own gate resolves them, plus the union's
    // own hedge where it publishes one. Every union row this engine published was REASONLESS (7,866 of 7,866 carrying
    // `Unknown` on the rust A/B's post arm), so a chained consumer read `unresolved` alone and a narrowed `deny
    // Unknown[native|reflect|…] <caller>` passed over the hole a contributor inherited. The class set is the gate's
    // own `reason_class_acc`; one RAW reason per class is carried, found by walking `calls` from the contributor
    // through units whose class set holds it to the unit whose own `unknownWhy` names it. The one class with no raw
    // source is the gate-side `unresolved` of a reasonless chained `Unknown` (`unknown_via_dep`): that contributor is
    // named by its own entry's key, `dep:<hash>` — §4's registered kind for exactly that, projecting to `unresolved`.
    let union_why = |q: &str, out: &mut BTreeSet<String>| {
        if !inferred.get(q).is_some_and(|s| s.contains("Unknown")) {
            return;
        }
        let classes = reason_class_acc.get(q).cloned().unwrap_or_default();
        if classes.is_empty() {
            out.insert(format!("dep:{crate_name}#{q}"));
            return;
        }
        for c in &classes {
            let mut seen: HashSet<String> = HashSet::new();
            let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::from([q.to_string()]);
            let mut found: Option<String> = None;
            while let Some(u) = queue.pop_front() {
                if !seen.insert(u.clone()) {
                    continue;
                }
                if let Some(w) = unknown_why.get(&u).and_then(|ws| {
                    ws.iter().find(|w| candor_classify::policy::ReasonClass::classify(w).token() == c.as_str())
                }) {
                    found = Some(w.clone());
                    break;
                }
                for v in calls.get(&u).into_iter().flatten() {
                    if reason_class_acc.get(v).is_some_and(|s| s.contains(c)) && !seen.contains(v) {
                        queue.push_back(v.clone());
                    }
                }
            }
            match found {
                Some(w) => {
                    out.insert(w);
                }
                None if c == candor_classify::policy::ReasonClass::Unresolved.token() => {
                    out.insert(format!("dep:{crate_name}#{q}"));
                }
                // Every other class enters `reason_class_acc` only from a raw `unknown_why` token, so the walk
                // finds one; reaching here would be a broken invariant, and a guessed token would be a class
                // nobody reaches.
                None => debug_assert!(false, "R1268: class {c} of {q} has no raw source"),
            }
        }
    };
    // ⟨0.40⟩ SOUNDNESS R1293 — AN IMPLEMENTOR THE UNION CANNOT NAME IS A CONTRIBUTOR WHOSE CLASSES IT CANNOT COMPLETE. An
    // implementor's 2-segment tail that SEVERAL units claim (`ResponseBody::poll_frame` in tower-http's `limit`, `metrics`
    // and `trace`) is refused by the never-guess rule, so neither its effects nor its reasons reach the union (R1246's leaf
    // identity, Wave B). Once R1268 gave the union the reasons of the contributors it DID name, the refused ones vanished
    // from its class set: a strict SUBSET, which §3.1 forbids (`deny Unknown[unresolved]` axum-core `Body::size_hint`
    // 1 -> 0 chained). §3.1's rule for exactly this: such a contributor contributes `unresolved`, as a `dep:` reason does
    // — here naming each claimant that carries `Unknown`, by its own entry key in this report. Reasons only: the union's
    // `inferred` is untouched (adding what the refused claimants carry is Wave B's resolution, not this disclosure).
    // SOUNDNESS R1324 (rustagent-0415) — A CONTRIBUTOR THAT DISPATCHES FURTHER. A union entry has no body, so it carries no
    // `dispatchesOn` of its own, and §2/§4 give it none to carry: a consumer that reaches it (a chained dependency's row
    // keyed `serde#ser::Serializer::serialize_some`) learns the union's EFFECTS and REASONS and nothing else. When a
    // contributor is pure but dispatches on ANOTHER member (aws-smithy-types `DocSerializer::serialize_some` ->
    // `serde#ser::Serialize::serialize`, the consumer's own value), the union therefore read `[]` — and the consumer's
    // implementor of that other member was never reached. EXECUTED rustagent-0415 `fx/cpaws/c_aws3`: the program runs the
    // consumer's `Serialize::serialize` (writes) through aws-smithy's union; chained over the REAL aws-smithy-types 1.6.1
    // report, `deny Unknown run3` was 1 on v0.40.14 only because a false `ambiguous:` reason rode the union, and 0 once
    // R1265 removed it. §3.1 ⟨0.40⟩ admits "the union's own dispatch hedge where it publishes one"; ⟨0.39⟩ has no
    // placement for `dispatchesOn` on a union entry, so this is the HEDGE: `Unknown` with `dispatch:<Trait>.<member>` for
    // each further member a contributor reaches. A contributor dispatching on the union's OWN member is the consumer's own
    // obligation-3 join already and is skipped. `CANDOR_R1324_OFF=1` disables it.
    // SOUNDNESS R1351 / R1353 — WHICH CONTRIBUTOR KEYS NAME A MEMBER THEIR TRAIT HAS. A contributor's foreign key is
    // minted from source and can name a member the trait does not declare: R549's chain drift (`visitor.visit_str(..)
    // .map_err(..)` keyed `Visitor::map_err`; aws-smithy `self.inner.front().map(Buf::chunk)` keyed `Buf::map`) and the
    // multi-bound spread (`T: Float + Zero + ..`, `x.is_nan()` keyed under EVERY bound, so `Zero::is_nan`). A reason built
    // from such a key is false (R1353), and a union whose only further keys are such is a false `Unknown`. The previous
    // answer was a NAME list (27 members, `map` `fmt` `clone` `into` …) that also dropped real members of those names —
    // `base#Mapper::map`, executed fx2A — and dropped the formatting/equality/hashing/cloning members SPEC §4 ⟨0.40⟩
    // (R1304) says a package abstraction's receiver DOES dispatch (R1351). Now the question is asked of the authority, the
    // owning package's own trait declaration (`depsig::DepSigs::trait_declares`): a key is left out only on a PROOF that
    // the trait and every supertrait lack the member; every doubt names it. Never asked of: a ⟨0.40⟩ protocol member
    // (named whatever the trait declares — the composition sentence), and a key of THIS crate (a local trait key is
    // member-checked where it is minted). Fails toward over-disclosure (an unreadable source names the key).
    // `CANDOR_R1351_OFF=1` turns the proof off (every key named).
    let r1351_sigs = std::cell::RefCell::new(crate::depsig::DepSigs::default());
    // SOUNDNESS F3 — every foreign key's mint sites, crate-wide: key -> [(fn qual, site)], and (fn qual, site) -> keys.
    let mut fd_prov: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut fd_site_keys: HashMap<(String, String), Vec<String>> = HashMap::new();
    // Fable REVIEW9 R-b — the keys whose call's VALUE escaped somewhere in this crate (collector `E` sites). Not mint sites: kept
    // out of `fd_prov`/`fd_site_keys`, whose drift proof reads only C/D sites.
    let mut fd_escaped: HashSet<String> = HashSet::new();
    for f in &fns {
        for e in &f.foreign_dispatch_prov {
            let Some((k, d)) = e.split_once('\u{1f}') else { continue };
            if d.starts_with("E\u{1f}") {
                fd_escaped.insert(k.to_string());
                continue;
            }
            fd_prov.entry(k.to_string()).or_default().push((f.qual.clone(), d.to_string()));
            fd_site_keys.entry((f.qual.clone(), d.to_string())).or_default().push(k.to_string());
        }
    }
    let r1351_off = candor_report::env::switch("CANDOR_R1351_OFF");
    // ⟨0.40⟩ SPEC §4 UNION CLAUSE (R1324 RESOLVED) — the members are CARRIED, not hedged. Rule 1 makes a narrow union
    // entry carry, in `dispatchesOn`, every member its contributors dispatch on transitively (spelling (a), which this
    // engine's rows already use: `dispatch_acc` is the transitive set); rule 3 makes the hedge below a disclosure for a
    // member the producer CANNOT name, which here is none — so the hedge survives only behind `CANDOR_R1324_HEDGE=1`
    // (the 0.40.15 behaviour, for the A/B's attribution). The R1351/R1353 filter above applies to the carried keys too:
    // a key whose trait provably does not declare the member is not made valid by appearing on a union (rule 1).
    let r1324_hedge = candor_report::env::switch("CANDOR_R1324_HEDGE");
    let r1723_off = candor_report::env::switch("CANDOR_R1723_OFF");
    let union_dispatch_hedge = |cand: &str, own_tail: &str, inf: &mut std::collections::BTreeSet<&'static str>, out: &mut BTreeSet<String>, don: &mut BTreeSet<String>| {
        if candor_report::env::switch("CANDOR_R1324_OFF") {
            return;
        }
        for k in dispatch_acc.get(cand).into_iter().flatten() {
            let (owner, path) = k.split_once('#').unwrap_or((crate_name.as_str(), k));
            let segs: Vec<&str> = path.split("::").collect();
            if segs.len() < 2 {
                continue;
            }
            let (tr, m) = (segs[segs.len() - 2], segs[segs.len() - 1]);
            // A contributor dispatching the union's OWN member skips itself — except, SOUNDNESS R1723, on a union for a
            // DEPENDENCY's trait (`serde#de::Visitor::expecting` over serde_path_to_error's `Wrap` / `CaptureKey`, which
            // forward `self.delegate.expecting(f)` to a caller-chosen visitor). There the member it forwards to is the
            // CONSUMER's implementor, which no union in this report contains: the key is carried, so a chained consumer's
            // join reaches its own implementor. Measured (L2, `rustagent-r1723/l2/spte`, live join): without it the
            // consumer's `Show::fmt` read `[]` over an EXECUTED write once R1723 removed the false-reason hedge v0.41.3 had
            // put on this row. A LOCAL trait's union keeps the skip. `CANDOR_R1723_OFF=1` restores it everywhere.
            if format!("{tr}::{m}") == own_tail && (owner == crate_name.as_str() || r1723_off) {
                continue;
            }
            let protocol = matches!(m, "fmt" | "to_string" | "eq" | "ne" | "hash" | "clone" | "clone_from");
            if !r1351_off && !protocol && owner != crate_name {
                let real = dep_renames.get(owner).map(String::as_str).unwrap_or(owner);
                if r1351_sigs.borrow_mut().trait_declares(dir, owner, real, tr, m) == Some(false) {
                    // SOUNDNESS F1 (Fable REVIEW rust0416) — "the trait does not declare it" is not "it is drift". A key minted
                    // from a PROJECTION receiver (`t.commit()` on `<S as Store>::T`, keyed `Store::commit`; serde's
                    // `S::SerializeSeq` `.end()`, keyed `Serializer::end`) names a member a trait the engine never looked up does
                    // declare, and the program runs a consumer's implementor of it. Only a member std itself has
                    // (`map_err`, `unwrap_or`, `into_iter` — R549 chain/spread drift), or one a local trait declares, with no
                    // associated-type bound declaring it is dropped. Everything else is a member set this producer could not complete: the union keeps the
                    // hedge (SPEC §4 ⟨0.40⟩ union clause rule 3), naming the trait that declares it where the source shows one.
                    let f1_off = candor_report::env::switch("CANDOR_F1_OFF");
                    let assoc = if f1_off { None } else { r1351_sigs.borrow_mut().assoc_member_trait(dir, owner, real, tr, m) };
                    // SOUNDNESS F3 (Fable REVIEW2 rust0416) — DRIFT IS PROVED ABOUT THE RECEIVER, NEVER ABOUT THE NAME. 729e181
                    // dropped a proven-absent key when its member was spelled like a std method or like any local trait's
                    // method, and fx/fb2 executed five real dispatches it silenced that way (an RPIT, a `Box<dyn>`, a
                    // where-bound projection, a supertrait's associated type, an unrelated local `commit`). A key is now
                    // dropped only when EVERY site that minted it is one of two receiver shapes, each read from source:
                    //   C — a method-chain receiver whose inner call is a member of the same trait DECLARED to return a std
                    //       type (`visitor.visit_str(v).map_err(..)`: `visit_str -> Result<..>`), so the outer call is a
                    //       method of that std type (R549 chain drift);
                    //   D — a multi-bound spread: the same site minted the same member under a sibling bound the owner's
                    //       source proves declares it (`T: Float + Zero`, `x.is_nan()`), so the call resolves there.
                    // Every other receiver — RPIT, `dyn`, a projection anywhere, a callback parameter, a key with no recorded
                    // site — is a member set this producer could not complete and keeps the hedge (SPEC §4 ⟨0.40⟩ union
                    // clause rule 3), whatever the member is called.
                    let drift = assoc.is_none() && !f1_off && fd_prov.get(k.as_str()).is_some_and(|sites| !sites.is_empty()
                        && sites.iter().all(|(q, d)| {
                            if let Some(rest) = d.strip_prefix("C\u{1f}") {
                                let Some((full, inner)) = rest.split_once('\u{1f}') else { return false };
                                let alias = full.split("::").next().unwrap_or("");
                                let tleaf = full.rsplit("::").next().unwrap_or("");
                                let areal = dep_renames.get(alias).map(String::as_str).unwrap_or(alias);
                                r1351_sigs.borrow_mut().returns_std(dir, alias, areal, tleaf, inner) == Some(true)
                            } else {
                                fd_site_keys.get(&(q.clone(), d.clone())).is_some_and(|ks| ks.iter().any(|k2| {
                                    if k2 == k { return false }
                                    let Some((o2, p2)) = k2.split_once('#') else { return false };
                                    let s2: Vec<&str> = p2.split("::").collect();
                                    if s2.len() < 2 || s2[s2.len() - 1] != m { return false }
                                    let r2 = dep_renames.get(o2).map(String::as_str).unwrap_or(o2);
                                    r1351_sigs.borrow_mut().trait_declares(dir, o2, r2, s2[s2.len() - 2], m) == Some(true)
                                }))
                            }
                        }));
                    if f1_off || drift {
                        if crate::lang::reach_debug() {
                            eprintln!("R1351SKIP\t{own_tail}\t{owner}#{tr}.{m}"); // §E1 REACH COUNTER
                        }
                        continue;
                    }
                    let tn = assoc.as_deref().unwrap_or(tr);
                    if crate::lang::reach_debug() {
                        eprintln!("R1351PROJ\t{own_tail}\t{owner}#{tr}.{m}\t{tn}"); // §E1 REACH COUNTER
                    }
                    inf.insert("Unknown");
                    out.insert(format!("dispatch:{tn}.{m}"));
                    continue;
                }
            }
            if crate::lang::reach_debug() {
                eprintln!("R1324UNION\t{own_tail}\t{tr}.{m}"); // §E1 REACH COUNTER
            }
            if r1324_hedge {
                inf.insert("Unknown");
                out.insert(format!("dispatch:{tr}.{m}"));
            } else {
                don.insert(k.clone());
                // SPEC §4 ⟨0.40⟩ union clause RULE 3 — Fable REVIEW9 R-b. A carried key set may REPLACE the hedge only when the
                // member set is complete, and it is not when a contributor dispatches on the VALUE a member returns through a
                // spelling the engine never keys: an element of `Vec<Box<dyn Tx>>` reached by a `for` loop, an index, a field,
                // a pattern, a `let` (fx/fb13 a–d, f; fb6L zb, zf — executed, 1/1 on v0.40.15, 0/0 once ec662b7 carried
                // `S::list` instead). So where the member's value ESCAPED (it was not the receiver of a further call, whose
                // own key or drift hedge speaks for it) and its declared return is not PROVABLY closed std (depsig, D1–D3), the
                // union also keeps `Unknown` with the true reason, beside the key (which still joins: a resolution survives).
                // A closed return (`bool`, `Option<u32>`, `()`) carries nothing the consumer supplied, so it never hedges.
                // A key of THIS crate is not read by depsig and is treated as open. `CANDOR_R1324_ESCAPE_OFF=1` disables it.
                if fd_escaped.contains(k.as_str()) && !candor_report::env::switch("CANDOR_R1324_ESCAPE_OFF") {
                    let closed = owner != crate_name && {
                        let real = dep_renames.get(owner).map(String::as_str).unwrap_or(owner);
                        r1351_sigs.borrow_mut().returns_std(dir, owner, real, tr, m) == Some(true)
                    };
                    if !closed {
                        if crate::lang::reach_debug() {
                            eprintln!("R1324ESCAPE\t{own_tail}\t{owner}#{tr}.{m}"); // §E1 REACH COUNTER
                        }
                        inf.insert("Unknown");
                        out.insert(format!("dispatch:{tr}.{m}"));
                    }
                }
            }
        }
    };
    let union_unnamed = |tail: &str, out: &mut BTreeSet<String>| {
        if let Some(ts) = by_tail2.get(tail).filter(|ts| ts.len() >= 2) {
            for t in ts {
                if inferred.get(t).is_some_and(|s| s.contains("Unknown")) {
                    out.insert(format!("dep:{crate_name}#{t}"));
                }
            }
        }
    };
    {
        let existing: std::collections::HashSet<String> = entries.iter().map(|e| e.hash.clone()).collect();
        // SOUNDNESS R598, THE DELEGATION HEDGE — **A TRAIT MEMBER WHOSE DEFAULT BODY CALLS SOMETHING OF
        // ITS OWN NAME MAY REALLY REACH THE IMPLEMENTING TYPE'S SAME-NAMED METHOD, SO THE NARROWING
        // BELOW MUST NOT FIRE ON IT.**
        //
        // Measured, on the full audit of the 14 keys the first form of the R598 narrowing removed over
        // the 1,626-crate corpus. Nine of them are this shape, and in every one the effect IS reachable:
        //
        //   `snapbox#data::IntoData::is`  — default body `self.into_data().is(format)` reaches the
        //                                   INHERENT `Data::is` (`data/mod.rs:576`).
        //   `diesel#query_dsl::QueryDsl::{limit,offset,having,find}` — default body
        //                                   `methods::LimitDsl::limit(self, limit)` reaches
        //                                   `CombinationClause::limit`, a member of a DIFFERENT trait.
        //
        // The engine did not resolve either call (a UFCS call to a trait requirement; a method call on a
        // returned value), so the default body's OWN row is pure and publishes nothing — which means
        // withholding the union's copy leaves the key silent, and that is the cardinal sin traded for a
        // fabrication. The hedge keys on the ONE fact both shapes share: a call of the member's own name
        // in the member's own default body. A deeper delegation (`self.helper()` → `self.m()`) is
        // charged to the default body transitively by the call graph and published at this same key by
        // R597's beside-union, so it needs no hedge of its own; this exists precisely for the calls the
        // resolver DROPS.
        let delegating_members: std::collections::HashSet<String> = fns.iter()
            .filter_map(|f| {
                let t2 = crate::lang::tail2(&f.qual)?;
                let leaf = t2.rsplit("::").next()?.to_string();
                f.calls.iter().any(|c| c.leaf == leaf).then_some(t2)
            })
            .collect();
        for (trait_leaf, lt) in merged.trait_decls.iter() {
            // AMBIGUOUS same-leaf traits (`mod a { trait T } mod b { trait T }`): `trait_decls`/`trait_impls`
            // are keyed by LEAF, so `lt` merges both traits' methods and `impls` merges both traits' impls —
            // a union entry over them could carry an UNRELATED trait's impl effect (a cross-crate fabrication).
            // The in-crate dispatch bails to Unknown here (collector.rs `lt.count > 1`); the emission must too:
            // skip the ambiguous leaf (an honest under-report, never a guess between traits).
            if lt.count > 1 {
                continue;
            }
            let impls = match merged.trait_impls.get(trait_leaf) {
                Some(v) => v,
                None => continue,
            };
            // ⟨0.40⟩ SOUNDNESS R652 — `trait_impls` IS KEYED BY LEAF AND RECORDS FOREIGN AND std IMPLS
            // TOO, SO A NON-EMPTY VECTOR IS NOT EVIDENCE **THIS** TRAIT HAS AN IMPLEMENTOR.
            //
            // `decls.rs` is `trait_impls.entry(leaf.ident.to_string()).or_default().push(ty)` with no
            // locality test at all. A crate that declares its own `trait Write { fn emit(&self); }` and
            // ALSO writes `impl std::io::Write for W` hands the LOCAL trait an `impls` of `["W"]` — and
            // `lt.count > 1` does not catch it, because `trait_decls` counts only local `Item::Trait`
            // declarations, so the leaf is UNAMBIGUOUS. Every lookup then resolves nothing (`W::emit`
            // does not exist), the union comes out empty, and since R609 an empty union PUBLISHES:
            // `crate#Write::emit  inferred: []  interfaceUnion: true  unresolved: false` — a §2 purity
            // claim about an abstraction with no implementor at all. Downstream that row lands in a
            // consumer's `deps_idx.by_key`, so ⟨0.40⟩ conjunct 3 reads the key as ANSWERED and the
            // `Unknown` R608 exists to charge is never charged.
            //
            // MEASURED, BOTH HALVES, on the two-crate fixture in `cli.rs`: `deny Net Unknown` over
            // `h.emit()` on a `&dyn dep::Write` exits 0 with `inferred: []`, and the SAME pair with the
            // `impl std::io::Write` line deleted exits 1 with `['Unknown']` / `dispatch:Write.emit`.
            // One variable. IN THE WILD: serde's `mod std_error { pub trait Error }` — zero
            // implementors, leaf shared with `de::Error` and `ser::Error`, which have many — published
            // `serde#std_error::Error::description` as a bare purity claim in 7 of the corpus's crates.
            //
            // THE EVIDENCE IS COMPUTED HERE AND CONSUMED AT THE PUBLICATION SITE ONLY, and that
            // placement is the load-bearing part of this fix rather than a tidiness choice. An earlier
            // form skipped the whole trait and the A/B priced it: it withheld 29 rows carrying CONCRETE
            // effects (`Log`, `Clock,Exec`, `Env,Exec,Fs,Ipc`), and a withheld row is read by conjunct 3
            // as `Unknown` — which under a bare `deny Log` is NOT denied. **That trades a fabrication
            // for a LOOSENED GATE, i.e. the cardinal direction.** R628 is about a PURITY CLAIM, so the
            // guard belongs exactly where the purity claim is minted and nowhere else: a union that has
            // an effect or a coverage gap to disclose keeps publishing it, whoever the implementors
            // turn out to be.
            //
            // THE TEST IS AN ALLOWLIST, WHICH INVERTS THIS FILE'S USUAL DIRECTION — see
            // `model::impl_confirms_local_trait`. The denylist rule holds where the starting point is a
            // SOUND over-approximation, and here it is not: a leaf collision MANUFACTURES a claim, so
            // the claim needs positive evidence and its absence must produce no row. Unconfirmed ⇒
            // nothing published ⇒ the consumer charges `Unknown`. Over-disclosure, never silence.
            //
            // RESIDUAL, STATED: a trait with BOTH a confirmed local implementor and a colliding-leaf
            // foreign one still unions the foreign one's effects. That is a fabrication, it predates
            // R609, and it is out of this row's scope; R652 removes the SILENT half.
            let decl_qual = union_trait_qual(&merged.trait_quals, trait_leaf).cloned();
            let any_local_impl = decl_qual.as_deref().is_some_and(|q| {
                impls.iter().any(|ty| {
                    crate::model::impl_confirms_local_trait(&merged.impl_members, trait_leaf, ty, q)
                })
            }) || candor_report::env::switch("CANDOR_R652_PRE");
            for method in &lt.methods {
                let mut inf_u: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
                let mut blind_u: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
                let mut why_u: BTreeSet<String> = BTreeSet::new(); // R1268
                let mut don_u: BTreeSet<String> = BTreeSet::new(); // R1324 — the members this union carries
                let own_hedge = r529_reason(&format!("{trait_leaf}::{method}")); // R1268 — this union's own hedge
                let mut r1246_seen: HashSet<String> = HashSet::new(); // R1246 — owner units already unioned
                for ty in impls {
                    let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                    // SOUNDNESS R598 — `{ty}::{method}` IS ALSO THE QUAL OF AN INHERENT `impl Ty`.
                    //
                    // `trait_impls` says only that `Ty` implements `Trait`; the lookups below then read
                    // ANY unit called `Ty::method` as that implementation. An `impl Ty { fn method }`
                    // beside an `impl Trait for Ty` that does NOT override `method` (because the trait
                    // supplies a default body) is a DIFFERENT FUNCTION. Live on hickory-proto 0.26.x:
                    // `BinEncodable::to_bytes` published a `Log` from the private inherent
                    // `RData::to_bytes` (`rr/record_data.rs:839`) while `impl BinEncodable for RData`
                    // (`:1133`) declares `emit` alone — a concrete claim about a function that does not
                    // perform it, and `deny Log` fires on it.
                    //
                    // **THE NARROWING WITHHOLDS A CLAIM, NEVER A DISCLOSURE, AND THAT LINE IS DRAWN
                    // WHERE IT IS BECAUSE OF THE FULL REMOVAL AUDIT — NOT BY TASTE.** The first form of
                    // this fix dropped the candidate entirely (`continue`). Over the 1,626-crate
                    // cargo-registry corpus that removed 14 keys, and tracing ALL FOURTEEN to a body
                    // found NINE where the effect really is reachable through the member — because the
                    // trait's DEFAULT BODY delegates to a same-named method on the receiver:
                    //
                    //   · `snapbox#data::IntoData::is` — the default body is `self.into_data().is(fmt)`
                    //     (`data/mod.rs:123`), which is the inherent `Data::is` at `:576`. Reachable.
                    //   · `diesel#query_dsl::QueryDsl::{limit,offset,having,find}` × 2 versions — every
                    //     `impl QueryDsl for X {}` block is EMPTY, and the default body is
                    //     `methods::LimitDsl::limit(self, limit)` (`query_dsl/mod.rs:961`), which for a
                    //     `CombinationClause` receiver IS `CombinationClause::limit`, the unit whose
                    //     `Unknown` the drop removed.
                    //
                    // So the unqualified drop trades a FABRICATION (an over-report) for a SILENCE (the
                    // cardinal sin), on 9 of 14 keys. The other 5 are true fabrications by two further
                    // mechanisms, kept for the record because a second cause behind the first is what
                    // gets missed: `axum#handler::Handler::{layer,with_state}` — `impl Handler for
                    // MethodRouter` declares `call` alone and the default body constructs a `Layered`,
                    // so the inherent `MethodRouter::layer` (`routing/method_routing.rs:967`) is
                    // genuinely unreachable; and `sea_orm#entity::base_entity::EntityName::table_ref` —
                    // the leaf `Entity` merges TWO unrelated types ([[candor-global-unit-identity]]), and
                    // the effects came from `dynamic::Entity::table_ref`, which does not implement that
                    // trait at all.
                    //
                    // THE DISCRIMINATION THAT SURVIVES BOTH: `Unknown` and `invisible` are the engine
                    // saying WHAT IT DOES NOT KNOW; a concrete effect is a claim about what a function
                    // DOES. Withholding a false claim costs nothing in the sin direction. Withholding a
                    // disclosure turns "we cannot see this" into "there is nothing here", which is the
                    // silent purity claim ⟨0.39⟩ exists to close. So a narrowed candidate contributes
                    // its `Unknown` and its `invisible` unchanged, and its concrete effects not at all.
                    // Measured consequence: REMOVED 0 over the same corpus, with the hickory `Log` gone.
                    //
                    // THE EVIDENCE GATE IS THE OTHER HALF. `impl_members` records "block seen", "member
                    // declared" and "block not fully readable" separately, and `impl_does_not_declare`
                    // narrows ONLY on seen && readable && absent — every other state leaves the
                    // over-approximation exactly as it was, because an allowlist over an index known to
                    // be incomplete is how a fabrication fix becomes a silent under-report.
                    //
                    // `CANDOR_R598_PRE` restores the pre-fix lookup for the A/B's pre arm. Its failure
                    // direction is OVER-CHARGE, never silence — which is why it is safe to ship.
                    let narrowed =
                        crate::model::impl_does_not_declare(&merged.impl_members, trait_leaf, ty, method)
                            && !delegating_members.contains(&format!("{trait_leaf}::{method}"))
                            && !candor_report::env::switch("CANDOR_R598_PRE");
                    // A CONCRETE effect of a narrowed candidate is withheld; `Unknown` rides through.
                    let admit = |e: &str| !narrowed || e == "Unknown";
                    // SOUNDNESS R1245 (the REFUSED half, union leg) — this implementor's block holds a macro item `mbe`
                    // could not expand, and no hand-written `method`: the macro may define it, and no unit stands for
                    // what it defines, so the union over the implementors this engine READ is not the union over the
                    // ones that run. Executed: a dependency's `impl Report for Reports { mlib::reports_impl!(fn
                    // summarize(&self)); }` published `Report::summarize` `inferred: []`, and a consumer's `&dyn
                    // Report` / `T: Report` caller read PURE over a file write. Same answer R529/R828 give a block
                    // they cannot name. A same-crate macro is expanded and leaves no opaque item, so this is the residue.
                    if merged.impl_members.contains(&crate::model::impl_opaque_key(trait_leaf, ty))
                        && !merged.impl_members.contains(&crate::model::impl_member_key(trait_leaf, ty, method))
                    {
                        if candor_report::env::trace("CANDOR_R1245_INSTR") {
                            eprintln!("R1245UNION\t{trait_leaf}::{method}\t{ty}"); // §E1 REACH COUNTER
                        }
                        inf_u.insert("Unknown");
                        why_u.insert("macro:impl members hidden by an unexpanded macro".to_string()); // R1268
                    }
                    if narrowed {
                        // §E1 REACH PROBE — fires only where this narrowing actually withholds
                        // something, so `CHANGED 0` and "the branch never ran" stay distinguishable.
                        if candor_report::env::trace("CANDOR_R598_INSTR") {
                            let had: Vec<&&str> = inferred.get(&format!("{ty}::{method}"))
                                .map(|s| s.iter().collect()).unwrap_or_default();
                            let tail: Vec<&&str> = by_tail2.get(&format!("{ty}::{method}"))
                                .filter(|ts| ts.len() == 1)
                                .and_then(|ts| inferred.get(&ts[0]))
                                .map(|s| s.iter().collect()).unwrap_or_default();
                            eprintln!("R598HIT\t{trait_leaf}::{method}\t{ty}\tdirect={had:?}\ttail2={tail:?}");
                        }
                    }
                    for cand in [format!("{ty}::{method}"), format!("{ty_leaf}::{method}")] {
                        if !inferred.contains_key(&cand) {
                            union_unnamed(&cand, &mut why_u); // R1293
                        }
                        if let Some(s) = inferred.get(&cand) {
                            for e in s.iter().filter(|e| admit(e)) {
                                inf_u.insert(e);
                            }
                            union_why(&cand, &mut why_u); // R1268 — `Unknown` is admitted on both sides of R598
                        }
                        union_dispatch_hedge(&cand, &format!("{trait_leaf}::{method}"), &mut inf_u, &mut why_u, &mut don_u); // R1324
                        if let Some(s) = blind_acc.get(&cand) {
                            for c in s.iter().filter(|c| global_blind.contains(*c)) {
                                blind_u.insert(c.clone());
                            }
                        }
                        // SOUNDNESS R513 — THE IMPL'S UNIT IS MODULE-QUALIFIED AND `trait_impls` IS NOT.
                        // `decls.rs` pushes the self type AS WRITTEN (`Net1`), never `backend::Net1`, so a
                        // trait+impl nested in `pub mod backend` makes both candidates above `Net1::size`
                        // while the analysed unit is `backend::Net1::size`. The lookup came back empty, no
                        // effect entered the union, and the entry was skipped as "pure across all impls" —
                        // so a chained consumer read `inferred: []` with NO `invisible`, which §2 makes a
                        // PURITY CLAIM: `deny Net` exit 0 over a dep whose sole implementor opens a
                        // TcpStream. The module form is the ORDINARY shape in real crates, so the leg was
                        // working only for the crate-root spelling.
                        //
                        // THE FOREIGN LEG OF THIS SAME RUNG ALREADY DOES THIS (the `foreign_impls` loop
                        // below resolves its 2-segment implementor tails through `by_tail2` for exactly
                        // this reason); the asymmetry was the bug. Same index, same refusal: ONLY when the
                        // tail is unambiguous — two units under one tail is `resolve_target`'s never-guess
                        // case, and picking one would charge a different type's effects to this trait.
                        if let Some(ts) = by_tail2.get(&cand) {
                            if ts.len() == 1 && ts[0] != cand {
                                // REACH PROBE — same channel and reason as `R503HIT`: fires only where
                                // this fallback resolves an implementor the exact-key lookups could not,
                                // so `CHANGED 0` and "the branch never ran" stay distinguishable.
                                if candor_report::env::trace("CANDOR_ALIAS_DEBUG") {
                                    eprintln!("R513HIT {}", ts[0]);
                                }
                                // SOUNDNESS R1352 — the R1324 hedge asks the SAME resolved unit the effects come
                                // from. `cand` is the 2-segment `{ty}::{method}` tail and `dispatch_acc` is keyed by
                                // the module-qualified qual, so for an implementor declared in a submodule (`pub mod
                                // inner { impl Sink for Doc { .. } }`, the ordinary crate shape) the call above found
                                // nothing and only this fallback reaches the unit. The foreign leg below already
                                // hedges both `cand` and `ts[0]`; the asymmetry was the bug (R513's own words).
                                // EXECUTED fx3/fx6 (one variable: the `mod`). `CANDOR_R1352_OFF=1` disables it.
                                if !candor_report::env::switch("CANDOR_R1352_OFF") {
                                    let w0 = why_u.len() + don_u.len();
                                    union_dispatch_hedge(&ts[0], &format!("{trait_leaf}::{method}"), &mut inf_u, &mut why_u, &mut don_u);
                                    if why_u.len() + don_u.len() != w0 && crate::lang::reach_debug() {
                                        eprintln!("R1352TAIL\t{trait_leaf}::{method}\t{}", ts[0]); // §E1 REACH COUNTER
                                    }
                                }
                                if let Some(s) = inferred.get(&ts[0]) {
                                    // R598 — the same `admit`: this fallback resolves the SAME
                                    // `{ty}::{method}` spelling one index over, so a narrowed candidate
                                    // must not re-enter through it (§F1 Q3, two paths one question).
                                    inf_u.extend(s.iter().copied().filter(|e| admit(e)));
                                    union_why(&ts[0], &mut why_u); // R1268
                                }
                                if let Some(s) = blind_acc.get(&ts[0]) {
                                    blind_u.extend(s.iter().filter(|c| global_blind.contains(*c)).cloned());
                                }
                            }
                        }
                    }
                    // SOUNDNESS R1246 — AND EACH IMPLEMENTOR BLOCK BY ITS OWNER. Both lookups above key on the self type's
                    // LEAF: when two types share it (`m1::Bytes`, `m2::Bytes`) the exact `{ty}::{method}` names neither
                    // module unit and R513's tail fallback refuses the two-claimant tail — also with ONE implementor beside
                    // an unrelated same-leaf type (an inherent `m2::Bytes::put`). Nothing entered the union and it was
                    // PUBLISHED as `inferred: []`, a purity claim a chained consumer's `deny Fs` passed on over a file
                    // write (EXECUTED, R1238 chain2). The owner (`model::impl_owner_key`) names the block's unit exactly,
                    // and only a block that DECLARES `method` contributes — an inheriting block runs the trait's default,
                    // which has its own row at this key (R597). Same contributions as an exact candidate.
                    // `CANDOR_R1246_OFF=1` disables it.
                    if !candor_report::env::switch("CANDOR_R1246_OFF") {
                        for o in impl_owners.get(trait_leaf).into_iter().flatten().filter(|o| &o.ty == ty && o.declares(method)) {
                            let q = format!("{}::{method}", o.owner);
                            if !unit_quals.contains(&q) || !r1246_seen.insert(q.clone()) {
                                continue;
                            }
                            if crate::lang::reach_debug() && !by_tail2.get(&format!("{ty_leaf}::{method}")).is_some_and(|v| v.len() == 1) {
                                eprintln!("R1246UNION\t{trait_leaf}::{method}\t{q}"); // §E1 REACH COUNTER (a tail R513 could not name)
                            }
                            if let Some(s) = inferred.get(&q) {
                                inf_u.extend(s.iter().copied().filter(|e| admit(e)));
                                union_why(&q, &mut why_u);
                            }
                            union_dispatch_hedge(&q, &format!("{trait_leaf}::{method}"), &mut inf_u, &mut why_u, &mut don_u);
                            if let Some(s) = blind_acc.get(&q) {
                                blind_u.extend(s.iter().filter(|c| global_blind.contains(*c)).cloned());
                            }
                        }
                    }
                }
                // SOUNDNESS R1367 — A UNION PAST THE BOUND IS PUBLISHED BROAD (SPEC §4 ⟨0.40⟩, the bound paragraph). The
                // in-scan dispatch over this trait reads `Unknown` once `trait_impls` holds more than 12 implementors
                // (`collector.rs` `dispatch_trait_member`, the SAME index and the SAME count), so a union published
                // narrow — pure when every implementor is — hands a chained consumer a purity claim this scan does not
                // make (PART 103 `c_13`: 13 pure implementors, chained `deny Unknown` 0 while one tree reads 1). The
                // bounded-CHA hedge on its own key, exactly the in-scan reason. `CANDOR_R1367_OFF=1` disables it.
                if impls.len() > 12 && !candor_report::env::switch("CANDOR_R1367_OFF") {
                    if crate::lang::reach_debug() {
                        eprintln!("R1367BROAD\tLOCAL\t{trait_leaf}::{method}\t{}", impls.len()); // §E1 REACH COUNTER
                    }
                    inf_u.insert("Unknown");
                    why_u.insert(own_hedge.clone());
                }
                // SOUNDNESS R529 — an implementor written inside a BLOCK is in no Pass A index, so the
                // union above is taken over the implementors this engine could NAME, not over the ones
                // this crate has. Publishing that union as a complete answer is the silent-purity claim
                // one spelling over from the one ⟨0.39⟩ closes; `Unknown` is what the union honestly is.
                // R828 — the NON-NOMINAL implementors' units, unioned like any other implementor's.
                if let Some(qs) = nonnominal.get(&format!("{trait_leaf}::{method}")) {
                    for q in qs {
                        if !unit_quals.contains(q) {
                            inf_u.insert("Unknown");
                            why_u.insert(own_hedge.clone()); // R1268
                            continue;
                        }
                        if let Some(s) = inferred.get(q) {
                            inf_u.extend(s.iter().copied());
                            union_why(q, &mut why_u); // R1268
                        }
                        // R1352 — the third contributor route of this union, hedged like the other two (§A.2).
                        if !candor_report::env::switch("CANDOR_R1352_OFF") {
                            let w0 = why_u.len() + don_u.len();
                            union_dispatch_hedge(q, &format!("{trait_leaf}::{method}"), &mut inf_u, &mut why_u, &mut don_u);
                            if why_u.len() + don_u.len() != w0 && crate::lang::reach_debug() {
                                eprintln!("R1352NONNOM\t{trait_leaf}::{method}\t{q}"); // §E1 REACH COUNTER
                            }
                        }
                        if let Some(s) = blind_acc.get(q) {
                            blind_u.extend(s.iter().filter(|c| global_blind.contains(*c)).cloned());
                        }
                    }
                }
                if merged.nested_impl_members.contains(&format!("{trait_leaf}::{method}"))
                    || merged.nested_impl_members.contains(&format!("{trait_leaf}::*")) // R828
                {
                    inf_u.insert("Unknown");
                    why_u.insert(own_hedge.clone()); // R1268
                    if candor_report::env::trace("CANDOR_R529_INSTR") {
                        eprintln!("R529HIT\tUNION-LOCAL\t{trait_leaf}::{method}"); // §E1 REACH PROBE
                    }
                }
                // ⟨0.40⟩ SOUNDNESS R609 — A PURE-ONLY UNION IS PUBLISHED, NOT DROPPED.
                //
                // This used to be `if inf_u.is_empty() && blind_u.is_empty() { continue }` —
                // "pure across all impls, silence = purity". The entry it dropped and the entry it
                // never had are the SAME BYTES on the wire, so a chained consumer cannot tell
                //   (a) this abstraction has implementors and every one of them is pure
                // from
                //   (b) this abstraction has no implementor anywhere.
                // (a) is legitimately pure — PART 92's `c3_pure_only` is the control that says so.
                // (b) is R608, the dispatch nobody answers. Conflating them is what forced R608's
                // consumer rule to hedge on WIRE ABSENCE, and it is a producer-side defect in its own
                // right: obligation 2 exists to publish the implementor union, and "the union is
                // empty of effects" is a fact about the union, not the absence of one.
                //
                // THE ROW IS A CLAIM, AND IT IS ONE THE ENGINE IS ALREADY MAKING. Its `inferred` is
                // `[]` with no `invisible` and no `Unknown`, which §2 reads as a purity claim — the
                // same claim the consumer was already deriving from silence, now said out loud where
                // a reader and a gate can see it. It is emitted ONLY where an implementor was
                // actually named (`!impls.is_empty()`): an abstraction with no implementor publishes
                // nothing, which is what keeps R608's conjunct 3 meaning what it says.
                //
                // REMOVED-DIRECTION RISK, stated because it is NOT zero by construction. A new key in
                // a consumer's `deps_idx.by_key` can satisfy the call-join at `scan.rs`'s
                // `dep_join_hit`, and `already_handled` reads that flag — so a call that previously
                // fell through to the R452/R190(e)/R128 disclosures could stop doing so. Measured
                // rather than reasoned: the numbers are in the commit message.
                // THE ZERO-IMPLEMENTOR CASE. This paragraph said the conjunct below was DEFENSIVE and
                // UNREACHABLE — `trait_impls`' own miss (`None => continue`) delivered the property,
                // `decls.rs` only ever `push`es, so no leaf reached here with an empty vec. **Every
                // literal statement in that was true and the conclusion was false**, which is R652:
                // the vector is keyed by trait LEAF and holds impls of FOREIGN and std traits, so a
                // non-empty vector was never evidence that THIS trait has an implementor. The conjunct
                // is now `!any_local_impl`, it is REACHABLE, and it is covered by a calibrated fixture
                // (`r652_…`). `r609_a_pure_only_union_is_published_and_a_zero_implementor_one_is_not`
                // still pins the property from the other side.
                if inf_u.is_empty() && blind_u.is_empty() && !any_local_impl {
                    // ⟨0.40⟩ SOUNDNESS R652 — `impls.is_empty()` USED TO BE THIS CONJUNCT, and it was
                    // labelled DEFENSIVE/UNCOVERED because `decls.rs` only ever `push`es, so no leaf
                    // reaches here with an empty vec. That reasoning was right about the VECTOR and
                    // wrong about the QUESTION: a non-empty vector is not evidence that this trait has
                    // an implementor, because the vector is keyed by LEAF. `!any_local_impl` is the
                    // same conjunct asking what it meant to ask, and it is now REACHABLE and COVERED —
                    // `r652_a_foreign_impl_sharing_a_trait_leaf_is_not_an_implementor_of_the_local_trait`
                    // drives it, calibration arm included.
                    //
                    // §E1 REACH PROBE — fires only where this withholds a row, so `REMOVED 0` and "the
                    // branch never ran" stay distinguishable. It prints the EVIDENCE (the declaration
                    // qual the comparison used, and every trait path this crate wrote for each named
                    // implementor), because a withheld row that turns out to have a real implementor
                    // has to be diagnosable from the probe line alone.
                    if candor_report::env::trace("CANDOR_R652_INSTR") {
                        let ev: Vec<String> = impls.iter().map(|ty| {
                            let pre = format!("{trait_leaf}\u{1f}{ty}\u{1f}!");
                            let paths: Vec<&str> = merged.impl_members.iter()
                                .filter_map(|k| k.strip_prefix(&pre)).collect();
                            format!("{ty}<={paths:?}")
                        }).collect();
                        eprintln!("R652HIT\t{trait_leaf}::{method}\tqual={decl_qual:?}\t{ev:?}");
                    }
                    continue; // no implementor OF THIS TRAIT was confirmed — there is no union to publish
                }
                // R503 — the ⟨0.23⟩ spelling, not the leaf. A trait declared at `backend::Backend`
                // publishes `iface#backend::Backend::size`, which is the SAME key the foreign half of
                // this rung emits for the same member (`collect_foreign_trait_impls`) and the same one
                // `dispatchesOn` now carries. Before this the local half said `iface#Backend::size` and
                // the foreign half said `iface#backend::Backend::size` — two names for one abstraction,
                // inside one engine, which is the ⟨0.34⟩ drift shape the clause exists to forbid.
                // A leaf with no unambiguous declaration qual is SKIPPED rather than published under a
                // guessed key: `trait_impls`/`trait_decls` are leaf-keyed, so an entry minted from a leaf
                // alone could name a different crate's trait to a consumer.
                let Some(hash) = union_member_key(&merged.trait_quals, &crate_name, trait_leaf, method)
                else {
                    continue;
                };
                // SOUNDNESS R597 — A REAL ROW AT THIS HASH NO LONGER DISCARDS THE UNION; IT SITS BESIDE IT.
                //
                // This used to be `if existing.contains(&hash) { continue }` — "a real entry already
                // claims this hash". It does, and it is answering a DIFFERENT QUESTION. The real row is
                // the trait member's DEFAULT BODY (`crate#Trait::m` is an analysed unit the moment the
                // method has one); this union is over the IMPLEMENTORS' OVERRIDES. Neither set contains
                // the other, so the discard published the default body alone under the one key ⟨0.39⟩
                // §4 obligation 3 tells a consumer to join a dyn dispatch on — R576(a) seen from the
                // consumer side, one cause, two rows.
                //
                // MEASURED before the change, over the 1,626-crate cargo-registry corpus (the
                // `CANDOR_R590_INSTR` probe below, which is why it is still here): 244 suppressions
                // reached, 233 where the union adds nothing, and 11 where IT KNEW MORE — five of them a
                // published key reading `inferred: []` with no disclosure at all (a §2 purity claim)
                // while this engine had computed `Unknown` for the dispatch target set and thrown it
                // away: `combine#parser::Parser::parse_mode_impl`, three `sea_query` builder members,
                // `tracing_subscriber#fmt::writer::MakeWriter::make_writer_for`.
                //
                // BESIDE, NOT MERGED INTO. Merging is the CHARGING direction and it fabricates: the real
                // row carries a `loc` and is a concrete body, so folding a sibling implementor's `Log`
                // into it charges an effect to a function at a line that does not perform it — which is
                // exactly what R598's `portable_pty#Child::wait` would have suffered. Two entries under
                // one key are UNIONED BY THE CONSUMER, ruled family-wide in
                // candor-spec/ENTRY-COLLISION-DECISION.md (shipped four-way 2026-08-02, pinned by
                // conformance PART 26) and implemented here in `deps.rs` — `idx.by_key.entry(k)
                // .or_default().union_with(&de)`, whose own comment says "TWO ENTRIES UNDER ONE KEY ARE
                // UNIONED — never withdrawn, never picked between", and whose item 2 names the
                // WITHIN-report collision as the same defect. So this emits no new resolution rule: it
                // supplies a contributor to one that already exists, and every published row keeps its
                // bytes. `candor-query` filters `interfaceUnion` rows at its ingress (`load.rs`, R511),
                // so no local verb sees a second unit.
                //
                // ONLY WHEN IT ADDS AN EFFECT, or the other 233 become duplicate rows carrying nothing.
                // The narrowing fails in the withholding direction if it is ever wrong, so it is exact
                // set arithmetic on the published fields rather than a heuristic — and `Unknown` on the
                // real row covers any effect, which is why those cases are not losses.
                //
                // THE COVERAGE LEG IS NOW EMITTED — R598 IS CLOSED, AND IT IS WHAT THIS LEG WAS
                // WAITING ON. `blind_adds` is the same suppression seen through `invisible`: the union
                // names an UNCOVERED package the real row does not. It was held back because a
                // beside-union row carries its EFFECTS too and R598 put a FABRICATED one in that set —
                // the `{ty}::{method}` lookup could not tell `impl Trait for Ty` from an inherent
                // `impl Ty`, so `hickory_proto#serialize::binary::BinEncodable::to_bytes` published a
                // `Log` sourced from the private inherent `RData::to_bytes` (record_data.rs:839). With
                // the narrowing above in place that lookup no longer reads an inherent method as an
                // implementation, so the trade this leg was refused on no longer exists: `invisible`
                // arms NO policy form (⟨0.30⟩'s non-gating ruling, SPEC.md ~:4379) and the effects it
                // rides beside are now the implementors' own.
                //
                // MEASURED, not assumed: `bin/corpus-ab.py` over the same 1,626 cargo-registry crates
                // the decision to hold it was taken on. The numbers are in the commit message.
                if existing.contains(&hash) {
                    let (eff_adds, blind_adds) = union_adds_over_row(&entries, &hash, &inf_u, &blind_u);
                    // §E1 REACH PROBE, and the population counter the pre-fix measurement was taken
                    // with — same line shapes, so 244/233/11 stays re-countable rather than re-argued.
                    if candor_report::env::trace("CANDOR_R590_INSTR") {
                        if let Some(real) = entries.iter().find(|e| e.hash == hash) {
                            if !inf_u.is_empty() {
                                if eff_adds {
                                    eprintln!("R590LOST {hash} union={inf_u:?} real={:?}", real.inferred);
                                } else {
                                    eprintln!("R590OK {hash}");
                                }
                            }
                            if blind_adds {
                                eprintln!(
                                    "R590BLIND {hash} union={blind_u:?} real={:?}", real.invisible);
                            }
                        }
                    }
                    // R1324 — …and every MEMBER: a union carrying a key the default body's row does not is not
                    // a duplicate (§4 ⟨0.40⟩ union clause, rule 1).
                    let don_adds = entries.iter().find(|e| e.hash == hash)
                        .is_some_and(|real| don_u.iter().any(|k| !real.dispatches_on.contains(k)));
                    // SOUNDNESS R1666 — …and every REASON CLASS. `Unknown` on the real row covers any EFFECT, but not a class
                    // gate: a union that carries `Unknown[unresolved]` beside a real row whose own reasons are all
                    // `dispatch` was suppressed as a duplicate, so `deny Unknown[unresolved]` went 1 -> 0 on the unit the day
                    // the real row gained an `Unknown` of its own (EXECUTED on object-0.24.0 `Object::symbol_map`, whose union
                    // row carries a `macro:` reason the R1666 floor's new `dispatch` reasons do not). Exact set arithmetic on
                    // the published `unknownWhy` fields. Kill switch `CANDOR_R1666B_OFF`.
                    let class_adds = !candor_report::env::switch("CANDOR_R1666B_OFF")
                        && inf_u.contains("Unknown")
                        && entries.iter().find(|e| e.hash == hash).is_some_and(|real| {
                            let real_cls: BTreeSet<&str> =
                                real.unknown_why.iter().map(|w| candor_classify::policy::ReasonClass::classify(w).token()).collect();
                            why_u.iter().any(|w| !real_cls.contains(candor_classify::policy::ReasonClass::classify(w).token()))
                        });
                    if class_adds && !eff_adds && !blind_adds && !don_adds && candor_report::env::trace("CANDOR_R1666B_INSTR") {
                        eprintln!("R1666BCLASS\t{hash}"); // §E1 REACH COUNTER: a union row emitted ONLY for its reason classes
                    }
                    if !eff_adds && !blind_adds && !don_adds && !class_adds {
                        // The real entry at this hash already carries every EFFECT **and** every
                        // UNCOVERED PACKAGE the union knows, so a second row would be a duplicate
                        // carrying nothing. 233 of the 244 measured suppressions are this case.
                        continue;
                    }
                }
                // §E1 REACH PROBE — fires ONLY on the rows R609 newly publishes (a union with no
                // effect and no uncovered package), so `CHANGED 0` and "the branch never ran" stay
                // distinguishable and this half's reach is never borrowed from the local leg's.
                if inf_u.is_empty() && blind_u.is_empty()
                    && candor_report::env::trace("CANDOR_R609_INSTR")
                {
                    eprintln!("R609HIT\tLOCAL\t{hash}");
                }
                if inf_u.contains("Unknown") && why_u.is_empty() {
                    why_u.insert(own_hedge.clone()); // R1268 — non-empty by construction (see `union_why`)
                }
                entries.push(ReportEntry {
                    func: hash.split_once('#').map(|(_, m)| m.to_string()).unwrap_or_default(),
                    inferred: inf_u.iter().map(|s| s.to_string()).collect(),
                    unresolved: inf_u.contains("Unknown"),
                    hash,
                    invisible: blind_u.into_iter().collect(),
                    interface_union: true,
                    unknown_why: if inf_u.contains("Unknown") { why_u.into_iter().collect() } else { Vec::new() },
                    dispatches_on: don_u.into_iter().collect(), // R1324 — §4 ⟨0.40⟩ union clause, spelling (a)
                    ..Default::default()
                });
            }
        }
        // ⟨0.39⟩ SPEC §4 obligation 2 — THE SAME UNION FOR AN ABSTRACTION THIS CRATE DOES NOT OWN, keyed
        // under the crate that DOES. The loop above covers LOCAL traits only, and in the measured instance
        // (SOUNDNESS R475, live on `ratatui`) that is exactly the leg that misses: `ratatui-core` declares
        // `Backend` and sees one pure implementor, the effectful `CrosstermBackend` lives in a THIRD
        // package, and a consumer chained onto both is told nothing by either report. Neither this entry
        // nor `dispatchesOn` is any use without the other, which is why §4 says no two of the three
        // obligations are separable.
        //
        // THE KEY TAKES NO NEW SPELLING RULE. `foreign_impls` is already keyed `{owner}#{trait qual}::
        // {method}` — the ⟨0.23⟩ `typeSurface` rule, fully qualified in the OWNING package's namespace,
        // the same namespace that package's own entry hashes use — so the consumer's ORDINARY chained
        // lookup resolves it and `load_dep_reports` needs no special case. `func` carries the member alone
        // so the index's tail2/full keys come out in the owner's namespace too.
        //
        // PROVENANCE IS CHECKED AGAINST THE MANIFEST, not against the spelling: `collect_foreign_trait_impls`
        // admits anything that is not std/`crate`/`self`/`super`, which a local module spelled like a crate
        // also passes. `deps` is Cargo.toml's real dependency set and is only known here.
        // SOUNDNESS R529 — the key set is `foreign_impls` UNION `nested_impl_foreign`. A crate whose
        // ONLY implementor of a dependency's abstraction is written inside a block has no `foreign_impls`
        // entry at all, so iterating that map alone publishes nothing and every chained consumer reads
        // the dependency's own (honest, and incomplete) purity. The two sets overlap where a member is
        // implemented both ways, and the loop is keyed so that case emits ONE entry carrying both.
        let mut union_keys: Vec<&String> = merged.foreign_impls.keys()
            .chain(merged.nested_impl_foreign.iter())
            .collect();
        union_keys.sort();
        union_keys.dedup();
        static NO_IMPLS: Vec<String> = Vec::new();
        for key in union_keys {
            let impl_quals = merged.foreign_impls.get(key).unwrap_or(&NO_IMPLS);
            let Some((owner, _)) = key.split_once('#') else { continue };
            if owner == crate_name || !deps.contains(owner) {
                continue;
            }
            let mut inf_u: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
            let mut blind_u: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            let mut why_u: BTreeSet<String> = BTreeSet::new(); // R1268
            let mut don_u: BTreeSet<String> = BTreeSet::new(); // R1324 — the members this union carries
            if merged.nested_impl_foreign.contains(key) {
                inf_u.insert("Unknown");
                why_u.insert(r529_reason(key)); // R1268 — this union's own hedge
                if candor_report::env::trace("CANDOR_R529_INSTR") {
                    eprintln!("R529HIT\tUNION-FOREIGN\t{key}"); // §E1 REACH PROBE
                }
            }
            let own_tail = crate::lang::tail2(key.split_once('#').map(|(_, p)| p).unwrap_or(key)).unwrap_or_default();
            // SOUNDNESS R1367, the FOREIGN leg — the same bound over this crate's implementors of THAT trait (every
            // `foreign_impls` key under the same `{owner}#{trait qual}::` prefix, counted by implementing type).
            if !candor_report::env::switch("CANDOR_R1367_OFF") {
                if let Some(pfx) = r533_trait_prefix(key) {
                    let tys: HashSet<&str> = merged.foreign_impls.iter()
                        .filter(|(k, _)| k.starts_with(pfx.as_str()) && !k[pfx.len()..].contains("::"))
                        .flat_map(|(_, v)| v.iter().map(|q| q.rsplit_once("::").map(|(t, _)| t).unwrap_or(q)))
                        .collect();
                    if tys.len() > 12 {
                        if crate::lang::reach_debug() {
                            eprintln!("R1367BROAD\tFOREIGN\t{key}\t{}", tys.len()); // §E1 REACH COUNTER
                        }
                        inf_u.insert("Unknown");
                        why_u.insert(r529_reason(key));
                    }
                }
            }
            for cand in impl_quals {
                if let Some(s) = inferred.get(cand) {
                    inf_u.extend(s.iter().copied());
                    union_why(cand, &mut why_u); // R1268
                } else {
                    union_unnamed(cand, &mut why_u); // R1293
                }
                union_dispatch_hedge(cand, &own_tail, &mut inf_u, &mut why_u, &mut don_u); // R1324
                if let Some(s) = blind_acc.get(cand) {
                    blind_u.extend(s.iter().filter(|c| global_blind.contains(*c)).cloned());
                }
                // `foreign_impls` values are 2-segment `Type::method` tails; a module-qualified unit is
                // reached through the same tail2 index every other resolution uses, and ONLY when the tail
                // is unambiguous — two units under one tail is `resolve_target`'s never-guess case, and
                // guessing here would charge one type's effects to another type's implementor.
                if let Some(ts) = by_tail2.get(cand) {
                    if ts.len() == 1 {
                        union_dispatch_hedge(&ts[0], &own_tail, &mut inf_u, &mut why_u, &mut don_u); // R1324
                        if let Some(s) = inferred.get(&ts[0]) {
                            inf_u.extend(s.iter().copied());
                            union_why(&ts[0], &mut why_u); // R1268
                        }
                        if let Some(s) = blind_acc.get(&ts[0]) {
                            blind_u.extend(s.iter().filter(|c| global_blind.contains(*c)).cloned());
                        }
                    }
                }
            }
            // ⟨0.40⟩ SOUNDNESS R609, the FOREIGN leg — the same publication, and the audit boundary is
            // not drawn around the local loop that triggered it. `union_keys` is `foreign_impls` ∪
            // `nested_impl_foreign`, and a key that came ONLY from `nested_impl_foreign` has already
            // inserted `Unknown` above, so the empty case here is exactly "this crate supplies
            // implementors of a dependency's abstraction and every one of them is pure" — the (a) case
            // the local leg's note describes, under the owning crate's namespace.
            // Same shape, same label: `impl_quals` is empty only for a key that came from
            // `nested_impl_foreign` alone, and that branch has already inserted `Unknown` above — so
            // this conjunct is UNREACHABLE at HEAD too, and is a defensive guard rather than a tested
            // one. See the local leg's note for why it is written out.
            if inf_u.is_empty() && blind_u.is_empty() && impl_quals.is_empty() {
                continue; // no implementor was named at all — there is no union to publish
            }
            // SOUNDNESS R597, the FOREIGN leg — the same suppression, and the audit boundary is not
            // drawn around the local loop that triggered it. The check moved BELOW the union
            // computation (it read `existing.contains(key)` above it and returned before there was
            // anything to compare), so a real row at this hash now withholds the entry only when it
            // already carries everything the union knows. A foreign key is `{owner}#…` with
            // `owner != crate_name`, so a collision needs a row published under another package's
            // namespace — obligation 2 is exactly the mechanism that mints those, and this leg emits
            // one per (owner, member) pair.
            //
            // EFFECTS **AND** COVERAGE, moved in step with the local loop above now that R598 is
            // closed. This leg still has ZERO REACH over the 1,626-crate corpus, so it is safety-only
            // and says so rather than borrowing the local leg's evidence.
            //
            // AND R598 CANNOT REACH THIS LEG AT ALL, which is worth stating rather than leaving to be
            // rediscovered: `foreign_impls` is built by `collect_foreign_trait_impls`, which iterates
            // `im.items` and keys ONE ENTRY PER DECLARED MEMBER (`{owner}#{qual}::{m} -> {ty}::{m}`).
            // It never asks "does `Ty` have something called `m`" — the asymmetry between the two
            // halves of this rung WAS R598, in the same shape as R513.
            if existing.contains(key) {
                let (eff_adds, blind_adds) = union_adds_over_row(&entries, key, &inf_u, &blind_u);
                if candor_report::env::trace("CANDOR_R590_INSTR") {
                    eprintln!("R590FOREIGN {key} adds_eff={eff_adds} adds_blind={blind_adds}");
                }
                let don_adds = entries.iter().find(|e| e.hash == *key)
                    .is_some_and(|real| don_u.iter().any(|k| !real.dispatches_on.contains(k))); // R1324
                if !eff_adds && !blind_adds && !don_adds {
                    continue; // the real entry already carries every EFFECT and every package it knows
                }
            }
            if inf_u.is_empty() && blind_u.is_empty()
                && candor_report::env::trace("CANDOR_R609_INSTR")
            {
                eprintln!("R609HIT\tFOREIGN\t{key}"); // §E1 REACH PROBE — see the local leg's note
            }
            if inf_u.contains("Unknown") && why_u.is_empty() {
                why_u.insert(r529_reason(key)); // R1268 — non-empty by construction (see `union_why`)
            }
            entries.push(ReportEntry {
                func: key.split_once('#').map(|(_, m)| m.to_string()).unwrap_or_default(),
                inferred: inf_u.iter().map(|s| s.to_string()).collect(),
                unresolved: inf_u.contains("Unknown"),
                hash: key.clone(),
                invisible: blind_u.into_iter().collect(),
                interface_union: true,
                unknown_why: if inf_u.contains("Unknown") { why_u.into_iter().collect() } else { Vec::new() },
                dispatches_on: don_u.into_iter().collect(), // R1324 — §4 ⟨0.40⟩ union clause, spelling (a)
                ..Default::default()
            });
        }
    }
    // ⟨0.39⟩ TIE-BREAK ON THE HASH, or the report is not byte-stable. `func` alone stopped being a total
    // order the moment obligation 2 let one package publish interface-union entries under SEVERAL owning
    // crates: `reqwest` emits `Service::call` under BOTH `tower#` and `tower_service#`, the two rows tie
    // on `func`, and a stable sort then preserves whatever order `foreign_impls`' HashMap iteration
    // happened to produce — so two runs of ONE binary over ONE crate gave different bytes with an
    // identical row multiset. Measured live on reqwest-0.13.5. `hash` is `crate#qual` and equals `func`'s
    // own qualification for every ordinary row, so this can only break ties, never reorder.
    //
    // SOUNDNESS R597 — AND ON `interfaceUnion` AFTER THAT, because a union entry may now sit BESIDE the
    // real row that claims its hash, so `func` AND `hash` both tie. Insertion order happens to settle it
    // (the real row is pushed in an earlier phase and the sort is stable), but that is a property of two
    // distant code paths rather than of this comparator — and `trait_decls` is a HashMap, so "happens to"
    // is the word the reqwest instance above was also true of until it wasn't. `false < true` puts the
    // analysed unit first, which is the order a reader expects.
    // ⟨0.40⟩ SOUNDNESS R1293 — ONE ABSTRACTION, TWO SPELLINGS. Obligation 2 keys a foreign implementor by the trait path
    // its `impl` WROTE, so futures-util publishes `futures_core#Stream::poll_next` (impls importing the re-export) BESIDE
    // `futures_core#stream::Stream::poll_next` (impls importing the module path) — one trait, two contributor sets, and a
    // consumer joins only the key it spelled. The other key's contributors are implementors this entry cannot attribute,
    // so where both carry `Unknown` each names the other (`dep:<its key>`, §3.1's `unresolved` for a contributor whose
    // classes the producer cannot complete). Same owner, same `Trait::member` tail; reasons only.
    {
        let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if !e.interface_union || !e.inferred.iter().any(|x| x == "Unknown") {
                continue;
            }
            let Some((owner, mem)) = e.hash.split_once('#') else { continue };
            if owner == crate_name {
                continue;
            }
            if let Some(t2) = tail2(mem) {
                groups.entry((owner.to_string(), t2)).or_default().push(i);
            }
        }
        for idxs in groups.values().filter(|v| v.len() >= 2) {
            let keys: Vec<String> = idxs.iter().map(|&i| entries[i].hash.clone()).collect();
            for &i in idxs {
                for k in &keys {
                    if *k != entries[i].hash {
                        let w = format!("dep:{k}");
                        if !entries[i].unknown_why.contains(&w) {
                            entries[i].unknown_why.push(w);
                        }
                    }
                }
                entries[i].unknown_why.sort();
            }
        }
    }
    // R1293 — one `dep:` token is enough for the `unresolved` class (§3.1: "one raw reason per class"); a union over many
    // unnamed claimants (rayon's `drive_unindexed`) would otherwise carry dozens. The first, so the bytes are stable.
    for e in entries.iter_mut().filter(|e| e.interface_union) {
        let mut seen_dep = false;
        e.unknown_why.sort();
        e.unknown_why.retain(|w| !w.starts_with("dep:") || !std::mem::replace(&mut seen_dep, true));
    }
    entries.sort_by(|a, b| {
        a.func
            .cmp(&b.func)
            .then_with(|| a.hash.cmp(&b.hash))
            .then_with(|| a.interface_union.cmp(&b.interface_union))
    });

    let meta = candor_report::ReportMeta {
        version: format!("scan-{}", env!("CARGO_PKG_VERSION")),
        toolchain: "stable".into(),
        spec: candor_report::SPEC_VERSION.into(),
    };
    // ⟨0.15 staged⟩ the `coverage` envelope field (spec §2): the κ ledger as data, so "what the scan
    // couldn't see" travels WITH the report instead of evaporating on stderr. Omitted when empty —
    // a fully-covered scan's report is byte-identical to a ⟨0.14⟩ one (wire-compatible rung).
    let coverage = (!coverage_ledger.is_empty()).then(|| candor_report::Coverage {
        uncovered: coverage_ledger
            .iter()
            .map(|(cr, n)| candor_report::CoverageEntry { name: cr.clone(), calls: *n })
            .collect(),
    });
    // ⟨0.21⟩ COMPLETENESS MANIFEST (Gap 1): the analyzed universe = every fn candor formed a judgment for =
    // `all` (the §2.2 callgraph node set, pure leaves included — NOT the effectful-only `entries`). count lets
    // a bare-envelope consumer compute the pure count (count − |functions|); digest fingerprints the set for
    // same-engine re-scan agreement. Recorded toward the --gate-json verdict too (analyzed:{count}).
    let analyzed = {
        // SOUNDNESS R1420(a) — a default's instance is not a unit (`analyzed.count == |callgraph nodes|`).
        let mut sorted: Vec<String> =
            all.iter().filter(|q| !inst_name.contains_key(q.as_str()) && !copath.contains_key(q.as_str())).cloned().collect();
        sorted.sort();
        candor_report::Analyzed { count: sorted.len(), digest: candor_report::fnv1a_hex(&sorted) }
    };
    // …BUT NOT FROM THE PEEK, WHICH IS A NESTED SCAN OF FILES THIS RUN EXCLUDED.
    //
    // The ⟨0.29⟩ peek re-enters `scan_one` over the excluded set to answer `outOfScope`, and its
    // contract is stated in the commit that added it: READ THE EXCLUDED FILES, CHANGE NO VERDICT.
    // `record_gate_analyzed` accumulates (`+= count`) into a process-global, so the peek's units were
    // landing in the --gate-json verdict — while the peek writes no report, so `gate --report` could
    // never see them. MEASURED on `crates/candor-query`: the scan route said `analyzed.count 276`, the
    // report it had just written said 129, and CI's §3.1 byte-equality row failed on 20 of 54 rows.
    //
    // The scan route is the WRONG one, twice over. `analyzed.count` is IN the verdict document, so
    // inflating it IS a verdict change — the peek breaking its own contract. And the count is the ⟨0.21⟩
    // completeness manifest, "every fn candor formed a judgment for": the peek forms no judgment the
    // gate uses, so counting its units tells a consumer 276 things were judged when 129 were. That is
    // the over-claim direction, which is the one that matters.
    //
    // `unanalyzed_units` is suppressed on the same branch and it is the sharper half: an excluded file
    // that fails to parse would otherwise push the RUN to `incomplete: true` / exit 2 — the peek turning
    // a deliberate exclusion into a failed gate, which is a verdict change in the loudest possible form.
    // The peek reads its own unanalyzed set out of the returned report body (see `peek_unread` below),
    // never out of this global, so nothing downstream loses information.
    crate::gate::record_gate_analyzed(analyzed.count, &unanalyzed_units);
    // ⟨typeSurface.returns⟩ THE PRODUCER (DEP-RECEIVER-TYPING-DESIGN.md half 2). A consumer cannot type
    // `let c = deplib::build()` because `build` is PURE and therefore absent from this report entirely —
    // publishing its return type is the only way that key can ever be formed.
    let mut type_surface = build_type_surface(&crate_name, &fns, &entries);
    // ⟨0.40⟩ THE DECLARED-TYPE SURFACE (SPEC §2 ⟨0.40⟩, SOUNDNESS R843): `holds`, `returnsProtocol`, the
    // `types` manifest and `adds`, from Pass A's per-file `FileSurface`s. See `typesurf.rs`.
    {
        let files: Vec<&crate::typesurf::FileSurface> = decls_per_file.iter().map(|(_, _, fd)| &fd.ts).collect();
        let ctx = crate::typesurf::BuildCtx {
            crate_name: &crate_name,
            deps: &deps,
            renames: &dep_renames,
            chained: &deps_idx.surface,
            incomplete: !unanalyzed_units.is_empty(),
        };
        crate::typesurf::build(&ctx, &files, &fns, &mut type_surface);
        // §E1 REACH, producer side: how much of the manifest is KIND-ONLY is the number SPEC §2 ⟨0.40⟩
        // records as unmeasured ("how many rust types lose `supers` this way").
        if r843_probe {
            let ko = type_surface.types.values().filter(|t| t.supers.is_none()).count();
            let why: BTreeSet<&str> = files.iter().flat_map(|f| f.unclosable.iter().map(String::as_str)).collect();
            eprintln!("R843TS\t{crate_name}\ttypes={}\tkindonly={ko}\tholds={}\trproto={}\tadds={}\twhy={}",
                type_surface.types.len(), type_surface.holds.len(), type_surface.returns_protocol.len(),
                type_surface.adds.len(), why.into_iter().collect::<Vec<_>>().join("|"));
        }
    }
    // ⟨0.29⟩ THE SCOPE, aggregated by class. The REASON is the engine's own rationale, verbatim in
    // substance from the comment at each `continue` — a consumer reads it to decide whether the exclusion
    // matches the question they are asking, so paraphrasing it into something vaguer would defeat the
    // block. Counts, never file lists: `build-output` covers `target/`, which is unbounded.
    // ── ⟨0.29⟩ THE PEEK ────────────────────────────────────────────────────────────────────────────
    // Read the files this scan deliberately did NOT judge, and say so when they hold an effect the
    // policy DENIES. ⟨0.30⟩ THE GATE'S VERDICT DOES MOVE — a non-empty block is `ok:false,
    // incomplete:true` at exit 2 (see the exit arm below) — though never as a VIOLATION, because a file the
    // gate declined to judge must not decide an exit code.
    //
    // A RECURSIVE `scan_one`, not a hand-written second pass. That is the design constraint, not an
    // implementation convenience: a bespoke walk over `build.rs` would be a SECOND OPINION, and a
    // drifted second opinion reported as a warning is worse than no warning — the reader cannot tell a
    // real finding from a disagreement between two code paths. Reusing the entry point makes "same
    // classifier, different file set" true by construction. `peek_excluded` suppresses recursion, so a
    // peek never peeks.
    //
    // POLICY-SCOPED, AND BOUNDED BY THE POLICY, which is what keeps it quiet: no policy ⇒ no peek ⇒ not
    // one new line; `deny Net` ⇒ nothing said about an `Exec` in the test tree. Without that bound the
    // noise floor is "everything you excluded", and a gate that prints noise is one people scroll past.
    // ⟨0.29⟩ DID THE PEEK ACTUALLY READ THESE FILES? `peeked` was a static fact about the exclusion
    // CLASS, so a peek that never ran — or ran and produced nothing readable — still published
    // `peeked: true` beside `outOfScope: []`, which is byte-identical to a clean peek. That is the
    // ⟨0.26⟩ partial-manifest failure inside the rung built to prevent it: the flag exists precisely so
    // `[]` cannot overclaim, and deriving it from a lookup table made it incapable of doing that job.
    // It is an OUTCOME now, set only where the recursion returned a report this run could parse.
    // ⟨0.29⟩ …AND DID IT READ THEM ALL? A report the parent could parse is not the same fact as every
    // excluded file having been opened. The child publishes its own `unanalyzed` (the ⟨0.21⟩ completeness
    // manifest) and the parent read only `functions`, so an excluded file that FAILED TO PARSE inside the
    // peek produced `peeked: true` beside `outOfScope: []` — the same overclaim one paragraph up, one
    // level down, and the child's stderr warning is the only reason a human could notice at all while the
    // machine consumer reads the opposite. `peeked` is per CLASS, so the answer is too: a class is peeked
    // only when no file of that class went unread.
    let mut peek_read = false;
    let mut peek_unread: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut peek_unattributed = false;
    // ⟨0.32⟩ DID THE PEEK ACTUALLY TRY? `peeked: false` has TWO causes — "opened it and failed" and
    // "never asked" — and only the first is evidence of unread code. MEASURED as a defect in the first
    // cut of this rung: the peek short-circuits when the policy carries no DENY rule (`rules` is the deny
    // list; allow/forbid/only live in their own), so an `allow`-only policy left every excluded class
    // `peeked: false` and a perfectly READABLE build.rs was refused with "this scan did not READ it" —
    // false as a statement, and it refused three whole policy shapes on any tree with an exclusion. The
    // flag is set where the reads actually begin, so it cannot drift from the short-circuit above it.
    let peek_attempted = std::cell::Cell::new(false);
    // ⟨0.33⟩ THE QUESTION THE PEEK WAS PUT (SPEC §2 ⟨0.33⟩) — the deny rules THIS scan HELD, in their
    // canonical expanded form. A `RefCell` rather than a return value because it must be set BEFORE
    // either short-circuit inside the closure below (an `allow`-only policy, or nothing excluded to look
    // at): the report must say *a policy stood and it denied nothing* (present-and-empty) in exactly
    // those cases, never *nothing was asked* (absent) — the same emission rule `outOfScope` follows, and
    // deliberately the same reasoning: present-and-empty and absent are different claims (⟨0.26⟩).
    let scanned_under_rules: std::cell::RefCell<Option<Vec<String>>> = std::cell::RefCell::new(None);
    // ⟨peek-scope-attribution⟩ Invert `calls` (caller -> callees) once, so the out-of-scope block below
    // can walk from a DIRECT dispatcher (`direct_dispatchers`) up to every in-scope ANCESTOR that could
    // reach it — a policy scoped several hops above the dispatch site (`deny Net App`, where `App::main`
    // calls `Service::run` calls `Runner::dispatch(&dyn Doer)`) needs the same treatment a normal,
    // non-excluded effect gets for free: propagation already carries an ordinary callee's effect up
    // through every intermediate caller before the gate ever tests a scope string. This mirrors that for
    // the one edge the primary scan cannot see (the excluded implementor), without re-running propagation.
    let mut rev_calls: HashMap<&str, Vec<&str>> = HashMap::new();
    for (caller, callees) in &calls {
        for callee in callees {
            rev_calls.entry(callee.as_str()).or_default().push(caller.as_str());
        }
    }
    // SOUNDNESS R525 — THE POLICY'S OWN VOCABULARY, DISCOVERED **ONCE** FOR EVERY CONSUMER IN THIS
    // FUNCTION. It used to be discovered only at the gate arm ~700 lines below, so the ⟨0.29⟩/⟨0.30⟩ PEEK
    // directly beneath this line parsed the same policy through the alias-LESS `parse_policy` — an empty
    // vocabulary. `deny Unknown[<a config alias>]` is then an unrecognised reason-class, which is FATAL,
    // so `fatal_messages()` came back non-empty, the peek's refusal arm returned `None`, and the ⟨0.30⟩
    // fail-closed verdict never armed. MEASURED on 0.39.0 over a crate whose `build.rs` opens a
    // `TcpStream`, with `unknown-alias corp = reflect` in the `.candor/config`:
    //
    //     deny Net                      → exit 2, `outOfScope` names build::grab, `peeked: true`
    //     deny Net + deny Unknown[corp] → exit 0 "policy ✓", `outOfScope` ABSENT, `peeked: false`
    //     deny Net + deny Unknown[reflect] (a BUILTIN class — the control) → exit 2, as the first
    //
    // So an UNRELATED extra rule, one the gate route honoured perfectly, silently erased the disclosure
    // the first policy made and turned a red verdict green. The control is what rules out "a second rule"
    // as the cause and pins it on the config-defined alias: the peek and the gate were reading one file
    // through two different vocabularies.
    //
    // ONE CALL, NOT TWO. The gate arm below now reads these bindings instead of re-discovering, because
    // this is the ⟨0.24⟩ "peek's parse IS the matcher" rule one level out — two `discover_config` calls
    // are two chances to anchor differently, which is the precise defect ⟨0.24⟩ closed between the scan
    // route and `gate --report`.
    //
    // ANCHORED AT THE POLICY FILE (SPEC §3.1) and GATED ON THE POLICY BEING READABLE — both deliberate.
    // The anchor is ⟨0.24⟩'s ruling, carried from the gate arm's comment below. The readability filter
    // keeps the PRECONDITION byte-identical to the one the gate arm already had: an unreadable policy
    // still refuses through its own arm below, with its own message, rather than being pre-empted by a
    // config refusal raised on behalf of a policy that was never going to be honoured.
    let policy_cfg_vocab: Option<(std::path::PathBuf, String)> = policy_path
        .as_ref()
        .filter(|pp| std::fs::read_to_string(pp).is_ok())
        .and_then(|pp| candor_classify::policy::discover_config(std::path::Path::new(pp)));
    let policy_unknown_aliases = policy_cfg_vocab
        .as_ref()
        .map(|(_, t)| candor_classify::policy::parse_unknown_aliases(t))
        .unwrap_or_default();
    let out_of_scope: Option<Vec<candor_report::OutOfScopeFinding>> = policy_path
        .as_ref()
        .and_then(|pp| std::fs::read_to_string(pp).ok())
        .and_then(|text| {
            // R525: `..._with_aliases`, never bare `parse_policy`. NOT `parse_policy_silent`: the `warn`
            // flag is left exactly as it was, so this change moves the VOCABULARY and nothing else — a
            // quieted parse here would be a second, invisible behaviour delta riding a soundness fix.
            let parsed =
                candor_classify::policy::parse_policy_with_aliases(&text, &policy_unknown_aliases);
            // ⟨0.29⟩ A REFUSED POLICY LEAVES THE KEY ABSENT (SPEC §2). `and_then`, not `map`, because the
            // distinction this returns is present-vs-absent and `map` cannot express it. The peek is a
            // producer reading the policy, so §3.1 binds it exactly as it binds the gate: over a policy
            // no route will honour, `outOfScope: []` claims a look taken against rules that never stood —
            // and the denied set it would have looked for is the parser's SALVAGE of an unhonourable
            // file, which is the rewriting `fatal_messages` exists to refuse. When this rung landed only
            // candor-java withheld here; all four engines do now (this `return None` is rust's half).
            if !parsed.fatal_messages().is_empty() {
                return None;
            }
            // ⟨0.33⟩ RECORDED HERE — after the policy is known to STAND, before either short-circuit
            // below — from `parsed.rules`, the very list the peek matches with two lines down. A
            // main-thread re-parse of the same file would be a second opinion about one file, the shape
            // ⟨0.29⟩ already forbids one level down (candor-java's reference comment makes the same
            // point: "the peek's parse IS the matcher").
            *scanned_under_rules.borrow_mut() =
                Some(candor_classify::policy::canonical_deny_set(&parsed.rules));
            // ⟨0.30⟩ THE TRIGGER IS "ARE THERE DENY RULES", and the matcher below is the GATE'S OWN.
            // This flattened the rules into a set of effect NAMES, which §6.2 already forbids ("THE GATE
            // AND THE DISCLOSURE MUST APPLY THE SAME RULE, AND SHOULD SHARE THE SAME CODE") and which was
            // wrong in BOTH directions once ⟨0.30⟩ made the block verdict-bearing — both MEASURED
            // four-way in review:
            //
            //   UNDER-REPORT: `pure` is a rule with an EMPTY effect list meaning "every effect except
            //   Unknown". Flattened it contributed NOTHING, so the set was empty, the peek never ran, and
            //   the STRICTEST policy passed at exit 0 while the weaker `deny Exec` exited 2 on the same
            //   tree. A four-way false all-clear.
            //
            //   OVER-CHARGE: `deny Net[known-partner]` denies one destination class; the name set held
            //   bare "Net", so a peeked fn reaching an UNKNOWN host — which that rule does not deny —
            //   turned the verdict red while the identical code IN scope passed. Rule SCOPES were dropped
            //   identically.
            if parsed.rules.is_empty() || excluded.is_empty() {
                return Some(Vec::new());   // the policy STOOD — asked-and-clear, key present
            }
            peek_attempted.set(true);
            let class_of: std::collections::BTreeMap<&str, &str> =
                excluded.iter().map(|(p, c)| (p.as_str(), *c)).collect();
            // ⟨0.31⟩ NOTHING THE PEEK SEES MAY REACH THE VERDICT — see `while_peeking` in gate.rs.
            // The peek's findings are not lost: it RETURNS a report body and this frame reads it, which
            // is how `outOfScope` has always worked. The peek is a source of data, never a writer of
            // verdict state.
            // ⟨peek-scope-attribution⟩ CLEARED, not merely about-to-be-overwritten: a peek that
            // short-circuits inside `scan_one` before reaching its OWN `type_to_traits` build (nothing
            // excluded, or a parse abort) would otherwise leave THIS thread's PRIOR value in place —
            // stale data from a different workspace member or a different `cargo test` case sharing the
            // thread, read below as if it were this crate's own.
            crate::gate::PEEK_TYPE_TO_TRAITS.with(|c| c.borrow_mut().clear());
            let (_, peeked) = crate::gate::while_peeking(|| scan_one(dir, ScanOpts {
                prefix: format!("{prefix}.peek"),
                want_json: true,
                include_tests,
                policy: None,          // the peek ASKS nothing; it only reports what it saw
                baseline: None,
                // ⟨0.31⟩ the peek analyses the INVERSE file set, so an unevaluable-target refusal is
                // meaningless here and would refuse the very look that names the finding.
                ws_member: true,
                quiet: true,
                deps_idx,
                peek_excluded: true,
            }, run));
            // ⟨peek-scope-attribution⟩ The peek's OWN trait-implementation index (built from files ONLY
            // the peek walked), read back via the same-thread side channel `while_peeking` already
            // establishes is safe for this recursion shape. Used below to learn which trait(s) an
            // excluded declaration's owning type implements, so its scope test can be widened to every
            // in-scope function that dispatches on that trait — see `direct_dispatchers`/`rev_calls`
            // above and their use a few dozen lines down.
            let peek_type_to_traits = crate::gate::PEEK_TYPE_TO_TRAITS.with(|c| c.borrow().clone());
            let Some(body) = peeked else { return Some(Vec::new()) };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { return Some(Vec::new()) };
            peek_read = true;   // the recursion returned a report this run could read — see `peek_read`
            for u in v["unanalyzed"].as_array().into_iter().flatten() {
                let path = u["path"].as_str().unwrap_or("");
                match class_of
                    .iter()
                    .find(|(p, _)| !path.is_empty() && path.ends_with(*p))
                {
                    Some((_, c)) => { peek_unread.insert((*c).to_string()); }
                    // The peek walks ONLY excluded files, so an unread path that matches no exclusion is a
                    // file this code cannot attribute. Fail closed across every class rather than let one
                    // unattributable file leave all of them claiming completeness.
                    None => peek_unattributed = true,
                }
            }
            let mut out = Vec::new();
            for f in v["functions"].as_array().into_iter().flatten() {
                // SOUNDNESS R511 — THE PEEK READS A REPORT, AND A REPORT NOW CARRIES ROWS THAT ARE NOT
                // UNITS. ⟨0.39⟩ un-gated ⟨0.23⟩, so the recursion's own body contains synthetic
                // `interfaceUnion` rows: the union over an abstraction member's implementors, with no
                // body and `loc: ""`. Everything below treats `f` as a judged declaration — it
                // scope-matches `f["fn"]`, and it derives the finding's `path` and its exclusion CLASS
                // from `f["loc"]`. A union row has no `loc`, so `path` came out EMPTY, `class` fell back
                // to the literal "excluded" (a class that appears nowhere in the `excluded` array this
                // same run publishes), and the finding's own reason spoke of "candor's ANALYSIS of that
                // file" while naming no file.
                //
                // VERDICT-AFFECTING, in the fail-closed direction — the same defect candor-swift
                // measured and fixed first. MEASURED here on a package whose EXCLUDED `examples/` tree
                // implements a dependency's trait: under `deny Net Backend` (a rule scoped to the trait
                // name, which the real implementor `examples::ex::Crossterm::size` does not match) the
                // ONLY `outOfScope` finding was the synthetic row, and ⟨0.30⟩ turns a non-empty
                // `outOfScope` into INCOMPLETE — exit 2. The control, byte-identical but for the scope
                // token (`deny Net Zzz`), exits 0. A green gate was turned red by a row with no body.
                //
                // The implementors the row unions over are in the peek's own `functions` array with
                // their real `loc`, so every effect this drops is still disclosed by the entry that
                // actually performs it: the unscoped arm published the union row BESIDE
                // `examples::ex::Crossterm::size`, not instead of it.
                if f["interfaceUnion"].as_bool().unwrap_or(false) {
                    continue;
                }
                let inferred: Vec<&str> = f["inferred"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str())
                    .collect();
                let qual = f["fn"].as_str().unwrap_or("");
                let net_classes: Vec<String> = f["netClass"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str())
                    .map(String::from)
                    .collect();
                let reason_classes: std::collections::BTreeSet<String> = f["unknownWhy"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str())
                    .map(String::from)
                    .collect();
                // ⟨peek-scope-attribution⟩ THE FIX. Before ⟨0.34⟩, the scope test just below ran ONLY
                // against `qual` — the peeked (excluded) declaration's OWN name — which a policy scoped
                // to an in-scope CALLER can never match: `deny Net Runner` tests `scope_matches("EvilDoer
                // ::work", "Runner")`, always false, so the excluded conformer's Net was never disclosed
                // under that rule even though `Runner::dispatch(&dyn Doer)` is precisely the code path
                // that reaches it (SPEC §2 ⟨0.29⟩ peek; BACKLOG "a peek finding is scope-matched against
                // the WRONG ENTITY"). candor-swift's `7378f4f` hit the analogous case via CHA-union;
                // rust's peek never unions the excluded file into any in-scope analysis at all, so the
                // fix here is smaller — no re-analysis, no union. It reuses two facts this run already
                // has lying around: `direct_dispatchers` (which in-scope fns dispatch on which
                // trait+method, recorded during THIS scan's own Pass B, `FnInfo::dispatch`) and
                // `peek_type_to_traits` (which trait(s) THIS excluded declaration's owning type
                // implements, learned from the peek's own — separate — parse of the excluded file, never
                // from re-analysing anything in-scope). Neither requires the primary scan to have ever
                // seen the excluded file's contents.
                //
                // `extra_scope_names` is every in-scope fn NAME that could reach this exact excluded
                // declaration through dynamic dispatch: for each trait the owning type implements, look
                // up who directly dispatches on (trait, this fn's own method leaf), then walk `rev_calls`
                // to include every transitive ANCESTOR of a direct dispatcher too — a policy scoped
                // several hops above the dispatch site must see it exactly as it would if this fn's own
                // effect had propagated there normally (see `rev_calls`'s doc comment). Attribution is
                // UNCHANGED: the finding pushed below still names `qual` (the excluded declaration
                // itself), never a caller — only which RULES are considered to have scope-matched it
                // widens. A rule that denies nothing this fn performs still contributes nothing, exactly
                // as before.
                let method_leaf = qual.rsplit("::").next().unwrap_or(qual);
                let owner_leaf = qual
                    .rsplit_once("::")
                    .map(|(rest, _)| rest)
                    .and_then(|s| s.rsplit("::").next())
                    .unwrap_or("");
                let mut extra_scope_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
                if let Some(implemented) = peek_type_to_traits.get(owner_leaf) {
                    for tr in implemented {
                        if let Some(direct) = direct_dispatchers.get(&(tr.clone(), method_leaf.to_string())) {
                            extra_scope_names.extend(crate::lang::reaching_ancestors(
                                direct.iter().map(|s| s.as_str()),
                                &rev_calls,
                            ));
                        }
                    }
                }
                // ⟨0.30⟩ the gate's own firing decision, per (rule, function) — scope test here, exactly
                // as `candor_classify::gate` does it, then `rule_hits` for what the rule charges.
                let mut hit_set: std::collections::BTreeSet<String> = Default::default();
                for r in &parsed.rules {
                    if let Some(sc) = &r.scope {
                        let own_matches = candor_classify::policy::scope_matches(qual, sc);
                        let reached_matches = extra_scope_names
                            .iter()
                            .any(|caller| candor_classify::policy::scope_matches(caller, sc));
                        if !own_matches && !reached_matches {
                            continue;
                        }
                    }
                    let rh = candor_classify::gate::rule_hits(
                        r,
                        &inferred,
                        if reason_classes.is_empty() { None } else { Some(&reason_classes) },
                        &net_classes,
                    );
                    for h in rh.hits {
                        hit_set.insert(h.to_string());
                    }
                }
                let hits: Vec<String> = hit_set.into_iter().collect();
                if hits.is_empty() {
                    continue;
                }
                let loc = f["loc"].as_str().unwrap_or("");
                let path = loc.split(':').next().unwrap_or("").to_string();
                let class = class_of
                    .iter()
                    .find(|(p, _)| !path.is_empty() && path.ends_with(*p))
                    .map(|(_, c)| (*c).to_string())
                    .unwrap_or_else(|| "excluded".to_string());
                out.push(candor_report::OutOfScopeFinding {
                    func: f["fn"].as_str().unwrap_or("").to_string(),
                    path,
                    effects: hits,
                    reason: format!(
                        "OUTSIDE this scan's scope ({class}) — the gate did NOT judge it. \
                         candor's ANALYSIS of that file reaches this effect, so the verdict above is \
                         INCOMPLETE rather than a pass (an analysis result, not a claim about what the \
                         code does at runtime)."
                    ),
                    class,
                });
            }
            out.sort_by(|a, b| (&a.path, &a.func).cmp(&(&b.path, &b.func)));
            Some(out)
        });
    // ⟨0.33⟩ …materialized. `None` here is reachable only where `out_of_scope` is also `None` (a refused
    // policy, or none configured) — the same emission rule, applied to the same `RefCell`, so the two
    // keys cannot drift into "asked" and "not asked" over one run.
    let scanned_under: Option<candor_report::ScannedUnder> =
        scanned_under_rules.into_inner().map(|deny| candor_report::ScannedUnder { deny });

    let excluded_classes: Vec<candor_report::ExcludedClass> = {
        let mut by_class: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for (_, class) in &excluded {
            *by_class.entry(class).or_insert(0) += 1;
        }
        by_class
            .into_iter()
            .map(|(class, count)| candor_report::ExcludedClass {
                class: class.to_string(),
                count,
                // The peek is THIS walk with the selection inverted, so it reads every class this engine
                // excludes — but only if it RAN. `peek_read` is false when no policy was configured, when
                // the policy denied nothing, or when the recursion produced nothing readable, and in each
                // of those cases no file of any class was opened. `peek_unread` subtracts the classes the
                // peek RAN over and could not read — parse failures are per file, so the claim is per class.
                peeked: peek_read && !peek_unattributed && !peek_unread.contains(class),
                // ⟨0.32⟩ the producer's own carve-out: `build-output` is a DERIVED copy of code this
                // scan already judged, so it hides nothing. `build-script` is NOT — `build.rs` runs on
                // every `cargo build` and can perform any effect, which is the case this file's own
                // ExcludedClass doc records going GREEN under `deny Exec`. It must fail closed.
                judged_elsewhere: class == "build-output",
                reason: match class {
                    "build-script" => "the Cargo build script runs at COMPILE time, not as the crate's \
                         runtime behaviour, so this scan does not judge it — but it runs on every \
                         `cargo build`"
                        .to_string(),
                    "non-library-target" => "tests/, benches/ and examples/ describe what the crate's \
                         HARNESS does, not what the crate does; --include-tests keeps them"
                        .to_string(),
                    // R457 — this used to read "…whose test-ness is declared at the `mod` site,
                    // invisible when walking files", which was both the rationale and the defect: the
                    // rule trusted the NAME and dropped production source that merely looked like a
                    // test module. The `mod` site is not invisible, so it is now read, and this text
                    // states the EVIDENCE that was found rather than the convention that was assumed.
                    // R459 — and the ANCESTOR clause, which the previous wording did not cover: a
                    // `#[cfg(test)]` DIRECTORY module takes its children with it, so a file can be
                    // excluded on evidence that sits two levels up rather than at its own `mod` site.
                    "test-module" => "a #[cfg(test)] file module: the declaring `mod` (or an enclosing \
                         module's) carries a cfg that cannot hold in a non-test build, or the file's own \
                         #![cfg(test)] says so, or no declaration was found for a `tests.rs`/`*_test.rs` \
                         file; --include-tests keeps them"
                        .to_string(),
                    "build-output" => "target/ and hidden directories hold build artifacts and tooling, \
                         not library code"
                        .to_string(),
                    other => format!("excluded by the scanner ({other})"),
                },
            })
            .collect()
    };
    // ⟨0.31⟩ the provenance of an ambient `net-partner` that moved a classification. Omitted when nothing
    // participated, so a project declaring no partners — or declaring some that never matched — stays
    // byte-identical to a pre-rung report: a declaration that changed nothing is not provenance.
    let net_partners_record = if partners_used.is_empty() {
        None
    } else {
        candor_classify::policy::discover_config(std::path::Path::new(dir)).map(|(cfg, _)| {
            candor_report::NetPartners {
                config: cfg.to_string_lossy().into_owned(),
                hosts: partners_used.iter().cloned().collect(),
            }
        })
    };
    // …AND NOT FROM THE PEEK, for the same reason `record_gate_analyzed` above is guarded — this is the
    // identical defect one key over. The peek runs with `policy: None`, but `net_partners_record` does not
    // come from the policy: it comes from `partners_used` plus `discover_config(dir)`, and the peek scans
    // the SAME dir, so it finds the same config and accumulates any partner host reached from a file this
    // scan deliberately excluded. MEASURED on a crate whose only mention of the declared partner is in
    // `build.rs`: the verdict said `netPartners: [{hosts: ["partner.example"]}]` while the report it had
    // just written said `null`.
    //
    // Both halves of that are the failure the FIRST net-partner attempt was reverted for. §3.1 route
    // equality breaks, because `gate --report` reads the report and can only ever answer `null`. And the
    // disclosure over-claims in the direction that matters: it says an ambient declaration moved this
    // run's classification when the gate judged nothing that used it.
    crate::gate::record_gate_net_partners(net_partners_record.as_ref());
    // SOUNDNESS R1578 — the dependency sources read to type a value (the owner's ruling: what may add a charge is recorded).
    let dependency_sources: Vec<candor_report::DependencySource> = r1578_sigs
        .borrow()
        .consulted
        .values()
        .filter(|d| d.typed > 0)
        .map(|d| candor_report::DependencySource {
            package: d.package.clone(), version: d.version.clone(), origin: d.origin.to_string(), typed: d.typed,
        })
        .collect();
    let body = candor_report::to_packaged_report_json_typed_src(
        &meta, &crate_name, &entries, coverage.as_ref(), &unanalyzed_units, Some(&analyzed),
        Some(&type_surface), &excluded_classes, out_of_scope.as_deref(), scanned_under.as_ref(),
        net_partners_record.as_ref(), &dependency_sources)
        .unwrap_or_default();
    // With want_json the body is RETURNED to the caller (which prints one document for a single
    // crate, or wraps N members in a JSON array) rather than printed here — printing per-call gave
    // concatenated, unparseable JSON for a workspace scan.
    let json_body = if want_json {
        Some(body.clone())
    } else {
        // ⟨0.31⟩ AN UNEVALUABLE TARGET IS A REFUSAL, NOT A CLEAN SCAN.
        //
        // The walk admitted NOTHING this engine can read — no `.rs` under the target at all. This engine
        // answered `policy ✓` at exit 0 here, which is a permanent green for a typo'd CI path; candor-ts,
        // candor-swift and candor-java all refuse the same shape. This engine ALREADY refuses a target
        // that does not exist, for the reason its own comment gives ("a typo'd path in CI is a PERMANENT
        // GREEN") — an existing path holding nothing readable is that same green one step along.
        //
        // THREE PLACEMENT CONSTRAINTS, each of which broke a previous attempt at this fix:
        //
        //  1. AFTER THE PEEK. `out_of_scope` is computed above; if the ⟨0.30⟩ peek named a denied effect
        //     in an excluded file (a `build.rs` running `curl`), that finding is the answer and the run
        //     reports it and exits 2 through the scope arm, WITH a report carrying `outOfScope`.
        //  2. BEFORE THE ENVELOPE. §3.1's byte-equality is quantified over "any report a scan produced",
        //     so a refusal that leaves a report hands `gate --report` an answer that disagrees with this
        //     exit code. Attempt #1 exited 2 after the verdict was written and broke exactly that
        //     (`scan --policy` 2 vs `gate --report` 0). Returning here writes no report at all.
        //  3. THE WALK'S FILE SET, not an analyzed count. Attempt #2 keyed on `gate::GATE_ANALYZED` and
        //     made a NORMAL crate with a real `src/lib.rs` exit 2 — that accumulator is not populated on
        //     the simple path and answers a different question. `paths` is what the walk admitted.
        //
        // PER-INVOCATION, NOT PER-MEMBER (`ws_member`): a workspace with one live member and one
        // scaffolded one stays green, and a member that judged nothing still publishes its ⟨0.24⟩
        // count-0 report. A per-member refusal would delete that shape and redden a benign layout — the
        // same reason swift does not refuse per `binary` target and java does not per aggregator module.
        if walk_admitted_files == 0
            && !ws_member
            && out_of_scope.as_deref().is_none_or(|o| o.is_empty())
        {
            if !quiet {
                eprintln!("candor-scan: no Rust sources under {dir}");
                eprintln!("        candor-scan reads `.rs` files — point it at a crate or workspace \
                           directory containing them. Exit 2 (unevaluable): a target this engine cannot \
                           read is not a clean scan.");
            }
            // ⟨0.31⟩ THIS RETURN IS ABOVE THE REPORT WRITE, and two things depend on saying so.
            //
            // (1) `scan_target` latches `mark_out_reports_written()` after `scan_one` returns, on the
            // stated premise that "the report write precedes every return it has". This return breaks
            // that premise, and the cost was measured: with `--out` naming a prefix that already held a
            // previous GREEN report, the placeholder armed correctly, the license was granted anyway,
            // and `disarm` handed the stale green back — after which `gate --report` certified it at
            // exit 0. §3.3.1 ⟨0.28⟩ is explicit that a refusal writes the fail-closed report to every
            // prefix named.
            //
            // (2) The refusal DOCUMENT needs the real cause. Without this the `--gate-json` reason fell
            // through to "the gate config did not load (exit 2)" — affirmatively false here, since the
            // config loaded fine and the TARGET is what could not be read. ⟨0.24⟩ pins that reason as
            // a string naming the cause, and this family rates a false disclosure worse than a missing
            // one: an operator reading it debugs the wrong file.
            crate::gate::note_refused_before_write();
            crate::gate::record_gate_refusal(format!(
                "no Rust sources under {dir} — candor-scan reads `.rs` files; point it at a crate or \
                 workspace directory containing them. Exit 2 (unevaluable): a target this engine \
                 cannot read is not a clean scan."
            ));
            return (2, None);
        }
        let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
        if let Some(parent) = Path::new(&prefix).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = format!("{prefix}.{crate_name}.scan.json");
        // Atomic write (temp + rename): a concurrent `candor-query` / `cargo candor watch` reader must
        // never see a half-written report (see candor_report::write_atomic).
        let _ = candor_report::write_atomic(Path::new(&file), body.as_bytes());
        let cgfile = format!("{prefix}.{crate_name}.scan.callgraph.json");
        let _ = candor_report::write_atomic(Path::new(&cgfile), serde_json::to_string(&cg).unwrap_or_default().as_bytes());
        if !quiet {
            // SOUNDNESS R511 — the count and the breakdown are about FUNCTIONS, so they count the rows
            // that are functions. ⟨0.39⟩'s synthetic `interfaceUnion` rows are the union over an
            // abstraction member's implementors; each is published BESIDE the implementors it unions,
            // so counting it both invents a unit and double-counts that unit's effect. Measured before
            // this filter on a crate with one trait and one effectful impl: "wrote 2 effectful
            // functions … Net 2" over a crate with one. The rows stay in the report — a chained
            // consumer joins on them — they are simply not units to tally.
            let units: Vec<&ReportEntry> = entries.iter().filter(|e| !e.interface_union).collect();
            eprintln!(
                "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
                units.len()
            );
            // Effect breakdown — make the result visible at a glance, not just a count + a file path.
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for e in &units {
                for x in &e.inferred {
                    *counts.entry(x.as_str()).or_insert(0) += 1;
                }
            }
            let breakdown = ["Net", "Llm", "Fs", "Db", "Exec", "Ipc", "Env", "Clipboard", "Clock", "Log", "Rand"]
                .iter()
                .filter_map(|k| counts.get(k).map(|n| format!("{k} {n}")))
                .collect::<Vec<_>>()
                .join(" · ");
            let unknown = counts.get("Unknown").copied().unwrap_or(0);
            if !breakdown.is_empty() || unknown > 0 {
                let u = if unknown > 0 {
                    format!("{}Unknown {unknown} (disclosed)", if breakdown.is_empty() { "" } else { "   ·   " })
                } else {
                    String::new()
                };
                eprintln!("  {breakdown}{u}");
            }
        }
        None
    };

    // The κ-coverage disclosure: dependencies the code demonstrably CALLS that the classifier knows
    // nothing about. Their effects are INVISIBLE — not Unknown — so the report's silence about them
    // is not purity evidence. This turns the curated-κ caveat from a doc footnote into per-scan,
    // named evidence (the argon2 lesson: the blind spot landed on exactly the call a security review
    // cared about). ⟨0.15 staged⟩ the list itself is `coverage_ledger`, computed once above and shared
    // with the report's `coverage` envelope field — this line and the JSON can never disagree.
    if !quiet {
        // ⟨0.15 staged⟩ ...and the same ledger rides the --gate-json verdict as an ADVISORY note
        // (spec §3.3 verb conditionality; verdict-preserving). Target/member scans only — quiet
        // dependency scans under --deps are not the gated surface, and their gaps are exactly what
        // chaining just closed. A no-op unless --gate-json was given.
        record_gate_coverage(&coverage_ledger);
    }
    if !coverage_ledger.is_empty() && !quiet {
        let shown: Vec<String> = coverage_ledger
            .iter()
            .take(8)
            .map(|(cr, n)| format!("{cr} ({n} call{})", if *n == 1 { "" } else { "s" }))
            .collect();
        let more =
            if coverage_ledger.len() > 8 { format!(" + {} more", coverage_ledger.len() - 8) } else { String::new() };
        eprintln!(
            "candor-scan: candor's classifier doesn't cover {} dependenc{} this code calls into — their effects are INVISIBLE to the scan (absent from the report, NOT a claim they're pure): {}{}",
            coverage_ledger.len(),
            if coverage_ledger.len() == 1 { "y" } else { "ies" },
            shown.join(", "),
            more
        );
        // SCAN-COMPLETENESS NUDGE. A high VOLUME of calls into uncovered dependencies is the signature
        // of a scan that is missing an INPUT rather than one whose classifier is imprecise: the code
        // leans hard on crates nothing in this run can see into, so what those crates do is simply
        // absent. Measured on a real 18.7k-fn webapp (the JVM engine, same threshold): scanned app-only
        // it could PROVE Net on 465 functions; re-scanned as the deployed artifact — the app AND its
        // 222 dependency jars — the same gate proved Net on 5,865. The reaches did not become more
        // precise, they became VISIBLE. Here the equivalent move is `--deps` (or chaining sibling
        // reports through CANDOR_DEPS), which is why the remedy names it.
        //
        // The nudge deliberately promises VISIBILITY ONLY — never dispatch resolution. On that same
        // webapp 23 of 26 unresolved dispatches were over the app's OWN broad hierarchies, which no
        // amount of extra dependency code can fix; promising otherwise would be a claim we can't keep.
        //
        // Keyed on the ledger's own per-dep call counts, so this line and the ledger above can never
        // disagree about the evidence. Rides inside the ledger's `!quiet` block: a dependency scan
        // under --deps is not the surface being reported on, and the user is already doing the thing.
        let uncovered_calls: usize = coverage_ledger.iter().map(|(_, n)| n).sum();
        if uncovered_calls >= UNCOVERED_CALLS_NUDGE_MIN {
            eprintln!(
                "candor-scan: hint — {} call{} go into {} dependenc{} that {} not scanned, so their effects are invisible here. If you scanned only your own crate, scan what it depends on too (`--deps` walks the Cargo.lock tree once and chains the reports; CANDOR_DEPS chains ones you already have): those reaches then resolve to DETERMINED effects instead of being absent.",
                uncovered_calls,
                if uncovered_calls == 1 { "" } else { "s" },
                coverage_ledger.len(),
                if coverage_ledger.len() == 1 { "y" } else { "ies" },
                if coverage_ledger.len() == 1 { "is" } else { "are" }
            );
        }
    }

    // The cold-repo hook: after the effect summary + κ ledger, surface the SINGLE most surprising
    // transitive reach + a ready-to-run `candor path` command — so the two-minute demo opener is
    // deterministic, not lucky. Pure call-graph + name analysis (no LLM); honest fallback when
    // nothing clears the bar. See surface.rs / SURFACE-BEST-FIND-DESIGN.md. STDERR only (stdout may
    // carry the JSON report), and AFTER the histogram + κ ledger in output order.
    // The surface opener is for EXPLORATION scans; under an active policy gate the headline is the VERDICT,
    // and a reassuring "nothing hidden" / "most surprising reach" line ABOVE the violation lines reads as a
    // contradiction in a CI log (#18). Suppress it when gating — the gate summary + fix-gate pointer carry.
    if !quiet && policy_path.is_none() {
        crate::surface::emit(&inferred, &direct, &calls, &loc, coverage_ledger.len());
    }

    // Human gate output (the violation lines AND the ✓/count summaries) goes to STDERR whenever
    // stdout carries a JSON document — the report (`--json`) or the streamed verdict
    // (`--gate-json -`) — so stdout stays a single pure JSON document (pipeable to `jq` /
    // candor-sarif). Without this, the AS-EFF lines interleave the stream and corrupt it.
    let stdout_is_json = want_json || matches!(GATE_JSON_PATH.get(), Some(Some(p)) if p == "-");

    // The AS-EFF-005 baseline regression guard (spec §7 item 5) — candor-java's checkBaseline is the
    // model; see check_baseline for the full contract. Runs BEFORE the policy gate so both record
    // toward the one --gate-json verdict; the exit code is the max of the two (2 short-circuits).
    let mut guard_code = 0;
    // A configured guard that could not compare (a parse failure, nothing gained) — the run must end 2 if
    // nothing else dominates. Carried, not returned: see the arm that sets it.
    let mut baseline_incomplete = false;
    if let Some(bv) = &baseline_value {
        // NOTE: the incomplete-analysis refusal is INSIDE the `Checked` arm below, not here. It used to
        // sit at this point — before `check_baseline` ran at all — and that ordering dropped real
        // findings. See the comment on the refusal for the measurement.
        match check_baseline(bv, dir, &crate_name, &all_pub, &inferred, crate::gate::unknown_ratchet(), crate::gate::baseline_from_config()) {
            BaselineOutcome::Inactive => {} // absent file: noted, exit unchanged
            BaselineOutcome::Invalid => {
                // diagnostic already printed by check_baseline
                crate::gate::record_gate_refusal(
                    "the baseline file could not be read as a baseline — see stderr above (exit 2, \
                     guard NOT evaluated)",
                );
                return (2, json_body);
            }
            BaselineOutcome::Checked(v) => {
                for gv in &v {
                    let line = format!("[{}] {}", gv.rule, gv.detail);
                    if stdout_is_json {
                        eprintln!("{line}");
                    } else {
                        println!("{line}");
                    }
                }
                record_gate_violations(&v, run); // toward the final --gate-json verdict
                // A configured guard over INCOMPLETE analysis must not certify: the unparsed file's
                // effects are absent, so a clean compare over it is a false-pure. But **a real
                // regression still dominates** (SPEC §3.3.1: *"A configured gate over
                // incompletely-analyzed code MUST fail closed (exit ≠ 0); a real violation (exit 1)
                // still dominates."*), so the refusal is gated on `v.is_empty()` and sits AFTER the
                // compare rather than before it.
                //
                // MEASURED, and the ordering was the whole defect. This check used to run BEFORE
                // `check_baseline` was called at all, so a crate with a real AS-EFF-005 regression AND
                // one unparseable file exited 2 and wrote `{ok:false, incomplete:true, violations: []}`:
                //
                //     regression alone              -> exit 1, violations: 1 [AS-EFF-005]
                //     regression + a parse failure  -> exit 2, violations: 0   <-- the finding, GONE
                //
                // The regression is not merely mis-coded, it is ABSENT FROM THE ARTIFACT a CI consumer
                // reads — a machine-consumer under-report, which is the cardinal sin wearing an exit
                // code. The POLICY gate below had exactly this defect and it was fixed on 2026-07-28;
                // this is its sibling site, and the fix did not reach it. Two identical sequences, one
                // repaired: check the other copy.
                //
                // WHY EVALUATING OVER AN INCOMPLETE SCAN IS SAFE IN THE DIRTY DIRECTION, which is what
                // licenses this: a parse failure makes the new scan see LESS, and AS-EFF-005 fires on
                // effects GAINED. Less evidence can only MASK a gain, never manufacture one — so a
                // regression found here is real, while a clean compare is exactly the false-pure the
                // refusal still exists to prevent. The asymmetry is the argument; without it this would
                // be trading a dropped finding for a fabricated one.
                //
                // AND THE REFUSAL DOES NOT RETURN — IT IS CARRIED TO THE END OF THE FUNCTION. It used to
                // `return (2, …)` here, which is the mirror of the defect described above: the guard is
                // the EARLIER producer, so returning from it meant the POLICY gate below never ran, and a
                // certain policy violation was never computed, never recorded, never in the document.
                // MEASURED: `deny Net` over a fn connecting to a host, plus one unparseable file →
                // exit 1, `violations:[AS-EFF-006]`; add a CLEAN baseline (no function gained anything)
                // → exit 2, `violations: []`. Configuring a second gate deleted the first gate's finding.
                // The policy arm already refuses on `had_parse_failure` with the right precedence, so
                // a policy run needs nothing more; a policy-FREE run reads `baseline_incomplete` at the
                // tail.
                if had_parse_failure && v.is_empty() {
                    let why = "baseline guard NOT evaluated — source failed to parse (see above); the \
                               guard cannot compare unanalyzed code";
                    eprintln!("candor-scan: {why}");
                    crate::gate::record_gate_refusal(why);
                    baseline_incomplete = true;
                } else if v.is_empty() {
                    eprintln!("candor-scan: baseline guard ✓ — no function gained an effect (syntactic floor: resolution-heavy code can under-report silently; see `candor blindspots`)");
                } else {
                    eprintln!("candor-scan: {} baseline regression(s) — a function gained an effect versus the baseline, or is absent from it and performs one (AS-EFF-005)", v.len());
                    guard_code = 1;
                }
            }
        }
    }

    // The stable policy gate (spec §6.2 / AS-EFF-006/008/009) — the ADVISORY FLOOR. The syntactic
    // backend under-reports (a missed effect can pass), so this is a floor, never the sound gate
    // (that's the nightly engine / the JVM engine). It still catches every boundary crossing the
    // scan CAN see, deterministically, with zero extra install.
    // ⟨0.29⟩ SAY IT ON STDERR, ABOVE THE VERDICT. The report block is for machines; an operator reading
    // `policy ✓` needs to know, in the same breath, that a file this scan did not judge holds the effect
    // they denied. Printed BEFORE the verdict so the two are read together — a caveat below a green tick
    // is a caveat nobody reaches.
    if !quiet {
        if let Some(oos) = out_of_scope.as_ref() {
            for f in oos.iter() {
                eprintln!(
                    "candor-scan: ⚠ {} performs {} — OUTSIDE this scan's scope ({}), so the gate did NOT judge it.",
                    f.func, f.effects.join("+"), f.class
                );
                if !f.path.is_empty() {
                    eprintln!("             {}", f.path);
                }
            }
            if !oos.is_empty() {
                eprintln!(
                    "             The verdict below is INCOMPLETE because of {}: the gate did not judge \
                     that code, so it cannot certify this tree. A build script runs on every \
                     `cargo build`; tests and examples run in CI.",
                    if oos.len() == 1 { "it".to_string() } else { format!("these {}", oos.len()) }
                );
            }
        }
    }
    // ⟨0.30⟩ ARM THE RECORDER BEFORE ANY EXIT CAN RETURN. This sat beside the ⟨0.30⟩ exit arm, BELOW the
    // parse-failure `return (2, …)`, so a scan with BOTH an unparseable file and a peek finding wrote a
    // `--gate-json` verdict with no `outOfScope` while the REPORT carried it and `gate --report` re-emitted
    // it — the two routes' documents were not byte-equal, which is §3.1's own acceptance test. Same class
    // as the ⟨0.24⟩ "record the violations FIRST" fix one cause over: a document written on the way out of
    // an early return is a document missing whatever had not been recorded yet. (Review, MEASURED.)
    crate::gate::record_gate_out_of_scope(out_of_scope.as_deref().unwrap_or(&[]));
    // ⟨0.32⟩ beside it, from the same local values and at the same point in the sequence.
    //
    // `peek_attempted`, NOT `out_of_scope.is_some()`. The two are not the same predicate and the
    // difference was MEASURED as a document/exit split (2026-08-24): `out_of_scope` is `Some(vec![])`
    // when the policy carries NO deny rule — the peek short-circuits and returns "asked and clear" —
    // so an `allow`-only or `forbid`-only policy over a tree with a build script recorded every class as
    // unread INTO THE VERDICT DOCUMENT, which came out `"ok": false, "incomplete": true` at **exit 0**.
    // The exit arm below already used `peek_attempted` and passed; the document was the half that
    // over-charged, and only a reader of the JSON could see it. One predicate now feeds both, and it is
    // the same question `gate --report` asks of its own policy: does this run's rule set need code
    // outside the scan's scope to have been read?
    crate::gate::record_gate_unpeeked(&excluded_classes, peek_attempted.get());
    if let Some(pp) = policy_path {
        let Ok(text) = std::fs::read_to_string(&pp) else {
            // A set-but-unreadable policy must be LOUD — silently passing would let a violation ship.
            let why = format!("policy {pp:?} could not be read; gate NOT enforced");
            eprintln!("candor-scan: {why}");
            crate::gate::record_gate_refusal(why.clone());
            // ⟨0.27⟩ …and the machine channel must say so even when a violation DOMINATES (SPEC §3.1's
            // composed-document clause): the exit-1 verdict carries no `refused` key, so an `unevaluated`
            // list is the only place a consumer can see the policy half of the gate never ran. An
            // unreadable policy has no lines to name — ONE entry naming the whole file (candor-ts's
            // spelling, the spec's model), never an empty list beside a violation.
            crate::gate::record_gate_unevaluated(&[candor_report::Unevaluated {
                rule: format!("(entire policy {pp} — unreadable, no rules parsed)"),
                why,
            }]);
            // ⟨0.24⟩ PRECEDENCE: A CERTAIN VIOLATION DOMINATES A REFUSAL. This returned 2
            // unconditionally, so an AS-EFF-005 baseline regression ALREADY RECORDED above was
            // downgraded by a typo'd token in the policy beside it — measured against java, ts and
            // swift, which all exit 1 on that shape. The violation fired on evidence the report
            // carries; `Reject` is upward-closed, so nothing the unanswerable rule would have resolved
            // to can un-reject it, and exit 1 is both certain and strictly more informative because it
            // NAMES the violation.
            //
            // The refusal is still recorded, and `write_gate_json` already carries both halves — the
            // violation that dominates and the refusal that says the policy never ran. Dropping the
            // second would be the mirror defect.
            // `guard_code` is the local the baseline arm sets; `holds_violation` covers the recorded
            // set. BOTH are needed: `guard_code` is this member's, `holds_violation` is the run's.
            // (Until ⟨0.30⟩ `record_gate_violations` was also a no-op without `--gate-json`, which made
            // the precedence apply on the machine-output path and not the plain one; recording is now
            // unconditional, and the accumulator thread-local so that is safe.)
            let code = if guard_code == 1 || crate::gate::holds_violation(run) { 1 } else { 2 };
            return (code, json_body);
        };
        // ⟨0.19⟩ reason-class aliases (SPEC §6.2): a multi-value `unknown-alias` config key the single-value
        // cfg map can't hold — read straight from the discovered config so `Unknown[<alias>]` resolves.
        //
        // ⟨0.24⟩ ANCHORED AT THE **POLICY FILE**, NOT THE SCAN TARGET (SPEC §3.1). This line used to say
        // `Path::new(dir)`, while `candor-query gate --report` anchored at the policy — and all four
        // engines had exactly that split. MEASURED (2026-07-28) with the policy filed outside the target
        // and `unknown-alias corp = reflect` beside it: `candor-scan . --policy P --gate-json v` exited
        // **1** (the alias never resolved, so `deny Unknown[corp]` widened to a bare `deny Unknown`)
        // while `gate --report R --policy P --gate-json v` exited **0** (the alias resolved and narrowed
        // to `reflect`, which the crate's `indirect` hole does not match). Two different documents from
        // the same report and the same policy — **§3.1's byte-equality MUST broken by a file that is
        // neither the report nor the policy.**
        //
        // Vocabulary travels with the policy that uses it. TARGET-SCOPED keys do NOT move: `deps`,
        // `net-partner` and the scan settings above still anchor at `dir`, because they describe the
        // thing being scanned rather than the language the rules are written in. Byte-equality now holds
        // by construction instead of by the two routes happening to be pointed at the same directory.
        //
        // SOUNDNESS R525 — AND IT IS DISCOVERED ONCE, ABOVE, WHERE THE PEEK CAN SEE IT TOO. This block
        // used to own the only `discover_config(&pp)` call in the function, which is why the peek had no
        // vocabulary to parse with and lost a ⟨0.30⟩ disclosure to an alias the gate resolved fine. The
        // binding moved; the ANCHOR did not, and everything the paragraphs above say about it still holds
        // verbatim. Reaching this arm means `pp` read successfully, which is exactly the filter the
        // hoisted binding applies — so the two are the same discovery, not two that happen to agree.
        let cfg_vocab = &policy_cfg_vocab;
        let unknown_aliases = &policy_unknown_aliases;
        // ⟨0.24⟩ THE POLICY COULD NOT BE HONOURED AS WRITTEN (SPEC §6.2) — the same UNREADABLE-POLICY
        // posture as the branch above, and deliberately at the same place in the flow, so this route and
        // `candor-query gate --report` refuse the same policy identically (exit 2, no verdict document).
        //
        // MEASURED (2026-07-28) before this refusal: `deny Unknown[dispatch,nativ]` printed "candor:
        // policy rule names unknown reason-class/alias `nativ`" and then gated on `[dispatch]` ALONE —
        // exit 0 over a crate whose only hole was native-caused. The single-token form `deny
        // Unknown[corp]` did the mirror: the filter emptied and the rule WIDENED to a bare `deny
        // Unknown`, while the same line claimed the rule was being ignored. One of those is a false
        // disclosure and the other is fail-open; the fail-open one is the common case, because a typo
        // lands beside correct tokens far more often than alone.
        let (perrs, used_aliases, pignored, paccepts) = crate::gate::policy_precheck(&text, unknown_aliases);
        // ⟨0.28⟩ SPEC §6.2: the dropped lines ride the VERDICT as `ignored` — recorded here, written
        // once by `write_gate_json` beside the violations and `zeroMatch`. The per-line stderr
        // warnings are unchanged; this is their machine half. (On the fatal/zero-rule refusal arms
        // below the run writes a REFUSAL document instead of a verdict, and the whole-policy
        // `unevaluated` entry is the disclosure there — a refused run has no verdict for a dropped
        // line to have shrunk.)
        crate::gate::record_gate_ignored(&pignored);
        // ⟨0.24⟩ …and if that config supplied vocabulary the verdict USED, the verdict must name it
        // (SPEC §3.1). Recorded here, written once by `write_gate_json` — the same shape the violations
        // and the κ ledger take, so a workspace scan discloses it once rather than per member.
        if let Some((cfg_path, _)) = cfg_vocab.as_ref() {
            record_gate_vocabulary(cfg_path, &used_aliases);
        }
        if !perrs.is_empty() {
            for (_, e) in &perrs {
                eprintln!("candor-scan: policy error — {e}");
            }
            let why = format!(
                "refusing to evaluate a policy that cannot be honoured AS WRITTEN (exit 2, gate NOT \
                 enforced) — dropping the token would silently REWRITE the rule, and when the token \
                 sits beside valid ones the rewrite NARROWS it, so the gate stops covering what the \
                 operator asked for while still looking armed. Fix the token, or define it as an \
                 `unknown-alias` in the `.candor/config` beside {pp}. Policy error(s): {}",
                perrs.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("  ·  ")
            );
            eprintln!("candor-scan: {why}");
            crate::gate::record_gate_refusal(why);
            // ⟨0.27⟩ EVERY RULE OF A REFUSED POLICY GOES UNEVALUATED, AND THE DOCUMENT SAYS SO PER RULE
            // (SPEC §3.1's composed-document clause; candor-java `unhonouredRules` is the model). Naming
            // only the offending line lets a consumer read `deny Fs` — absent from the list on an exit-1
            // document — as evaluated-and-passed, a per-rule false all-clear arriving through the
            // disclosure itself (measured in candor-ts). The unhonourable line(s) carry their specific
            // cause; every other rule line carries the whole-policy refusal. These same entries ride the
            // SOLE-refusal document too (the arm in `write_gate_json` reads the same accumulator).
            let fatal_by: std::collections::BTreeMap<&str, &str> =
                perrs.iter().map(|(r, m)| (r.as_str(), m.as_str())).collect();
            let entries: Vec<candor_report::Unevaluated> = text
                .lines()
                .filter_map(|raw| {
                    let line = raw.split('#').next().unwrap_or("").trim();
                    if line.is_empty() {
                        return None;
                    }
                    Some(match fatal_by.get(line) {
                        Some(m) => candor_report::Unevaluated {
                            rule: line.to_string(),
                            why: format!(
                                "{m} — this rule is NOT evaluated; the policy is refused rather than \
                                 silently rewritten into a different one"
                            ),
                        },
                        None => candor_report::Unevaluated {
                            rule: line.to_string(),
                            why: "NOT evaluated — a rule elsewhere in this policy cannot be honoured as \
                                  written (named beside its own entry in this list), and a policy is \
                                  evaluated as a whole or not at all: a verdict from its readable subset \
                                  would be a verdict on a policy nobody wrote"
                                .to_string(),
                        },
                    })
                })
                .collect();
            crate::gate::record_gate_unevaluated(&entries);
            // …and the SAME precedence as the unreadable-policy arm above: a certain violation
            // dominates. Measured against java, ts and swift, which all exit 1 on this shape — an
            // AS-EFF-005 baseline regression recorded earlier in this function, beside a typo'd token
            // in the policy. Returning 2 here downgraded a violation that had already FIRED on evidence
            // the report carries, which §3.1 calls "byte-identical in harm" to deleting it.
            // `guard_code` is the local the baseline arm sets; `holds_violation` covers the recorded
            // set. BOTH are needed: `guard_code` is this member's, `holds_violation` is the run's.
            // (Until ⟨0.30⟩ `record_gate_violations` was also a no-op without `--gate-json`, which made
            // the precedence apply on the machine-output path and not the plain one; recording is now
            // unconditional, and the accumulator thread-local so that is safe.)
            let code = if guard_code == 1 || crate::gate::holds_violation(run) { 1 } else { 2 };
            return (code, json_body);
        }
        // ⟨0.28⟩ A CONFIGURED POLICY THAT YIELDED ZERO RULES IS A BROKEN GATE CONFIG (SPEC §6.2) — the
        // same refusal posture as the two branches above, and for the reason §6.2 already gives for an
        // unreadable file: "a typo'd policy path that runs green is a gate that silently passes
        // everything". MEASURED four-way 2026-08-10: `--policy <a README>` wrote `{"ok":true,
        // "violations":[]}` and exited 0 on every engine — byte-identical to a gate that ran and found
        // nothing, AND byte-identical to the no-gate-configured verdict, so the machine channel cannot
        // tell "your code is clean" from "your gate had no rules". The per-line "ignoring policy rule"
        // warnings go to stderr, which is not the machine channel.
        //
        // The line-level leniency is UNTOUCHED and still right: an unrecognized line stays
        // ignored-with-a-warning, because silent reinterpretation is the one thing a security gate must
        // not do, and an engine meeting a rule kind from a newer rung must not refuse the file over it.
        // This is about what that leniency COMPOSES TO — every line ignored is a gate that asked nothing.
        //
        // THE CONTROL, which is what makes this a rule and not a blanket: reaching here at all means a
        // policy was CONFIGURED (`--policy`, CANDOR_POLICY, or the config `policy` key). A run that
        // configured no gate never enters this block and stays exit 0 — that is the honest way to say
        // "I am not gating", and it is precisely why a configured zero-rule policy is never a legitimate
        // expression of that intent.
        // ALL THREE RULE VECTORS, and the first draft of this check read only `rules`. `ParsedPolicy`
        // splits the four kinds across `rules` (deny/pure), `allow_rules` and `layer_rules`, so keying on
        // one vector made an allow-only or layer-only policy — `allow Net api.stripe.com`, a perfectly
        // ordinary allowlist gate — refuse as if it had no rules at all. Caught by
        // `masking_fs_path_and_db_table_gate_fails_closed`, which gates on `allow Fs /var/app` and went
        // from exit 1 to exit 2. A zero-rule test that reads a subset of the rule kinds is the same
        // false-refusal shape this rung exists to prevent, pointed the other way.
        let parsed_zr = candor_classify::policy::parse_policy_silent(&text, unknown_aliases);
        if parsed_zr.rules.is_empty() && parsed_zr.allow_rules.is_empty() && parsed_zr.layer_rules.is_empty()
            && parsed_zr.only_rules.is_empty() {
            let why = format!(
                "the policy at {pp} yielded NO RULES — refusing (exit 2, gate NOT enforced). Every line \
                 was ignored (see the `ignoring policy rule` warnings above), the file is empty, or it \
                 holds only comments. A gate with no rules cannot have caught anything, and reporting \
                 `ok: true` here would be indistinguishable from a gate that ran and found nothing. If \
                 you did not mean to gate this run, remove the `policy` setting rather than pointing it \
                 at a file with no rules in it."
            );
            eprintln!("candor-scan: {why}");
            crate::gate::record_gate_refusal(why);
            // SPEC §3.1 — the whole-policy entry, the shape pinned for a policy with no lines to name.
            crate::gate::record_gate_unevaluated(&[candor_report::Unevaluated {
                rule: format!("(entire policy {pp} — no rules parsed)"),
                why: "the configured policy yielded zero rules, so nothing was evaluated and no rule \
                      can have passed"
                    .to_string(),
            }]);
            // …and the SAME precedence as both branches above: a certain violation dominates a refusal
            // (§3.1, `Reject` is upward-closed). No POLICY violation can exist with zero rules, but an
            // AS-EFF-005 baseline regression is a finding from evidence this run carries and it outranks.
            let code = if guard_code == 1 || crate::gate::holds_violation(run) { 1 } else { 2 };
            return (code, json_body);
        }
        let mut outcome = policy_violations(&text, &crate_name, &all_pub, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc, &incompleteacc, &reason_class_acc, unknown_aliases, &net_partners);
        // ⟨0.41⟩ SPEC §6.2 `accept`: settle THIS member's rows provisionally (its own exit decisions below read `v`),
        // recording what `finalize_accept` needs to settle the whole run. A no-op without an entry.
        crate::gate::settle_member(&paccepts, &crate_name, &all_pub, &mut outcome, run);
        // ⟨0.29⟩ THE NAME RULES STOP AT THE SCAN BOUNDARY, AND NOW SAY SO. `forbid A -> B` and
        // `only A -> B …` match over the call graph; a chained dependency contributes EFFECTS, not EDGES,
        // so a function calling into a dep has an EMPTY adjacency and the crossing is invisible to them.
        // MEASURED with a dep chained: `only model -> util` answered `policy ✓` over
        // `model::via_dep() -> deplib::infra::db_read()` while a LOCAL unpermitted scope in the same run
        // fired AS-EFF-011 — the rule was armed; the boundary was the gap.
        //
        // WORSE FOR `only` THAN `forbid`: `forbid` asks whether ONE named crossing is present, so a missed
        // dep crossing under-reports one prohibition; `only` asserts A reaches the listed scopes AND
        // NOTHING ELSE — a COMPLETENESS claim — and exists because `forbid` fails open. A package that
        // calls a third-party library is not a leaf, and without this the gate called it one.
        //
        // DISCLOSURE, NOT A VERDICT CHANGE. Making the rules cross needs dep-report EDGES and would force
        // operators to enumerate third-party scopes in an `only` list — the enumeration-that-rots that form
        // was designed to escape. The ⟨0.29⟩ `outOfScope` posture: say what was not judged, leave the exit
        // code alone.
        if !deps_idx.crates.is_empty() {
            // SOUNDNESS R525 — the THIRD `parse_policy` in this function, converted with the other two.
            // Its VERDICT does not move: it counts `forbid`/`only` rules, and alias resolution touches
            // only the `Unknown[…]` filter inside a `deny`, so the count is identical either way. It is
            // converted anyway because an alias-blind parse of a policy the rest of the run reads
            // alias-aware is a divergence waiting for its first consumer, and because leaving one of
            // three siblings unconverted is exactly how R525 was created — `parse_policy_silent` had
            // already landed two lines' worth of sibling above while these two stayed bare.
            let np = candor_classify::policy::parse_policy_with_aliases(&text, unknown_aliases);
            let named = np.layer_rules.len() + np.only_rules.len();
            if named > 0 {
                eprintln!(
                    "candor-scan: ⚠ {named} name-matching rule(s) (`forbid`/`only`) were matched over \
                     THIS scan's call graph only — a chained dependency contributes effects, not call \
                     edges, so a crossing INTO a dependency is invisible to them. `deny`/`allow` still \
                     cross (effects propagate); an `only` rule cannot certify that a package is a leaf \
                     when it calls into one of its dependencies."
                );
            }
        }
        // ⟨0.27⟩ SPEC §4 — a rule whose SCOPE bound NO function is UNANSWERABLE, and is DISCLOSED rather
        // than scored as satisfied. MEASURED here before the fix: `deny Fs orders` exits 1 on a real
        // violation and `deny Fs ordrs` exits 0 in silence, so a one-character typo in a layer name is a
        // permanently green gate — and `unverified` then calls the layer "PROVABLY clean". The remedy is
        // disclosure, NOT refusal: a zero-match rule is legitimate when one policy is shared across
        // repositories and a layer exists in only some, so the exit code is deliberately untouched.
        for raw in &outcome.zero_match {
            eprintln!(
                "candor: policy rule matched NO function — `{raw}`. It was evaluated and bound nothing, \
                 so it cannot have caught anything. Legitimate when one policy is shared across repos; \
                 a typo'd layer name otherwise."
            );
        }
        // SOUNDNESS R301 — and the OVER-bound counterpart, on the same channel and with the same rule
        // about the exit code: this is a DISCLOSURE, never a verdict. §6.2 scope matching is a PREFIX
        // match and that is the documented behaviour; what was wrong is that it was INVISIBLE. An
        // unbound rule announced itself and an over-bound one did not, so `deny Fs either` reading
        // exit 1 looked like evidence about `either` when it was evidence about `either_ifelse`.
        for (raw, bound) in &outcome.multi_match {
            eprintln!(
                "candor: policy rule `{raw}` bound {} functions, not one — {}. \u{a7}6.2 scope matching is a PREFIX match, so a result under this rule is not evidence about the name you wrote. Rename or qualify the scope if you meant exactly one.",
                bound.len(),
                bound.join(", ")
            );
        }
        // ⟨0.27⟩ …and the SAME list rides the `--gate-json` verdict as `zeroMatch` (SPEC §4): stderr is
        // not the machine channel, and a wrapper that reads the document could not see that a rule bound
        // nothing — the typo'd-scope silent green, one channel over. Recorded toward the single verdict
        // like the violations; the exit code is untouched either way.
        crate::gate::record_gate_zero_match(&outcome.zero_match);
        let v = outcome.violations;
        // ⟨0.24⟩ WITHHELD `(rule, function)` PAIRS — SPEC §3.1. On THIS route the classifier is in the
        // loop, so a narrowing filter with nothing to read means the signature itself lacks a class set
        // for a function that carries `Unknown` — which candor-scan's own `reason_class_direct`
        // contribution is supposed to make unreachable (the §4 invariant beside the writer is a
        // `debug_assert`, so release builds have no net). It used to reach the gate as a FIRING anyway,
        // via the matcher's `unresolved` floor, and assert a reason nobody recorded.
        //
        // Withheld is NOT tolerated: with no violation to dominate it, the gate could not be evaluated and
        // says so (exit 2, the refusal posture), rather than printing `policy ✓` over a rule that never
        // ran. With a violation beside it, exit 1 dominates (`Reject` is upward-closed) and the withheld
        // rule is disclosed alongside — the same precedence the report route applies.
        if !outcome.withheld.is_empty() {
            eprintln!(
                "candor-scan: {} policy rule(s) could NOT be evaluated on {} function(s) — the narrowing \
                 filter had no evidence to read, so the rule is WITHHELD there rather than charged or \
                 tolerated (SPEC §3.1). This is a candor defect, not a policy one: report it.",
                outcome.withheld.iter().map(|w| &w.rule).collect::<BTreeSet<_>>().len(),
                outcome.withheld.len()
            );
            for w in &outcome.withheld {
                eprintln!("    `{}` narrows on the {} class, but `{}` carries no class set to narrow on", w.rule, w.filter, w.func);
            }
            // ⟨0.24⟩ …AND ONTO THE DOCUMENT, not stderr alone (SPEC §3.1 `fc4b5f6`). This disclosure had
            // exactly the shape the `gate --report` route's did — correct, complete, and on the wrong
            // channel — so a machine consumer of an exit-1 verdict here could not see that a rule had
            // gone unanswered either. ONE ENTRY PER RULE: the first function that defeats it is the
            // example, and `record_gate_unevaluated` de-duplicates on `rule` across workspace members.
            let disclosures: Vec<candor_report::Unevaluated> = outcome
                .withheld
                .iter()
                .map(|w| candor_report::Unevaluated {
                    rule: w.rule.trim().to_string(),
                    why: format!(
                        "it narrows on the {} class, but `{}` carries no class set to narrow on — the \
                         filter had no evidence to read, so the rule is WITHHELD there rather than \
                         charged (which would assert a class nobody recorded) or tolerated (which would \
                         relax a fail-closed gate for lack of evidence).",
                        w.filter, w.func
                    ),
                })
                .collect();
            crate::gate::record_gate_unevaluated(&disclosures);
        }
        for gv in &v {
            let line = format!("[{}] {}", gv.rule, gv.detail);
            if stdout_is_json {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
        record_gate_violations(&v, run); // toward the final --gate-json verdict (written once, by scan_main)
        // A configured gate over INCOMPLETE analysis (a source file failed to parse) must NOT report
        // green: the unparsed file's effects are absent, so a `policy ✓` over it is a false-pure. Fail
        // exit 2 (mirroring the unreadable-policy posture) — never exit 0/1 with a clean-looking ✓.
        //
        // RECORDING THE VIOLATIONS FIRST is the fix, and the ORDER is the whole of it (measured
        // 2026-07-28): this branch used to return BEFORE `record_gate_violations`, so a crate with a real
        // `deny Net` hit AND one unparseable file wrote `{ok:false, incomplete:true, violations: []}` —
        // the finding was printed to stderr and then DELETED from the document a CI consumer reads.
        // SPEC §3.3 asks for both halves: fail closed (exit ≠ 0), and "a real violation (exit 1) still
        // dominates". So when `v` is non-empty the run falls through to the ordinary violation exit (1)
        // below, and either way the verdict now carries what was actually found alongside
        // `incomplete`/`unanalyzed`. `write_gate_json` still writes NO document for the other exit-2
        // cause — a gate CONFIG that never loaded — where there is nothing faithful to say.
        // ⟨0.30⟩ THE SAME THREE CONJUNCTS AS THE SCOPE ARM BELOW. This keyed on `v` alone — the POLICY
        // gate's own list — so a run holding a CERTAIN violation from another producer still returned 2.
        // MEASURED: a function gaining `Fs` against a frozen baseline, plus one unparseable file, plus a
        // clean `deny Net` policy → exit 2 while the verdict carried `violations:[AS-EFF-005]`. Adding a
        // CLEAN policy downgraded a certain violation from 1 to 2, and the document then disagreed with
        // the exit code. The note three lines above describes this exact conflation, and the sibling arm
        // twenty lines below was given the conjuncts while this one was not — the sibling-route habit,
        // inside the fix for it.
        //
        // SOUNDNESS R1231 — AND THE FOUR INCOMPLETENESS ARMS BELOW NOW SHARE ONE PREDICATE AND ONE SHAPE.
        // The parse-failure and scope arms had the three conjuncts; the sole-withholding and unread-class
        // arms further down tested `v.is_empty()` ALONE — the identical conflation this note records,
        // surviving in the two arms the ⟨0.30⟩ fix did not reach. MEASURED: a function gaining `Fs`
        // against a frozen baseline, plus an unparseable `build.rs` (an unread `build-script` class), plus
        // a clean `deny Net` → exit 2 while the verdict carried `violations:[AS-EFF-005]`; drop the
        // `build.rs` and the same tree exits 1. So the question is asked ONCE, here, as `held`, and every
        // arm answers it the same way: print why the POLICY could not be certified, then exit 1 when the
        // run already holds a certain violation (§3.3(c)) and 2 when it does not. The two arms that used
        // to SKIP on `held` fell through to `policy ✓`, a green tick over a gate that did not certify —
        // which the withheld arm's own comment forbids — so all four RETURN now rather than skip.
        // `guard_code == 1` implies `holds_violation` (the baseline arm records before it sets the code);
        // both are kept because that is this function's stated idiom at every refusal arm above.
        // A plain value, not a closure: a closure is a new unit in this engine's own self-guard baseline.
        let held = guard_code == 1 || crate::gate::holds_violation(run);
        let incomplete_code = if held { 1 } else { 2 };
        if had_parse_failure && v.is_empty() {
            eprintln!("candor-scan: policy NOT enforced — source failed to parse (see above); gate cannot be green over unanalyzed code");
            return (incomplete_code, json_body);
        }
        // ⟨0.30⟩ THE SCOPE HALF OF THE SAME POSTURE. `had_parse_failure` above is "I opened this file and
        // could not read it"; this is "I never opened it, and when the peek looked afterwards it performed
        // the effect this policy denies". Both mean the gate could not see enough of the tree to certify
        // it, so both are exit 2. Reverses ⟨0.29⟩'s "the verdict does not move" on the measurement that
        // the peek resolves a CONCRETE denied effect rather than uncertainty.
        //
        // EXIT 2, NOT 1: these functions are not in `violations` and not in `functions`, because the gate
        // did not judge them — exit 1 would claim "I judged your code and it breaks the policy", false in
        // the other direction. A certain violation dominates (SPEC §3.3(c)) — and that is asked of the
        // SHARED accumulator, not of `v`. `v` is the POLICY gate's own list; the AS-EFF-005 baseline guard
        // is a different producer that runs earlier, so `v.is_empty()` was true on runs that had already
        // recorded certain violations. MEASURED on `clap` under `pure`: 0.29.1 exits 1, this exited 2 with
        // 27 violations sitting in the verdict — "I could not evaluate" over a tree it had just judged.
        // The ⟨0.24⟩ note in `write_gate_json` records this exact conflation one cause over; it was read
        // while writing this line and the wrong predicate was used anyway.
        let oos_findings = out_of_scope.as_deref().unwrap_or(&[]);
        if !oos_findings.is_empty() && v.is_empty() {
            eprintln!(
                "candor-scan: policy NOT enforced — {} function(s) OUTSIDE this scan's scope perform an \
                 effect this policy denies (named above); the gate did not judge them, so the verdict is \
                 incomplete rather than a pass",
                oos_findings.len()
            );
            return (incomplete_code, json_body);
        }
        // ⟨0.24⟩ A SOLE WITHHOLDING is a refusal (SPEC §3.1). Ordered AFTER the violation exit so the
        // precedence holds — a rule that fired on carried evidence dominates. Never `policy ✓`: the
        // operator asked for a rule that did not run. (R1231: `v.is_empty()` was NOT the whole of the
        // condition — `v` is the policy gate's own list, and a baseline regression dominates too; see
        // `held` above.)
        if !outcome.withheld.is_empty() && v.is_empty() {
            let why = format!(
                "policy NOT enforced — {} (rule, function) pair(s) could not be evaluated (see above); a \
                 gate cannot be green over a rule that never ran",
                outcome.withheld.len()
            );
            eprintln!("candor-scan: {why}");
            crate::gate::record_gate_refusal(why);
            return (incomplete_code, json_body);
        }
        // ⟨0.32⟩ THE THIRD CAUSE — a class this scan did not READ. ⟨0.30⟩'s arm keys on what the peek
        // FOUND, and a peek that could not open a file finds nothing, which is byte-identical to
        // finding it clean. Decided from the LOCAL `excluded_classes`, never from GATE_UNPEEKED: that
        // accumulator is fed only when `--gate-json` was requested, and an exit code must not depend on
        // whether a machine-readable sink was asked for (the rule stated at gate.rs:670).
        //
        // AFTER the violation and withheld arms, deliberately — a certain violation dominates, and a
        // rule that never ran is a better message than "something went unread".
        {
            let unread: Vec<&str> = excluded_classes
                .iter()
                .filter(|e| !e.peeked && !e.judged_elsewhere)
                .map(|e| e.class.as_str())
                .collect();
            if !unread.is_empty() && peek_attempted.get() && v.is_empty() {
                let why = format!(
                    "gate NOT certified — this scan did not READ {}. Their effects are absent because \
                     nothing looked, not because there are none, so the verdict is INCOMPLETE rather \
                     than a pass",
                    unread.join(", ")
                );
                eprintln!("candor-scan: {why}");
                return (incomplete_code, json_body);
            }
        }
        // Provable-purity disclosure (advisory — NEVER changes the verdict/exit): pure/deny layers that PASS
        // but are Unknown. Surfaces the gap automatically so an author learns their "pure" layer isn't
        // PROVABLY pure (eval/fixloop/DISPATCH-NOTE.md); the `candor-query unverified` query has the detail.
        // ⟨0.24⟩ The SAME two accumulators the gate matched on, and the SAME alias vocabulary it
        // resolved through — the disclosure names the holes the gate declined to clear, so anything it
        // reads differently is a second gate wearing the first one's name.
        let hole_nets = crate::gate::net_class_map(&all_pub, &inferred, &hostsacc, &incompleteacc, &net_partners);
        let holes = crate::gate::unverified_holes(
            &text,
            &all_pub,
            &inferred,
            &reason_class_acc,
            &hole_nets,
            unknown_aliases,
        );
        // R443 THE LINES COME FROM THE SHARED RENDERER NOW — `candor-query gate --report` prints the
        // identical note from the identical function, so the two routes can no longer disagree about the
        // WORDS any more than they can about the SET. Byte-identical to the hand-written block it
        // replaces — pinned by `unverified_note_lines_render_one_note_for_both_routes` (candor-classify)
        // and end-to-end by `gate_note_matches_the_scan_route` (candor-query tests/cli.rs).
        for line in candor_classify::policy::unverified_note_lines("candor-scan", &holes) {
            eprintln!("{line}");
        }
        if v.is_empty() {
            // SPEC §7 item 7 REQUIRES this profile to disclose that it can under-report silently, so the
            // disclosure stays. What changed (SOUNDNESS R456, 2026-09-16) is the SECOND clause, which used
            // to read "the nightly engine is the sound gate" and pointed users at an engine THE REST OF
            // THIS PRODUCT DISCOUNTS: the dylint lint stamps `spec: 0.38` over a pre-⟨0.21⟩ report with no
            // `analyzed` manifest, so `candor-query gate` answers its output with "re-scan with a current
            // engine". It is the family's oracle instrument, not a route to send a user down.
            //
            // AND THE FIRST CLAUSE WAS WRONG IN THE OTHER DIRECTION — "under-reports", unqualified, is more
            // pessimistic than anything measured. The numbers below are this repo's own standing gates, so
            // the disclosure is now item 7 done properly: what it misses, stated as a measurement.
            // ⟨0.41⟩ …unless an `accept` entry exists: whether the RUN is clean (no stale entry anywhere, no twin
            // across members) is not decidable until the last member has been settled, so the line moves to
            // `finalize_accept` rather than claiming a pass the whole-run settlement may withdraw.
            if paccepts.is_empty() {
                eprintln!("candor-scan: policy ✓ (syntactic floor — a clean run is necessary, not sufficient: \
                       resolution-heavy code can under-report SILENTLY. Measured: 0 silent-pure across 19 \
                       syscall-oracle drivers on executed Fs/Net/Exec, and 0 fabrications across 76 \
                       curated-pure crates. Run `candor blindspots` for what this scan could not see.)");
            } else {
                crate::gate::defer_policy_ok(run);
            }
            // A CLEAN gate has no violation lines for the exploration opener to contradict, so emit it HERE
            // (after the ✓) — the pre-gate suppression at the top only exists to avoid a "nothing hidden"
            // line ABOVE a FAILING gate's violations (#18); a passing gated scan should not lose it (#8).
            if !quiet {
                crate::surface::emit(&inferred, &direct, &calls, &loc, coverage_ledger.len());
            }
        } else {
            eprintln!("candor-scan: {} policy violation(s) (syntactic floor — a clean run is necessary, not sufficient)", v.len());
            // Append-only remedy pointer (gate-FAILURE path only): the summary line above is
            // conformance-pinned, so this extra line must never alter it, the violation lines,
            // or the exit code — and a zero-violation run stays byte-identical.
            eprintln!("→ candor-query fix-gate names the remedy for each (or `candor fix <fn> <Effect>` for one)");
            return (1, json_body);
        }
    }
    // No-gate runs record nothing; scan_main's final write_gate_json emits { ok: true, [] } for them.
    // A clean policy still exits 1 when the baseline guard fired above (the codes join by max).
    // A guard that could not compare over unparsed code refuses here — reached only with no policy, or
    // with one whose own parse-failure arm already returned; `guard_code` is 0 whenever it is set.
    if baseline_incomplete && !crate::gate::holds_violation(run) {
        return (2, json_body);
    }
    (guard_code, json_body)
}

/// Fan a resolved workspace MEMBER `m` into the leaf crate dir(s) `scan_target`'s loop should hand to
/// `scan_one`: `m` itself, unless `m` ALSO declares its own `[workspace]` table, in which case `m` is
/// expanded exactly the way `scan_target` expands its own root — `m`'s own root package (if it has one)
/// plus every one of ITS resolved members, each checked again for the same thing.
///
/// `cargo metadata` refuses this layout outright ("multiple workspace roots found in the same
/// workspace") — but that is an argument about whether `cargo build` would accept the tree, not about
/// whether this scanner should read it, and `find_workspace_root`'s own doc comment (`verified_workspace_root`,
/// deps.rs) already rejects "cargo would refuse to build this" as grounds for trusting a scan: candor-scan's
/// stated purpose is reading source WITHOUT building it, so unbuildable or adversarial layouts are
/// exactly the population it is asked to see. A member that is a nested workspace root is real,
/// reachable input, not a hostile edge case invented for this fix.
///
/// RECURSIVE, and capped at `depth` 64 (matching the fixpoint caps elsewhere in this file) — not because
/// a legitimate tree nests this deep, but because a manifest's own `exclude`/glob resolving back onto an
/// ancestor is an authoring mistake this function must not spin on; past the cap `m` is scanned as a
/// single leaf crate with a loud warning rather than silently dropped.
/// SOUNDNESS R1537 (c) — one inherent/trait co-path set: the two member quals, the trait's leaf, each side's receiver rank (0
/// where the side's bodies disagree or record none), the inherent side's `impl_flags` (ANDed over its bodies) and the module
/// its impl block sits in (for a private method's visibility).
pub(crate) struct CoPathC {
    pub(crate) inh: String,
    pub(crate) tr: String,
    pub(crate) tr_leaf: String,
    pub(crate) inh_rank: u8,
    pub(crate) tr_rank: u8,
    pub(crate) inh_flags: u8,
    pub(crate) module: String,
}

/// SOUNDNESS R1537 (c) — the inherent/trait co-path sets among `fns` (see the call site in `scan_one`). The member names are the
/// bare qual plus `(<self type leaf>)` and `(<trait leaf>)`; a set whose two names would coincide, or would collide with a
/// unit that already exists, is left merged.
pub(crate) fn copath_sets(fns: &[FnInfo], unit_quals: &HashSet<String>) -> HashMap<String, CoPathC> {
    let mut by_q: HashMap<&str, Vec<&FnInfo>> = HashMap::new();
    for f in fns {
        by_q.entry(f.qual.as_str()).or_default().push(f);
    }
    let mut out = HashMap::new();
    for (q, fs) in by_q {
        if fs.len() < 2
            || fs.iter().any(|f| {
                !matches!(f.impl_kind, 1 | 2)
                    || f.extern_decl
                    || f.reexport_alias
                    || f.derive_synth
                    || f.mbe_prim_op
                    || f.mbe_op
                    || f.inst_of.is_some()
            })
        {
            continue;
        }
        let inh: Vec<&&FnInfo> = fs.iter().filter(|f| f.impl_kind == 1).collect();
        let trs: Vec<&&FnInfo> = fs.iter().filter(|f| f.impl_kind == 2).collect();
        // ONE trait by its written path (arguments included), not just by leaf — see `FnInfo::impl_trait_path`.
        if trs.iter().map(|f| f.impl_trait_path.as_str()).collect::<BTreeSet<_>>().len() != 1 {
            continue;
        }
        let leaves: Vec<&str> = trs.iter().map(|f| f.impl_trait.as_str()).collect::<BTreeSet<_>>().into_iter().collect();
        let (Some(ty_leaf), [tr_leaf]) = (q.rsplit("::").nth(1), leaves.as_slice()) else {
            continue;
        };
        if inh.is_empty() || tr_leaf.is_empty() || *tr_leaf == ty_leaf || q.ends_with(')') {
            continue;
        }
        let (mi, mt) = (format!("{q}({ty_leaf})"), format!("{q}({tr_leaf})"));
        if unit_quals.contains(&mi) || unit_quals.contains(&mt) {
            continue;
        }
        let rank = |v: &[&&FnInfo]| -> u8 {
            let r = v[0].recv_rank;
            if v.iter().all(|f| f.recv_rank == r) { r } else { 0 }
        };
        let module = q.rsplitn(3, "::").nth(2).unwrap_or("").to_string();
        out.insert(
            q.to_string(),
            CoPathC {
                inh: mi,
                tr: mt,
                tr_leaf: tr_leaf.to_string(),
                inh_rank: rank(&inh),
                tr_rank: rank(&trs),
                inh_flags: inh.iter().fold(u8::MAX, |a, f| a & f.impl_flags),
                module,
            },
        );
    }
    out
}

/// SOUNDNESS R1537 (c) — the member of `cp` that call `c` in `caller` provably reaches, or `None` (the union):
///  · an edge to a NAMED trait's implementation (`Call::cside` `Some(tr)`): the trait member, when it is this set's trait;
///  · a written associated path naming no trait (`Some("")`): the inherent member, when it applies to every instance and the
///    caller can see it — path resolution takes the type's inherent item before any trait's;
///  · a method-call expression on a receiver typed as the set's type (`dot && typed`): the inherent member when Rust's probe
///    reaches it no later than the trait's (`model::inherent_probe_wins`) and it applies and is visible — within one probe
///    step the inherent method is preferred, and a trait not in scope is no candidate at all.
///
/// Everything else — an unknown receiver form with differing ranks, a conditional or invisible inherent method, a trait
/// that may win at an earlier step — is the union, which over-approximates and never drops a member.
pub(crate) fn copath_member<'a>(cp: &'a CoPathC, c: &Call, caller: &str) -> Option<&'a String> {
    let inherent_ok = || {
        cp.inh_flags & crate::model::IMPL_UNCOND != 0
            && (cp.inh_flags & crate::model::IMPL_PUB != 0
                || cp.module.is_empty()
                || caller.starts_with(&format!("{}::", cp.module)))
    };
    match c.cside.as_deref() {
        Some("") => inherent_ok().then_some(&cp.inh),
        Some(tr) => (tr == cp.tr_leaf).then_some(&cp.tr),
        None if c.dot && c.typed && c.method && !c.is_macro => {
            (crate::model::inherent_probe_wins(cp.inh_rank, cp.tr_rank, c.recv_form) && inherent_ok()).then_some(&cp.inh)
        }
        None => None,
    }
}

fn expand_nested_workspace_member(m: String, depth: u32, out: &mut Vec<String>) {
    let mp = Path::new(&m);
    if !has_workspace_table(mp) {
        out.push(m);
        return;
    }
    if depth > 64 {
        eprintln!("candor-scan: workspace nesting under `{m}` exceeds 64 levels — scanning it as a \
                   single crate rather than recursing further");
        out.push(m);
        return;
    }
    let inner = workspace_members(mp);
    if inner.is_empty() {
        // Mirrors `scan_target`'s own top-level "zero resolved members" arm: warn loudly and still
        // scan `m` as one crate (covering its own root package's sources, if any) rather than dropping
        // it — an unresolved nested workspace must not vanish either.
        eprintln!("candor-scan: `{m}` declares [workspace] but no members resolved — check \
                   `members`/globs; scan member crates directly to gate them");
        out.push(m);
        return;
    }
    if read_crate_name(mp).is_some() {
        out.push(m.clone()); // the nested workspace manifest also declares its own root package
    }
    for im in inner {
        expand_nested_workspace_member(im, depth + 1, out);
    }
}

/// Scan a TARGET — a single crate, or a `[workspace]` root fanned out into one report per member
/// under the shared prefix. The one place both the plain and `--deps` paths funnel through, so a
/// workspace is never scanned as one merged package (colliding same-named fns) nor pruned to an
/// empty report by the nested-package filter. With `want_json`, prints ONE JSON document for a
/// single crate and a JSON ARRAY for a workspace — never concatenated documents. Returns the exit code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn scan_target(
    dir: &str,
    prefix: String,
    want_json: bool,
    include_tests: bool,
    policy: Option<String>,
    baseline: Option<String>,
    deps_idx: &DepIndex,
    // ⟨0.31⟩ carried, never minted here: the `[workspace]` member loop below is the exact place the
    // one-run-one-thread invariant lives, and `RunToken` is !Send so parallelising it will not compile.
    // See the note on `gate::RunToken`.
    run: &crate::gate::RunToken,
) -> i32 {
    let members = workspace_members(Path::new(dir));
    if members.is_empty() {
        if has_workspace_table(Path::new(dir)) {
            // A [workspace] with zero RESOLVED members: scanning the root as one crate would let the
            // nested-package filter prune every member into an empty report that passes any gate
            // vacuously (§6.2's forbidden state). Warn loudly; the single-crate scan below still
            // covers the root package's own sources, if any.
            eprintln!("candor-scan: `{dir}` declares [workspace] but no members resolved — \
                       check `members`/globs; scan member crates directly to gate them");
        }
        let (code, json) = scan_one(dir, ScanOpts {
            prefix, want_json, include_tests, policy, baseline, ws_member: false, quiet: false, deps_idx, peek_excluded: false,
        }, run);
        if let Some(b) = json {
            println!("{b}");
            // ⟨0.28⟩ Latch: a successful report went to stdout, so a later `exit2_refused` MUST NOT
            // also write a fail-closed placeholder there — two documents on one stream is the shape
            // the two-stream-refusal clause already exists to prevent, arriving through a different door.
            let _ = crate::gate::REPORT_STREAM_WRITTEN.set(true);
        }
        // ⟨0.28⟩ The write phase is over (scan_one's report write precedes every return it has), which
        // is the license `disarm_unwritten_out_reports` requires — see mark_out_reports_written.
        crate::gate::mark_out_reports_written();
        return code;
    }
    let prefix = if prefix.is_empty() { format!("{dir}/.candor/report") } else { prefix };
    // ⟨0.32⟩ A run given no `--out` still writes reports HERE, and a refusal after this point leaves the
    // previous run's bytes readable as current. `note_report_prefix` is a no-op if `--out` already
    // latched one (OnceLock), so the named sink keeps precedence.
    crate::gate::note_report_prefix(&prefix);
    let mut dirs: Vec<String> = Vec::new();
    if read_crate_name(Path::new(dir)).is_some() {
        dirs.push(dir.to_string()); // the workspace manifest also declares a root package
    }
    // ⟨2026-08-29, adversarial review⟩ EVERY RESOLVED MEMBER, FANNED OUT AGAIN if it is ITSELF a
    // `[workspace]` root — see `expand_nested_workspace_member` below for why this is not exotic-input
    // paranoia: `workspace_members` was computed ONCE here and never asked whether a resolved member
    // needed its own fan-out, so a member that is itself a workspace root fell straight into `scan_one`
    // as a plain crate. `scan_one`'s OWN nested-package filter then pruned that member's inner members —
    // they carry their own Cargo.toml, the identical "different package" rule this very fan-out exists
    // to honour — with NO `excluded` entry, NO `outOfScope` entry, and no stderr: an entire subtree
    // vanished from the report as if it had never been part of the scan, not merely unread within it.
    // Reproduced live: `deny Net` exited 0, `policy ✓`, over an inner member reaching Net through an
    // impostor `{ workspace = true }` dependency.
    for m in members {
        expand_nested_workspace_member(m, 1, &mut dirs);
    }
    // DEDUPE BY CANONICAL PATH — the root can also be a MEMBER, spelled differently.
    //
    // `members = ["codegen/swagger", "codegen/proto", "."]` is a real and legal layout (bollard ships
    // it), and `workspace_members` dedupes STRINGS: `.` survives as `<root>/.`, which is a different
    // string and the same directory as the `<root>` pushed just above. `scan_one` then ran twice over
    // one package and:
    //   · called `record_gate_analyzed` twice, so the --gate-json verdict said `analyzed.count 856`
    //     where its own three reports summed to 592 — an over-claim of 264, and §3.1 byte-equality
    //     against `gate --report` broken, since that route can only ever see the reports;
    //   · pushed the same body into `bodies` twice, so `--json` emitted the package TWICE in its array.
    // The report FILES were unharmed (the second write is identical), which is why nothing else noticed.
    //
    // MEASURED on bollard v0.16.1 by the corpus round's §3.1 oracle over third-party trees — the
    // in-repo gate-equivalence fixtures cannot reach it, because candor's own workspace does not list
    // its root as a member.
    {
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| {
            let key = std::fs::canonicalize(d).unwrap_or_else(|_| std::path::PathBuf::from(d));
            seen.insert(key)
        });
    }
    let mut rc = 0;
    let mut bodies: Vec<String> = Vec::new();
    for d in &dirs {
        let (code, json) = scan_one(d, ScanOpts {
            prefix: prefix.clone(), want_json, include_tests, policy: policy.clone(),
            baseline: baseline.clone(), quiet: false, ws_member: true, deps_idx, peek_excluded: false,
        }, run);
        // ⟨0.30⟩ SPEC PRECEDENCE, NOT NUMERIC MAX. `rc.max(code)` makes 2 beat 1, so one member's
        // "I could not evaluate" displaced another member's CERTAIN violation — the inverse of §3.3's
        // "a real violation (exit 1) still dominates". Latent while exit 2 was rare (a parse failure);
        // ⟨0.30⟩ made it reachable on any workspace with an excluded file, and MEASURED on `regex` under
        // `pure`: 198 violations printed, verdict carrying all 198, and the run exited 2. The order the
        // members happen to be walked in decided it, which is why no per-member check can: the arm
        // returns before later members have recorded anything.
        rc = if rc == 1 || code == 1 { 1 } else { rc.max(code) };
        if let Some(b) = json {
            bodies.push(b);
        }
    }
    if want_json {
        println!("[{}]", bodies.join(","));
        // ⟨0.28⟩ Latch — see the single-crate branch above.
        let _ = crate::gate::REPORT_STREAM_WRITTEN.set(true);
    } else {
        eprintln!("candor-scan: workspace — {} package report(s) under one prefix", dirs.len());
    }
    // ⟨0.28⟩ Every member's write phase is over — the loop above has no early exit — so the disarm
    // hand-back is licensed. See mark_out_reports_written.
    crate::gate::mark_out_reports_written();
    rc
}

/// `--deps`: read Cargo.lock, scan every REGISTRY dependency's unbuilt source from
/// `~/.cargo/registry/src/<index>/` into `<dir>/.candor/deps/`, then scan the root crate chained
/// over those reports (plus anything CANDOR_DEPS already names). Path/git/workspace deps have no
/// registry checkout and are skipped with a note — chain them by scanning them yourself.
pub(crate) fn run_with_deps(dir: &str, prefix: String, want_json: bool, include_tests: bool,
                            policy: Option<String>, baseline: Option<String>,
                            run: &crate::gate::RunToken) -> i32 {
    let lock = match std::fs::read_to_string(format!("{dir}/Cargo.lock")) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("candor-scan: --deps needs {dir}/Cargo.lock (run `cargo generate-lockfile` first)");
            return 2;
        }
    };
    // [[package]] blocks: name + version + source. Only registry deps have a checkout to scan;
    // the root crate itself has no `source` line and is naturally skipped.
    let mut pkgs: Vec<(String, String)> = Vec::new();
    let (mut name, mut version, mut registry) = (String::new(), String::new(), false);
    let flush = |name: &mut String, version: &mut String, registry: &mut bool, pkgs: &mut Vec<(String, String)>| {
        if *registry && !name.is_empty() && !version.is_empty() {
            pkgs.push((name.clone(), version.clone()));
        }
        name.clear();
        version.clear();
        *registry = false;
    };
    for line in lock.lines() {
        let l = line.trim();
        if l == "[[package]]" {
            flush(&mut name, &mut version, &mut registry, &mut pkgs);
        } else if let Some(v) = l.strip_prefix("name = ") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = l.strip_prefix("version = ") {
            version = v.trim_matches('"').to_string();
        } else if l.starts_with("source = ") && l.contains("registry+") {
            registry = true;
        }
    }
    flush(&mut name, &mut version, &mut registry, &mut pkgs);

    let registry_roots: Vec<std::path::PathBuf> = dirs_cargo_registry_src();
    let deps_dir = format!("{dir}/.candor/deps");
    let _ = std::fs::create_dir_all(&deps_dir);
    let (mut scanned, mut cached, mut missing) = (0usize, 0usize, Vec::new());
    let no_deps = DepIndex::default();
    // The dep scans used to be wrapped in a scoped `CANDOR_WORKSPACE_CHAIN=1`, because the
    // interfaceUnion emission was gated on that variable and `--deps` never set it — so the mechanism
    // was shipped-but-off in exactly the flow the help text tells people to use, and a `&dyn DepTrait`
    // dispatch read silent-pure (SOUNDNESS-VEIN-crossing-the-scan-boundary.md, cause 3). ⟨0.39⟩ REMOVED
    // THE GATE ITSELF (§4: the entry is REQUIRED, its absence a non-conformance), so the wrapper is gone
    // with it rather than left to read as a live precaution. Nothing reads the variable now.
    for (n, v) in &pkgs {
        let Some(src) = registry_roots.iter().map(|r| r.join(format!("{n}-{v}"))).find(|p| p.is_dir()) else {
            missing.push(format!("{n}-{v}"));
            continue;
        };
        // One subdirectory PER name@version: two locked versions of one crate must not overwrite
        // each other's report (review: last-write-wins silently fed the root the wrong version's
        // effects); with both present, conflicting keys drop as ambiguous — never-guess intact.
        let sub = format!("{deps_dir}/{n}@{v}");
        let already = std::fs::read_dir(&sub).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                let f = e.file_name();
                let f = f.to_string_lossy();
                f.ends_with(".scan.json") && !f.contains("callgraph")
            })
        });
        if already {
            cached += 1; // registry checkouts are immutable per name@version — the report stands
            continue;
        }
        let _ = std::fs::create_dir_all(&sub);
        // Dep scans are quiet, unchained, report-only, and GATE-FREE (the resolved root policy and
        // baseline are deliberately not passed): their job is the report files. A registry dep is a
        // single published package, so scan_one (not scan_target) is right; the json body is unused.
        let _ = scan_one(&src.to_string_lossy(), ScanOpts {
            prefix: format!("{sub}/report"),
            want_json: false,
            include_tests: false,
            policy: None,
            baseline: None,
            ws_member: true,   // ⟨0.31⟩ a nested sub-scan, not the invocation's own target
            quiet: true,
            deps_idx: &no_deps, peek_excluded: false,
        }, run);
        scanned += 1;
    }
    eprintln!(
        "candor-scan: --deps scanned {scanned} of {} registry dependencies into {deps_dir}{}{} \
(floor-engine reports: a dep's silent misses pass through — the κ caveat applies to the chain too)",
        pkgs.len(),
        if cached > 0 { format!(" ({cached} already scanned — cached)") } else { String::new() },
        if missing.is_empty() {
            String::new()
        } else {
            format!(" ({} without a local checkout: {}{})", missing.len(),
                missing.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                if missing.len() > 5 { ", …" } else { "" })
        }
    );
    // Chain the fresh dep reports (plus anything CANDOR_DEPS already names) under the root scan.
    // load_dep_reports dedups canonical paths, so deps_dir appearing in CANDOR_DEPS too is safe.
    let spec = match candor_report::env::operator("CANDOR_DEPS") {
        Ok(extra) if !extra.is_empty() => format!("{deps_dir}:{extra}"),
        _ => deps_dir.clone(),
    };
    let mut idx = load_dep_reports(Some(&spec));
    r1743_facade_owner_keys(&mut idx, dir);
    // The final root scan goes through scan_target so `--deps <workspace>` fans out over members
    // too — the nested-package filter would otherwise prune them all into an empty, gate-passing report.
    scan_target(dir, prefix, want_json, include_tests, policy, baseline, &idx, run)
}

/// SOUNDNESS R1743 — A CHAINED ENTRY KEYED UNDER A FACADE, JOINED BY A CONSUMER THAT SPELLS THE OWNER. Every producer that
/// depends on `serde` (not `serde_core`) keys its dispatch and its union rows from its own `use` map: serde-xml-rs 0.8.2
/// publishes `serde#de::Visitor::visit_u32` and a union row `serde#de::Deserializer::deserialize_u32`. A consumer that
/// implements and bounds the trait under its true owner (`impl serde_core::de::Visitor`, `D: serde_core::Deserializer`)
/// asks `serde_core#…` and joins neither: EXECUTED, its `Visitor` deletes a file and `deny Fs` read 0 on 0.41.3, 0.42.0 and
/// b805747a. [[R1715]]'s reverse join resolves the producer's key only for a facade the consumer itself depends on, and a
/// consumer of `serde_core` does not depend on `serde`.
///
/// The facade is the same one R1715 reads (`DepSigs::facade_owners`, the lock-pinned root re-exports — here the CONSUMER's
/// Cargo.lock, which pins the producer's `serde` transitively). Every chained key `F#p` whose facade `F` re-exports `p`'s
/// trait from an owner `O` is ALSO filed under `O#p'` (and its tail2), and every `dispatchesOn` member `F#p` gains `O#p'`
/// beside it. ADDITIVE ONLY (a key already present is unioned, nothing is renamed or withdrawn), so a consumer that spells
/// the facade joins exactly as before. No lock-pinned source for `F`: nothing is added (the reading may only add). The
/// packages read are recorded on the index and named in `dependencySources` by `scan_one`. `CANDOR_R1743_OFF=1` disables it.
pub(crate) fn r1743_facade_owner_keys(idx: &mut crate::deps::DepIndex, dir: &str) {
    if candor_report::env::switch("CANDOR_R1743_OFF") || idx.by_key.is_empty() {
        return;
    }
    let mut sigs = crate::depsig::DepSigs::default();
    // per facade package: does it re-export anything at all (else every key under it is its own)
    let mut fac: HashMap<String, bool> = HashMap::new();
    let mut memo: HashMap<String, Vec<String>> = HashMap::new();
    // `F#path::m` -> the owner keys it also names (full and tail2), `[]` when it names only itself
    let mut owner_keys = |k: &str, sigs: &mut crate::depsig::DepSigs, fac: &mut HashMap<String, bool>| -> Vec<String> {
        if let Some(v) = memo.get(k) {
            return v.clone();
        }
        let mut out: Vec<String> = Vec::new();
        if let Some((f, path)) = k.split_once('#') {
            let live = *fac.entry(f.to_string()).or_insert_with(|| {
                sigs.facade(dir, f, f).is_some_and(|x| !x.names.is_empty() || !x.globs.is_empty())
            });
            // a trait the facade DECLARES itself is its own, even where it also re-exports a same-named item from another
            // crate (zerocopy's `trait FromBytes` beside `pub use zerocopy_derive::FromBytes`, the derive macro; serde's
            // `Serialize` derive): the re-export is the macro namespace, never the trait's owner
            let own = |tp: &str, sigs: &mut crate::depsig::DepSigs| sigs.facade(dir, f, f).is_some_and(|x| x.declared.contains(tp));
            if let (true, Some((tp, m))) = (live, path.rsplit_once("::").filter(|(tp, _)| !own(tp, sigs))) {
                for (o, rest, _) in sigs.facade_owners(dir, f, f, tp) {
                    if o == f || sigs.facade(dir, &o, &o).is_some_and(|x| x.proc_macro) {
                        continue;
                    }
                    out.push(format!("{o}#{rest}::{m}"));
                    if let Some(t2) = tail2(&format!("{rest}::{m}")) {
                        out.push(format!("{o}#{t2}"));
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        memo.insert(k.to_string(), out.clone());
        out
    };
    let keys: Vec<String> = idx.by_key.keys().cloned().collect();
    let mut adds: Vec<(String, String)> = Vec::new(); // (new key, source key)
    for k in &keys {
        for nk in owner_keys(k, &mut sigs, &mut fac) {
            adds.push((nk, k.clone()));
        }
    }
    let mut member_adds: Vec<(String, String, Vec<String>)> = Vec::new(); // (entry key, member, owner members)
    for (k, d) in idx.by_key.iter() {
        for mem in &d.dispatches_on {
            let om: Vec<String> = owner_keys(mem, &mut sigs, &mut fac).into_iter().filter(|o| !d.dispatches_on.contains(o)).collect();
            if !om.is_empty() {
                member_adds.push((k.clone(), mem.clone(), om));
            }
        }
    }
    if adds.is_empty() && member_adds.is_empty() {
        return;
    }
    let mut read: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (k, mem, om) in member_adds {
        if let Some(f) = mem.split_once('#').map(|x| x.0) {
            read.insert(f.to_string());
        }
        let d = idx.by_key.get_mut(&k).expect("key from the same map");
        let union = d.union_dispatches_on.contains(&mem);
        for o in om {
            if candor_report::env::trace("CANDOR_R1743_INSTR") {
                eprintln!("R1743MEMBER\t{k}\t{mem}\t{o}"); // §E1 REACH COUNTER
            }
            if union {
                d.union_dispatches_on.insert(o.clone());
            }
            d.dispatches_on.insert(o);
        }
    }
    for (nk, src) in adds {
        let Some(d) = idx.by_key.get(&src).cloned() else { continue };
        if let Some(f) = src.split_once('#').map(|x| x.0) {
            read.insert(f.to_string());
        }
        if candor_report::env::trace("CANDOR_R1743_INSTR") {
            eprintln!("R1743KEY\t{src}\t{nk}"); // §E1 REACH COUNTER
        }
        idx.by_key.entry(nk.clone()).or_default().union_with(&d);
        if let Some(own) = idx.by_key_own.get(&src).cloned() {
            idx.by_key_own.entry(nk.clone()).or_default().union_with(&own);
        }
        if idx.union_keys.contains(&src) {
            idx.union_keys.insert(nk);
        }
    }
    idx.r1743_read.extend(read);
}

/// SOUNDNESS R145 — does this call's callee get looked up in a module of `scope`? The CALLER's module is
/// its qual minus the leaf, or minus two segments when the second-to-last names a local type or trait (a
/// method or trait default). A bare path is looked up there; a qualified one in the module it names,
/// read both crate-rooted and relative to the caller (the collector has already stripped a written
/// `crate::`/`self::`/`super::`, so either reading may be the meant one, and both only widen the hedge).
fn opaque_scope_names(
    caller: &str,
    path: &str,
    local_types: &std::collections::HashSet<String>,
    trait_decls: &HashMap<String, crate::model::LocalTrait>,
    scope: &std::collections::HashSet<String>,
) -> bool {
    let segs: Vec<&str> = caller.split("::").collect();
    let n = segs.len();
    let module = if n >= 2 && (local_types.contains(segs[n - 2]) || trait_decls.contains_key(segs[n - 2])) {
        segs[..n - 2].join("::")
    } else {
        segs[..n.saturating_sub(1)].join("::")
    };
    let p = path.strip_prefix("crate::").unwrap_or(path);
    match p.rsplit_once("::") {
        None => scope.contains(&module),
        Some((owner, _)) => {
            scope.contains(owner) || (!module.is_empty() && scope.contains(&format!("{module}::{owner}")))
        }
    }
}


/// EXACT-FIRST (Track A step 2) — whether the module a held qual names (`sys` in `sys::R::fmt`) settles that name in
/// EVERY configuration this scanner reads, so the claimant whose qual equals it is the one the program runs. It does
/// not where the module ALSO binds the name by `use` (or imports a glob) — Rust allows that beside a declaration
/// under exclusive `#[cfg]`s, and this scanner reads both arms (executed: `fx/adv/b3_*`, `b5_child`, `b6_glob*`,
/// `c3_modtwin` all wrote the file while the declared, dead arm was pure) — nor where the module, or one it sits in,
/// is placed by a `#[path]` this scanner does not follow (`c6_path_nat_rev`: the naturally placed file is the DEAD
/// arm), nor where a macro hid some of its items. Each refusal keeps today's answer (the contested tail's hedge).
pub(crate) struct ExactGuard<'a> {
    pub(crate) use_bound: &'a HashSet<String>,
    pub(crate) redirect_mods: &'a [String],
    pub(crate) macro_modules: &'a HashSet<String>,
    pub(crate) twin_field_tys: &'a HashSet<String>,
}

pub(crate) fn exact_ty_module_settled(target: &str, writer: &str, g: &ExactGuard, caller: &str, path: &str) -> bool {
    let ExactGuard { use_bound, redirect_mods, macro_modules, twin_field_tys } = *g;
    let Some((ty_qual, _member)) = target.rsplit_once("::") else { return false };
    let (module, name) = ty_qual.rsplit_once("::").unwrap_or(("", ty_qual));
    let why = if twin_field_tys.contains(&format!("crate::{ty_qual}")) || twin_field_tys.contains(ty_qual) {
        // The held type is one a twin owner's leaf-keyed field answers with: it may be the OTHER twin's field.
        Some("twin-owner field type")
    } else if std::iter::successors(Some(ty_qual), |m| m.rsplit_once("::").map(|(p, _)| p))
        .any(|m| use_bound.contains(&format!("{}{m}", crate::decls::ARM_SRC_MARK)))
    {
        // The held type, or a module it sits in, is ONE arm of a `#[cfg]`-duplicated `use` (`pub use crate::x86 as
        // default;` beside its `arm`/`scalar` twins): the typer's answer is one configuration's.
        Some("cfg-arm of a duplicated use")
    } else if use_bound.contains(&crate::decls::qualify(module, name)) {
        Some("use-bound name")
    } else if use_bound.contains(&crate::decls::qualify(module, "*")) {
        Some("glob in module")
    } else if [module, writer].iter().any(|&start| {
        redirect_mods.iter().any(|m| start == m || start.starts_with(&format!("{m}::")))
            || std::iter::successors(Some(start), |m| m.rsplit_once("::").map(|(p, _)| p))
                .any(|m| use_bound.contains(&format!("{}{m}", crate::decls::UNSETTLED_MOD_MARK)))
    }) {
        Some("path-redirected module")
    } else if macro_modules.contains(module) || macro_modules.contains(writer) {
        Some("macro-hidden module")
    } else if use_bound.contains(&format!("{}{}", crate::decls::CONTESTED_NAME_MARK, crate::decls::qualify(writer, name))) {
        // The module the name was WRITTEN in both declares it and binds it by `use`/glob: its `use`-map answer is
        // one configuration's (`fx/adv/b7_*`).
        Some("contested name in the writing module")
    } else {
        None
    };
    if let Some(why) = why {
        if candor_report::env::trace("CANDOR_EXACTTY_INSTR") {
            eprintln!("EXACTTY\tREFUSE\t{caller}\t{path}\t{target}\t{why}"); // §E1 — the guard's own reach
        }
        return false;
    }
    true
}

/// SOUNDNESS R1484 — the arm of path-redirected module `m` that the caller `qual` (defined at `loc`) PROVABLY compiles
/// inside, or `None` when that is not proven. The arm is the longest of `arms` the qual sits under (`m` itself — a twin at
/// the module's own name — only when no other arm holds it). Proof: every module from the arm down to the caller is placed
/// by exactly ONE `mod` item (`places`, `decls::collect_mod_places`), the arm by `m`'s own and each deeper one by an item
/// declared inside the chain, and every module between the arm and the caller's FILE is placed at all (an unplaced one
/// was declared somewhere this scan cannot see). A crate where a macro may place a module refuses outright.
fn r1484_own_arm(
    qual: &str,
    loc: &str,
    m: &str,
    arms: &[String],
    places: &HashMap<String, std::collections::BTreeSet<String>>,
) -> Option<String> {
    if places.contains_key(crate::decls::HIDDEN_MOD_PLACE) {
        return None;
    }
    let under = |q: &str, a: &str| q.strip_prefix(a).is_some_and(|r| r.starts_with("::"));
    let arm = arms
        .iter()
        .filter(|a| a.as_str() != m && under(qual, a))
        .max_by_key(|a| a.len())
        .cloned()
        .or_else(|| under(qual, m).then(|| m.to_string()))?;
    let file = loc.rsplitn(3, ':').nth(2).unwrap_or("");
    if file.is_empty() {
        return None;
    }
    let file_mod = crate::lang::module_path(Path::new(file));
    let segs: Vec<&str> = qual.split("::").collect();
    let arm_n = arm.split("::").count();
    for n in arm_n..segs.len() {
        let q = segs[..n].join("::");
        let to_file = q == file_mod || under(&file_mod, &q);
        match places.get(&q) {
            Some(by) if by.len() == 1 => {
                let d = by.iter().next().unwrap();
                if n == arm_n {
                    if d != m {
                        return None;
                    }
                } else {
                    let parent = d.rsplit_once("::").map_or("", |(p, _)| p);
                    if !(parent == arm || under(parent, &arm)) || !(qual == parent || under(qual, parent)) {
                        return None;
                    }
                }
            }
            Some(_) => return None,
            None if to_file || n == arm_n => return None,
            None => {}
        }
    }
    Some(arm)
}
