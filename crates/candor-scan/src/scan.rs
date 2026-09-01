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
fn prescan_argv(args: &[String]) -> PreScan {
    let mut ps = PreScan { gate_sinks: Vec::new(), policy: None, outs: Vec::new(), target: None };
    let mut it = args.iter().peekable();
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
            _ => {
                if !a.starts_with('-') {
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

/// SPEC §3.3.1 ⟨0.27⟩ — is this one artifact under two names?
///
/// NOT a path-component comparison. The guard that shipped here compared `Path::new(pp) ==
/// Path::new(gp)`, which a review defeated with `--policy /w/P --gate-json ./P` run from `/w`: same
/// file, different spelling, policy destroyed, exit 0 with `ok: true`. Canonicalisation resolves `.`,
/// `..` and symlinks; where the sink does not exist yet (the normal case — we are about to create it)
/// its parent is canonicalised and the file name appended. "Resolve the artifact, not just the string"
/// is the rule that caught the release verifier; it applies here for the same reason.
fn same_artifact(a: &str, b: &str) -> bool {
    if a == "-" || b == "-" {
        return false;
    }
    fn resolve(p: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(p);
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        let parent = p.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        Some(parent.canonicalize().ok()?.join(p.file_name()?))
    }
    // ⟨0.28⟩ DEVICE+INODE FIRST, where the platform offers it. Path equality alone called two HARDLINKS
    // to one inode two different sinks and refused a legal command — the mirror of the stale green, and
    // measured 1-vs-3 across the engines. §3.3.1 asks for device+inode and that was read as advisory.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            if ma.dev() == mb.dev() && ma.ino() == mb.ino() {
                return true;
            }
        }
    }
    // …and a symlink whose target does not exist YET still names that target: `canonicalize` fails on a
    // dangling link, so resolve it explicitly before falling back to the parent-directory form.
    let (ra, rb) = (
        candor_report::resolve_sink_artifact(std::path::Path::new(a)),
        candor_report::resolve_sink_artifact(std::path::Path::new(b)),
    );
    if ra != std::path::Path::new(a) || rb != std::path::Path::new(b) {
        if let (Some(x), Some(y)) = (resolve(&ra.to_string_lossy()), resolve(&rb.to_string_lossy())) {
            if x == y {
                return true;
            }
        }
    }
    match (resolve(a), resolve(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
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
        if let Ok(v) = std::env::var(var) {
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
    if let Ok(d) = std::env::var("CANDOR_DEPS") {
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
        // Exactly one sink: the ordinary single-sink guard. It exits 2 having written nothing, which is
        // the whole of rule (2) when there is no other sink with a reader waiting.
        refuse_gate_json_over_any_input(gp, pre_target.as_deref().unwrap_or("."), pre_policy.as_deref());
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
        crate::gate::note_report_prefix(&format!("{t}/.candor/report"));
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
        .or_else(|| std::env::var("CANDOR_POLICY").ok())
        .or_else(|| cfg.get("policy").cloned());
    // The AS-EFF-005 baseline (spec §7 item 5), resolved once like the policy: CANDOR_BASELINE env
    // over the config `baseline` key (already home-anchored by load_candor_config). Dependency scans
    // under --deps run guard-free — a dep's internals are not this repo's ratchet.
    // WHICH SOURCE supplied the baseline decides what a MISSING file means (see `check_baseline`):
    // `CANDOR_BASELINE` is set unconditionally by the adopt workflow, so an absent path there is "the
    // ratchet is not adopted yet"; a checked-in `baseline` line DECLARES this repo has one, so an absent
    // path there was deleted or never committed — and passing green over it is a gate that stopped
    // gating in silence.
    let _ = crate::gate::BASELINE_FROM_CONFIG.set(std::env::var("CANDOR_BASELINE").is_err() && cfg.contains_key("baseline"));
    let baseline = std::env::var("CANDOR_BASELINE").ok().or_else(|| cfg.get("baseline").cloned());
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
    if deps_mode {
        let code = run_with_deps(&dir, prefix, want_json, include_tests, policy, baseline, &run_tok);
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
    let deps_spec = std::env::var("CANDOR_DEPS").ok().or_else(|| cfg.get("deps").cloned());
    let deps_idx = load_dep_reports(deps_spec.as_deref());
    // scan_target handles both a single crate and a `[workspace]` root (one report per member under
    // one prefix — candor-query's multi-crate merge consumes them together; the policy gates each).
    let code = scan_target(&dir, prefix, want_json, include_tests, policy, baseline, &deps_idx, &run_tok);
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
        let Some(tp) = &f.ret_bound_type else { continue };
        // EXACT match on the full qual, and that is the whole disambiguation. Both sides are now
        // module-qualified in this crate's own namespace — `ret_bound_type` resolves a bare name
        // against its DECLARING module — so `mock::Client` and `sync::Client` are different strings
        // and neither can stand in for the other. A suffix/leaf match here is defect 1.
        if !nonpure.contains(tp.as_str()) {
            if nonpure.iter().any(|q| q.rsplit("::").next() == tp.rsplit("::").next()) {
                near_misses += 1; // same type LEAF, different module — the case that MUST not resolve
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
    if std::env::var("CANDOR_TYPESURFACE_DEBUG").is_ok() {
        let bound: usize = fns.iter().filter(|f| f.ret_bound_type.is_some()).count();
        eprintln!(
            "TYPESURFACE {crate_name}: fns={} bound_returns={bound} nonpure_type_quals={} \
             published={} leaf_near_misses={near_misses}",
            fns.len(), nonpure.len(), map.len()
        );
    }
    candor_report::TypeSurface { returns: map }
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
    // When peeking, every `continue` below becomes a KEEP and every keep becomes a skip — one flag, one
    // walk, so the two file sets are exact complements and no file can fall between them.
    let peeking = peek_excluded;

    for entry in walkdir::WalkDir::new(root)
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
        // A `#[cfg(test)] mod tests;` FILE module is invisible here — its test-ness is declared at the
        // `mod` site, not in the file — so a `tests.rs` / `*_tests.rs` / `*_test.rs` file's effects (a
        // seeded RNG, a temp file) would be mis-read as the crate's. By convention these stems are test
        // modules; skip them by default. (base64's `engine/tests.rs` otherwise reported a phantom `Rand`.)
        if !include_tests {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                if is_test_file_stem(stem) {
                    excluded.push((rel.to_string_lossy().into_owned(), "test-module"));
                    if peeking { paths.push((p.to_path_buf(), rel.to_string_lossy().into_owned())); }
                    continue;
                }
            }
        }
        if peeking {
            continue;   // an in-scope file is exactly what the peek is NOT about
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
    let hashes: Vec<(String, String)> = paths
        .par_iter()
        .map(|(p, rel)| (rel.clone(), std::fs::read(p).map(|b| fnv1a(&b)).unwrap_or_default()))
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
    let round1: Vec<Option<ParsedFile>> = per_file
        .par_iter()
        .map(|(rel, _, cached)| {
            if cached.is_some() {
                return None; // decls reusable from cache — defer the parse (it may not be needed at all)
            }
            let p = &paths.iter().find(|(_, r)| r == rel)?.0;
            let text = std::fs::read_to_string(p).ok()?;
            let file = syn::parse_file(&text).ok()?;
            let mut locs = Vec::new();
            fn_locs(&file.items, rel, include_tests, &mut locs);
            // SAFETY: see `SendFile` — freshly parsed, uniquely owned, moved once, then single-threaded.
            Some((SendFile(file), locs))
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
                let Some((sf, locs)) = r1 else { continue };
                let fd = file_decls(&sf.0.items, include_tests, Path::new(&rel));
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
    let decl_index_hash = decl_index_digest(&merged);
    // Keep only unambiguous fn-leaf -> return-type / enum-variant-payload mappings (the `None`s drop).
    let returns: ReturnIndex =
        merged.rets.iter().filter_map(|(k, v)| v.clone().map(|t| (k.clone(), t))).collect();
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
    let trait_fields = &merged.trait_fields;
    let traits = TraitIndexes { impls: trait_impls, decls: trait_decls, fields: trait_fields };
    let elems = ElemIndexes { field_elem, field_elem_trait, enum_variants: &enum_variants, enum_variant_traits: &enum_variant_traits, ambiguous_enum_leaves: &ambiguous_enum_leaves };
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
    let owned_drops: HashMap<String, BTreeSet<String>> = if merged.drop_types.is_empty() {
        HashMap::new()
    } else {
        // A field type's LEAF (`type_path` may qualify it — `inner: ffi::Deflate` — but `owned_drops`,
        // `drop_types`, and the struct keys are all LEAF-keyed, so compare by leaf or the transitive
        // owner chain breaks across modules).
        let candidates = |t: &str| -> Vec<String> {
            let leaf = |ty: &String| ty.rsplit("::").next().unwrap_or(ty).to_string();
            let mut v: Vec<String> = Vec::new();
            if let Some(m) = fields.get(t) {
                v.extend(m.values().map(&leaf));
            }
            if let Some(m) = field_elem.get(t) {
                v.extend(m.values().map(&leaf));
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
        owned
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
        .collect();

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
                .and_then(|t| syn::parse_file(&t).ok())
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
            if std::env::var("CANDOR_PANIC_ON_FILE").is_ok_and(|v| v == *rel) {
                panic!("CANDOR_PANIC_ON_FILE: injected fault while walking {rel}");
            }
            let mut out: Vec<FnInfo> = Vec::new();
            let mut idx = loc_idx;
            let mut u = uses.clone();
            scan_items(&file.items, &modpath, locs, &mut idx, include_tests, fields, &returns, traits, elems, lazy_statics, const_strings, local_macros, &drop_relevant, &mut u, &mut out);
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
    if incremental {
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
    // dep crate root -> count of FLOORED call sites into it. Floored only: the tally has to mean the
    // same thing as the crate name beside it — calls whose effects this scan could not see.
    let mut dep_seen: HashMap<String, usize> = HashMap::new();
    // fn -> the dep crates it DIRECTLY calls into where the classifier floored the call. Post-filtered to
    // the genuinely-blind crates (not calibrated, no sibling report) + propagated transitively → the
    // per-fn `invisible` honesty disclosure (the κ ledger, but attributed per function).
    let mut blind_direct: HashMap<String, BTreeSet<String>> = HashMap::new();
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
    let mut by_leaf: HashMap<String, Vec<String>> = HashMap::new();
    let mut by_tail2: HashMap<String, Vec<String>> = HashMap::new();
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
        let is_lazy_unit = f.qual.starts_with(LAZY_UNIT_PREFIX);
        if !is_lazy_unit {
            by_leaf.entry(f.leaf.clone()).or_default().push(f.qual.clone());
        }
        if let Some(t2) = tail2(&f.qual) {
            if let Some(ty) = t2.split("::").next() {
                if ty.chars().next().is_some_and(|c| c.is_uppercase()) {
                    local_types.insert(ty.to_string());
                }
            }
            by_tail2.entry(t2).or_default().push(f.qual.clone());
        }
    }
    // ⟨peek-scope-attribution⟩ `(trait_leaf, method_leaf) -> the in-scope fns that DIRECTLY dispatch
    // there` (see `FnInfo::dispatch`'s doc comment for why this exists and what it does NOT do — it
    // never contributes an effect, only a reachability fact). Built once, from this run's own Pass-B
    // output; consulted only by the ⟨0.29⟩ peek's out-of-scope block, far below, to test a policy's scope
    // against every in-scope function that could REACH a peeked declaration via dynamic dispatch — not
    // only the peeked declaration's own name.
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
    let by_reexport = reexport_aliases(&merged.reexports, &fns);

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
    // Per-fn DIRECT Unknown-origin reasons (the receipt's `unknownWhy`, spec §2). Coarse, like the lint's
    // per-trait tag: a callback we can't see through, an FFI/extern boundary, or a genuinely-unresolvable
    // bare call. Tracked so the disclosure names WHY, not just that an Unknown exists.
    // `String`, not `&'static str`: a chained dependency's own `unknownWhy` is carried VERBATIM across the
    // join (`dispatch:<owner>.<member>` has a normative detail that re-deriving would destroy), so the set
    // can no longer be only compile-time literals.
    let mut unknown_why: HashMap<String, BTreeSet<String>> = HashMap::new();
    for f in &fns {
        loc.entry(f.qual.clone()).or_insert_with(|| f.loc.clone());
        // The body invoked a callable the scan can't see through (closure / fn-pointer value): it could
        // perform any effect, so record an honest `Unknown` (propagated like any effect, surfaced in the
        // receipt's unresolved count) instead of silently certifying the function pure.
        if f.unresolved {
            direct.entry(f.qual.clone()).or_default().insert("Unknown");
            unknown_why.entry(f.qual.clone()).or_default().insert("callback:unresolved call".to_string());
        }
        // DROP-GLUE (#3): local types this fn CONSTRUCTS that have a local `impl Drop`. The `Drop::drop`
        // body runs at scope exit — an implicit edge the call graph misses, so a guard that flushes/closes
        // on drop read silent-pure. Collected per-call below (a `T::*` associated-fn call where `T` has a
        // local Drop), then edged to `T::drop` after the call loop. Over-approximates toward the SOUND
        // direction (a constructed value is assumed to drop in this scope); gated to LOCAL drop types only,
        // so an external type's invisible Drop is never fabricated.
        let mut drops_here: BTreeSet<String> = BTreeSet::new();
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
                // THIRD conjunct, and it is what makes this precise rather than noisy: the dep must be
                // CHAINED. For an UNCHAINED dep the κ ledger already discloses `invisible: [cr]`, so the
                // reader is warned and a second disclosure buys nothing but false uncertainty — measured,
                // that arm fired on `let finds = dep::best_finds(); finds.first()`, where the value came
                // from a dep but `.first()` is a std Vec method. Only when the dep IS chained does the
                // ledger fall silent (covered, correctly, per §2 rule 3) — and that silence is the
                // confident purity claim this exists to prevent.
                if deps_idx.crates.contains(cr_real) {
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
                    if std::env::var("CANDOR_TYPESURFACE_DEBUG").is_ok() {
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
            let classified = candor_classify::classify(cr_real, &path_real)
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
                }
                // FIELD edition (R49): constructing `ty` also runs the drop glue of any local drop-type
                // it transitively OWNS through a field. The collector's lexical escape gate has already
                // refused an escaping construction; `returns_escapable` is kept here as a SECOND, purely
                // NARROWING conjunct on this half alone, so the field route can only lose charges
                // relative to what shipped — R49's A/B is not being re-opened by this change.
                if !returns_escapable {
                    if let Some(owned) = owned_drops.get(&ty) {
                        drops_here.extend(owned.iter().cloned());
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
                    && tail2(&c.path)
                        .and_then(|t2| t2.split("::").next().map(str::to_string))
                        .is_some_and(|ty| local_types.contains(&ty))
            } else {
                !matches!(cr, "std" | "core" | "alloc")
            };
            // A `Type::assoc()` whose `Type` is a NON-NOMINAL alias (`type Inner = [u8; N]`) names a type
            // with no local impl — its assoc fn is std/core's, NOT a same-named local STRUCT's. Skip the
            // local link so the array alias's `Inner::default()` doesn't inherit `struct Inner`'s
            // effectful `Default` (the sled IVec fabrication).
            let aliased = tail2(&c.path)
                .and_then(|t2| t2.split("::").next().map(str::to_string))
                .is_some_and(|ty| merged.prim_aliases.contains(&ty));
            // Did this call resolve to a LOCAL definition (free fn, method, or a unique trait-default)?
            // If so the local def is AUTHORITATIVE and its effects flow through the `calls` edge — the
            // crate/FFI classifier MUST NOT also fire, or a pure local fn whose NAME collides with an FFI
            // tier (`sqlite3_step`/`git_clone`/`curl_*`/`SSL_*`) or a whole-crate rule (`getrandom`/
            // `fastrand`) inherits that crate's effect: FABRICATION on a provably-pure path, transitively
            // poisoning every caller (a fabrication — the precision failure the syntactic floor must never commit). The
            // bare-leaf-METHOD suppression below was the special case of this; this covers the general
            // case (free fns and qualified `Type::method` calls the bare-leaf guard missed).
            let mut resolved_local = false;
            if resolvable && !aliased {
                let targets = resolve_target(&c.path, &c.leaf, c.method, &by_tail2, &by_leaf)
                    .or_else(|| reexport_target(&c.path, c.method, &by_tail2, &by_reexport));
                if let Some(targets) = targets {
                    resolved_local = true;
                    for t in targets {
                        if t != &f.qual {
                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                        }
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
                            if hits.len() == 1 && hits[0] != &f.qual {
                                resolved_local = true;
                                calls.entry(f.qual.clone()).or_default().insert(hits[0].clone());
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
                                }
                            }
                        }
                        // AUTO-DEREF fallback (last, after inherent + trait-default — Rust's resolution
                        // order): a custom `impl Deref for t_type { type Target = U }` makes `recv.leaf()`
                        // dispatch to `U::leaf`. Chase the Deref chain (bounded) and edge to the first
                        // `U::leaf` that resolves — the user-Deref analog of the Box/Arc/Rc peel (a newtype
                        // `impl Deref` dropped `wrapper.method()` to silent-pure — corpus find). `.clone()`
                        // is guarded at the typed-call emit, so no pointee-clone fabrication recurs.
                        if !resolved_local {
                            let mut cur = t_type.clone();
                            let mut hops = 0;
                            while let Some(target) = merged.deref_target.get(&cur).cloned() {
                                if hops >= 8 { break; }
                                hops += 1;
                                if let Some(ts) = resolve_target(&format!("{target}::{}", c.leaf), &c.leaf, false, &by_tail2, &by_leaf) {
                                    resolved_local = true;
                                    for t in ts {
                                        if t != &f.qual {
                                            calls.entry(f.qual.clone()).or_default().insert(t.clone());
                                        }
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
            if classified.is_none() && !resolved_local && !suppress_bare_leaf
                && c.path.contains("::") && deps_idx.crates.contains(cr_real)
            {
                let rel = c.path.strip_prefix(&format!("{cr}::")).unwrap_or(&c.path);
                let key = if rel.contains("::") {
                    tail2(rel).map(|t2| format!("{cr_real}#{t2}"))
                } else {
                    Some(format!("{cr_real}#{rel}"))
                };
                let hit = key.as_ref().and_then(|k| deps_idx.by_key.get(k));
                if let Some(de) = hit {
                    dep_join_hit = true;
                    apply_dep_fn(de, &f.qual, DepSink {
                        direct: &mut direct, hosts: &mut hosts, cmds: &mut cmds, paths: &mut paths,
                        tables: &mut tables, incomplete: &mut incomplete, unknown_why: &mut unknown_why,
                        blind_direct: &mut blind_direct, dep_invisible: &mut dep_invisible,
                            unknown_via_dep: &mut unknown_via_dep,
                    });
                    // (No coverage marking here. A crate whose sibling report we joined is already
                    // covered by the `deps_idx.crates` arm of the ledger filter below — that arm is the
                    // reviewed claim, and it holds whether or not any single join happened to fire.)
                }
            }
            if let Some(eff) = classified.filter(|_| !suppress_bare_leaf && !resolved_local) {
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
                if model_sdk && !suppress_bare_leaf && !resolved_local {
                    let d = direct.entry(f.qual.clone()).or_default();
                    d.insert("Llm");
                    d.insert("Net");
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
                    if eff == "Net" && candor_classify::is_net_establishing(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Net");
                    } else if eff == "Exec" && candor_classify::is_cmd_naming_method(&c.leaf) {
                        incomplete.entry(f.qual.clone()).or_default().insert("Exec");
                    } else if eff == "Fs" && !c.method && candor_classify::is_fs_path_arg(&c.leaf) {
                        // A path-NAMING Fs call (`fs::write(p,…)`/`File::open(p)` — a free fn / constructor,
                        // `method=false`) with NO captured path literal → the path is a runtime value,
                        // invisible to the gate. Mark Fs incomplete so a benign sibling literal can't certify
                        // the masked path (`allow Fs` fails closed). The `!c.method` gate excludes the
                        // path-stat METHODS (`p.metadata()`/`p.exists()`) whose path is the RECEIVER, not an
                        // arg — same establishing-allowlist discipline as Net/Exec (matches candor-java).
                        incomplete.entry(f.qual.clone()).or_default().insert("Fs");
                    } else if eff == "Db" && candor_classify::is_db_query_arg(&c.leaf) {
                        // A SQL-QUERY-bearing Db call (`con.execute(sql,…)`/`query`/`prepare`) with NO captured
                        // query literal → the table is a runtime value, invisible to the gate. Mark Db
                        // incomplete so a benign sibling literal can't certify the masked table. The allowlist
                        // excludes build-then-execute terminals (`fetch_all`/`load`/`all`) and lifecycle ops
                        // (`connect`/`open`/`begin`) whose query is built structurally (no maskable string).
                        incomplete.entry(f.qual.clone()).or_default().insert("Db");
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
                if eff == "Net" && candor_classify::is_net_binding(&c.leaf) {
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
            let already_handled = classified.is_some() || resolved_local || suppress_bare_leaf || dep_join_hit;
            if !c.is_macro && !already_handled && merged.extern_fns.contains(&c.leaf) {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("native:extern fn".to_string()); // FFI is a native boundary — canonical `native:` (SPEC §4 ⟨0.7⟩)
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
            if !c.is_macro && !c.method && classified.is_none() && !resolved_local && suppress_bare_leaf
                && !c.path.contains("::")
                && by_leaf.get(&c.leaf).is_some_and(|v| v.len() >= 2)
            {
                direct.entry(f.qual.clone()).or_default().insert("Unknown");
                unknown_why.entry(f.qual.clone()).or_default().insert("ambiguous:same-name local defs".to_string());
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
                }
            }
        }
    }

    // `all` DELIBERATELY KEEPS ONE ENTRY PER `FnInfo`, DUPLICATES INCLUDED — see the doc note on
    // `blind_direct`'s population loop above for why a genuine per-cfg-branch keying fix belongs there,
    // not here, and why deduplicating THIS list is the wrong repair (it was tried and reverted: it
    // silently drops a report entry, which `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two`
    // (candor-scan/tests/cli.rs) pins as a REQUIRED shape — "the report itself still lists both units,
    // so this is a GATE de-duplication, not a lost entry").
    let all: Vec<String> = fns.iter().map(|f| f.qual.clone()).collect();
    let inferred = propagate(&direct, &calls, &all);
    let hostsacc = propagate_str(&hosts, &calls, &all);
    // `fs` kinds propagate like the literal surfaces; the "?" poison propagates WITH them, which is what
    // makes a caller of an undetermined-kind function inherit the suppression rather than a half-answer.
    let fskindsacc = propagate_str(&fskinds, &calls, &all);
    let cmdsacc = propagate_str(&cmds, &calls, &all);
    let pathsacc = propagate_str(&paths, &calls, &all);
    let tablesacc = propagate_str(&tables, &calls, &all);
    let incompleteacc = propagate(&incomplete, &calls, &all); // transitive masking-incompleteness
    let blind_acc = propagate_str(&blind_direct, &calls, &all); // transitive per-fn blind reach
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
    let non_registry_names: std::collections::HashSet<String> =
        non_registry_lock_names(dir).into_iter().chain(non_registry_manifest_names(dir)).collect();
    let mut coverage_ledger: Vec<(String, usize)> = dep_seen
        .iter()
        .filter(|(cr, _)| {
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
            let fully_calibrated = candor_classify::CALIBRATED_CRATES.contains(&cr.as_str())
                && !candor_classify::CALIBRATED_BUT_PARTIAL_CRATES.contains(&cr.as_str())
                && !impostor;
            !covered
                && !fully_calibrated
                && !(candor_classify::PATH_CALIBRATED_CRATES.contains(&cr.as_str()) && !impostor)
                && !(candor_classify::CALIBRATED_PREFIXES.iter().any(|p| cr.starts_with(p)) && !impostor)
                // REVIEWED-PURE: read and found to perform no effect of its own, so its silence is an
                // answer rather than a gap. A separate list from the calibrated ones because those must
                // carry a live rule (`calibrated_crates_are_live`) and a pure crate has none — see the
                // evidence requirement on `REVIEWED_PURE_CRATES`. Same impostor carve-out: a reviewed-
                // pure verdict is about the real crate too.
                && !(candor_classify::REVIEWED_PURE_CRATES.contains(&cr.as_str()) && !impostor)
        })
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
    for q in &all {
        // SPEC §2.2: the sidecar records EVERY analyzed function — including a LEAF with no local
        // callees, as an empty list. Omitting leaves made an uncalled FFI-only fn (nix `unistd::pipe`)
        // invisible to `whatif`/`callers` ("no function matching") even though it's in the report;
        // an always-present key also lets a consumer distinguish "no callers" from "no such function".
        cg.insert(q.clone(), calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default());
        let inf = inferred.get(q).cloned().unwrap_or_default();
        // Keep a pure fn if it has a BLIND reach — so the honesty disclosure survives on exactly the
        // `inferred: []` fns that need it (else `invisible` would be dropped with the pure entry).
        let has_blind = blind_acc.get(q).is_some_and(|s| s.iter().any(|c| global_blind.contains(c)));
        if inf.is_empty() && !has_blind {
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
            calls: calls.get(q).map(|cs| cs.iter().cloned().collect()).unwrap_or_default(),
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
        });
    }
    // ⟨workspace-chain, gated⟩ TRAIT-CHA union entries — the candor-ts/swift `interfaceUnion` analog. A
    // cross-crate consumer calling a trait method on a `&dyn Trait` whose Trait is imported from HERE keys
    // the chain lookup on `crate#Trait::method` (tail2), which has no body → no entry → the call reads pure.
    // Emit a synthetic entry = the UNION over local impls of that method's effects (inferred + invisible),
    // reusing `trait_impls`/`trait_decls` (the CHA universe in-crate dispatch already uses). Sound
    // over-approximation; a `Trait::method` a consumer never resolves is harmless. GATED so a default scan
    // stays byte-identical (four-way conformance unaffected until the rung is pinned).
    if std::env::var_os("CANDOR_WORKSPACE_CHAIN").is_some() {
        let existing: std::collections::HashSet<String> = entries.iter().map(|e| e.hash.clone()).collect();
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
            for method in &lt.methods {
                let mut inf_u: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
                let mut blind_u: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
                for ty in impls {
                    let ty_leaf = ty.rsplit("::").next().unwrap_or(ty);
                    for cand in [format!("{ty}::{method}"), format!("{ty_leaf}::{method}")] {
                        if let Some(s) = inferred.get(&cand) {
                            for e in s {
                                inf_u.insert(e);
                            }
                        }
                        if let Some(s) = blind_acc.get(&cand) {
                            for c in s.iter().filter(|c| global_blind.contains(*c)) {
                                blind_u.insert(c.clone());
                            }
                        }
                    }
                }
                if inf_u.is_empty() && blind_u.is_empty() {
                    continue; // pure across all impls — silence = purity
                }
                let hash = format!("{crate_name}#{trait_leaf}::{method}");
                if existing.contains(&hash) {
                    continue; // a real entry already claims this hash
                }
                entries.push(ReportEntry {
                    func: format!("{trait_leaf}::{method}"),
                    inferred: inf_u.iter().map(|s| s.to_string()).collect(),
                    unresolved: inf_u.contains("Unknown"),
                    hash,
                    invisible: blind_u.into_iter().collect(),
                    interface_union: true,
                    ..Default::default()
                });
            }
        }
    }
    entries.sort_by(|a, b| a.func.cmp(&b.func));

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
        let mut sorted = all.clone();
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
    let type_surface = build_type_surface(&crate_name, &fns, &entries);
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
    let out_of_scope: Option<Vec<candor_report::OutOfScopeFinding>> = policy_path
        .as_ref()
        .and_then(|pp| std::fs::read_to_string(pp).ok())
        .and_then(|text| {
            let parsed = candor_classify::policy::parse_policy(&text);
            // ⟨0.29⟩ A REFUSED POLICY LEAVES THE KEY ABSENT (SPEC §2). `and_then`, not `map`, because the
            // distinction this returns is present-vs-absent and `map` cannot express it. The peek is a
            // producer reading the policy, so §3.1 binds it exactly as it binds the gate: over a policy
            // no route will honour, `outOfScope: []` claims a look taken against rules that never stood —
            // and the denied set it would have looked for is the parser's SALVAGE of an unhonourable
            // file, which is the rewriting `fatal_messages` exists to refuse. candor-java already
            // withheld here; this engine, candor-ts and candor-swift did not.
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
                    "test-module" => "a `tests.rs`/`*_test.rs` file module is a #[cfg(test)] tree whose \
                         test-ness is declared at the `mod` site, invisible when walking files"
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
    let body = candor_report::to_packaged_report_json_typed(
        &meta, &crate_name, &entries, coverage.as_ref(), &unanalyzed_units, Some(&analyzed),
        Some(&type_surface), &excluded_classes, out_of_scope.as_deref(), scanned_under.as_ref(),
        net_partners_record.as_ref())
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
            eprintln!(
                "candor-scan: wrote {} effectful functions to {file} (stable syntactic backend — see --help)",
                entries.len()
            );
            // Effect breakdown — make the result visible at a glance, not just a count + a file path.
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for e in &entries {
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
    if let Some(bv) = &baseline_value {
        // NOTE: the incomplete-analysis refusal is INSIDE the `Checked` arm below, not here. It used to
        // sit at this point — before `check_baseline` ran at all — and that ordering dropped real
        // findings. See the comment on the refusal for the measurement.
        match check_baseline(bv, dir, &crate_name, &all, &inferred, crate::gate::unknown_ratchet(), crate::gate::baseline_from_config()) {
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
                if had_parse_failure && v.is_empty() {
                    let why = "baseline guard NOT evaluated — source failed to parse (see above); the \
                               guard cannot compare unanalyzed code";
                    eprintln!("candor-scan: {why}");
                    crate::gate::record_gate_refusal(why);
                    return (2, json_body);
                }
                if v.is_empty() {
                    eprintln!("candor-scan: baseline guard ✓ — no function gained an effect (advisory floor: the syntactic backend under-reports)");
                } else {
                    eprintln!("candor-scan: {} baseline regression(s) — an existing function gained an effect (AS-EFF-005)", v.len());
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
        let cfg_vocab = candor_classify::policy::discover_config(std::path::Path::new(&pp));
        let unknown_aliases = cfg_vocab
            .as_ref()
            .map(|(_, t)| candor_classify::policy::parse_unknown_aliases(t))
            .unwrap_or_default();
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
        let (perrs, used_aliases, pignored) = crate::gate::policy_precheck(&text, &unknown_aliases);
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
        let parsed_zr = candor_classify::policy::parse_policy_silent(&text, &unknown_aliases);
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
        let outcome = policy_violations(&text, &crate_name, &all, &inferred, &calls, &hostsacc, &cmdsacc, &pathsacc, &tablesacc, &incompleteacc, &reason_class_acc, &unknown_aliases, &net_partners);
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
            let np = candor_classify::policy::parse_policy(&text);
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
        if had_parse_failure && v.is_empty() && guard_code != 1 && !crate::gate::holds_violation(run) {
            eprintln!("candor-scan: policy NOT enforced — source failed to parse (see above); gate cannot be green over unanalyzed code");
            return (2, json_body);
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
        if !oos_findings.is_empty() && v.is_empty() && guard_code != 1 && !crate::gate::holds_violation(run) {
            eprintln!(
                "candor-scan: policy NOT enforced — {} function(s) OUTSIDE this scan's scope perform an \
                 effect this policy denies (named above); the gate did not judge them, so the verdict is \
                 incomplete rather than a pass",
                oos_findings.len()
            );
            return (2, json_body);
        }
        // ⟨0.24⟩ A SOLE WITHHOLDING is a refusal (SPEC §3.1). Ordered AFTER the violation exit so the
        // precedence holds — a rule that fired on carried evidence dominates, and `v.is_empty()` is the
        // whole of the condition. Never `policy ✓`: the operator asked for a rule that did not run.
        if !outcome.withheld.is_empty() && v.is_empty() {
            let why = format!(
                "policy NOT enforced — {} (rule, function) pair(s) could not be evaluated (see above); a \
                 gate cannot be green over a rule that never ran",
                outcome.withheld.len()
            );
            eprintln!("candor-scan: {why}");
            crate::gate::record_gate_refusal(why);
            return (2, json_body);
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
                return (2, json_body);
            }
        }
        // Provable-purity disclosure (advisory — NEVER changes the verdict/exit): pure/deny layers that PASS
        // but are Unknown. Surfaces the gap automatically so an author learns their "pure" layer isn't
        // PROVABLY pure (eval/fixloop/DISPATCH-NOTE.md); the `candor-query unverified` query has the detail.
        // ⟨0.24⟩ The SAME two accumulators the gate matched on, and the SAME alias vocabulary it
        // resolved through — the disclosure names the holes the gate declined to clear, so anything it
        // reads differently is a second gate wearing the first one's name.
        let hole_nets = crate::gate::net_class_map(&all, &inferred, &hostsacc, &incompleteacc, &net_partners);
        let holes = crate::gate::unverified_holes(
            &text,
            &all,
            &inferred,
            &reason_class_acc,
            &hole_nets,
            &unknown_aliases,
        );
        if !holes.is_empty() {
            let mut upgrades: BTreeSet<String> = BTreeSet::new();
            eprintln!(
                "candor-scan: note — {} function(s) PASS the policy but are Unknown (purity NOT verified — the Unknown could hide a forbidden effect):",
                holes.len()
            );
            for (fq, up) in &holes {
                eprintln!("    `{fq}`  → add  `{up}`");
                upgrades.insert(up.clone());
            }
            eprintln!(
                "  (advisory; add the upgrade(s) to REQUIRE provable purity, or run `candor-query unverified` for detail — the gate verdict is unchanged)"
            );
        }
        if v.is_empty() {
            eprintln!("candor-scan: policy ✓ (advisory floor — the syntactic backend under-reports; the nightly engine is the sound gate)");
            // A CLEAN gate has no violation lines for the exploration opener to contradict, so emit it HERE
            // (after the ✓) — the pre-gate suppression at the top only exists to avoid a "nothing hidden"
            // line ABOVE a FAILING gate's violations (#18); a passing gated scan should not lose it (#8).
            if !quiet {
                crate::surface::emit(&inferred, &direct, &calls, &loc, coverage_ledger.len());
            }
        } else {
            eprintln!("candor-scan: {} policy violation(s) (advisory floor — a clean run is necessary, not sufficient)", v.len());
            // Append-only remedy pointer (gate-FAILURE path only): the summary line above is
            // conformance-pinned, so this extra line must never alter it, the violation lines,
            // or the exit code — and a zero-violation run stays byte-identical.
            eprintln!("→ candor-query fix-gate names the remedy for each (or `candor fix <fn> <Effect>` for one)");
            return (1, json_body);
        }
    }
    // No-gate runs record nothing; scan_main's final write_gate_json emits { ok: true, [] } for them.
    // A clean policy still exits 1 when the baseline guard fired above (the codes join by max).
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
    // Emit the interfaceUnion entries in the DEP scans. Those entries are what lets a consumer resolve a
    // call on a value typed by the dependency's trait — and the emission is gated on
    // CANDOR_WORKSPACE_CHAIN, which `--deps` never set, so the mechanism was shipped-but-off in exactly the
    // flow the help text tells people to use: a `&dyn DepTrait` dispatch read silent-pure
    // (SOUNDNESS-VEIN-crossing-the-scan-boundary.md, cause 3). candor-swift already sets it on its child
    // scans, which is the sole reason its equivalent case is recovered and rust's was not.
    // Scoped to this loop and restored afterwards: the child scans run sequentially on this thread, and the
    // ROOT scan's own emission must stay off so a default scan remains byte-identical.
    let prior_chain = std::env::var_os("CANDOR_WORKSPACE_CHAIN");
    if prior_chain.is_none() {
        std::env::set_var("CANDOR_WORKSPACE_CHAIN", "1");
    }
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
    if prior_chain.is_none() {
        std::env::remove_var("CANDOR_WORKSPACE_CHAIN");
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
    let spec = match std::env::var("CANDOR_DEPS") {
        Ok(extra) if !extra.is_empty() => format!("{deps_dir}:{extra}"),
        _ => deps_dir.clone(),
    };
    let idx = load_dep_reports(Some(&spec));
    // The final root scan goes through scan_target so `--deps <workspace>` fans out over members
    // too — the nested-package filter would otherwise prune them all into an empty, gate-passing report.
    scan_target(dir, prefix, want_json, include_tests, policy, baseline, &idx, run)
}
