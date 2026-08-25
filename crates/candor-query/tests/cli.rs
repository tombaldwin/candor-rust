//! End-to-end CLI tests that drive the COMPILED `candor-query` binary as a subprocess, so they can
//! assert on the real exit code + the stdout/stderr split — things an in-process call can't observe.
//! (Cargo sets `CARGO_BIN_EXE_candor-query` to the built binary for this integration test.)
//!
//! These tests build report/callgraph files BY HAND (the candor-scan binary is a sibling crate, not a
//! dependency of this test target) — the on-disk shape is the stable spec §2 report schema candor-scan
//! emits and `candor-query` consumes, so a write-by-hand fixture exercises the exact load path.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-query")
}

/// A throwaway prefix under the temp dir; `cleanup` removes its files. Returns the prefix string a
/// `candor-query` command takes (`<prefix>`), with a `<prefix>.<crate>.scan.json` + callgraph sidecar
/// already written when `with_report` is true.
struct Fixture {
    dir: PathBuf,
    prefix: String,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("candor-query-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prefix = dir.join("r").to_string_lossy().into_owned();
        Fixture { dir, prefix }
    }

    /// Write a minimal valid two-function report (`outer -> inner`, inner does Fs) + its callgraph
    /// sidecar — enough for `map`/`whatif`/`callers` to resolve a real graph.
    fn write_report(&self) {
        let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.7" },
  "package": "rpt",
  "functions": [
    { "fn": "inner", "loc": "src/lib.rs:2:1", "inferred": ["Fs"], "direct": ["Fs"], "hash": "rpt#inner", "paths": ["/x"] },
    { "fn": "outer", "loc": "src/lib.rs:1:1", "inferred": ["Fs"], "hash": "rpt#outer", "paths": ["/x"], "calls": ["inner"] }
  ]
}"#;
        std::fs::write(format!("{}.rpt.scan.json", self.prefix), report).unwrap();
        std::fs::write(format!("{}.rpt.scan.callgraph.json", self.prefix), r#"{"inner":[],"outer":["inner"]}"#).unwrap();
    }

    fn report_path(&self) -> String {
        format!("{}.rpt.scan.json", self.prefix)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ── version / help ────────────────────────────────────────────────────────────────────────────────

#[test]
fn version_exits_0() {
    for flag in ["--version", "-V"] {
        let out = Command::new(bin()).arg(flag).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0), "{flag} must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        assert!(stdout.lines().next().unwrap_or("").starts_with("candor-query "),
                "{flag} must print the build banner, got:\n{stdout}");
    }
}

#[test]
fn help_exits_0() {
    for flag in ["--help", "-h"] {
        let out = Command::new(bin()).arg(flag).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0), "{flag} must exit 0");
        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        assert!(stdout.contains("USAGE"), "{flag} must print a USAGE line, got:\n{stdout}");
    }
}

// ── unknown command ───────────────────────────────────────────────────────────────────────────────

#[test]
fn unknown_command_exits_2() {
    let out = Command::new(bin()).arg("boguscmd").output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "an unknown command must exit 2");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("unknown command"), "must report `unknown command`, got:\n{stderr}");
}

#[test]
fn no_command_exits_2() {
    // No command at all (empty `cmd`) falls through to the unknown-command arm → exit 2, never 0.
    let out = Command::new(bin()).output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "no command must exit 2");
}

// ── corrupt / truncated report: clean error, NOT an uncaught panic ────────────────────────────────

#[test]
fn corrupt_report_does_not_panic() {
    // A present-but-unparseable report is DISCLOSED loud on stderr and OMITTED — the merged query is
    // tolerant (one corrupt file doesn't kill it), but the failure must never be an uncaught panic
    // (exit 101) and the blind spot must be visible.
    let f = Fixture::new("corrupt");
    std::fs::write(f.report_path(), "{ this is : not valid json @@@").unwrap();
    let out = Command::new(bin()).arg("map").arg(&f.prefix).arg("0").output().expect("run candor-query");
    assert_ne!(out.status.code(), Some(101), "a corrupt report must not panic the query");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("failed to parse"), "a corrupt report must be disclosed on stderr, got:\n{stderr}");
}

#[test]
fn truncated_report_does_not_panic() {
    // A mid-write / truncated report (valid JSON prefix, abruptly cut) is the same class as corruption:
    // disclosed + omitted, never a panic.
    let f = Fixture::new("truncated");
    std::fs::write(f.report_path(), r#"{"candor":{"version":"scan-test","spec":"0.7","toolch"#).unwrap();
    let out = Command::new(bin()).arg("map").arg(&f.prefix).arg("0").output().expect("run candor-query");
    assert_ne!(out.status.code(), Some(101), "a truncated report must not panic the query");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("failed to parse"), "a truncated report must be disclosed on stderr, got:\n{stderr}");
}

// ── a sidecar is NOT a report candidate: no FALSE disclosure, and the real one still fires ────────

/// Write the 2-segment sidecar shape SPEC §2.2 licenses — `<prefix>.<pkg>.hierarchy.json` and
/// `<prefix>.<pkg>.callgraph.json`. Each engine pairs a sidecar to its OWN report stem, so the segment
/// count is not fixed by the spec, and these land exactly on the `<crate>.<type>` report shape.
fn write_two_segment_sidecars(prefix: &str) {
    std::fs::write(format!("{prefix}.app.hierarchy.json"), r#"{"app.Sub":["app.Base"]}"#).unwrap();
    std::fs::write(format!("{prefix}.app.callgraph.json"), r#"{"outer":["inner"],"inner":[]}"#).unwrap();
}

/// A FALSE DISCLOSURE, and the reason it is worth a test: the report-locator glob picked a SIDECAR up
/// as a report candidate, failed to find `functions` in it, and reported that as data loss —
/// "report … failed to parse — its functions are OMITTED from this query (corrupt or mid-write);
/// re-run the scan" — over a scan that was completely fine. Nothing was omitted. A disclosure channel
/// is only worth anything if a message in it means something, and this one spent that on noise while
/// telling the user to re-run a good scan (the `net-partner` "ignoring unknown config key" class,
/// which was printed WHILE the key was being honoured).
///
/// Fixed at the GLOB (`candor_report::SIDECAR_KINDS`), not at the parse: the sidecar never enters the
/// candidate set, so there is nothing to diagnose. Suppressing the message instead would have left the
/// file in the set and both REAL consequences below live.
#[test]
fn a_wellformed_sidecar_is_never_diagnosed_as_a_corrupt_report() {
    let f = Fixture::new("sidecar-quiet");
    f.write_report();
    write_two_segment_sidecars(&f.prefix);

    // Every verb that resolves a report locator — `callers --include-unknown` is the reported repro.
    for args in [
        vec!["callers", &f.prefix, "inner", "1", "--include-unknown"],
        vec!["map", &f.prefix, "0"],
        vec!["where", "Fs", "--report", &f.prefix],
        vec!["show", "outer", "--report", &f.prefix],
        vec!["path", "outer", "Fs", "--report", &f.prefix],
        vec!["tour", "--report", &f.prefix],
        vec!["reachable", "--report", &f.prefix],
        vec!["containment", "--report", &f.prefix],
        vec!["blindspots", "--report", &f.prefix],
        vec!["impact", "inner", "--report", &f.prefix],
    ] {
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
        assert!(
            !stderr.contains("failed to parse"),
            "`{}` diagnosed a well-formed SIDECAR as a corrupt report — a false disclosure over a good \
             scan:\n{stderr}",
            args.join(" ")
        );
    }

    // (a) `reports` is the canonical "what counts as a report" oracle (the wrapper's `--exists` check
    //     joins on it) — it must name the report and neither sidecar.
    let out = Command::new(bin()).arg("reports").arg(&f.prefix).output().expect("run candor-query");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(stdout.contains("r.rpt.scan.json"), "the real report must be listed, got:\n{stdout}");
    assert!(!stdout.contains("hierarchy") && !stdout.contains(".app.callgraph"),
            "a sidecar was listed as a report, got:\n{stdout}");

    // (b) THE CONSEQUENCE THAT WAS NOT JUST NOISE. The sidecar's parse failure set the `hard_fail` bit
    //     `load_entries_loud` uses to tell "an effect-free crate" from "every report was corrupt". So a
    //     legitimately EFFECT-FREE crate (a well-formed `functions: []` report) standing beside a
    //     sidecar was REFUSED at exit 2 — the query answered nothing at all.
    let g = Fixture::new("sidecar-empty");
    std::fs::write(
        format!("{}.rpt.scan.json", g.prefix),
        r#"{"candor":{"version":"scan-test","toolchain":"stable","spec":"0.7"},"package":"rpt","functions":[]}"#,
    ).unwrap();
    write_two_segment_sidecars(&g.prefix);
    let out = Command::new(bin()).arg("where").arg("Fs").arg("--report").arg(&g.prefix)
        .output().expect("run candor-query");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert_eq!(out.status.code(), Some(0),
               "an effect-free crate beside a sidecar must still answer, not be refused as corrupt:\n{stderr}");
    assert!(!stderr.contains("refusing to report an empty"),
            "the corruption guard fired on a sidecar, not on a corrupt report:\n{stderr}");
}

/// THE CONTROL, and without it the test above cannot tell "fixed the false disclosure" from "disabled
/// the disclosure". With the SAME sidecars present, a genuinely corrupt REPORT must still be disclosed
/// — and the message must name the REPORT, not a sidecar.
#[test]
fn a_corrupt_report_beside_a_sidecar_is_still_disclosed() {
    let f = Fixture::new("sidecar-control");
    std::fs::write(f.report_path(), "{ this is : not valid json @@@").unwrap();
    write_two_segment_sidecars(&f.prefix);

    let out = Command::new(bin()).arg("map").arg(&f.prefix).arg("0").output().expect("run candor-query");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("failed to parse"),
            "a REAL corrupt report went undisclosed — the sidecar exclusion disabled the channel \
             instead of un-poisoning it:\n{stderr}");
    assert!(stderr.contains("r.rpt.scan.json"),
            "the disclosure must name the corrupt REPORT, got:\n{stderr}");
    assert!(!stderr.contains("hierarchy.json"),
            "the disclosure named a SIDECAR, got:\n{stderr}");
    // …and the corruption guard still refuses to answer an empty (all-clear) query over it.
    assert_eq!(out.status.code(), Some(2),
               "a wholly corrupt report must still fail loud, got:\n{stderr}");
}

#[test]
fn nonexistent_prefix_fails_loud_exit_2() {
    // A prefix matching NO report files must fail LOUD (exit 2), never read as an authoritative empty
    // answer — the gateless-green-on-typo'd-path failure.
    let f = Fixture::new("nopfx"); // dir exists, but no report files written
    let out = Command::new(bin()).arg("map").arg(&f.prefix).arg("0").output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "a prefix with no reports must exit 2");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("no report files"), "must report `no report files`, got:\n{stderr}");
}

// ── parsepolicy ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn parsepolicy_unreadable_exits_2() {
    // `parsepolicy <unreadable>` must exit 2 cleanly (read error), not 0 or a panic.
    let f = Fixture::new("ppol");
    let missing = f.dir.join("does-not-exist.policy");
    let out = Command::new(bin())
        .arg("parsepolicy")
        .arg(missing.to_string_lossy().as_ref())
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "parsepolicy over an unreadable file must exit 2");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("cannot read policy"), "must report the read failure, got:\n{stderr}");
}

#[test]
fn parsepolicy_missing_arg_exits_2() {
    let out = Command::new(bin()).arg("parsepolicy").output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "parsepolicy with no path must exit 2 (usage)");
}

#[test]
fn parsepolicy_valid_dumps_json_exit_0() {
    // A readable policy → canonical JSON ({deny,allow,forbid}) on stdout, exit 0.
    let f = Fixture::new("ppolok");
    let pp = f.dir.join("p.policy");
    std::fs::write(&pp, "deny Net Db domain\nallow Net in billing api.stripe.com\n").unwrap();
    let out = Command::new(bin())
        .arg("parsepolicy")
        .arg(pp.to_string_lossy().as_ref())
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "a valid parsepolicy must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("parsepolicy must emit JSON");
    assert!(v.get("deny").is_some() && v.get("allow").is_some() && v.get("forbid").is_some(),
            "parsepolicy JSON must carry deny/allow/forbid keys, got:\n{stdout}");
}

// ── whatif: a typo'd / nonexistent policy path must NOT read as gate-green ─────────────────────────

#[test]
fn whatif_nonexistent_policy_exits_2() {
    // A SPECIFIED-but-unreadable policy must FAIL LOUD (exit 2), never silently yield a clean verdict —
    // a typo'd CANDOR_POLICY path otherwise reads as "no violations" and an agent ships a forbidden edit.
    let f = Fixture::new("wfpol");
    f.write_report();
    let bogus = f.dir.join("typo.policy");
    let out = Command::new(bin())
        .arg("whatif")
        .arg(&f.prefix)
        .arg("inner")
        .arg("Net")
        .arg(bogus.to_string_lossy().as_ref())
        .arg("0")
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "whatif with an unreadable policy must exit 2, not gate-green");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("could not be read"), "must report the policy read failure, got:\n{stderr}");
}

// ── containment: the AS-EFF-010 ratchet exit contract (violation → 1, clean → 0) ──────────────────

/// Write a `<prefix>.<crate>.scan.json` report whose `web` layer does (or doesn't) perform Db
/// directly — the containment ratchet's leak shape. Layers derive from the module segment after the
/// common `app::` root.
fn write_containment_report(dir: &std::path::Path, name: &str, web_has_db: bool) -> String {
    let prefix = dir.join(name).to_string_lossy().into_owned();
    let web_direct = if web_has_db { r#","direct":["Db"]"# } else { "" };
    let report = format!(
        r#"{{"candor":{{"version":"scan-test","toolchain":"stable","spec": "0.23"}},"package":"cnt","functions":[
            {{"fn":"app::data::save","inferred":["Db"],"direct":["Db"]}},
            {{"fn":"app::web::page","inferred":["Db"]{web_direct}}}
        ]}}"#
    );
    std::fs::write(format!("{prefix}.cnt.scan.json"), report).unwrap();
    prefix
}

#[test]
fn containment_ratchet_violation_exits_1_and_names_the_leak() {
    // AS-EFF-010: a boundary effect (Db) performed DIRECTLY in a layer (`web`) it didn't occupy in
    // the baseline is a leak — exit 1, with the `[AS-EFF-010]` line naming `Db → web`.
    let f = Fixture::new("cnt-leak");
    let base = write_containment_report(&f.dir, "base", false);
    let cur = write_containment_report(&f.dir, "cur", true);
    let out = Command::new(bin()).arg("containment").arg(&cur).arg(&base).output().expect("run");
    assert_eq!(out.status.code(), Some(1), "a containment leak must exit 1 (the ratchet bites)");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("[AS-EFF-010]"), "the ratchet line carries its AS-EFF code: {stdout}");
    assert!(stdout.contains("Db → web"), "the leak names effect and layer: {stdout}");
}

#[test]
fn containment_ratchet_clean_exits_0() {
    let f = Fixture::new("cnt-clean");
    let base = write_containment_report(&f.dir, "base", true);
    let cur = write_containment_report(&f.dir, "cur", true);
    let out = Command::new(bin()).arg("containment").arg(&cur).arg(&base).output().expect("run");
    assert_eq!(out.status.code(), Some(0), "an unchanged containment picture must exit 0");
    // …and cleaning a layer UP is exit 0 too (informational, never a failure).
    let cur2 = write_containment_report(&f.dir, "cur2", false);
    let out = Command::new(bin()).arg("containment").arg(&cur2).arg(&base).output().expect("run");
    assert_eq!(out.status.code(), Some(0), "a cleanup (layer left) must exit 0");
    assert!(String::from_utf8(out.stdout).unwrap().contains("improved"),
            "the cleanup is noted informationally");
}

// ── gate-verdict: assemble the §3.3 verdict from NDJSON records ────────────────────────────────────

#[test]
fn gate_verdict_absent_parts_is_the_clean_verdict() {
    // No parts file = no violations recorded = the spec's clean verdict { ok: true, [] } (exit 0).
    let f = Fixture::new("gv-clean");
    let out = Command::new(bin())
        .arg("gate-verdict").arg(f.dir.join("nosuch.parts").to_string_lossy().as_ref()).arg("-")
        .output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["spec"], candor_report::SPEC_VERSION);   // DERIVED: a floor bump must not need an edit here
    assert_eq!(v["violations"], serde_json::json!([]));
}

#[test]
fn gate_verdict_assembles_and_sorts_records_into_a_failing_verdict() {
    let f = Fixture::new("gv-viol");
    let parts = f.dir.join("gate.json.parts");
    std::fs::write(&parts, concat!(
        r#"{"rule":"AS-EFF-009","fn":"b","effects":[],"detail":"z"}"#, "\n",
        r#"{"rule":"AS-EFF-006","fn":"a","effects":["Net"],"detail":"y"}"#, "\n",
    )).unwrap();
    let outfile = f.dir.join("gate.json");
    let out = Command::new(bin())
        .arg("gate-verdict").arg(parts.to_string_lossy().as_ref()).arg(outfile.to_string_lossy().as_ref())
        .output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&outfile).unwrap()).unwrap();
    assert_eq!(v["ok"], false);
    let arr = v["violations"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["rule"], "AS-EFF-006", "records sort by (rule, detail)");
    assert_eq!(arr[0]["fn"], "a");
    assert_eq!(arr[0]["effects"], serde_json::json!(["Net"]));
}

#[test]
fn gate_verdict_corrupt_record_fails_closed() {
    // A dropped record would make the verdict under-report vs the gate's exit code — the §3.3
    // forbidden disagreement. Exit 2, never a partial verdict.
    let f = Fixture::new("gv-corrupt");
    let parts = f.dir.join("gate.json.parts");
    std::fs::write(&parts, "{not json@@\n").unwrap();
    let outfile = f.dir.join("gate.json");
    let out = Command::new(bin())
        .arg("gate-verdict").arg(parts.to_string_lossy().as_ref()).arg(outfile.to_string_lossy().as_ref())
        .output().expect("run");
    assert_eq!(out.status.code(), Some(2), "a corrupt record must fail closed");
    assert!(!outfile.exists(), "no partial verdict may be written");
}

#[test]
fn gate_verdict_flag_shaped_report_value_is_a_usage_error_not_a_green_verdict() {
    // SPEC §3.2 ⟨0.28⟩ "given no value": `--report` used to consume the next token WHATEVER its shape.
    // Measured on this verb: `gate-verdict <parts> - --report --json` exited 0 and printed the CLEAN
    // verdict `{"ok": true}` — `resolve_locator("--json")` failed SILENTLY inside the coverage loader,
    // so a mistyped `--report` dropped the coverage advisory and shipped a green verdict. A flag-shaped
    // next token is a usage error at exit 2, never a locator.
    let f = Fixture::new("gv-b13");
    let out = Command::new(bin())
        .arg("gate-verdict").arg(f.dir.join("nosuch.parts").to_string_lossy().as_ref()).arg("-")
        .arg("--report").arg("--json")
        .output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "--report was given no value — a usage error:\n{stderr}");
    assert!(!stdout.contains("\"ok\""), "no verdict of any colour may be emitted, got:\n{stdout}");
    assert!(stderr.contains("--report") && stderr.contains("--json"),
            "stderr names the flag given no value AND the token that is not one: {stderr}");
    // The boundary: a REAL locator after --report still rides through (the ⟨0.15⟩ advisory path) — the
    // clean verdict of gate_verdict_absent_parts_is_the_clean_verdict is unaffected by the refusal.
    f.write_report();
    let out = Command::new(bin())
        .arg("gate-verdict").arg(f.dir.join("nosuch.parts").to_string_lossy().as_ref()).arg("-")
        .arg("--report").arg(f.report_path())
        .output().expect("run");
    assert_eq!(out.status.code(), Some(0), "a value-shaped --report locator is unaffected");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    assert_eq!(v["ok"], true);
}

// ── reports --exists/--backend, engine-version, diff ──────────────────────────────────────────────

#[test]
fn reports_exists_and_backend_probe() {
    let f = Fixture::new("rp-probe");
    // absent → --exists is falsy (nonzero exit)
    let out = Command::new(bin()).arg("reports").arg(&f.prefix).arg("--exists").output().expect("run");
    assert_ne!(out.status.code(), Some(0), "no reports → --exists must be falsy");
    f.write_report();
    let out = Command::new(bin()).arg("reports").arg(&f.prefix).arg("--exists").output().expect("run");
    assert_eq!(out.status.code(), Some(0), "a present report → --exists exit 0");
    let out = Command::new(bin()).arg("reports").arg(&f.prefix).arg("--backend").output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "scan",
               "a `.scan.json` report probes as the scan backend");
}

#[test]
fn reports_clear_other_given_no_value_is_a_usage_error_not_a_silent_no_op() {
    // SPEC §3.2 ⟨0.28⟩ "given no value": `--clear-other` used to read the next token whatever its
    // shape, or fall through when nothing followed — and BOTH wrong readings were a silent exit-0.
    // Measured: `reports <p> --clear-other --exists` "cleared" with keep=`--exists` (removing nothing,
    // probing nothing), and a trailing `--clear-other` listed the reports as if the flag had not been
    // typed. Exit 2, both spellings — and the report files must survive the refusal.
    let f = Fixture::new("rp-b13");
    f.write_report();
    for extra in [&["--clear-other", "--exists"][..], &["--clear-other"][..]] {
        let out = Command::new(bin()).arg("reports").arg(&f.prefix).args(extra).output().expect("run");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(2), "{extra:?} is a usage error, never a silent no-op:\n{stderr}");
        assert!(stderr.contains("--clear-other"), "the refusal names the broken flag: {stderr}");
        assert!(std::path::Path::new(&f.report_path()).exists(),
                "a refused clear must not have removed anything");
    }
    // The boundary: a value-shaped keep still clears. keep=lint drops the scan report (+ its sidecar).
    let out = Command::new(bin()).arg("reports").arg(&f.prefix).args(["--clear-other", "lint"]).output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    assert!(!std::path::Path::new(&f.report_path()).exists(), "keep=lint clears the scan report");
}

#[test]
fn engine_version_reads_the_embedded_tag() {
    let f = Fixture::new("ev-tag");
    let lib = f.dir.join("libfake.so");
    std::fs::write(&lib, b"\x7fELFjunk candor-build-version=abc1234 morejunk").unwrap();
    let out = Command::new(bin()).arg("engine-version").arg(lib.to_string_lossy().as_ref()).output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "abc1234");
    // a binary with NO tag → nonzero (the wrapper then falls back to git HEAD)
    let bare = f.dir.join("bare.so");
    std::fs::write(&bare, b"nothing here").unwrap();
    let out = Command::new(bin()).arg("engine-version").arg(bare.to_string_lossy().as_ref()).output().expect("run");
    assert_ne!(out.status.code(), Some(0), "no embedded tag → non-zero");
}

#[test]
fn diff_reports_a_gained_effect() {
    let f = Fixture::new("df-gain");
    let base = f.dir.join("b").to_string_lossy().into_owned();
    let cur = f.dir.join("c").to_string_lossy().into_owned();
    let mk = |prefix: &str, effs: &str| {
        std::fs::write(format!("{prefix}.d.scan.json"), format!(
            r#"{{"candor":{{"version":"scan-test","toolchain":"stable","spec": "0.23"}},"functions":[
                {{"fn":"worker","inferred":[{effs}],"direct":[{effs}]}}]}}"#)).unwrap();
    };
    mk(&base, r#""Fs""#);
    mk(&cur, r#""Fs","Net""#);
    let out = Command::new(bin())
        .arg("diff").arg(&cur).arg(&base).arg("1").arg("v1").arg("v1")
        .output().expect("run");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    let gained = v.pointer("/changed/0/gained").or_else(|| v.pointer("/gained"));
    assert!(serde_json::to_string(&v).unwrap().contains("Net"),
            "the +Net gain must appear in the diff JSON: {v} (gained={gained:?})");
}

#[test]
fn diff_and_rewire_reject_a_flag_they_do_not_take_loud() {
    // SPEC §3.3.1: "a typo'd or a not-applicable flag stays an exit-2 error, never a silent swallow" —
    // the rule gains' bespoke parser carries and these two bespoke parsers did not (the P8 sink-surface
    // matrix, 2026-08-12). Measured pre-fix: `diff A B --report --json` ran to a clean exit-0 answer
    // with `baseline_version: "--report"` (the flag fell through to the version-stamp slot and was
    // PUBLISHED), and `rewire A B --report --json` dropped BOTH tokens at exit 0. An exit-code check
    // alone would pass on a wrong-cause refusal, so each refusal must also NAME the flag.
    let f = Fixture::new("df-rw-flags");
    f.write_report();
    let rep = f.report_path();
    // Every flag the family's --help derives as value-taking, plus a plain typo — not just the pair the
    // failing matrix row happened to use (the sibling-route habit).
    for flag in ["--report", "--policy", "--class", "--clear-other", "--gate-json", "--polciy"] {
        let d = Command::new(bin()).args(["diff", &rep, &rep, flag, "--json"]).output().expect("run");
        let de = String::from_utf8_lossy(&d.stderr).into_owned();
        assert_eq!(d.status.code(), Some(2), "diff {flag} --json must be a usage error, not a clean answer:\n{de}");
        assert!(de.contains(flag), "diff's refusal names the broken flag `{flag}`: {de}");
        let r = Command::new(bin()).args(["rewire", &f.prefix, &f.prefix, flag, "--json"]).output().expect("run");
        let re = String::from_utf8_lossy(&r.stderr).into_owned();
        assert_eq!(r.status.code(), Some(2), "rewire {flag} --json must be a usage error, not a clean answer:\n{re}");
        assert!(re.contains(flag), "rewire's refusal names the broken flag `{flag}`: {re}");
    }
    // The boundary, so the reject cannot over-reach: `--json` is honoured, `--text` (candor-ts's
    // output-mode flag, #2) is tolerated, and rewire's `--json` now actually selects JSON (pre-fix it
    // was silently prose — only a literal `1` in the third slot meant JSON).
    let ok = Command::new(bin()).args(["diff", &rep, &rep, "--json", "--text"]).output().expect("run");
    assert_eq!(ok.status.code(), Some(0), "--json/--text stay accepted: {}", String::from_utf8_lossy(&ok.stderr));
    let rj = Command::new(bin()).args(["rewire", &f.prefix, &f.prefix, "--json"]).output().expect("run");
    assert_eq!(rj.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&rj.stdout).contains("\"dropped\""),
            "rewire --json emits the JSON shape, not prose");
}

#[test]
fn whatif_unknown_effect_exits_2() {
    // A typo'd/lowercase effect (`net`) matches no deny rule and would print a false-green verdict —
    // it must be rejected as a usage error (exit 2).
    let f = Fixture::new("wfeff");
    f.write_report();
    let out = Command::new(bin())
        .arg("whatif")
        .arg(&f.prefix)
        .arg("inner")
        .arg("net")
        .arg("0")
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "whatif with an unknown effect must exit 2");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    assert!(stderr.contains("unknown effect"), "must report `unknown effect`, got:\n{stderr}");
}

#[test]
fn where_unknown_effect_exits_2() {
    // corpus-audit #3: a typo'd / unknown effect NAME must be a LOUD error (exit 2), never a false-empty
    // {directly:[],inherited:[]} at exit 0 (reads as "nothing performs Net" when the user typed "Network").
    let f = Fixture::new("wheff");
    f.write_report();
    let out = Command::new(bin()).arg("where").arg("Networkxyz").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "where with an unknown effect must exit 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown effect"),
        "must report `unknown effect`, got:\n{}", String::from_utf8_lossy(&out.stderr));
    // a VALID effect that is simply absent stays a legitimate 0-result at exit 0
    let ok = Command::new(bin()).arg("where").arg("Db").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_eq!(ok.status.code(), Some(0), "a known-but-absent effect is a valid 0-result, not an error");
}

#[test]
fn callers_nonexistent_fn_exits_2() {
    // corpus-audit #3: a nonexistent function must exit 2, like path/impact — never empty at exit 0.
    let f = Fixture::new("cleff");
    f.write_report();
    let out = Command::new(bin()).arg("callers").arg("zzz_no_such_fn").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "callers of a nonexistent fn must exit 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no function matching"),
        "must report `no function matching`, got:\n{}", String::from_utf8_lossy(&out.stderr));
    // a real function resolves normally at exit 0
    let ok = Command::new(bin()).arg("callers").arg("inner").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_eq!(ok.status.code(), Some(0), "callers of a real fn resolves at exit 0");
}

#[test]
fn unknown_flag_exits_2_but_cross_engine_flag_tolerated() {
    // corpus re-audit #2: a typo'd flag (`--polciy`) must be a LOUD error (exit 2) with a did-you-mean —
    // never swallowed as a positional (which runs the query with NO policy and exits green: a CI author who
    // typos --policy ships a gate that never fires). But a VALID cross-engine flag (`--text`, candor-ts's
    // output-mode flag) must be TOLERATED (rust prose is the default) so `candor <verb> --text` never errors.
    let f = Fixture::new("flag");
    f.write_report();
    let bad = Command::new(bin()).arg("where").arg("Fs").arg("--polciy").arg("/x").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_eq!(bad.status.code(), Some(2), "a typo'd flag must exit 2, not run green with no policy");
    let se = String::from_utf8_lossy(&bad.stderr);
    assert!(se.contains("unknown flag") && se.contains("--policy"),
        "must name the unknown flag + suggest --policy, got:\n{se}");
    let txt = Command::new(bin()).arg("where").arg("Fs").arg("--text").arg("--report").arg(f.report_path())
        .output().expect("run candor-query");
    assert_ne!(txt.status.code(), Some(2), "--text (a valid cross-engine flag) must be tolerated, not rejected");
}

// ── fix: the boundary hoist (integrations/FIX-SPEC.md) — the remedial inverse of whatif ────────────

/// The `orderflow` shape: `api::get_quote → domain::quote_bulk → domain::price_quote → infra::fetch_rate`,
/// every function carrying Net, the leaf performing it directly. A `deny Net domain` policy makes the two
/// domain functions a violation — the api caller is the allowed-layer hoist target. Returns the prefix.
fn write_orderflow_fixture(f: &Fixture) {
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "api::get_quote",     "loc": "src/api.rs:3:1",    "inferred": ["Net"], "hash": "of#gq", "paths": ["/x"], "calls": ["domain::quote_bulk"] },
    { "fn": "domain::quote_bulk", "loc": "src/domain.rs:5:1", "inferred": ["Net"], "hash": "of#qb", "paths": ["/x"], "calls": ["domain::price_quote"] },
    { "fn": "domain::price_quote","loc": "src/domain.rs:9:1", "inferred": ["Net"], "hash": "of#pq", "paths": ["/x"], "calls": ["infra::fetch_rate"] },
    { "fn": "infra::fetch_rate",  "loc": "src/infra.rs:2:1",  "inferred": ["Net"], "direct": ["Net"], "hash": "of#fr", "paths": ["/x"], "calls": [] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
}

fn write_policy(f: &Fixture, name: &str, body: &str) -> String {
    let p = f.dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.to_string_lossy().into_owned()
}

#[test]
fn fix_orderflow_hoists_net_to_api() {
    // GROUND TRUTH (FIX-SPEC worked example): Net violates `deny Net domain` at price_quote; the direct
    // site is the infra leaf; the two domain functions are the pure span; the hoist target is the nearest
    // allowed-layer caller, api::get_quote. The plan must name exactly those, and offer the `allow` edit.
    let f = Fixture::new("fixof");
    write_orderflow_fixture(&f);
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net", &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "a computable fix must exit 0");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).expect("fix --json must emit JSON");
    assert_eq!(v["cleanHoist"], serde_json::json!(true), "a clean hoist exists");
    assert_eq!(v["layer"], serde_json::json!("domain"));
    assert_eq!(v["site"], serde_json::json!(["infra::fetch_rate"]), "the direct site is the infra leaf");
    assert_eq!(v["hoistTo"], serde_json::json!(["api::get_quote"]), "hoist to the nearest allowed caller");
    assert_eq!(
        v["deniedSpan"],
        serde_json::json!(["domain::price_quote", "domain::quote_bulk"]),
        "the pure span is exactly the two domain functions"
    );
    assert_eq!(v["policyAlternative"], serde_json::json!("allow Net domain"));
    // api::get_quote is the top of this graph — no allowed-layer caller above it, so no higher option.
    assert_eq!(v["hoistHigher"], serde_json::json!([]), "the frontier is the top; no higher hoist");
}

#[test]
fn fix_surfaces_higher_hoist_tradeoff() {
    // With an allowed-layer entry point ABOVE the minimal frontier, candor surfaces the trade-off: the
    // minimal hoist is still `api::get_quote`, but `main::run` (which calls it, also allowed) is a higher
    // option — hoisting there keeps api::get_quote pure too, threading the value through one more signature.
    let f = Fixture::new("fixhigher");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "main::run",          "loc": "src/main.rs:1:1",  "inferred": ["Net"], "hash": "of#mr", "paths": ["/x"], "calls": ["api::get_quote"] },
    { "fn": "api::get_quote",     "loc": "src/api.rs:3:1",    "inferred": ["Net"], "hash": "of#gq", "paths": ["/x"], "calls": ["domain::quote_bulk"] },
    { "fn": "domain::quote_bulk", "loc": "src/domain.rs:5:1", "inferred": ["Net"], "hash": "of#qb", "paths": ["/x"], "calls": ["domain::price_quote"] },
    { "fn": "domain::price_quote","loc": "src/domain.rs:9:1", "inferred": ["Net"], "hash": "of#pq", "paths": ["/x"], "calls": ["infra::fetch_rate"] },
    { "fn": "infra::fetch_rate",  "loc": "src/infra.rs:2:1",  "inferred": ["Net"], "direct": ["Net"], "hash": "of#fr", "paths": ["/x"], "calls": [] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net", &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["hoistTo"], serde_json::json!(["api::get_quote"]), "the MINIMAL frontier is unchanged");
    assert_eq!(v["hoistHigher"], serde_json::json!(["main::run"]), "main::run is the higher hoist option");
    // the text render carries the trade-off note.
    let text = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net", &pol, "0"])
        .output()
        .expect("run candor-query");
    let s = String::from_utf8(text.stdout).unwrap();
    assert!(s.contains("TRADE-OFF") && s.contains("main::run"), "text must surface the higher-hoist trade-off, got:\n{s}");
}

#[test]
fn fix_prefers_the_effect_performing_match() {
    // A bare leaf `save` matches BOTH a pure `cache::save` (sorts first) and the effectful, denied
    // `repo::save`. Resolution must prefer the match that performs the effect — otherwise `fix save Net`
    // resolves to `cache::save`, prints "nothing to hoist", and gives a false all-clear while ts/swift
    // (which prefer the effectful match) emit the real fix. (/code-review — start-resolution parity.)
    let f = Fixture::new("fixresolve");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "cache::save", "loc": "src/c.rs:1:1", "inferred": [], "hash": "of#cs", "paths": ["/x"], "calls": [] },
    { "fn": "repo::save",  "loc": "src/r.rs:1:1", "inferred": ["Net"], "direct": ["Net"], "hash": "of#rs", "paths": ["/x"], "calls": [] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net repo\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "save", "Net", &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["fn"], serde_json::json!("repo::save"), "must resolve to the effectful, denied match");
    assert_eq!(v["site"], serde_json::json!(["repo::save"]), "repo::save is the direct site");
}

#[test]
fn fix_sandwiched_layer_is_not_a_clean_hoist() {
    // A forbidden layer SANDWICHES an allowed one: domain::top → api::mid → domain::inner → infra::fetch,
    // `deny Net domain`. The nearest allowed frontier is `api::mid`, but it's CALLED BY `domain::top`, so
    // hoisting Net to api::mid would leave domain::top violating — NOT a clean hoist. (/code-review.)
    let f = Fixture::new("fixsandwich");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "domain::top",   "loc": "src/d.rs:1:1", "inferred": ["Net"], "hash": "of#t", "paths": ["/x"], "calls": ["api::mid"] },
    { "fn": "api::mid",      "loc": "src/a.rs:1:1", "inferred": ["Net"], "hash": "of#m", "paths": ["/x"], "calls": ["domain::inner"] },
    { "fn": "domain::inner", "loc": "src/d.rs:9:1", "inferred": ["Net"], "hash": "of#i", "paths": ["/x"], "calls": ["infra::fetch"] },
    { "fn": "infra::fetch",  "loc": "src/i.rs:1:1", "inferred": ["Net"], "direct": ["Net"], "hash": "of#f", "paths": ["/x"], "calls": [] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "inner", "Net", &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["cleanHoist"], serde_json::json!(false), "a sandwiched frontier is NOT a clean hoist");
    // and the text names the sandwiched reason (not the generic "every caller is forbidding").
    let text = Command::new(bin())
        .args(["fix", &f.prefix, "inner", "Net", &pol, "0"])
        .output()
        .expect("run candor-query");
    let s = String::from_utf8(text.stdout).unwrap();
    assert!(s.contains("CALLED BY") && s.contains("sandwich"), "text must explain the sandwich, got:\n{s}");
}

#[test]
fn fix_no_clean_hoist_offers_port_and_policy() {
    // When every caller up to the entry is ALSO in the forbidden layer, candor does NOT invent a target:
    // it names the two honest options (port / policy relax), and cleanHoist is false.
    let f = Fixture::new("fixnc");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "nc",
  "functions": [
    { "fn": "domain::main_flow",   "loc": "src/d.rs:1:1", "inferred": ["Net"], "hash": "nc#mf", "paths": ["/x"], "calls": ["domain::price_quote"] },
    { "fn": "domain::price_quote", "loc": "src/d.rs:9:1", "inferred": ["Net"], "direct": ["Net"], "hash": "nc#pq", "paths": ["/x"], "calls": [] }
  ]
}"#;
    std::fs::write(format!("{}.nc.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net", &pol, "0"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("NO CLEAN HOIST"), "must say no clean hoist exists, got:\n{stdout}");
    // The eval-driven advice (eval/fixloop/DISPATCH-NOTE.md): lead with the composition-root hoist (PROVABLY
    // pure), recommend fn/closure over a trait port, and name the Unknown-hole trade-off.
    assert!(stdout.contains("NEW ENTRY POINT"), "must offer the composition-root hoist, got:\n{stdout}");
    assert!(stdout.contains("PROVABLY pure"), "must note the hoist is provably pure, got:\n{stdout}");
    assert!(stdout.contains("fn/closure") && stdout.contains("trait"), "must recommend fn-injection over a trait port, got:\n{stdout}");
    assert!(stdout.contains("Unknown"), "must name the fn-injection Unknown-hole trade-off, got:\n{stdout}");
    assert!(stdout.contains("allow Net domain"), "must offer the policy-relax edit, got:\n{stdout}");
}

#[test]
fn fix_non_violation_is_a_no_op() {
    // A function that performs the effect in an ALLOWED layer isn't a boundary crossing — no fix, exit 0,
    // and it must say so rather than manufacturing a hoist.
    let f = Fixture::new("fixok");
    write_orderflow_fixture(&f);
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "get_quote", "Net", &pol, "0"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("no policy forbids it"), "must report a non-violation, got:\n{stdout}");
}

#[test]
fn fix_unreadable_policy_exits_2() {
    // Same fail-loud contract as whatif: a specified-but-unreadable policy must never yield a confident
    // plan against a silently-empty ruleset.
    let f = Fixture::new("fixbadpol");
    write_orderflow_fixture(&f);
    let bogus = f.dir.join("typo.policy");
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net", bogus.to_string_lossy().as_ref(), "0"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "an unreadable policy must exit 2, not emit a plan");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("could not be read"), "must report the read failure, got:\n{stderr}");
}

#[test]
fn fix_no_policy_exits_2() {
    // A fix is defined relative to a boundary — with no policy there is no boundary, and the command must
    // fail loud (exit 2) rather than print an empty or misleading plan.
    let f = Fixture::new("fixnopol");
    write_orderflow_fixture(&f);
    let out = Command::new(bin())
        .args(["fix", &f.prefix, "price_quote", "Net"])
        .env_remove("CANDOR_POLICY")
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "no policy must exit 2");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("policy is required"), "must explain a policy is required, got:\n{stderr}");
}

#[test]
fn fix_gate_collapses_inheritors_to_one_remedy() {
    // fix-gate computes a remedy for EVERY deny/pure crossing, but the two domain functions that both carry
    // Net are ONE root cause — the dedup must collapse them to a single plan (same site, same hoist), not
    // emit a near-identical plan per inheritor. This is what the loop folds into the block message.
    let f = Fixture::new("fixgate");
    write_orderflow_fixture(&f);
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["fix-gate", &f.prefix, &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).expect("fix-gate --json must emit JSON");
    assert_eq!(v["ok"], serde_json::json!(false), "a crossing exists → not ok");
    let rem = v["remedies"].as_array().expect("remedies array");
    assert_eq!(rem.len(), 1, "the two domain inheritors collapse to one remedy, got {}", rem.len());
    assert_eq!(rem[0]["hoistTo"], serde_json::json!(["api::get_quote"]));
    assert_eq!(rem[0]["site"], serde_json::json!(["infra::fetch_rate"]));
}

#[test]
fn fix_gate_clean_report_is_ok() {
    // No deny/pure crossing → ok:true, empty remedies, exit 0. (The scope pattern matches no function.)
    let f = Fixture::new("fixgateok");
    write_orderflow_fixture(&f);
    let pol = write_policy(&f, "p.policy", "deny Net nonexistentlayer\n");
    let out = Command::new(bin())
        .args(["fix-gate", &f.prefix, &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true), "no crossing → ok");
    assert_eq!(v["remedies"].as_array().unwrap().len(), 0, "no remedies when clean");
}

#[test]
fn fix_gate_strict_exits_1_on_a_crossing_advisory_otherwise() {
    // #3 exit-code contract: fix-gate is ADVISORY (exit 0) by default so the agent fix-loop reads the remedy
    // and edits — but `--strict` makes a non-empty remedy set a CI failure (exit 1), matching `unverified
    // --strict`. Same crossing, two exit codes by flag; JSON `ok` is unchanged (still false).
    let f = Fixture::new("fixgatestrict");
    write_orderflow_fixture(&f);
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    // default: advisory, exit 0
    let adv = Command::new(bin()).args(["fix-gate", "--report", &f.prefix, "--policy", &pol])
        .output().expect("run candor-query");
    assert_eq!(adv.status.code(), Some(0), "fix-gate is advisory by default (exit 0)");
    // --strict: a crossing exists → exit 1
    let strict = Command::new(bin()).args(["fix-gate", "--report", &f.prefix, "--policy", &pol, "--strict"])
        .output().expect("run candor-query");
    assert_eq!(strict.status.code(), Some(1), "--strict with an outstanding crossing must exit 1");
    // --strict --json: exit follows ok, ok stays false
    let sj = Command::new(bin()).args(["fix-gate", "--report", &f.prefix, "--policy", &pol, "--strict", "--json"])
        .output().expect("run candor-query");
    assert_eq!(sj.status.code(), Some(1), "--strict --json with a crossing exits 1");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(sj.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false), "ok is still false regardless of --strict");
    // --strict over a CLEAN report → exit 0 (no crossing to fail on)
    let cleanpol = write_policy(&f, "clean.policy", "deny Net nonexistentlayer\n");
    let clean = Command::new(bin()).args(["fix-gate", "--report", &f.prefix, "--policy", &cleanpol, "--strict"])
        .output().expect("run candor-query");
    assert_eq!(clean.status.code(), Some(0), "--strict over a clean report exits 0");
}

#[test]
fn unverified_and_fix_gate_answer_a_judged_nothing_report_at_exit_0_with_the_caveat() {
    // ⟨0.28⟩ THE TWO CAUSES `entries.is_empty()` CONFLATED, ruled in OPPOSITE directions. A report that
    // JUDGED NOTHING (`functions: []`, `analyzed.count: 0` — the ⟨0.21⟩ Row-1 shape, the standard
    // post-failure artifact) must be ANSWERED at exit 0 with the pinned travelling caveat
    // (`incomplete: true` + `judgedNothing` as an ARRAY OF REPORT PATHS — SPEC §2 ⟨0.24⟩: "A DISCLOSURE,
    // NOT AN EXIT CODE"; `gate --report` exits 0 over these bytes, so a verb exiting 2 would claim it
    // got LESS far than the gate on identical input). A locator naming NO report at all stays a loud
    // exit-2 refusal (§3.2). Before this fix both verbs exited 2 on BOTH causes — the outlier posture
    // on the rung this engine's own commit `e1a341f` defined, and java/ts/swift all answer at exit 0.
    let f = Fixture::new("judgednothing");
    let judged_nothing = r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.28"},"package":"lib","functions":[],"analyzed":{"count":0,"digest":"0"}}"#;
    let report_file = format!("{}.lib.scan.json", f.prefix);
    std::fs::write(&report_file, judged_nothing).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net app\n");

    for verb in ["unverified", "fix-gate"] {
        // JSON channel, --strict (the CI form): exit 0 and the caveat rides the document.
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &pol, "--json", "--strict"])
            .output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0),
            "{verb} --strict must ANSWER a judged-nothing report at exit 0, not refuse at 2 — \
             count-0 is a disclosure, not an exit code (⟨0.24⟩)");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap())
            .unwrap_or_else(|_| panic!("{verb} must emit a JSON document over a judged-nothing report"));
        assert_eq!(v["incomplete"], serde_json::json!(true), "{verb}: the caveat flag");
        assert_eq!(v["judgedNothing"], serde_json::json!([report_file]),
            "{verb}: `judgedNothing` is an ARRAY OF REPORT PATHS (SPEC §2 ⟨0.28⟩), never a boolean");
        assert!(v.get("ok").is_none(),
            "{verb}: `ok` is withheld — a judged-nothing report licenses it no more than an unanalyzed one");

        // Prose channel, --strict: the SAME exit as JSON (a literal `true` in unverified's prose branch
        // made the two channels disagree — prose exited 2 where --json exited 0, measured), and the note
        // must not claim the gate exits 2 over bytes the gate answers at 0.
        let prose = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &pol, "--strict"])
            .output().expect("run candor-query");
        assert_eq!(prose.status.code(), Some(0),
            "{verb} prose --strict must exit 0 like its own --json channel over identical bytes");
        let text = String::from_utf8_lossy(&prose.stdout).into_owned();
        assert!(text.contains("JUDGED NOTHING"), "{verb}: the prose caveat must name the cause");
        assert!(!text.contains("`gate --report` exits 2 over these bytes"),
            "{verb}: the note claims the gate refuses, but `gate --report` exits 0 over a \
             judged-nothing report — the disclosure discrediting itself");

        // The OTHER cause: no report at all stays a LOUD exit-2 refusal (§3.2), never an answer.
        let none = Command::new(bin())
            .args([verb, "--report", &format!("{}-nowhere", f.prefix), "--policy", &pol, "--json"])
            .output().expect("run candor-query");
        assert_eq!(none.status.code(), Some(2),
            "{verb}: a locator naming NO report is a loud failure, not a judged-nothing answer");

        // INTACT-INPUT CONTROL: a healthy report keeps `ok` and gains no caveat key — the fix must not
        // spend the thing it protects. (Byte-level identity vs the pre-fix binary was verified out of
        // band; this pins the key set.)
        let healthy = Fixture::new(&format!("judgednothing-ctl-{verb}"));
        healthy.write_report();
        let hpol = write_policy(&healthy, "p.policy", "deny Net app\n");
        let h = Command::new(bin())
            .args([verb, "--report", &healthy.prefix, "--policy", &hpol, "--json", "--strict"])
            .output().expect("run candor-query");
        assert_eq!(h.status.code(), Some(0), "{verb}: healthy control exits 0");
        let hv: serde_json::Value = serde_json::from_str(&String::from_utf8(h.stdout).unwrap()).unwrap();
        assert_eq!(hv["ok"], serde_json::json!(true), "{verb}: healthy control keeps ok:true");
        assert!(hv.get("incomplete").is_none() && hv.get("judgedNothing").is_none(),
            "{verb}: a complete report must gain NO caveat key from this fix");
    }
}

/// ⟨0.28⟩ SPEC §2 — **THE THIRD ROW IS NOT THE FIRST ROW**, end to end through the shipped binary.
///
/// A report carrying NO `analyzed` key (§2's row 3, a pre-⟨0.21⟩ producer) hedges under the pinned
/// `noManifest` key, NOT under `judgedNothing` — which ⟨0.28⟩ defines as *reports declaring
/// `analyzed.count: 0`*, something a row-3 report does not do. MEASURED before the split: every verb
/// below listed it under `judgedNothing` and the prose called it *"JUDGED NOTHING (`analyzed.count:
/// 0`)"* — a false disclosure, which this family rates worse than a missing one.
///
/// The two CONTROLS are load-bearing and are asserted in the same test, because the failure mode of a
/// half-done fix is a RENAME (row 1 moves too) and the failure mode of an over-done one is a feature
/// that has been disabled (row 2 hedges as well, withdrawing legitimate all-pure claims).
#[test]
fn a_report_with_no_analyzed_manifest_hedges_as_row_three_across_every_verb() {
    let f = Fixture::new("nomanifest");
    let pol = write_policy(&f, "p.policy", "deny Net app\n");
    let mk = |suffix: &str, body: &str| {
        let pre = format!("{}.{suffix}", f.prefix);
        std::fs::write(format!("{pre}.lib.scan.json"), body).unwrap();
        (pre.clone(), format!("{pre}.lib.scan.json"))
    };
    let (row3, row3_file) =
        mk("row3", r#"{"candor":{"version":"t","spec":"0.20"},"package":"lib","functions":[]}"#);
    let (row1, row1_file) = mk("row1",
        r#"{"candor":{"version":"t","spec":"0.28"},"package":"lib","functions":[],"analyzed":{"count":0,"digest":"0"}}"#);
    let (row2, _) = mk("row2",
        r#"{"candor":{"version":"t","spec":"0.28"},"package":"lib","functions":[],"analyzed":{"count":7,"digest":"0"}}"#);

    let json = |args: &[&str]| -> (i32, serde_json::Value) {
        let out = Command::new(bin()).args(args).output().expect("run candor-query");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (out.status.code().unwrap(),
         serde_json::from_str(&text).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}):\n{text}")))
    };

    for verb in [
        vec!["where", "Fs"],
        vec!["blindspots"],
        vec!["reachable"],
        vec!["map"],
        vec!["unverified"],
        vec!["fix-gate"],
    ] {
        let mut a = verb.clone();
        a.extend(["--report", &row3, "--policy", &pol, "--json"]);
        let (rc, v) = json(&a);
        assert_eq!(rc, 0, "{verb:?}: row 3 is a DISCLOSURE, not an exit code — the gate exits 0 too");
        assert_eq!(v["incomplete"], serde_json::json!(true),
                   "{verb:?}: row 3's own instruction is `no manifest, no claim`: {v}");
        assert_eq!(v["noManifest"], serde_json::json!([row3_file]),
                   "{verb:?}: SPEC §2 pins `noManifest: [\"<report path>\", …]` verbatim: {v}");
        assert!(v.get("judgedNothing").is_none(),
                "{verb:?}: the report DECLARES nothing — filing it under `judgedNothing` asserts an \
                 `analyzed.count: 0` that is not on the wire, and makes one key mean two things: {v}");

        // CONTROL, ROW 1: `analyzed.count: 0` keeps `judgedNothing` and never becomes `noManifest`.
        let mut a = verb.clone();
        a.extend(["--report", &row1, "--policy", &pol, "--json"]);
        let (rc, v) = json(&a);
        assert_eq!(rc, 0);
        assert_eq!(v["judgedNothing"], serde_json::json!([row1_file]),
                   "{verb:?}: the split goes both ways or it is a rename: {v}");
        assert!(v.get("noManifest").is_none(), "{verb:?}: row 1 HAS a manifest; it declares 0: {v}");

        // CONTROL, ROW 2: `count: 7` + `functions: []` is a legitimate all-pure claim §2 rule 3
        // requires a consumer to BELIEVE. A fix that hedges all three rows has disabled the feature.
        let mut a = verb.clone();
        a.extend(["--report", &row2, "--policy", &pol, "--json"]);
        let (rc, v) = json(&a);
        assert_eq!(rc, 0);
        assert!(v.get("incomplete").is_none() && v.get("noManifest").is_none()
                    && v.get("judgedNothing").is_none(),
                "{verb:?}: row 2 MUST NOT hedge — over 1997 JVM dependency jars a predicate keyed on \
                 `functions` being empty would withdraw 104 real claims to catch 6: {v}");
    }

    // The PROSE channel stops asserting `analyzed.count: 0` about a report that declares nothing, and
    // points at row 3's repair (a producer that emits a manifest) rather than row 1's (a scan that
    // reaches a conclusion). Both channels or neither — the mutant that survived a whole suite once was
    // exactly one channel going quiet.
    let out = Command::new(bin()).args(["map", "--report", &row3]).output().expect("run");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("NO `analyzed` manifest"), "the prose must name the real cause:\n{text}");
    assert!(!text.contains("JUDGED NOTHING") && !text.contains("analyzed.count: 0"),
            "…and must not re-assert row 1's claim in prose after removing it from the wire:\n{text}");

    // `gains` rests on TWO reports and discloses each side separately, `baseline`-prefixed (SPEC §2).
    let (_, v) = json(&["gains", &row3, &row1, "--json"]);
    assert_eq!(v["noManifest"], serde_json::json!([row3_file]), "{v}");
    assert_eq!(v["baselineJudgedNothing"], serde_json::json!([row1_file]), "{v}");
    assert!(v.get("judgedNothing").is_none() && v.get("baselineNoManifest").is_none(), "{v}");
    let (_, v) = json(&["gains", &row1, &row3, "--json"]);
    assert_eq!(v["judgedNothing"], serde_json::json!([row1_file]), "{v}");
    assert_eq!(v["baselineNoManifest"], serde_json::json!([row3_file]), "{v}");

    // THE OTHER ROW-3 CONTROL: manifest-less but it LISTS a function. It judged something and said so
    // the only way a pre-⟨0.21⟩ producer could, so it is not hedging at all — the standing SPEC §2's
    // manifest-absent row has always given it, and `report_judged_nothing` still decides that.
    let (row3full, _) = mk("row3full",
        r#"{"candor":{"version":"t","spec":"0.20"},"package":"lib","functions":[{"fn":"app.reads","inferred":["Fs"],"direct":["Fs"]}]}"#);
    let (rc, v) = json(&["where", "Fs", "--report", &row3full, "--json"]);
    assert_eq!(rc, 0);
    assert_eq!(v["directly"], serde_json::json!(["app.reads"]), "{v}");
    assert!(v.get("incomplete").is_none() && v.get("noManifest").is_none(),
            "a manifest-less report that LISTS entries is not hedging: {v}");
}

#[test]
fn gains_strict_exits_1_and_rejects_silently_swallowed_policy() {
    // #3: gains is a diff view (exit 0 by default). Two fixes: (a) `--strict` fails on ANY gained effect so a
    // supply-chain CI job can require a bump introduce no new capability; (b) an unknown flag (notably a
    // `--policy` a user reaches for expecting a gate) is REJECTED loud (exit 2), never swallowed → an exit-0
    // false-clean. The effect-specific gate stays the scan-time `deny <E> gained` policy.
    let f = Fixture::new("gainsstrict");
    // baseline: a fn doing Fs; current: same fn now does Fs+Net → a gained Net effect.
    let base_pre = format!("{}.base", f.prefix);
    let cur_pre = format!("{}.cur", f.prefix);
    let base_report = r#"{"candor":{"version":"t","spec":"0.23"},"package":"lib","functions":[{"fn":"lib::f","loc":"s:1","inferred":["Fs"],"hash":"h"}]}"#;
    let cur_report = r#"{"candor":{"version":"t","spec":"0.23"},"package":"lib","functions":[{"fn":"lib::f","loc":"s:1","inferred":["Fs","Net"],"hash":"h"}]}"#;
    std::fs::write(format!("{base_pre}.lib.scan.json"), base_report).unwrap();
    std::fs::write(format!("{cur_pre}.lib.scan.json"), cur_report).unwrap();
    let (curs, bases) = (cur_pre.clone(), base_pre.clone());
    // default: advisory, exit 0 even though Net was gained
    let adv = Command::new(bin()).args(["gains", &curs, &bases]).output().expect("run");
    assert_eq!(adv.status.code(), Some(0), "gains is advisory by default (exit 0)");
    assert!(String::from_utf8_lossy(&adv.stdout).contains("Net"), "gained Net must be listed");
    // --strict: a gain exists → exit 1
    let strict = Command::new(bin()).args(["gains", &curs, &bases, "--strict"]).output().expect("run");
    assert_eq!(strict.status.code(), Some(1), "--strict with a gained effect must exit 1");
    // a silently-swallowed --policy is now a loud exit-2 error pointing at the real gate
    let pol = Command::new(bin()).args(["gains", &curs, &bases, "--policy", "/x"]).output().expect("run");
    assert_eq!(pol.status.code(), Some(2), "an unknown flag (--policy) must be rejected, not swallowed");
    let se = String::from_utf8_lossy(&pol.stderr);
    assert!(se.contains("unknown flag") && se.contains("gained"),
        "must name the unknown flag + point at the `deny <E> gained` scan gate, got:\n{se}");
    // Fable-review finding C: a SINGLE-dash typo must also reject (exit 2), not fall through to a positional
    // and be dropped (which ran the gate disarmed at exit 0). Matches the shared grammar's `-`+len>1 rule.
    let dash = Command::new(bin()).args(["gains", &curs, &bases, "-strict"]).output().expect("run");
    assert_eq!(dash.status.code(), Some(2), "a single-dash typo (`-strict`) must reject, not silently drop");
    // Fable-review finding D: a valid cross-engine output flag (`--text`) must be TOLERATED (exit != 2), like
    // every other verb under the #2 contract — the bespoke gains parser must not be the one that rejects it.
    let txt = Command::new(bin()).args(["gains", &curs, &bases, "--text"]).output().expect("run");
    assert_ne!(txt.status.code(), Some(2), "--text (a valid cross-engine flag) must be tolerated by gains");
}

#[test]
fn valueless_policy_flag_exits_2_not_silent() {
    // Fable-review finding G: `--policy` with no value must exit 2 (like `--report`), never warn-and-continue
    // with policy=None — which silently gated against CANDOR_POLICY/.candor/config (a DIFFERENT policy than
    // named) or answered a policy-optional verb with no verdict at exit 0.
    let f = Fixture::new("valpol");
    f.write_report();
    let out = Command::new(bin()).args(["where", "Fs", "--report", &f.report_path(), "--policy"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "a valueless --policy must exit 2");
    assert!(String::from_utf8_lossy(&out.stderr).contains("--policy requires"),
        "must name the missing argument");
}

#[test]
fn fix_gate_unreadable_policy_exits_2() {
    // Same fail-loud contract: an unreadable policy must exit 2, never emit an empty (falsely-clean) verdict.
    let f = Fixture::new("fixgatebadpol");
    write_orderflow_fixture(&f);
    let bogus = f.dir.join("typo.policy");
    let out = Command::new(bin())
        .args(["fix-gate", &f.prefix, bogus.to_string_lossy().as_ref(), "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "an unreadable policy must exit 2");
}

#[test]
fn unverified_flags_an_unknown_in_a_deny_scope() {
    // A domain fn is Unknown (not the denied Net, so `deny Net domain` PASSES it) — but the Unknown could
    // hide Net (a fn/closure-injected port). `unverified` discloses it + names the `deny Net Unknown domain`
    // upgrade; `--strict` exits 1. A provably-pure domain fn is not flagged. (eval/fixloop/DISPATCH-NOTE.md.)
    let f = Fixture::new("fixunv");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "domain::price", "loc": "src/d.rs:1:1", "inferred": ["Unknown"], "unknownWhy": ["callback:injected"], "hash": "of#p", "paths": ["/x"] },
    { "fn": "domain::calc",  "loc": "src/d.rs:9:1", "inferred": [], "hash": "of#c", "paths": ["/x"] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["unverified", &f.prefix, &pol, "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "advisory by default → exit 0");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false), "an Unknown-in-scope hole exists");
    let items = v["unverified"].as_array().unwrap();
    assert_eq!(items.len(), 1, "only the Unknown fn is flagged, not the provably-pure one");
    assert_eq!(items[0]["fn"], serde_json::json!("domain::price"));
    assert_eq!(items[0]["upgrade"], serde_json::json!("deny Net Unknown domain"));
    // --strict → exit 1 (CI can require provable purity).
    let strict = Command::new(bin())
        .args(["unverified", &f.prefix, &pol, "--strict"])
        .output()
        .expect("run candor-query");
    assert_eq!(strict.status.code(), Some(1), "--strict must exit 1 on an unverified hole");
}

#[test]
fn unverified_provably_pure_scope_is_clean() {
    // A domain with only real-effect-free, resolvable functions → no Unknown holes → clean, exit 0 even strict.
    let f = Fixture::new("fixunvok");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "domain::calc", "loc": "src/d.rs:1:1", "inferred": [], "hash": "of#c", "paths": ["/x"] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "p.policy", "deny Net domain\n");
    let out = Command::new(bin())
        .args(["unverified", &f.prefix, &pol, "--strict", "1"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "no Unknown holes → clean, exit 0 even under --strict");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true));
}

// ── callers --include-unknown: the unresolved-dispatch frontier (⟨0.7⟩ — was conformance-only) ─────
// TESTING.md §3: engine-local behavior needs in-repo coverage; this arm previously lived only in the
// candor-spec conformance suite. Names are dot-separated (the swift/JVM report shape this arm serves).

/// The dot-free `unknownWhy` detail candor-scan emits for a call whose receiver it could not type at
/// all (`crates/candor-scan/src/scan.rs`) — the DOMINANT dispatch reason on this engine.
const DOT_FREE_REASON: &str = "untyped cross-package receiver";

/// Write a report + callgraph (+ optionally a hierarchy sidecar) for the frontier scenario:
/// confirmed chain `mod.Sub.handle → mod.Target.work`, plus Unknown-dispatch carriers whose disclosure
/// depends on the hierarchy gate, one DOT-FREE carrier (⟨0.24⟩ — condition (3) is unanswerable, so it is
/// disclosed verbatim in every arm), and two CONTROLS that must stay OUT in every arm: `mod.Other.run`
/// (a well-formed dotted reason on an unrelated owner+member, i.e. condition (3) genuinely FAILS) and
/// `mod.NoWhy.run` (Unknown with no `dispatch:` reason at all, i.e. condition (1) fails).
fn write_frontier_fixture(f: &Fixture, with_hierarchy: bool) {
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "app",
  "functions": [
    { "fn": "mod.Target.work", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "mod.Sub.handle", "inferred": ["Fs"], "calls": ["mod.Target.work"] },
    { "fn": "mod.Caller.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Base.handle"] },
    { "fn": "mod.Other.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Unrelated.frob"] },
    { "fn": "mod.NotSub.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Elsewhere.handle"] },
    { "fn": "mod.NoWhy.run", "inferred": ["Unknown"], "unknownWhy": ["callback:opaque fn pointer"] },
    { "fn": "mod.Dotfree.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:untyped cross-package receiver"] }
  ]
}"#;
    std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
    std::fs::write(
        format!("{}.app.scan.callgraph.json", f.prefix),
        r#"{"mod.Sub.handle":["mod.Target.work"],"mod.Target.work":[]}"#,
    )
    .unwrap();
    if with_hierarchy {
        // type → its supertypes: Sub is (only) a subtype of Base.
        //
        // ⟨0.26⟩ `mod.Base` carries its OWN key with an empty array. §2.2 makes the KEY SET the manifest,
        // so a root is not optional bookkeeping: without it a walk up from `mod.Sub` runs off the indexed
        // set and every question it was asked becomes UNANSWERABLE — including the ones this fixture's
        // controls need ANSWERED NO. See `callers_include_unknown_unindexed_type_is_unanswerable_not_no`
        // for the pre-rung shape and what it costs.
        std::fs::write(
            format!("{}.app.hierarchy.json", f.prefix),
            r#"{"mod.Sub":["mod.Base"],"mod.Base":[]}"#,
        )
        .unwrap();
    }
}

#[test]
fn callers_include_unknown_discloses_the_dispatch_frontier_via_the_hierarchy() {
    // With the hierarchy sidecar: a `dispatch:OWNER.member` source is disclosed iff a CONFIRMED
    // reacher overrides OWNER.member — same simple method AND a subtype of OWNER. `mod.Caller.run`
    // (dispatch on Base.handle; Sub <: Base reaches the target) is IN; a same-named method on an
    // unrelated owner (`mod.NotSub.run` → Elsewhere.handle) and a different method (`mod.Other.run`
    // → Unrelated.frob) are OUT. Disclosed as possible — never asserted into `transitive`.
    // `mod.Dotfree.run` rides along under ⟨0.24⟩: no owner to test, so it is disclosed verbatim.
    let f = Fixture::new("frontier-hier");
    write_frontier_fixture(&f, true);
    let out = Command::new(bin())
        .arg("callers").arg(&f.prefix).arg("work").arg("1").arg("--include-unknown")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    assert_eq!(v["of"], serde_json::json!(["mod.Target.work"]));
    assert_eq!(v["direct"], serde_json::json!(["mod.Sub.handle"]));
    assert_eq!(v["transitive"], serde_json::json!(["mod.Sub.handle"]),
               "frontier candidates must NOT be asserted into the confirmed set: {v}");
    let poss = v["possibleViaUnknownDispatch"].as_array().expect("frontier array");
    assert_eq!(poss.len(), 2, "the hierarchy-confirmed overrider + the unanswerable dot-free one: {v}");
    assert_eq!(poss[0]["fn"], "mod.Caller.run");
    assert_eq!(poss[0]["viaDispatchOn"], "handle");
    assert_eq!(poss[1]["fn"], "mod.Dotfree.run");
    assert_eq!(poss[1]["viaDispatchOn"], DOT_FREE_REASON);
}

#[test]
fn callers_include_unknown_without_hierarchy_over_lists_by_simple_name() {
    // No hierarchy sidecar → the documented fallback: a simple-METHOD-name match, which over-lists
    // (the safe direction — a possible reacher is disclosed, never silently dropped). Both `handle`
    // dispatchers now appear; the different-method one still doesn't.
    let f = Fixture::new("frontier-flat");
    write_frontier_fixture(&f, false);
    let out = Command::new(bin())
        .arg("callers").arg(&f.prefix).arg("work").arg("1").arg("--include-unknown")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    let fns: Vec<&str> = v["possibleViaUnknownDispatch"].as_array().unwrap()
        .iter().filter_map(|p| p["fn"].as_str()).collect();
    assert_eq!(fns, vec!["mod.Caller.run", "mod.Dotfree.run", "mod.NotSub.run"],
               "empty hierarchy must fall back to simple-name over-listing: {v}");
}

#[test]
fn callers_without_the_flag_omits_the_frontier_key() {
    // The ⟨0.7⟩ flag is additive: without it the {of,direct,transitive} shape is unchanged — a
    // pre-0.7 consumer never sees the new key.
    let f = Fixture::new("frontier-off");
    write_frontier_fixture(&f, true);
    let out = Command::new(bin())
        .arg("callers").arg(&f.prefix).arg("work").arg("1")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    assert!(v.get("possibleViaUnknownDispatch").is_none(),
            "no --include-unknown → no frontier key: {v}");
    assert_eq!(v["direct"], serde_json::json!(["mod.Sub.handle"]));
}

/// ⟨0.24⟩ THE DISPATCH FRONTIER MUST KEY OFF THE **KIND**, NOT THE **CLASS**, AND THIS IS THE ONE
/// FIXTURE WHERE THE TWO ANSWER DIFFERENTLY.
///
/// §6.2 projects `ambiguous:` to class `dispatch`, so a frontier that selected sources by class would
/// admit every one of them. But `ambiguous:` means the analyser's own name resolution failed and NO
/// OWNER WAS EVER FORMED — condition (3), "some confirmed reacher is an override of OWNER.member", has
/// nothing to resolve against. The `strip_prefix("dispatch:")` in `callers.rs` excludes them for free.
///
/// This is not a corner on this engine: `ambiguous:` is **8710 of 19607** `unknownWhy` entries over a
/// 1062-report census, so a class-keyed frontier would flood — and under the ⟨0.24⟩ dot-free rule it
/// would flood LOUDLY (each admitted entry disclosed verbatim) rather than silently. Still wrong.
///
/// The same fixture carries the END-TO-END half of the §4 forward-compatibility control: a FABRICATED
/// `banana:whatever` kind must round-trip verbatim through the binary and classify through the
/// conservative catch-all. `blindspots --class` is the class-keyed selector, run side by side with the
/// kind-keyed frontier over ONE report — so the test states the distinction rather than asserting it.
#[test]
fn callers_include_unknown_keys_off_the_kind_so_ambiguous_and_off_vocabulary_stay_out() {
    let f = Fixture::new("frontier-kind-vs-class");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "app",
  "functions": [
    { "fn": "app.Target.work", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "app.Sub.handle", "inferred": ["Fs"], "calls": ["app.Target.work"] },
    { "fn": "app.Real.run", "inferred": ["Unknown"], "direct": ["Unknown"], "unknownWhy": ["dispatch:app.Base.handle"] },
    { "fn": "app.Amb.run", "inferred": ["Unknown"], "direct": ["Unknown"], "unknownWhy": ["ambiguous:same-name local defs"] },
    { "fn": "app.Banana.run", "inferred": ["Unknown"], "direct": ["Unknown"], "unknownWhy": ["banana:whatever"] }
  ]
}"#;
    std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
    std::fs::write(format!("{}.app.scan.callgraph.json", f.prefix),
                   r#"{"app.Sub.handle":["app.Target.work"],"app.Target.work":[]}"#).unwrap();
    std::fs::write(format!("{}.app.hierarchy.json", f.prefix), r#"{"app.Sub":["app.Base"]}"#).unwrap();

    // (1) THE FRONTIER — kind-keyed. Only the genuine `dispatch:` source, which has an owner to resolve.
    assert_eq!(frontier_of(&f.prefix), vec![("app.Real.run".to_string(), "handle".to_string())],
               "`ambiguous:` (class `dispatch`, no owner) and an off-vocabulary kind must both stay OUT");

    // (2) THE CLASS-KEYED SELECTOR over the SAME report — and it answers `dispatch` for the ambiguous
    // one. That is the whole point: the projection is correct, and it is still not what the frontier
    // may select on. Were `callers.rs` keyed on `ReasonClass::classify(w) == Dispatch`, (1) would list
    // `app.Amb.run` too.
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--class").arg("dispatch").arg("--json")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    let names: Vec<&str> = v["sources"].as_array().unwrap().iter().map(|s| s["fn"].as_str().unwrap()).collect();
    assert_eq!(names, ["app.Amb.run", "app.Real.run"],
               "§6.2 projects `ambiguous:*` to class `dispatch` — a class-keyed frontier WOULD admit it: {v}");

    // (3) THE FABRICATED KIND, end to end (§4 ⟨0.24⟩'s SHOULD). It reaches the catch-all class…
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--class").arg("unresolved").arg("--json")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    let names: Vec<&str> = v["sources"].as_array().unwrap().iter().map(|s| s["fn"].as_str().unwrap()).collect();
    assert_eq!(names, ["app.Banana.run"], "an unrecognised kind classifies `unresolved`, never dropped: {v}");
    // …and its raw text survives the whole binary byte for byte, never normalised toward a known kind.
    assert_eq!(v["sources"][0]["why"], serde_json::json!(["banana:whatever"]),
               "the fabricated kind must round-trip verbatim: {v}");
}

/// The frontier's `possibleViaUnknownDispatch` entries as `(fn, viaDispatchOn)`, for one arm.
fn frontier_of(prefix: &str) -> Vec<(String, String)> {
    frontier_of_q(prefix, "work")
}

/// As `frontier_of`, for a fixture whose target is not `work`.
fn frontier_of_q(prefix: &str, q: &str) -> Vec<(String, String)> {
    let out = Command::new(bin())
        .arg("callers").arg(prefix).arg(q).arg("1").arg("--include-unknown")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    v["possibleViaUnknownDispatch"].as_array().expect("frontier array").iter()
        .map(|p| (p["fn"].as_str().unwrap().to_string(),
                  p["viaDispatchOn"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn callers_include_unknown_discloses_a_dot_free_dispatch_reason_verbatim_in_both_arms() {
    // ⟨0.24⟩ A `dispatch:` detail with NO DOT names no owner and no member, so condition (3) — "some
    // confirmed reacher is an OVERRIDE of OWNER.M" — is UNANSWERABLE. An unanswerable condition must not
    // be scored as a failed one, so the source is DISCLOSED with the raw detail verbatim.
    //
    // MEASURED before the fix: `mod.Dotfree.run` (carrying candor-scan's own dominant reason,
    // `dispatch:untyped cross-package receiver`) appeared NOWHERE in the output in EITHER arm and no
    // diagnostic named it — `simple_method`/`declaring_type` fall back to the whole string with no dot,
    // so `by_method.get(m)` could never hit. That omission reads to a consumer as "no function may reach
    // the target through an unresolved dispatch": a false all-clear on the query.
    for with_hier in [true, false] {
        let f = Fixture::new(if with_hier { "frontier-dotfree-hier" } else { "frontier-dotfree-flat" });
        write_frontier_fixture(&f, with_hier);
        let got = frontier_of(&f.prefix);
        let entry = got.iter().find(|(fname, _)| fname == "mod.Dotfree.run");
        assert!(entry.is_some(),
                "dot-free dispatch source must be disclosed (hierarchy={with_hier}): {got:?}");
        assert_eq!(entry.unwrap().1, DOT_FREE_REASON,
                   "viaDispatchOn must be the RAW detail verbatim (hierarchy={with_hier})");
    }
}

/// ⟨0.26⟩ THE SIDECAR'S KEY SET IS ITS MANIFEST (SPEC §2.2), so a type with NO key was never analysed
/// and NOTHING may be concluded about it.
///
/// `hier.get(t)` returning `None` used to skip the frame silently, collapsing "indexed, no supertypes"
/// and "never analysed" into the same answer: `false`. That is a positive claim about a type nobody
/// analysed, and here it removes a reacher from a disclosure with no diagnostic at all.
///
/// This engine writes NO hierarchy sidecar (candor-scan emits none), so every hierarchy it walks came
/// from candor-java or candor-ts — the producer's completeness is not this engine's to assume, which is
/// what makes the tri-state load-bearing on this side of the family.
///
/// The chain is deliberately TWO levels (`Sub` → `Mid` → `Base`). A flat one cannot see this defect: the
/// walk hits OWNER as an immediate supertype and returns YES before it ever reaches an unindexed type,
/// so old and new code agree. The gap has to be IN the path.
fn write_two_level_frontier_fixture(f: &Fixture, hier: &str) {
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "app",
  "functions": [
    { "fn": "mod.Target.work", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "mod.Sub.handle", "inferred": ["Fs"], "calls": ["mod.Target.work"] },
    { "fn": "mod.Caller.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Base.handle"] }
  ]
}"#;
    std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
    std::fs::write(
        format!("{}.app.scan.callgraph.json", f.prefix),
        r#"{"mod.Sub.handle":["mod.Target.work"],"mod.Target.work":[]}"#,
    )
    .unwrap();
    if !hier.is_empty() {
        std::fs::write(format!("{}.app.hierarchy.json", f.prefix), hier).unwrap();
    }
}

#[test]
fn callers_include_unknown_unindexed_type_is_unanswerable_not_no() {
    let names = |hier: &str, tag: &str| -> Vec<String> {
        let f = Fixture::new(tag);
        write_two_level_frontier_fixture(&f, hier);
        frontier_of(&f.prefix).into_iter().map(|(n, _)| n).collect()
    };
    let disclosed = vec!["mod.Caller.run".to_string()];

    // COMPLETE sidecar: every type on the walk is indexed and the chain reaches `mod.Base` — YES.
    assert_eq!(
        names(r#"{"mod.Sub":["mod.Mid"],"mod.Mid":["mod.Base"],"mod.Base":[]}"#, "sidecar-complete"),
        disclosed);

    // PARTIAL sidecar: `mod.Sub` is indexed and names `mod.Mid`, but `mod.Mid` itself was never indexed,
    // so the chain beyond it is unreadable and the relation cannot be ruled out. UNANSWERABLE → disclose.
    // PRE-⟨0.26⟩ THIS ARM RETURNED `[]` — the reacher vanished because a type nobody analysed was read
    // as "has no supertypes".
    assert_eq!(names(r#"{"mod.Sub":["mod.Mid"]}"#, "sidecar-partial"), disclosed);

    // ABSENT sidecar: the documented simple-name fallback, which over-lists. PARTIAL INFORMATION MUST NOT
    // BE WORSE THAN NONE — this pair is the invariant the defect broke, and the reason the repair needed a
    // format change rather than a consumer-side patch: without the manifest neither side can tell a
    // producer's silence from its answer.
    assert_eq!(names("", "sidecar-absent"), disclosed);

    // THE NEGATIVE CONTROL, so this is not "always disclose". Every type the walk touches IS indexed and
    // the chain ends without reaching `mod.Base` — a complete NO, and the reacher drops out. Without this
    // row the tri-state would pass by answering YES to everything.
    assert_eq!(
        names(r#"{"mod.Sub":["mod.Mid"],"mod.Mid":[],"mod.Base":[]}"#, "sidecar-answered-no"),
        Vec::<String>::new());

    // A POSITIVE DOMINATES an unknown branch: `mod.Sub` reaches `mod.Base` down one arm and an unindexed
    // type down another. The established relation is not weakened by the branch nobody analysed, so this
    // is YES for a REASON rather than by falling through to the unanswerable rule.
    assert_eq!(
        names(r#"{"mod.Sub":["mod.Base","mod.Unseen"],"mod.Base":[]}"#, "sidecar-positive-dominates"),
        disclosed);
}

#[test]
fn callers_include_unknown_dot_free_disclosure_is_not_a_blanket() {
    // THE CONTROL — without it a fix is indistinguishable from "disclose everything". Two functions must
    // stay OUT of the frontier in the arm where the conditions ARE answerable (hierarchy present):
    //   • `mod.NoWhy.run`  — Unknown, but no `dispatch:` reason at all → condition (1) fails.
    //   • `mod.Other.run`  — a well-formed dotted reason (`mod.Unrelated.frob`) whose owner AND member
    //     are unrelated to any confirmed reacher → condition (3) is ANSWERED, and answered "no".
    //   • `mod.NotSub.run` — same member, wrong owner: (3) answered "no" via the hierarchy.
    // Only the two entitled entries are listed.
    let f = Fixture::new("frontier-dotfree-control");
    write_frontier_fixture(&f, true);
    let got = frontier_of(&f.prefix);
    let fns: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(fns, vec!["mod.Caller.run", "mod.Dotfree.run"],
               "an ANSWERED-no condition stays out; only the unanswerable one is added: {got:?}");
    for out in ["mod.NoWhy.run", "mod.Other.run", "mod.NotSub.run"] {
        assert!(!fns.contains(&out), "{out} must NOT be disclosed: {got:?}");
    }
}

#[test]
fn callers_include_unknown_dot_free_reason_never_matches_a_dot_free_rust_qual() {
    // TASK-(2) REGRESSION PIN, and a real defect measured on the pre-fix binary. Rust function quals are
    // `::`-separated and contain NO dot, so `simple_method`/`declaring_type` fell back to the WHOLE
    // STRING on BOTH sides — the reason detail and the confirmed reacher's qual. The override test then
    // degenerated into whole-string equality, and its outcome depended on accidents of spelling:
    //   • `dispatch:app::Sub::handle` (== a reacher's whole qual) → MATCHED in both arms, the "subtype"
    //     check passing only by reflexivity (`ty == owner`) over a string that is not a type name.
    //   • `dispatch:handle` (== a DOTTED reacher's simple method) → matched with no hierarchy, dropped
    //     with one — the same detail flipping on the presence of a sidecar.
    //   • `dispatch:untyped cross-package receiver` → dropped in both arms.
    // The structural dot-free branch runs FIRST, so no dot-free detail ever reaches `by_method` again:
    // all three are now disclosed verbatim, identically in both arms. No new false positive is possible
    // because the branch does not consult the reacher index at all.
    for with_hier in [true, false] {
        let f = Fixture::new(if with_hier { "frontier-rustqual-hier" } else { "frontier-rustqual-flat" });
        let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "app",
  "functions": [
    { "fn": "app::Target::work", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "app::Sub::handle", "inferred": ["Fs"], "calls": ["app::Target::work"] },
    { "fn": "app::EqQual::run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:app::Sub::handle"] },
    { "fn": "app::BareLeaf::run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:handle"] },
    { "fn": "app::Prose::run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:untyped cross-package receiver"] }
  ]
}"#;
        std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
        std::fs::write(
            format!("{}.app.scan.callgraph.json", f.prefix),
            r#"{"app::Sub::handle":["app::Target::work"],"app::Target::work":[]}"#,
        ).unwrap();
        if with_hier {
            std::fs::write(format!("{}.app.hierarchy.json", f.prefix), r#"{"app::Sub":["app::Base"]}"#).unwrap();
        }
        let got = frontier_of(&f.prefix);
        assert_eq!(
            got,
            vec![
                ("app::BareLeaf::run".to_string(), "handle".to_string()),
                ("app::EqQual::run".to_string(), "app::Sub::handle".to_string()),
                ("app::Prose::run".to_string(), DOT_FREE_REASON.to_string()),
            ],
            "every dot-free detail is disclosed verbatim, arm-independently (hierarchy={with_hier}): {got:?}"
        );
    }
}

#[test]
fn callers_include_unknown_mixed_source_joins_members_and_raw_details_in_sorted_order() {
    // ⟨0.24⟩ THE MIXED SOURCE, raised by the java engine and now the cross-engine fixture. A function
    // carrying SEVERAL `dispatch:` reasons — dotted ones that PASS condition (3) and dot-free ones that
    // cannot be evaluated at all — gets ONE entry, whose `viaDispatchOn` is the sorted, deduplicated,
    // comma-joined union of the passing members `M` and the raw details. Rust satisfies this by
    // construction (`BTreeSet<&str>` + `join(",")`), so this is a PIN, not a fix — and unpinned
    // conformance-by-accident is what the next refactor removes.
    //
    // ENCOUNTER ORDER AND SORT ORDER DISAGREE ON PURPOSE — that is the whole value of the test. The
    // reasons are fed in the order (dot-free, write, run), which is neither sorted nor kind-grouped, and
    // `write` sorts AFTER the dot-free detail ('w' > 'u'), so the expected string INTERLEAVES the two
    // kinds. An encounter-order join yields "untyped cross-package receiver,write,run"; a "dotted members
    // first, then dot-free" join yields "run,write,untyped cross-package receiver". Both must FAIL.
    let f = Fixture::new("frontier-mixed");
    // `app.Impl.run` and `app.Zed.write` are both confirmed reachers of `app.Sink.touch`; the hierarchy
    // puts Impl under BOTH Base and Other (so `app.Dedup.go`'s two distinct owners resolve to the same
    // member `run` and must collapse) and Zed under Base.
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.27" },
  "package": "app",
  "functions": [
    { "fn": "app.Sink.touch", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "app.Impl.run", "inferred": ["Fs"], "calls": ["app.Sink.touch"] },
    { "fn": "app.Zed.write", "inferred": ["Fs"], "calls": ["app.Sink.touch"] },
    { "fn": "app.Mixed.go", "inferred": ["Unknown"], "unknownWhy": [
        "dispatch:untyped cross-package receiver",
        "dispatch:app.Base.write",
        "dispatch:app.Base.run" ] },
    { "fn": "app.Dedup.go", "inferred": ["Unknown"], "unknownWhy": [
        "dispatch:app.Base.run",
        "dispatch:app.Other.run" ] }
  ]
}"#;
    std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
    std::fs::write(
        format!("{}.app.scan.callgraph.json", f.prefix),
        r#"{"app.Impl.run":["app.Sink.touch"],"app.Zed.write":["app.Sink.touch"],"app.Sink.touch":[]}"#,
    ).unwrap();
    std::fs::write(
        format!("{}.app.hierarchy.json", f.prefix),
        r#"{"app.Impl":["app.Base","app.Other"],"app.Zed":["app.Base"]}"#,
    ).unwrap();
    let got = frontier_of_q(&f.prefix, "touch");
    assert_eq!(
        got,
        vec![
            // Two owners (`app.Base`, `app.Other`), one member: Impl is a subtype of both, so both
            // reasons pass (3) and yield `run` — DEDUPLICATED to a single term, not "run,run".
            ("app.Dedup.go".to_string(), "run".to_string()),
            // Three reasons, two kinds, interleaved by sort — NOT grouped by kind, NOT encounter order.
            ("app.Mixed.go".to_string(), "run,untyped cross-package receiver,write".to_string()),
        ],
        "mixed source: sorted+deduplicated union of passing members and raw details: {got:?}"
    );
}

#[test]
fn callers_include_unknown_join_sorts_by_unicode_code_point_not_utf16_code_unit() {
    // ⟨0.24⟩ THE COLLATION CLAUSE: "sorted" means by UNICODE CODE POINT, equivalently UTF-8 byte order.
    // The two orders agree everywhere in the BMP and DISAGREE above it: java's `String.compareTo` and
    // JS's default `Array.sort` compare UTF-16 code units, and a supplementary character's leading
    // surrogate (U+D800..U+DBFF) sorts BELOW a Private-Use/BMP-tail character in U+E000..U+FFFF even
    // though its code point is far above.
    //
    // Here: U+FF21 (FULLWIDTH LATIN CAPITAL A, UTF-8 `EF BC A1`, UTF-16 `FF21`) vs U+1F600 (GRINNING
    // FACE, UTF-8 `F0 9F 98 80`, UTF-16 `D83D DE00`). Code point / UTF-8: FF21 FIRST. UTF-16: 1F600
    // first. The two reasons are fed 1F600-first so encounter order also differs from the expected
    // order — this test fails on a UTF-16 comparator AND on any lost sort.
    //
    // Rust gets this right for free: `Ord for str` is byte-wise over UTF-8, which IS code-point order,
    // and no plausible rust idiom yields UTF-16 order. Kept anyway because it is the rust arm of a
    // cross-engine fixture the other engines CAN fail, and because it goes red under a real mutation
    // (verified against a `sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()))` join).
    let f = Fixture::new("frontier-collation");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.27" },
  "package": "app",
  "functions": [
    { "fn": "app.Sink.touch", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "app.Impl.run", "inferred": ["Fs"], "calls": ["app.Sink.touch"] },
    { "fn": "app.Wide.go", "inferred": ["Unknown"], "unknownWhy": [
        "dispatch:\ud83d\ude00-supplementary",
        "dispatch:\uff21-bmp-tail" ] }
  ]
}"#;
    // (Written as JSON's OWN escapes inside a rust raw string, so the code points are explicit and
    // survive any editor round-trip, and so a FULLWIDTH `\uff21` is never mistaken for an ASCII `A`
    // when read: `\ud83d\ude00` is the surrogate pair for U+1F600. serde_json decodes both to the real
    // characters before candor ever sees them.)
    std::fs::write(format!("{}.app.scan.json", f.prefix), report).unwrap();
    std::fs::write(
        format!("{}.app.scan.callgraph.json", f.prefix),
        r#"{"app.Impl.run":["app.Sink.touch"],"app.Sink.touch":[]}"#,
    ).unwrap();
    let got = frontier_of_q(&f.prefix, "touch");
    assert_eq!(
        got,
        vec![("app.Wide.go".to_string(), "\u{FF21}-bmp-tail,\u{1F600}-supplementary".to_string())],
        "code-point order puts U+FF21 before U+1F600; UTF-16 code-unit order would invert it: {got:?}"
    );
}

// ── blindspots: the Unknown sources ranked by blast radius (SPEC §3.1 ⟨0.6⟩ — was conformance-only) ─

#[test]
fn blindspots_ranks_sources_by_unknown_blast_radius() {
    // Two SOURCES (entries carrying their own unknownWhy): src_a smears Unknown up a two-hop caller
    // chain (reaches 2), src_b reaches nobody. Ranked most-smearing first; `affected` is the sorted
    // transitive caller set; totalUnknown counts every Unknown-carrying fn (sources + inheritors).
    let f = Fixture::new("blindspots");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "bs",
  "functions": [
    { "fn": "src_a", "inferred": ["Unknown"], "unknownWhy": ["callback:unresolved call"] },
    { "fn": "mid", "inferred": ["Unknown"], "calls": ["src_a"] },
    { "fn": "top", "inferred": ["Unknown"], "calls": ["mid"] },
    { "fn": "src_b", "inferred": ["Unknown"], "unknownWhy": ["native:extern fn"] }
  ]
}"#;
    std::fs::write(f.report_path().replace(".rpt.", ".bs."), report).unwrap();
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--json")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    assert_eq!(v["totalUnknown"], 4, "every Unknown-carrying fn counts: {v}");
    let sources = v["sources"].as_array().expect("sources");
    assert_eq!(sources.len(), 2, "only unknownWhy CARRIERS are sources (mid/top are not): {v}");
    assert_eq!(sources[0]["fn"], "src_a", "most-smearing source ranks first: {v}");
    assert_eq!(sources[0]["reaches"], 2);
    assert_eq!(sources[0]["affected"], serde_json::json!(["mid", "top"]));
    assert_eq!(sources[0]["why"], serde_json::json!(["callback:unresolved call"]));
    assert_eq!(sources[1]["fn"], "src_b");
    assert_eq!(sources[1]["reaches"], 0);

    // human mode agrees on the headline numbers
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix)
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("2 Unknown source(s) explaining 4 Unknown function(s)"),
            "headline must count sources + explained fns, got:\n{text}");
}

#[test]
fn blindspots_stats_reason_class_distribution() {
    // ⟨0.20⟩ `--stats`: the reason-class distribution over the Unknown SOURCES. src_a is callback:→indirect,
    // src_b is native:→native; mid/top are transitive-only (no direct reason) → not sources.
    let f = Fixture::new("blindspots-stats");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "bs",
  "functions": [
    { "fn": "src_a", "inferred": ["Unknown"], "unknownWhy": ["callback:unresolved call"] },
    { "fn": "mid", "inferred": ["Unknown"], "calls": ["src_a"] },
    { "fn": "src_b", "inferred": ["Unknown"], "unknownWhy": ["native:extern fn"] }
  ]
}"#;
    std::fs::write(f.report_path().replace(".rpt.", ".bs."), report).unwrap();
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--stats").arg("--json")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    // all six classes present (0 when absent) + the two sources classified
    for c in ["reflect", "dispatch", "indirect", "native", "unresolved", "setup"] {
        assert!(v["byClass"].get(c).is_some(), "byClass must carry every class: {v}");
    }
    assert_eq!(v["byClass"]["indirect"], 1, "callback → indirect: {v}");
    assert_eq!(v["byClass"]["native"], 1, "native → native: {v}");
    assert_eq!(v["sources"], 2);
    assert_eq!(v["totalUnknown"], 3);

    // --class drill-down: keep only sources of the named class(es). native → src_b only.
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--class").arg("native").arg("--json")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    let names: Vec<&str> = v["sources"].as_array().unwrap().iter().map(|s| s["fn"].as_str().unwrap()).collect();
    assert_eq!(names, ["src_b"], "--class native keeps only the native source: {v}");
    // --stats composes with --class: the distribution restricted to `indirect`.
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix).arg("--stats").arg("--class").arg("indirect").arg("--json")
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    assert_eq!(v["sources"], 1, "--stats --class indirect counts only the indirect source: {v}");
    assert_eq!(v["byClass"]["native"], 0);
}

#[test]
fn blindspots_clean_report_says_so_exit_0() {
    // A report with no unknownWhy sources is the honest all-resolved answer, not an error.
    let f = Fixture::new("blindspots-clean");
    f.write_report(); // outer→inner, Fs only, no Unknown
    let out = Command::new(bin())
        .arg("blindspots").arg(&f.prefix)
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("no Unknown sources"));
}

// ── rewire: the de-wiring detector (was conformance-only) ──────────────────────────────────────────

#[test]
fn rewire_reports_a_dropped_edge_exit_1_and_clean_exit_0() {
    // An agent can satisfy an effect gate by DISCONNECTING functionality — invisible to the effect
    // diff, visible in the call graph. A baseline edge the current graph no longer has → exit 1 with
    // {caller, no_longer_calls}; an unchanged graph → exit 0.
    let f = Fixture::new("rewire");
    let base = f.dir.join("base").to_string_lossy().into_owned();
    let cur = f.dir.join("cur").to_string_lossy().into_owned();
    std::fs::write(format!("{base}.app.scan.callgraph.json"),
        r#"{"api.handle":["pricing.quote","util.log"],"pricing.quote":[]}"#).unwrap();
    std::fs::write(format!("{cur}.app.scan.callgraph.json"),
        r#"{"api.handle":["util.log"],"pricing.quote":[]}"#).unwrap();
    let out = Command::new(bin())
        .arg("rewire").arg(&cur).arg(&base).arg("1")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1), "a dropped edge must exit 1 (verify the fix didn't gut the feature)");
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).expect("json");
    assert_eq!(v["ok"], false);
    assert_eq!(v["dropped"], serde_json::json!([
        {"caller": "api.handle", "no_longer_calls": ["pricing.quote"]}
    ]), "exactly the dropped edge, not the kept one: {v}");
    // unchanged graph → clean exit 0
    let out = Command::new(bin())
        .arg("rewire").arg(&base).arg(&base).arg("0")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("nothing de-wired"));
}

#[test]
fn rewire_missing_either_side_fails_loud_exit_2() {
    // A typo'd CURRENT prefix once read as "every baseline edge dropped" (a wall of false de-wiring);
    // a missing BASELINE can't be compared at all. Both fail loud, never a fabricated verdict.
    let f = Fixture::new("rewire-miss");
    let real = f.dir.join("real").to_string_lossy().into_owned();
    std::fs::write(format!("{real}.app.scan.callgraph.json"), r#"{"a":["b"]}"#).unwrap();
    let missing = f.dir.join("nosuch").to_string_lossy().into_owned();
    for (cur, base) in [(&missing, &real), (&real, &missing)] {
        let out = Command::new(bin())
            .arg("rewire").arg(cur).arg(base).arg("0")
            .output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(2), "a missing side must exit 2, not a false verdict");
    }
}

// ── locate: the newest-by-mtime artifact locator (smoke) ────────────────────────────────────────────

#[test]
fn locate_finds_the_scan_binary_and_misses_cleanly() {
    let f = Fixture::new("locate");
    std::fs::write(f.dir.join("candor-scan"), b"#!fake").unwrap();
    let out = Command::new(bin())
        .arg("locate").arg("scan").arg(f.dir.to_string_lossy().as_ref())
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().trim().ends_with("candor-scan"));
    // no matching artifact → exit 1, empty stdout (the wrapper's fall-through signal)
    let out = Command::new(bin())
        .arg("locate").arg("lib").arg(f.dir.to_string_lossy().as_ref())
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

// ── §6.2 ⟨0.24⟩ THE `--class` VALUE GRAMMAR ────────────────────────────────────────────────────────
//
// `--class <c>[,<c>…]` takes ONE comma-separated list; it is NOT repeatable, and an UNRECOGNISED token is
// a usage error (exit 2) naming the token and listing the accepted set.
//
// WHY THIS IS NOT THE POLICY SIDE'S DROP-WITH-A-WARNING, since the asymmetry is deliberate and looks like
// an inconsistency until you write it down: a token dropped out of `deny E Unknown[reflect,dyanmic]`
// leaves the WIDER rule standing, so the mistake surfaces as a gate that over-fires and somebody comes to
// look. The same token dropped out of `--class` leaves a NARROWER filter — `unverified --class dyanmic`
// answers a question the user never asked, with a SMALLER number, and a smaller number out of the verb
// whose whole job is "green, but not provably so" is indistinguishable from a real all-clear. Before this
// change both engines exited 0 on the typo. A query flag that cannot be honoured is refused.
//
// THE MESSAGE IS ASSERTED, NOT JUST THE EXIT CODE. Every path through this parser can exit 2 for some
// unrelated reason (an unknown flag, a report that does not resolve, a `--class` whose value was never
// consumed and fell through to the deprecated leading-report peel), so a bare `code == 2` would pass
// against a mutation that removed the rule entirely.

/// The fixture both verbs share: two DIRECT Unknown sources of different classes, and one function that
/// INHERITS from the `dispatch` one — enough for a filter to select a proper subset, so the regression
/// control below can assert a COUNT rather than only an exit code.
fn write_class_grammar_report(f: &Fixture) -> String {
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.23" },
  "package": "of",
  "functions": [
    { "fn": "domain::src_dispatch", "loc": "src/d.rs:1:1", "inferred": ["Unknown"], "direct": ["Unknown"], "unknownWhy": ["dispatch:Base::run"], "hash": "of#sd", "paths": ["/x"] },
    { "fn": "domain::inherits",     "loc": "src/d.rs:9:1", "inferred": ["Unknown"], "hash": "of#in", "paths": ["/x"], "calls": ["domain::src_dispatch"] },
    { "fn": "domain::src_native",   "loc": "src/d.rs:17:1", "inferred": ["Unknown"], "direct": ["Unknown"], "unknownWhy": ["native:strlen"], "hash": "of#sn", "paths": ["/x"] }
  ]
}"#;
    std::fs::write(format!("{}.of.scan.json", f.prefix), report).unwrap();
    write_policy(f, "p.policy", "deny Exec domain\n")
}

/// The names `unverified --json` selected, with the exit code, so a caller can assert both.
fn unverified_names(prefix: &str, pol: &str, class: Option<&str>) -> (i32, Vec<String>) {
    let mut args: Vec<String> =
        vec!["unverified".into(), "--report".into(), prefix.into(), "--policy".into(), pol.into(), "--json".into()];
    if let Some(c) = class {
        args.push("--class".into());
        args.push(c.into());
    }
    let out = Command::new(bin()).args(&args).output().expect("run candor-query");
    let code = out.status.code().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let names = serde_json::from_str::<serde_json::Value>(text.trim())
        .map(|v| {
            v["unverified"].as_array().cloned().unwrap_or_default().iter()
                .map(|h| h["fn"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (code, names)
}

#[test]
fn class_grammar_accepts_a_token_a_list_and_both_aliases() {
    let f = Fixture::new("class-grammar-ok");
    let pol = write_class_grammar_report(&f);

    // (6) THE REGRESSION CONTROL, first: a well-formed filter must still select exactly what it selected
    // before the refusal was added. A COUNT and the NAMES, not an exit code — this change must alter no
    // selection and no verdict for input the grammar accepts.
    let (code, all) = unverified_names(&f.prefix, &pol, None);
    assert_eq!(code, 0);
    let mut all_sorted = all.clone();
    all_sorted.sort();
    assert_eq!(all_sorted, ["domain::inherits", "domain::src_dispatch", "domain::src_native"],
               "the unfiltered baseline the filters are a subset of");

    // (1) ONE valid token — a PROPER subset (the `dispatch` source and the fn inheriting from it), which
    // is what makes the control above discriminating: a filter that had stopped filtering would return 3.
    let (code, mut got) = unverified_names(&f.prefix, &pol, Some("dispatch"));
    got.sort();
    assert_eq!((code, got.as_slice()), (0, ["domain::inherits".to_string(), "domain::src_dispatch".to_string()].as_slice()),
               "--class dispatch selects the dispatch source + its inheritor, and nothing else");

    // (2) a valid COMMA LIST is a union of its tokens.
    let (code, list) = unverified_names(&f.prefix, &pol, Some("dispatch,native"));
    assert_eq!(code, 0);
    assert_eq!(list.len(), 3, "--class dispatch,native unions the two classes: {list:?}");
    // …and the whitespace a shell-quoted list tends to carry is trimmed, not treated as a token.
    let (code, spaced) = unverified_names(&f.prefix, &pol, Some(" dispatch , native "));
    assert_eq!((code, spaced.len()), (0, 3), "a spaced list is the same list");

    // (3) BOTH ALIASES. `dynamic` is not optional: §6.2's own normative diagnostic (`--class dynamic` ==
    // unfiltered minus setup-only) is stated in terms of it, so an engine that refused it as unrecognised
    // would break the standing test every engine carries. `*` is all six.
    let (code, dynamic) = unverified_names(&f.prefix, &pol, Some("dynamic"));
    assert_eq!((code, dynamic.len()), (0, 3), "`dynamic` is every genuine class; no fixture entry is setup-only");
    let (code, star) = unverified_names(&f.prefix, &pol, Some("*"));
    assert_eq!((code, star.len()), (0, 3), "`*` is all six classes");
    // the mirror control: a valid class nothing here carries selects NOTHING, at exit 0. This is what
    // separates "the token was accepted" from "the filter stopped filtering".
    let (code, none) = unverified_names(&f.prefix, &pol, Some("reflect"));
    assert_eq!((code, none.len()), (0, 0), "a valid class with no candidate is an empty answer, not an error");
}

#[test]
fn class_grammar_refuses_an_unrecognised_token_exit_2() {
    let f = Fixture::new("class-grammar-typo");
    let pol = write_class_grammar_report(&f);
    for verb in [
        vec!["unverified", "--report", &f.prefix, "--policy", &pol, "--json"],
        vec!["blindspots", "--report", &f.prefix, "--json"],
    ] {
        let mut args = verb.clone();
        args.push("--class");
        args.push("dyanmic"); // the transposition a human actually makes
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let err = String::from_utf8(out.stderr).unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?} must be a usage error, got:\n{err}");
        // NAME THE TOKEN. Without this the assertion passes against any other exit-2 path in the parser.
        assert!(err.contains("dyanmic"), "the message must name the offending token, got:\n{err}");
        assert!(err.contains("unrecognised reason-class"),
                "…and say what is wrong with it, rather than exiting 2 for some other reason:\n{err}");
        // LIST THE ACCEPTED SET — all six classes and both aliases, so the line can be fixed from the
        // message alone.
        for t in ["reflect", "dispatch", "indirect", "native", "unresolved", "setup", "dynamic", "*"] {
            assert!(err.contains(t), "the accepted set must list `{t}`, got:\n{err}");
        }
        // NO PARTIAL ANSWER. A refused filter must not also print a (narrower) result document — that is
        // the exact fail-open, one exit code away.
        assert!(out.stdout.is_empty(), "a refused --class must emit no answer at all: {:?}", String::from_utf8(out.stdout));
    }
}

#[test]
fn class_grammar_refuses_a_repeated_flag_exit_2() {
    let f = Fixture::new("class-grammar-repeat");
    let pol = write_class_grammar_report(&f);
    for verb in [
        vec!["unverified", "--report", &f.prefix, "--policy", &pol, "--json"],
        vec!["blindspots", "--report", &f.prefix, "--json"],
    ] {
        let mut args = verb.clone();
        args.extend(["--class", "unresolved", "--class", "native"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let err = String::from_utf8(out.stderr).unwrap();
        assert_eq!(out.status.code(), Some(2), "a repeated --class must be a usage error, got:\n{err}");
        // BOTH tokens are individually VALID, so this cannot be the unrecognised-token path — and it must
        // not be the unknown-flag path either (which is what a `--class` that stopped consuming its value
        // would produce). Asserting the message is what makes the two rules distinguishable: an
        // exit-code-only test passes against a mutation that deleted this one.
        assert!(err.contains("more than once"), "the message must say the flag was repeated, got:\n{err}");
        assert!(err.contains("not a union"),
                "…and that the two lists are not unioned — last-wins and union are BOTH silent misreadings:\n{err}");
        assert!(!err.contains("unrecognised reason-class"),
                "`unresolved` and `native` are both valid tokens; this is the repeat rule, not the token rule:\n{err}");
        assert!(out.stdout.is_empty(), "a refused --class must emit no answer at all");
    }
}

#[test]
fn blindspots_class_grammar_keeps_its_selection() {
    // The regression control for the OTHER verb: `blindspots --class` is the SOURCE view (§6.2: direct-only
    // by definition there), so its counts differ from `unverified`'s and must be pinned separately.
    let f = Fixture::new("class-grammar-bs");
    write_class_grammar_report(&f);
    let names = |class: Option<&str>| -> (i32, Vec<String>) {
        let mut args: Vec<String> = vec!["blindspots".into(), "--report".into(), f.prefix.clone(), "--json".into()];
        if let Some(c) = class {
            args.push("--class".into());
            args.push(c.into());
        }
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let code = out.status.code().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap_or(serde_json::json!({}));
        let mut n: Vec<String> = v["sources"].as_array().cloned().unwrap_or_default().iter()
            .map(|s| s["fn"].as_str().unwrap_or_default().to_string()).collect();
        n.sort();
        (code, n)
    };
    assert_eq!(names(None), (0, vec!["domain::src_dispatch".to_string(), "domain::src_native".to_string()]),
               "both DIRECT sources; `domain::inherits` is inherited-only and is not a source");
    assert_eq!(names(Some("native")), (0, vec!["domain::src_native".to_string()]));
    assert_eq!(names(Some("dispatch,native")), (0, vec!["domain::src_dispatch".to_string(), "domain::src_native".to_string()]));
    assert_eq!(names(Some("dynamic")).1.len(), 2, "`dynamic` drops nothing here — no setup-class source");
    assert_eq!(names(Some("*")).1.len(), 2);
    assert_eq!(names(Some("reflect")), (0, vec![]), "a valid class with no source is an empty answer");
}

// ── ⟨0.24⟩ `gate --report <locator> --policy <file>` (SPEC §3.1) ──────────────────────────────────
//
// Driven through the SHIPPED binary, deliberately: this verb's contract is an EXIT CODE and a
// stdout/stderr split, and an in-process call can observe neither. (The reference engine found a defect
// its own unit test passed against, because a `static` had captured stdout at class load.)

/// A hand-written report under `<dir>/report.app.scan.json` — rust's §3.3.1 locator is the PREFIX
/// `<dir>/report`. Returns the locator. Deletes the directory first: a stale artefact is a flattering
/// datapoint, and every row below is a control for another row.
fn gate_fixture(dir: &std::path::Path, sub: &str, report: &str, callgraph: Option<&str>) -> String {
    let d = dir.join(sub);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("report.app.scan.json"), report).unwrap();
    if let Some(cg) = callgraph {
        std::fs::write(d.join("report.app.scan.callgraph.json"), cg).unwrap();
    }
    d.join("report").to_string_lossy().into_owned()
}

fn run_gate(locator: &str, policy: &std::path::Path, extra: &[&str]) -> (i32, String, String) {
    let mut args: Vec<String> = vec![
        "gate".into(),
        "--report".into(),
        locator.into(),
        "--policy".into(),
        policy.to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let out = Command::new(bin()).args(&args).output().expect("run candor-query");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn pol(dir: &std::path::Path, name: &str, text: &str) -> std::path::PathBuf {
    let p = dir.join(format!("{name}.policy"));
    std::fs::write(&p, text).unwrap();
    p
}

/// SPEC §3.1 ⟨0.24⟩'s MUST NOT: *an ABSENT entry is absent — the ⟨0.21⟩ purity claim — and MUST NOT be
/// back-filled from a callgraph sidecar or a chained dep.* All three back-fill channels are opened at
/// once (the `.callgraph.json` sidecar naming `app.hidden` and its effectful callee, a chained dep
/// report on `CANDOR_DEPS`, and a `.candor/config` `deps` key beside the report), and `deny Fs` must
/// still exit 0.
///
/// THE NEGATIVE CONTROL IS THE HALF THAT MAKES IT A TEST: the same policy over a report that DOES carry
/// the effect must exit 1. Without it an engine that ignored the policy entirely would pass — "did not
/// back-fill" and "never evaluated" are the same green.
#[test]
fn gate_report_does_not_backfill_an_absent_entry_and_the_control_fires() {
    let f = Fixture::new("gate-mustnot");
    let absent = r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
        "analyzed":{"count":3,"digest":"0"},
        "functions":[{"fn":"app.visible","inferred":["Net"],"direct":["Net"],
                      "hosts":["example.com"],"netClass":["unknown-host"]}]}"#;
    let present = r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
        "analyzed":{"count":3,"digest":"0"},
        "functions":[{"fn":"app.visible","inferred":["Net"],"direct":["Net"],
                      "hosts":["example.com"],"netClass":["unknown-host"]},
                     {"fn":"app.hidden","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#;
    let cg = r#"{"app.visible":[],"app.hidden":["dep.readCfg"],"dep.readCfg":[]}"#;
    let dep = f.dir.join("dep.json");
    std::fs::write(
        &dep,
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"dep",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"dep.readCfg","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#,
    )
    .unwrap();
    let deny_fs = pol(&f.dir, "denyfs", "deny Fs\n");

    let mut codes = Vec::new();
    for (sub, body) in [("absent", absent), ("present", present)] {
        let loc = gate_fixture(&f.dir, sub, body, Some(cg));
        // channel 3: the `.candor/config` `deps` key, in the one directory beside the report.
        let cfg = f.dir.join(sub).join(".candor");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config"), format!("deps = {}\n", dep.display())).unwrap();
        let out = Command::new(bin())
            .args(["gate", "--report", &loc, "--policy", &deny_fs.to_string_lossy()])
            .env("CANDOR_DEPS", &dep) // channel 2
            .output()
            .expect("run candor-query");
        codes.push((sub, out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned()));
    }
    assert_eq!(
        codes[0].1, 0,
        "an ABSENT entry was back-filled: `deny Fs` fired over a report carrying no Fs, with the \
         callgraph sidecar + CANDOR_DEPS + config `deps` all supplying it\n{}",
        codes[0].2
    );
    assert_eq!(
        codes[1].1, 1,
        "NEGATIVE CONTROL: the same policy over a report that DOES carry Fs must exit 1 — without it \
         this row cannot tell 'not back-filled' from 'policy never evaluated'\n{}",
        codes[1].2
    );
}

/// SPEC §3.1 ⟨0.24⟩ ANSWERABILITY, whole-policy arm: `forbid` and `allow` are REFUSED (exit 2), never
/// evaluated. `forbid` because a report's `calls` is EFFECT-RELEVANT, so a crossing into a wholly pure
/// unit is invisible while `forbid` matches on NAME; `allow` because the AS-EFF-008 surface-completeness
/// marker does not ride the wire as a gate-usable fact. Both are FAIL-OPEN if approximated — the
/// refusal is what stops a user believing a rule is enforced that never ran.
#[test]
fn gate_report_refuses_forbid_and_allow_whole_policy() {
    let f = Fixture::new("gate-refuse-policy");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["example.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    for (name, text, needle) in [
        ("forbid", "forbid app -> dep\n", "`forbid`"),
        ("allow", "allow Net example.com\n", "allow "),
        // A policy whose `deny` half does NOT fire is still refused whole: enforcing the answerable half
        // and exiting 0 is gateless-green — the user believes a rule is enforced that never ran. This
        // row used to read `deny Net` and expect 2; see the row below for why it had to change.
        ("mixed_no_hit", "deny Exec\nforbid app -> dep\n", "`forbid`"),
    ] {
        let (rc, stdout, err) = run_gate(&loc, &pol(&f.dir, name, text), &[]);
        assert_eq!(rc, 2, "{name} must be REFUSED (exit 2), never evaluated:\n{err}");
        assert!(err.contains(needle), "the refusal must name the offending rule kind, got:\n{err}");
        assert!(err.contains("scan time"), "…and carry the remedy (gate at scan time), got:\n{err}");
        assert!(stdout.is_empty(), "a refused policy must produce no verdict at all");
    }
    // THE CONTROL: the same report under a rule this verb CAN answer must fire, or the three rows above
    // prove only that the fixture is inert.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "bare", "deny Net\n"), &[]);
    assert_eq!(rc, 1, "the answerable control must FIRE, or the refusals prove nothing:\n{err}");

    // ⟨0.24⟩ **AND A CERTAIN VIOLATION DOMINATES THESE REFUSALS TOO** (candor-spec `1503368`, which
    // removes the carve-out). This assertion is the reason the `mixed` row above had to change: it read
    // `deny Net` + `forbid`, and `deny Net` FIRES on this fixture, so it was pinning exactly the
    // suppression the ruling removes. MEASURED before the fix: exit 2, with the certain `Net` violation
    // absent from the `--gate-json` document — byte-identical in harm to the per-(rule, function) case
    // `8b97e5c` fixed, surviving one branch higher because that was not where the measurement was taken.
    //
    // **Lemma 2 does not care which KIND of refusal stands beside the firing rule.** Whole-policy
    // granularity governs which rules go UNEVALUATED; it is not a licence to suppress one that was
    // evaluated and certain.
    for (name, text) in
        [("dom_forbid", "deny Net\nforbid app -> dep\n"), ("dom_allow", "deny Net\nallow Net other.example.com\n")]
    {
        let out = f.dir.join(format!("{name}.json"));
        let _ = std::fs::remove_file(&out);
        let (rc, _, err) =
            run_gate(&loc, &pol(&f.dir, name, text), &["--gate-json", &out.to_string_lossy()]);
        assert_eq!(rc, 1, "{name}: a firing `deny` dominates a whole-policy refusal:\n{err}");
        let doc = std::fs::read_to_string(&out)
            .unwrap_or_else(|e| panic!("{name}: the certain violation must reach the document ({e}):\n{err}"));
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        let fns: Vec<&str> =
            v["violations"].as_array().unwrap().iter().map(|x| x["fn"].as_str().unwrap()).collect();
        assert_eq!(fns, vec!["app.egress"], "{name}: the exit code is one bit, the document is the evidence:\n{doc}");
        // …and the refused KIND is still disclosed. Exit 1 reports what it is sure of; it does not
        // conceal the part it could not read.
        assert!(err.contains("scan time"), "{name}: the dominated refusal must still be named:\n{err}");
    }
}

/// SPEC §3.1 ⟨0.24⟩ ANSWERABILITY, per-(rule, function) arm — and the LIVE fail-open, not a theoretical
/// one: `deny Net[unknown-host]` over a `Net`-bearing entry with NO `netClass` matched the empty set and
/// returned exit 0, where the bare `deny Net` returns 1. An absent optional field silently un-scoping a
/// fail-closed security gate. The bare arms are asserted beside each scoped one — that is what makes the
/// scoped exit-2 a REFUSAL of a relaxation rather than a signature that simply does not violate.
#[test]
fn gate_report_refuses_a_scoped_deny_whose_scoping_datum_is_absent() {
    let f = Fixture::new("gate-refuse-scoped");
    let net = gate_fixture(
        &f.dir,
        "net",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],"hosts":["example.com"]}]}"#,
        None,
    );
    let unk = gate_fixture(
        &f.dir,
        "unk",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.murky","inferred":["Unknown"]}]}"#,
        None,
    );
    let cases = [
        (&net, "netscoped", "deny Net[unknown-host]\n", 2, "netbare", "deny Net\n"),
        (&unk, "unkscoped", "deny Unknown[dispatch]\n", 2, "unkbare", "deny Unknown\n"),
    ];
    for (loc, sname, stext, swant, bname, btext) in cases {
        let (rc, _, err) = run_gate(loc, &pol(&f.dir, sname, stext), &[]);
        assert_eq!(rc, swant, "the scoped rule must be REFUSED, not silently narrowed:\n{err}");
        assert!(err.contains("Refusing"), "the refusal must say so, got:\n{err}");
        let (brc, _, berr) = run_gate(loc, &pol(&f.dir, bname, btext), &[]);
        assert_eq!(brc, 1, "the BARE rule must fire — else the scoped exit 2 proves nothing:\n{berr}");
    }
    // A scoped rule whose evidence IS present evaluates normally: the refusal is per (rule, function),
    // not a blanket ban on scoped rules.
    let carried = gate_fixture(
        &f.dir,
        "carried",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["example.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    let (rc, _, err) = run_gate(&carried, &pol(&f.dir, "netscoped2", "deny Net[unknown-host]\n"), &[]);
    assert_eq!(rc, 1, "a scoped rule whose scoping datum is PRESENT must evaluate, not refuse:\n{err}");
    let (rc, _, err) = run_gate(&carried, &pol(&f.dir, "nettel", "deny Net[known-telemetry]\n"), &[]);
    assert_eq!(rc, 0, "…and tolerate when the class does not match:\n{err}");
}

/// SPEC §3.1 ⟨0.24⟩ **PRECEDENCE: A CERTAIN VIOLATION DOMINATES A REFUSAL** (candor-spec `7271c69`,
/// which CORRECTS the "refusal > violation" clause written an hour earlier). One policy carrying BOTH a
/// firing `deny Fs` and one unanswerable scoped rule exited 2 and wrote NO `--gate-json` document on
/// rust, java, ts and swift alike — four-way agreement, and four-way wrong. `Reject` is upward-closed
/// (PAPER3 Lemma 2), so however the unanswerable rule would have resolved cannot un-reject a policy a
/// firing rule has already rejected: exit 1 is CERTAIN there, and strictly more informative.
///
/// **THE ASSERTION IS DOCUMENT-SIDE, AND THAT IS THE WHOLE POINT.** The harm was never the exit code —
/// it was the certain violation being deleted from the machine-consumer channel, exactly as `ff34070`
/// measured one rung down for the incomplete case. A test that only checked `rc == 1` would pass on a
/// route that exits 1 and still writes `violations: []`.
#[test]
fn gate_report_reports_a_certain_violation_over_an_unanswerable_rule_beside_it() {
    let f = Fixture::new("gate-precedence");
    // ONE report: an Fs unit the firing rule catches, and a Net unit with NO `netClass` — the entry that
    // makes a `Net[unknown-host]` filter unanswerable.
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":2,"digest":"0"},
            "functions":[
              {"fn":"app.fsUnit","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/x"]},
              {"fn":"app.netNoClass","inferred":["Net"],"direct":["Net"],"hosts":["h.example.com"]}]}"#,
        None,
    );
    let out = f.dir.join("verdict.json");
    let _ = std::fs::remove_file(&out);
    let both = pol(&f.dir, "both", "deny Fs app.fsUnit\ndeny Net[unknown-host] app.netNoClass\n");
    let (rc, _, err) = run_gate(&loc, &both, &["--gate-json", &out.to_string_lossy()]);
    assert_eq!(rc, 1, "a rule that FIRES on evidence the report carries dominates the refusal:\n{err}");
    let doc = std::fs::read_to_string(&out)
        .unwrap_or_else(|e| panic!("a refusal must not delete the verdict document ({e}):\n{err}"));
    let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert_eq!(v["ok"], false);
    let fns: Vec<&str> =
        v["violations"].as_array().unwrap().iter().map(|x| x["fn"].as_str().unwrap()).collect();
    assert_eq!(
        fns,
        vec!["app.fsUnit"],
        "THE CERTAIN VIOLATION MUST BE IN THE DOCUMENT — an exit code is one bit, the document is the \
         evidence, and this is the channel a PR comment is built from:\n{doc}"
    );
    // …AND THE UNANSWERED RULE IS STILL DISCLOSED. Exit 1 reports the violation it is sure of; it does
    // not conceal the part it could not read (SPEC §3.1). Without this the fix would trade one silence
    // for another: the operator would read "1 violation" and never learn a second rule never ran.
    assert!(
        err.contains("deny Net[unknown-host] app.netNoClass"),
        "the dominated refusal must still name the rule it could not evaluate:\n{err}"
    );

    // CONTROL 1 — the refusal is REAL. Drop the firing rule and the same unanswerable rule refuses,
    // exit 2. Without this row the test cannot tell "violation dominates" from "the refusal was never
    // triggered by this fixture at all".
    let sole = pol(&f.dir, "sole", "deny Net[unknown-host] app.netNoClass\n");
    let (rc2, _, err2) = run_gate(&loc, &sole, &[]);
    assert_eq!(rc2, 2, "the unanswerable rule alone must still REFUSE:\n{err2}");

    // CONTROL 2 — the firing rule is REAL, and fires alone.
    let only_fs = pol(&f.dir, "onlyfs", "deny Fs app.fsUnit\n");
    let (rc3, _, err3) = run_gate(&loc, &only_fs, &[]);
    assert_eq!(rc3, 1, "the firing rule alone must exit 1:\n{err3}");
}

/// SPEC §6.2 ⟨0.24⟩ **EVERY VERB THAT REASONS ABOUT A POLICY MUST READ IT THE WAY THE GATE DOES** —
/// found in candor-java and confirmed live here. `whatif`, `fix`/`fix-gate` and `unverified` all called
/// bare `parse_policy`, which loads no `unknown-alias` vocabulary and reports no policy errors, while the
/// gate loads both.
///
/// MEASURED 2026-07-28 with `unknown-alias corp = reflect` beside the policy, `deny Unknown[corp]
/// app.nat`, and a NATIVE-caused hole: the gate exits 0, `whatif` answered "⚠ WOULD VIOLATE policy",
/// `fix-gate` named a hoist remedy, and `unverified` answered "PROVABLY clean ✓" — three over-reports
/// and, in the disclosure verb, a hole DELETED. These are the verbs an agent consults BEFORE editing.
///
/// THIS ROW PINS THE FIRST HALF: the vocabulary is loaded (so a malformed `unknown-alias` — a fact ONLY
/// visible to a verb that reads the config at all — refuses) and `ParsedPolicy::errors` travels.
///
/// ⟨0.24⟩ THE TWO RESIDUALS THIS COMMENT RECORDED ARE NOW CLOSED, in that order and not the other. The
/// FILTER-blind matching went first (`unverified`/`fix-gate` and the shared predicate under both —
/// `unverified_and_fix_gate_answer_the_narrowed_rule_the_gate_actually_applies`), because fixing the
/// PRINTED rule while the verdict stayed unfiltered would have attributed an unfiltered verdict to the
/// operator's own narrowed line: a worse sentence than the one it replaced. `whatif`'s rendering
/// followed (`whatif_names_the_operators_own_rule_and_discloses_what_a_narrowed_verdict_rests_on`).
#[test]
fn every_policy_reasoning_verb_refuses_what_the_gate_refuses() {
    let f = Fixture::new("verb-policy");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.nat","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["native:extern fn"]}]}"#,
        Some(r#"{"app.nat":[]}"#),
    );
    std::fs::create_dir_all(f.dir.join(".candor")).unwrap();
    let p = pol(&f.dir, "aliased", "deny Unknown[corp] app.nat\n");
    let ps = p.to_string_lossy().into_owned();
    let run = |verb: &str, extra: &[&str]| -> i32 {
        let mut args: Vec<String> = vec![verb.into()];
        args.extend(extra.iter().map(|s| s.to_string()));
        args.extend(["--report".into(), loc.clone(), "--policy".into(), ps.clone()]);
        Command::new(bin()).args(&args).output().expect("run candor-query").status.code().unwrap_or(-1)
    };
    let verbs: [(&str, &[&str]); 4] =
        [("gate", &[]), ("whatif", &["app.nat", "Unknown"]), ("fix-gate", &[]), ("unverified", &[])];

    // A WELL-FORMED alias: every verb must evaluate, none may refuse. Without this row the assertions
    // below are satisfied by a verb that refuses every aliased policy.
    std::fs::write(f.dir.join(".candor/config"), "unknown-alias corp = reflect\n").unwrap();
    for (verb, extra) in verbs {
        assert_ne!(run(verb, extra), 2, "`{verb}` must EVALUATE a policy whose alias resolves");
    }

    // A MALFORMED alias definition — a fact only a verb that reads the CONFIG can see at all, which is
    // what makes this row prove the vocabulary is loaded rather than merely that errors are checked.
    std::fs::write(f.dir.join(".candor/config"), "unknown-alias corp = dispatch,nativ\n").unwrap();
    for (verb, extra) in verbs {
        assert_eq!(
            run(verb, extra),
            2,
            "`{verb}` must refuse a policy the GATE refuses — answering from a rule the gate will not \
             apply is the worse failure in a verb consulted BEFORE the edit"
        );
    }

    // …and an unrecognised token with no alias in play, so the refusal is not merely alias-shaped.
    std::fs::write(f.dir.join(".candor/config"), "").unwrap();
    let bad = pol(&f.dir, "badtok", "deny Unknown[dispatch,nativ] app.nat\n");
    let bs = bad.to_string_lossy().into_owned();
    for (verb, extra) in verbs {
        let mut args: Vec<String> = vec![verb.into()];
        args.extend(extra.iter().map(|s| s.to_string()));
        args.extend(["--report".into(), loc.clone(), "--policy".into(), bs.clone()]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(2), "`{verb}`: an unrecognised token is a policy error");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("nativ"),
            "`{verb}`: …and the refusal must name the token"
        );
    }
}

/// ⟨0.24⟩ **THE DISCLOSURE MAY NOT CLAIM A FIRING THAT DID NOT HAPPEN, AND MAY NOT NAME AN EXIT IT DID
/// NOT TAKE.** Two false sentences, one cause: text written for the case it was found in, then reached
/// by a case the precedence correction had just created.
///
/// MEASURED 2026-07-28 with `deny Unknown[unresolved] app.murky` as the SOLE rule in the policy —
///
///   - the note read *"The verdict stands anyway: a rule FIRED on evidence this report carries"* when
///     **no rule had**: the only rule in the policy was the unanswerable one. The sentence was attached
///     unconditionally to the refusal path rather than conditioned on a violation being recorded;
///   - and the per-rule reason ended *"Refusing (exit 2)."* on a run that exits **1**, because the
///     disposition was baked into a string written when exit 2 was the only thing that could follow.
///
/// A FALSE DISCLOSURE IS WORSE THAN A MISSING ONE — conformance PART 13b is this family's precedent
/// (`net-partner` reported as an ignored config key WHILE BEING HONOURED). Both halves are asserted in
/// BOTH directions here: each case must say its own true thing AND must not say the other's.
#[test]
fn gate_report_claims_a_firing_rule_only_where_one_actually_fired() {
    let f = Fixture::new("gate-claim");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":2,"digest":"0"},
            "functions":[
              {"fn":"app.murky","inferred":["Unknown"]},
              {"fn":"app.writes","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#,
        Some(r#"{"app.murky":[],"app.writes":[]}"#),
    );
    const FIRED: &str = "FIRED on evidence this report carries";
    const REFUSING: &str = "Refusing (exit 2)";

    // CASE 1 — SOLE unanswerable rule. It refuses, so it may say so; and NOTHING fired, so it may not
    // claim a firing. The old build printed both sentences here and one of them was false.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "sole", "deny Unknown[unresolved] app.murky\n"), &[]);
    assert_eq!(rc, 2, "{err}");
    assert!(err.contains(REFUSING), "a refusal must name its own disposition:\n{err}");
    assert!(
        !err.contains(FIRED),
        "THE FALSE CLAIM: no rule fired on carried evidence — the only rule in this policy is the \
         unanswerable one:\n{err}"
    );

    // CASE 2 — a firing rule DOMINATES the same unanswerable one. Now the firing claim is true and must
    // be made (with the count, so it cannot be printed truthfully at zero), and the exit is 1, so
    // "Refusing (exit 2)" must NOT appear anywhere in the output.
    let (rc, _, err) = run_gate(
        &loc,
        &pol(&f.dir, "both", "deny Fs app.writes\ndeny Unknown[unresolved] app.murky\n"),
        &[],
    );
    assert_eq!(rc, 1, "{err}");
    assert!(
        err.contains("the 1 violation(s) reported below FIRED on evidence this report carries"),
        "the claim must be COUNTED from the verdict, not asserted beside it:\n{err}"
    );
    assert!(
        !err.contains(REFUSING),
        "THE OTHER FALSE CLAIM: this run exits 1, so no line of it may announce exit 2:\n{err}"
    );
    // …and the withheld rule is still named. Removing a false sentence must not remove the true one.
    assert!(err.contains("deny Unknown[unresolved] app.murky"), "{err}");

    // CASE 3 — a CLEAN policy says neither thing, which is what makes cases 1 and 2 discriminating
    // rather than merely wordy.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "clean", "deny Exec app\n"), &[]);
    assert_eq!(rc, 0, "{err}");
    assert!(!err.contains(FIRED) && !err.contains(REFUSING), "{err}");
}

/// SPEC §3.1 ⟨0.24⟩ **WITHHOLD PER (RULE, FUNCTION)** — candor-spec `5a8cf48`, the half of the
/// precedence ruling that makes it safe. Applying `8b97e5c` WITHOUT this FABRICATES.
///
/// **THE NEW-REACHABILITY DEFECT.** Once a firing rule stops short-circuiting the refusal, `gate()` runs
/// on inputs it had never been handed. `candor_classify::policy::reason_class_matches` floors an
/// absent/empty class set at `unresolved` — the correct fail-closed default for a MATCHER ("could this
/// rule apply?") and the WRONG basis for a FIRING ("did it?"). MEASURED 2026-07-28 on this exact
/// fixture: `deny Unknown[unresolved] app.opaque` ALONE printed a note ending *"Refusing (exit 2)"* and
/// then **exited 1 with a violation record for that rule and function in the `--gate-json` document**,
/// for an entry whose determinable class set is EMPTY. The record refuted itself — it carried no
/// `reasonClass` key at all, because the floor lives in the predicate and never in the data.
///
/// **THE MIRROR IS ROW D, AND IT IS NOT OPTIONAL.** Killing a fabrication is exactly where a silent
/// under-report gets introduced, and the fixture proving the fabrication is closed cannot show the reach
/// closed with it. So the same rule, over an entry whose `unresolved` is INHERITED through a `calls`
/// edge, must still fire — that class set is EVIDENCED (contributed at the entry, before the fixpoint),
/// and conformance `R1_EXPECT["unresolved"]` pins the same property.
#[test]
fn gate_report_withholds_an_unevidenced_scoped_rule_instead_of_fabricating_a_violation() {
    let f = Fixture::new("gate-withhold");
    // `app.opaque` — `Unknown` inferred, NO `direct`, NO `unknownWhy`, NO `calls` edge. Its class set is
    // determinable from the entry alone as EMPTY, which is the whole of the unanswerable condition.
    // `app.writes` beside it is the certain violation the precedence rung must still deliver.
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":2,"digest":"0"},
            "functions":[
              {"fn":"app.opaque","inferred":["Unknown"]},
              {"fn":"app.writes","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#,
        Some(r#"{"app.opaque":[],"app.writes":[]}"#),
    );
    let doc_of = |name: &str, text: &str| -> (i32, String, serde_json::Value) {
        let out = f.dir.join(format!("{name}.verdict.json"));
        // DELETE BEFORE MEASURING: a stale document from the previous row is exactly the flattering
        // artifact this suite exists to refuse.
        let _ = std::fs::remove_file(&out);
        let (rc, _, err) =
            run_gate(&loc, &pol(&f.dir, name, text), &["--gate-json", &out.to_string_lossy()]);
        let v = std::fs::read_to_string(&out)
            .ok()
            .and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok())
            .unwrap_or(serde_json::Value::Null);
        (rc, err, v)
    };

    // ROW A — THE FABRICATION. The unanswerable rule ALONE. It must refuse, and the document must carry
    // NO violation for `app.opaque`: the assertion is document-side because that is the channel the
    // fabricated record reached.
    let (rc, err, v) = doc_of("scoped", "deny Unknown[unresolved] app.opaque\n");
    assert_eq!(rc, 2, "a rule with no evidence to fire on must be WITHHELD, not charged:\n{err}");
    assert_eq!(v["refused"], true, "…and a sole withholding is the refusal posture:\n{v}");
    assert!(
        v["violations"].as_array().map(|a| a.is_empty()).unwrap_or(true),
        "A VIOLATION HERE IS A FABRICATION — `app.opaque`'s determinable class set is empty, so this \
         record asserts a reason nobody recorded:\n{v}"
    );

    // ROW B — THE LIVE-FIXTURE CONTROL. The BARE rule asks a question the effect set alone answers, so
    // it fires. Without this row, row A passes on a fixture that simply does not violate.
    let (rc, err, v) = doc_of("bare", "deny Unknown app.opaque\n");
    assert_eq!(rc, 1, "the bare rule must FIRE — else row A proves nothing:\n{err}");
    assert_eq!(v["violations"][0]["fn"], "app.opaque", "{v}");

    // ROW C — BOTH PROPERTIES AT ONCE, which is the requirement `5a8cf48` adds to `8b97e5c`: the certain
    // violation still reaches the document (restoring the short-circuit would re-break that), AND the
    // unevidenced one is not charged beside it. Withholding is per (rule, function), never whole-policy.
    let (rc, err, v) = doc_of("both", "deny Fs app.writes\ndeny Unknown[unresolved] app.opaque\n");
    assert_eq!(rc, 1, "the certain violation dominates the withholding:\n{err}");
    let fns: Vec<&str> =
        v["violations"].as_array().unwrap().iter().map(|x| x["fn"].as_str().unwrap()).collect();
    assert_eq!(
        fns,
        vec!["app.writes"],
        "EXACTLY the evidenced violation, and exactly one: `app.writes` must be present (the `8b97e5c` \
         property) and `app.opaque` absent (the `5a8cf48` property):\n{v}"
    );
    assert!(
        err.contains("deny Unknown[unresolved] app.opaque"),
        "…and the WITHHELD rule is still disclosed — withholding it silently is the mirror defect:\n{err}"
    );

    // ROW D — THE MIRROR. An `unresolved` that is INHERITED is EVIDENCED, and the identical rule must
    // still fire on it. `app.src` raises `Unknown` DIRECTLY and names no reason, so it CONTRIBUTES
    // `unresolved` at the entry; `app.inherits` reaches it through `calls` and accumulates the same
    // class over the gate's own reach (SPEC §6.2). Both must be charged, and both must carry the
    // `reasonClass` that proves the charge was evidenced.
    let mloc = gate_fixture(
        &f.dir,
        "m",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":3,"digest":"0"},
            "functions":[
              {"fn":"app.inherits","inferred":["Unknown"],"calls":["app.src"]},
              {"fn":"app.src","inferred":["Unknown"],"direct":["Unknown"]},
              {"fn":"app.opaque","inferred":["Unknown"]}]}"#,
        Some(r#"{"app.inherits":["app.src"],"app.src":[],"app.opaque":[]}"#),
    );
    let out = f.dir.join("mirror.verdict.json");
    let _ = std::fs::remove_file(&out);
    let (rc, _, err) = run_gate(
        &mloc,
        &pol(&f.dir, "mirror", "deny Unknown[unresolved] app\n"),
        &["--gate-json", &out.to_string_lossy()],
    );
    assert_eq!(rc, 1, "AN EVIDENCED `unresolved` MUST STILL FIRE — this is the under-report mirror:\n{err}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let hits: Vec<&str> =
        v["violations"].as_array().unwrap().iter().map(|x| x["fn"].as_str().unwrap()).collect();
    assert_eq!(
        hits,
        vec!["app.inherits", "app.src"],
        "the direct reasonless Unknown AND the caller inheriting it, and ONLY those:\n{v}"
    );
    for gv in v["violations"].as_array().unwrap() {
        assert_eq!(
            gv["reasonClass"][0], "unresolved",
            "a charged Unknown must carry the class that evidenced the charge — the fabricated record \
             carried none, and that absence is the tell:\n{gv}"
        );
    }
}

/// SPEC §3.1 ⟨0.24⟩ **A REFUSAL MUST STILL WRITE A `--gate-json` DOCUMENT** (candor-spec `107755b`).
///
/// **THE HAZARD, AND WHY THE STALE FILE IS SEEDED HERE.** A refusal wrote nothing at the requested path,
/// so the canonical CI wrapper — `candor-query gate … --gate-json v.json; jq .ok v.json` — re-read **the
/// previous run's document as current**. A green file from yesterday's clean run, still on disk, is how
/// a refusal becomes an all-clear. The fixture writes exactly that green file first, because a test that
/// starts from an ABSENT path can only show a file was created; it cannot show the reader was rescued
/// from the value that was actually there. Deleting the path is not the fix either — a consumer that
/// reads a missing file as "nothing to report" fails open by a different route.
///
/// **`violations` MUST BE ABSENT, NOT EMPTY.** The gate is making no claim about violations, and `[]` is
/// precisely the claim it cannot make: every consumer in existence reads it as "we looked and found
/// none". That assertion is the one that separates this fix from the shape a hurried version would take.
#[test]
fn a_refusal_overwrites_a_stale_verdict_and_makes_no_claim_about_violations() {
    let f = Fixture::new("gate-refusal-doc");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.netNoClass","inferred":["Net"],"direct":["Net"],
                          "hosts":["h.example.com"]}]}"#,
        None,
    );
    // Each of the three §3.1 ANSWERABILITY refusals, all of which used to leave the path untouched.
    for (name, text) in [
        ("scoped", "deny Net[unknown-host] app.netNoClass\n"),
        ("forbidr", "forbid app -> dep\n"),
        ("allowr", "allow Net example.com\n"),
    ] {
        let v = f.dir.join(format!("v-{name}.json"));
        // YESTERDAY'S GREEN, on disk, exactly as a CI wrapper would find it.
        std::fs::write(&v, "{\n  \"spec\": \"0.24\",\n  \"ok\": true,\n  \"violations\": []\n}\n").unwrap();
        let (rc, _, err) = run_gate(&loc, &pol(&f.dir, name, text), &["--gate-json", &v.to_string_lossy()]);
        assert_eq!(rc, 2, "{name} must still refuse:\n{err}");
        let doc = std::fs::read_to_string(&v).expect("the path still exists");
        let d: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(
            d["ok"], false,
            "{name}: a consumer keying ONLY on `ok` must land on FAIL — it read yesterday's `true`:\n{doc}"
        );
        assert_eq!(d["refused"], true, "{name}: …and one keying on `refused` learns why:\n{doc}");
        assert!(
            d.get("violations").is_none(),
            "{name}: `violations` must be ABSENT, not empty — an empty array is exactly the claim a \
             refusal cannot make, and every consumer reads it as 'we looked and found none':\n{doc}"
        );
        assert!(
            d["reason"].as_str().is_some_and(|s| !s.is_empty()),
            "{name}: the document must carry the refusal reason:\n{doc}"
        );
    }
    // CONTROL — the same machinery on a policy this verb CAN answer still writes a real verdict, with a
    // `violations` key. Without it, "always write a refusal document" would pass by refusing everything.
    let v = f.dir.join("v-ok.json");
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "bare", "deny Net\n"), &["--gate-json", &v.to_string_lossy()]);
    assert_eq!(rc, 1, "the answerable control must FIRE:\n{err}");
    let d: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v).unwrap()).unwrap();
    assert!(d.get("refused").is_none(), "a real verdict is not a refusal: {d:#}");
    assert_eq!(d["violations"].as_array().unwrap().len(), 1, "{d:#}");
}

/// SPEC §3.1 ⟨0.24⟩ **THE AMBIENT VOCABULARY MUST BE NAMED ON THE VERDICT** (candor-spec `99eb4e9`).
///
/// A `.candor/config` `unknown-alias` beside the policy moves this verdict 0→1, and discovery WALKS
/// PARENT DIRECTORIES, so the file that moved it can sit several levels above the one the operator was
/// looking at. That is the fourth channel §3.1's MUST NOT never named. The remedy is the one used
/// everywhere else here — not to forbid the input (an alias IS policy vocabulary), but to make it
/// impossible for it to act unnamed.
#[test]
fn the_config_that_supplied_the_vocabulary_is_named_on_the_verdict() {
    let f = Fixture::new("gate-vocab");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.viaFfi","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["native:libc::open"]}]}"#,
        None,
    );
    // The policy lives in its OWN directory, with its vocabulary beside it — and the config is written
    // one level ABOVE the policy, so the row also pins that a PARENT-directory config is disclosed.
    let home = f.dir.join("polhome");
    std::fs::create_dir_all(home.join("rules")).unwrap();
    std::fs::create_dir_all(home.join(".candor")).unwrap();
    let cfgpath = home.join(".candor/config");
    std::fs::write(&cfgpath, "unknown-alias corp = native\n").unwrap();
    let p = pol(&home.join("rules"), "org", "deny Unknown[corp]\n");

    let v = f.dir.join("verdict.json");
    let (rc, _, err) = run_gate(&loc, &p, &["--gate-json", &v.to_string_lossy()]);
    assert_eq!(rc, 1, "the alias resolves from a config ABOVE the policy:\n{err}");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v).unwrap()).unwrap();
    let named = j["policyVocabulary"]["config"].as_str().map(|s| std::fs::canonicalize(s).ok()).unwrap_or(None);
    assert_eq!(
        named,
        std::fs::canonicalize(&cfgpath).ok(),
        "a verdict changed by a file the operator cannot see NAMED is ambient input:\n{j:#}"
    );
    // ⟨0.24⟩ AN OBJECT, name → the classes it EXPANDED TO (SPEC §3.1 `7f5b5ba`) — see the
    // `…_names_what_the_alias_expanded_to…` row below for why the bare-name array was ruled out.
    assert_eq!(
        j["policyVocabulary"]["aliases"],
        serde_json::json!({"corp": ["native"]}),
        "the alias's DEFINITION is what moved the verdict, so it travels with the name:\n{j:#}"
    );
    // …under the name §3.1 ⟨0.24⟩ pins (`b4e9155`), on the REPORT route too, and with the old key gone.
    assert!(j.get("vocabulary").is_none(), "the pre-`b4e9155` key must not survive beside it:\n{j:#}");

    // THE DISCRIMINATION CONTROL: the same alias pointing somewhere else goes GREEN, so the row above
    // shows the alias STEERING the verdict rather than merely being present.
    std::fs::write(&cfgpath, "unknown-alias corp = reflect\n").unwrap();
    let v2 = f.dir.join("verdict2.json");
    let (rc2, _, err2) = run_gate(&loc, &p, &["--gate-json", &v2.to_string_lossy()]);
    assert_eq!(rc2, 0, "the alias narrows the rule:\n{err2}");

    // AND AN UNUSED ALIAS IS NOT NAMED — a verdict with no ambient vocabulary stays byte-identical to a
    // pre-⟨0.24⟩ one, and naming a file that changed nothing trains the reader to ignore the field.
    let bare = pol(&home.join("rules"), "bare", "deny Unknown\n");
    let v3 = f.dir.join("verdict3.json");
    let (rc3, _, _) = run_gate(&loc, &bare, &["--gate-json", &v3.to_string_lossy()]);
    assert_eq!(rc3, 1);
    let j3: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&v3).unwrap()).unwrap();
    assert!(j3.get("policyVocabulary").is_none(), "an alias the policy never mentions is not disclosed:\n{j3:#}");
}

/// SPEC §3.1 ⟨0.24⟩ **`policyVocabulary.aliases` IS AN OBJECT — EACH ALIAS MAPS TO THE CLASSES IT
/// EXPANDS TO** (candor-spec `7f5b5ba`).
///
/// **THE MEASUREMENT THIS ROW IS.** Two `.candor/config` files, ONE unchanged policy line, and two
/// DIFFERENT verdicts — 1 violation against 2. Under the bare-name array this engine shipped, both
/// verdicts disclosed the identical `["corp"]`, so a reader handed either document could not tell which
/// gate had run. That is the same failure §3.1 already rejects `configSources: [path]` for, one level
/// down: *a disclosure that names the source but not the content leaves the reader knowing they were
/// affected and not how*. candor-ts kept the object and argued it from that sentence; three engines
/// including this one moved.
///
/// The assertion is on the DIFFERENCE between the two documents, not on either alone. An engine that
/// emits a constant object would satisfy a single-document shape check and leave the divergence exactly
/// where it was — the same trap the `vocabulary`/`policyVocabulary` row above guards with its
/// absent-key assert.
///
/// **AND THE MIRROR**, because the object is a strict SUPERSET and must lose nothing: the `config` path
/// is still named on both, and the alias NAME is still recoverable from both (the keys ARE the old
/// array). A "fix" that swapped the names out for the classes would pass a naive difference check while
/// deleting the half the previous rung added.
#[test]
fn the_vocabulary_disclosure_names_what_the_alias_expanded_to_not_merely_that_one_was_used() {
    let f = Fixture::new("gate-vocab-value");
    // Two holes of DIFFERENT reason classes, so one alias definition can cover one and the wider
    // definition can cover both — the verdict moves without the policy moving.
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":2,"digest":"0"},
            "functions":[{"fn":"app.viaFfi","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["native:libc::open"]},
                         {"fn":"app.viaRefl","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["reflect:Any::downcast"]}]}"#,
        None,
    );
    let home = f.dir.join("polhome");
    std::fs::create_dir_all(home.join("rules")).unwrap();
    std::fs::create_dir_all(home.join(".candor")).unwrap();
    let cfgpath = home.join(".candor/config");
    // ONE policy line, unchanged across both runs. Everything that moves below is the config's.
    let p = pol(&home.join("rules"), "org", "deny Unknown[corp]\n");

    let vocab_of = |cfg: &str, out: &str| -> (i32, serde_json::Value) {
        std::fs::write(&cfgpath, cfg).unwrap();
        let v = f.dir.join(out);
        let (rc, _, err) = run_gate(&loc, &p, &["--gate-json", &v.to_string_lossy()]);
        let j: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&v).unwrap_or_else(|e| panic!("no verdict at {out} ({e}):\n{err}")),
        )
        .unwrap();
        (rc, j)
    };

    let (rc_narrow, narrow) = vocab_of("unknown-alias corp = reflect\n", "narrow.json");
    let (rc_wide, wide) = vocab_of("unknown-alias corp = reflect,native\n", "wide.json");

    // THE TWO GATES ARE DIFFERENT — the premise the disclosure has to carry.
    assert_eq!(rc_narrow, 1, "the narrow definition catches the reflect hole:\n{narrow:#}");
    assert_eq!(rc_wide, 1, "the wide definition catches both:\n{wide:#}");
    assert_eq!(narrow["violations"].as_array().unwrap().len(), 1, "{narrow:#}");
    assert_eq!(wide["violations"].as_array().unwrap().len(), 2, "{wide:#}");

    // …AND THE DISCLOSURE SAYS SO. Under the array form both of these were `["corp"]`.
    assert_ne!(
        narrow["policyVocabulary"]["aliases"], wide["policyVocabulary"]["aliases"],
        "two configs that gate DIFFERENTLY under one unchanged policy line must not produce the same \
         vocabulary disclosure — that is the whole of `7f5b5ba`:\nnarrow {narrow:#}\nwide {wide:#}"
    );
    assert_eq!(
        narrow["policyVocabulary"]["aliases"],
        serde_json::json!({"corp": ["reflect"]}),
        "{narrow:#}"
    );
    assert_eq!(
        wide["policyVocabulary"]["aliases"],
        serde_json::json!({"corp": ["native", "reflect"]}),
        "the classes are sorted, so the document is deterministic across runs:\n{wide:#}"
    );

    // THE MIRROR — the object is a SUPERSET, so neither half of the previous rung may go missing.
    for (label, j) in [("narrow", &narrow), ("wide", &wide)] {
        let named =
            j["policyVocabulary"]["config"].as_str().map(|s| std::fs::canonicalize(s).ok()).unwrap_or(None);
        assert_eq!(
            named,
            std::fs::canonicalize(&cfgpath).ok(),
            "{label}: the config path must survive the shape change:\n{j:#}"
        );
        let keys: Vec<&String> =
            j["policyVocabulary"]["aliases"].as_object().expect("an OBJECT").keys().collect();
        assert_eq!(keys, vec!["corp"], "{label}: the keys ARE the old array — the names are not lost:\n{j:#}");
    }
}

/// SPEC §6.2 ⟨0.24⟩ **AN UNRECOGNISED REASON-CLASS TOKEN IN A POLICY IS A POLICY ERROR** (candor-spec
/// `382a7e0`, which withdraws its own "a dropped policy token can only WIDEN, so the failure is loud"
/// asymmetry). Measured four-way, dropping the token does both, and one direction is fail-open:
///
///   - `deny Unknown[corp]` — the ONLY token is unrecognised, the filter empties, and the rule WIDENS to
///     a bare `deny Unknown` while the engine prints "ignoring policy rule" and then keeps and re-scopes
///     it. A FALSE DISCLOSURE, but at least loud in the strict direction.
///   - `deny Unknown[dispatch,nativ]` — **a typo BESIDE valid tokens.** Dropped, the rule NARROWS to
///     `[dispatch]`, and it stops gating native-caused holes entirely while the operator reads a gate
///     that looks armed. **That is the fail-open, and it is the common case: a typo lands beside correct
///     tokens far more often than alone.**
///
/// THE FIXTURE'S ONLY HOLE IS NATIVE-CAUSED, which is what makes the narrowing row a measurement rather
/// than a taxonomy: before the fix that row exited **0**, a green gate over exactly the hole the
/// operator wrote the rule to catch.
#[test]
fn gate_report_refuses_an_unrecognised_reason_class_token_including_beside_valid_ones() {
    let f = Fixture::new("gate-badclass");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.viaFfi","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["native:libc::open"]}]}"#,
        None,
    );
    // THE CONTROL FIRST, because every row below is only meaningful if the fixture is live: spelled
    // correctly, the rule FIRES.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "good", "deny Unknown[dispatch,native]\n"), &[]);
    assert_eq!(rc, 1, "the correctly-spelled rule must FIRE, or the rows below prove nothing:\n{err}");

    for (name, text, token) in [
        // THE FAIL-OPEN ROW. Before the fix: exit 0 — narrowed to `[dispatch]`, so the native hole the
        // rule was written for went ungated, silently.
        ("typo_beside_valid", "deny Unknown[dispatch,nativ]\n", "nativ"),
        // The widening row: exit 1 before the fix, but on a rule the engine claimed to be IGNORING.
        ("sole_unrecognised", "deny Unknown[corp]\n", "corp"),
    ] {
        let (rc, stdout, err) = run_gate(&loc, &pol(&f.dir, name, text), &[]);
        assert_eq!(
            rc, 2,
            "{name}: a policy that cannot be honoured AS WRITTEN must be refused, never silently \
             rewritten into a different policy:\n{err}"
        );
        assert!(err.contains(token), "{name}: the refusal must NAME the token:\n{err}");
        assert!(
            err.contains("unresolved") && err.contains("dispatch"),
            "{name}: …and list the ACCEPTED set, which is the only thing that makes it fixable:\n{err}"
        );
        assert!(stdout.is_empty(), "{name}: a refused policy produces no verdict");
    }
    // A CONFIG-DEFINED alias is still vocabulary, not an error — the refusal must not swallow the
    // ⟨0.19⟩ `unknown-alias` feature it looks exactly like from the parser's seat.
    let sub = f.dir.join("aliased");
    std::fs::create_dir_all(sub.join(".candor")).unwrap();
    std::fs::write(sub.join(".candor/config"), "unknown-alias corp = native\n").unwrap();
    let p = pol(&sub, "alias", "deny Unknown[corp]\n");
    let (rc, _, err) = run_gate(&loc, &p, &[]);
    assert_eq!(rc, 1, "an alias DEFINED beside the policy resolves and the rule fires:\n{err}");

    // ⟨0.24⟩ **THE RULE BINDS THE ALIAS DEFINITION TOO** (candor-spec `be0b9a9`) — and this is the
    // sharper of the two siblings, because the typo is in the VOCABULARY the policy is written against
    // rather than in the policy, and it fails open identically. MEASURED: `= dispatch,nativ` silently
    // became `{dispatch}` and this exact fixture — whose only hole is NATIVE-caused — exited **0**,
    // where `= dispatch,native` exits 1. Every disclosure fired correctly about a definition that was
    // not the one on disk.
    //
    // The definition is refused WHOLE rather than narrowed, so `corp` is undefined and the policy naming
    // it lands on the token-error path above. Narrowing it to the tokens that happened to parse is the
    // same silent rewrite one level down.
    std::fs::write(sub.join(".candor/config"), "unknown-alias corp = dispatch,nativ\n").unwrap();
    let (rc, stdout, err) = run_gate(&loc, &p, &[]);
    assert_eq!(rc, 2, "a typo in the ALIAS DEFINITION must refuse, not narrow the definition:\n{err}");
    assert!(err.contains("nativ"), "the refusal must name the token in the CONFIG:\n{err}");
    assert!(stdout.is_empty());
    // …AND THE BLAST RADIUS IS USE, NOT PRESENCE. A typo'd alias NO POLICY MENTIONS changed nothing, so
    // it must not turn an unrelated gate red — the mirror over-reach. Same config, a policy that never
    // names `corp`.
    std::fs::write(sub.join(".candor/config"), "unknown-alias unused = dispatch,nativ\n").unwrap();
    let bare_p = pol(&sub, "barealias", "deny Unknown\n");
    let (rc, _, err) = run_gate(&loc, &bare_p, &[]);
    assert_eq!(rc, 1, "a typo'd alias the policy never mentions must not refuse the gate:\n{err}");
}

/// SPEC §6.2 ⟨0.24⟩ **THE SAME RULE ON THE NET DESTINATION-CLASS LIST** (candor-spec `be0b9a9`, which
/// widens `382a7e0` from "reason-class token" to *any* policy value list: *"each place I let it stay
/// narrow is a place the same fail-open survives under a different key"*).
///
/// MEASURED 2026-07-28: `deny Net[known-telemetry,unknown-hosst]` → **exit 0**, where the correctly
/// spelled rule exits 1. Byte-identical in shape to the reason-class typo and byte-identical in harm —
/// the token is dropped, the filter NARROWS to `[known-telemetry]`, and the gate stops covering
/// unidentifiable destinations while the operator reads a gate that looks armed.
#[test]
fn gate_report_refuses_an_unrecognised_net_destination_class_token() {
    let f = Fixture::new("gate-badnetclass");
    // The fixture's only Net destination is UNKNOWN-HOST — the class the typo'd token was meant to name,
    // so the fail-open row is a measurement and not a taxonomy.
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["h.example.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    // CONTROL FIRST: spelled correctly, the rule fires.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "good", "deny Net[known-telemetry,unknown-host]\n"), &[]);
    assert_eq!(rc, 1, "the correctly-spelled rule must FIRE, or the rows below prove nothing:\n{err}");

    for (name, text, token) in [
        // THE FAIL-OPEN ROW: exit 0 before the fix.
        ("typo_beside_valid", "deny Net[known-telemetry,unknown-hosst]\n", "unknown-hosst"),
        // The sole-token row: the filter empties and the rule WIDENS to a bare `deny Net`, which exits 1
        // — but on a rule the engine claimed to be ignoring. A false disclosure, the mirror direction.
        ("sole_unrecognised", "deny Net[nope]\n", "nope"),
    ] {
        let (rc, stdout, err) = run_gate(&loc, &pol(&f.dir, name, text), &[]);
        assert_eq!(rc, 2, "{name}: a policy that cannot be honoured AS WRITTEN must be refused:\n{err}");
        assert!(err.contains(token), "{name}: the refusal must NAME the token:\n{err}");
        assert!(
            err.contains("unknown-host") && err.contains("known-partner"),
            "{name}: …and list the ACCEPTED set, which is the only thing that makes it fixable:\n{err}"
        );
        assert!(stdout.is_empty(), "{name}: a refused policy produces no verdict");
    }
    // `Net[*]` is not an unrecognised token — the wildcard must survive the new strictness.
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "star", "deny Net[*]\n"), &[]);
    assert_eq!(rc, 1, "`Net[*]` means every destination and must still evaluate:\n{err}");
}

/// SPEC §3.1 ⟨0.24⟩ THE MINIMAL-REFUSAL RULE. A class-scoped `deny` is NOT unanswerable merely because
/// evidence is missing: the class set only GROWS (§6.2 CONTRIBUTES) and `Reject` is upward-closed, so
/// when the classes determinable FROM THE ENTRY ALONE are non-empty the answer is certain either way.
/// The ⟨0.24⟩ CONTRIBUTES counterexample — a DIRECT `Unknown` naming no reason — contributes
/// `unresolved` from the entry with no transitive step, so `deny Unknown[unresolved]` FIRES and must
/// not be refused. (candor-swift's original refusal here is recorded in SPEC as over-broad.)
#[test]
fn gate_report_answers_the_contributes_counterexample_rather_than_refusing_it() {
    let f = Fixture::new("gate-contributes");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":3,"digest":"0"},
            "functions":[
              {"fn":"app.reasonless","inferred":["Unknown"],"direct":["Unknown"]},
              {"fn":"app.reasoned","inferred":["Unknown"],"direct":["Unknown"],
               "unknownWhy":["dispatch:app::Trait"]},
              {"fn":"app.both","inferred":["Unknown"],"calls":["app.reasonless","app.reasoned"]}]}"#,
        None,
    );
    let fired = |name: &str, text: &str| -> (i32, Vec<String>) {
        let p = pol(&f.dir, name, text);
        let out = f.dir.join(format!("{name}.verdict.json"));
        let _ = std::fs::remove_file(&out);
        let (rc, _, err) = run_gate(&loc, &p, &["--gate-json", &out.to_string_lossy()]);
        assert!(rc != 2, "`{text}` must be ANSWERED, not refused (exit 2):\n{err}");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).expect("a verdict")).unwrap();
        let mut fns: Vec<String> = v["violations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["fn"].as_str().unwrap().to_string())
            .collect();
        fns.sort();
        (rc, fns)
    };
    let (rc, fns) = fired("unres", "deny Unknown[unresolved]\n");
    assert_eq!(rc, 1);
    assert_eq!(
        fns,
        vec!["app.both".to_string(), "app.reasonless".to_string()],
        "the reasonless DIRECT Unknown contributes `unresolved` AT THE ENTRY, so it composes: the caller \
         of BOTH a reasonless and a reasoned dep is caught too — the §6.2 counterexample in which adding \
         a call turned a red verdict green"
    );
    // THE DISCRIMINATION CONTROL. Without it this row cannot tell the rule from "contribute
    // `unresolved` unconditionally": `app.reasoned` named its own class and must stay OUT.
    let (_, fns) = fired("disp", "deny Unknown[dispatch]\n");
    assert_eq!(
        fns,
        vec!["app.both".to_string(), "app.reasoned".to_string()],
        "a named direct Unknown keeps ONLY its own class — the naive unconditional contribution would \
         put `app.reasonless` here too"
    );
}

/// `--json` IS `--gate-json -` (SPEC §3.1 ⟨0.24⟩): on a scan `--json <file>` writes the REPORT and there
/// is none here, so a second meaning would be the one place a consumer could tell the two routes apart.
/// stdout therefore stays a single pure JSON document — the violation prose goes to stderr, the class of
/// defect that corrupted the reference engine's stream.
#[test]
fn gate_report_json_is_gate_json_dash_and_stdout_stays_parseable() {
    let f = Fixture::new("gate-json");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":2,"digest":"0"},
            "coverage":{"uncovered":[{"name":"zeta","calls":2},{"name":"alpha","calls":1}]},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["example.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    let p = pol(&f.dir, "denynet", "deny Net\n");
    let file = f.dir.join("verdict.json");
    let (rc_json, stdout, err) = run_gate(&loc, &p, &["--json"]);
    assert_eq!(rc_json, 1);
    let streamed: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or_else(|e| panic!("stdout must be pure JSON ({e}): {stdout}\n{err}"));
    assert!(err.contains("AS-EFF-006"), "the prose belongs on stderr:\n{err}");
    let (rc_file, stdout2, _) = run_gate(&loc, &p, &["--gate-json", &file.to_string_lossy()]);
    assert_eq!(rc_file, 1);
    let written: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(streamed, written, "`--json` and `--gate-json <file>` must be the same document");
    assert!(stdout2.contains("AS-EFF-006"), "without a stdout JSON document the prose goes to stdout");
    // The ⟨0.21⟩ manifest and the ⟨0.15⟩ κ ledger travel ON the report, so the verdict carries them —
    // this is half of why the document can be byte-equal to `candor-scan --policy`'s.
    assert_eq!(streamed["analyzed"]["count"], 2);
    assert_eq!(streamed["coverage"]["uncovered"], 2);
    assert_eq!(streamed["coverage"]["packages"], serde_json::json!(["alpha", "zeta"]));
    assert_eq!(streamed["spec"], candor_report_spec());
}

fn candor_report_spec() -> &'static str {
    // The verdict declares the spec the BINARY implements; read it from the same constant the report
    // envelope is stamped from so this assertion can never pin a stale string.
    candor_report::SPEC_VERSION
}

/// A ⟨0.21⟩ INCOMPLETE report cannot yield a green gate: the manifest travelled with it, so the same
/// verdict follows from it that the producing scan reached — exit 2, `ok:false`, `incomplete:true`.
#[test]
fn gate_report_will_not_certify_over_a_report_that_declares_itself_incomplete() {
    let f = Fixture::new("gate-incomplete");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "unanalyzed":[{"path":"src/bad.rs","reason":"source failed to parse"}],
            "functions":[{"fn":"app.pure_enough","inferred":[]}]}"#,
        None,
    );
    let file = f.dir.join("verdict.json");
    let (rc, _, err) = run_gate(&loc, &pol(&f.dir, "denyfs", "deny Fs\n"), &["--gate-json", &file.to_string_lossy()]);
    assert_eq!(rc, 2, "a gate cannot be green over code candor never analyzed:\n{err}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["incomplete"], true);
    assert_eq!(v["unanalyzed"][0]["path"], "src/bad.rs");
    // The report above carries no forbidden effect, so `violations` is empty for the RIGHT reason. The
    // test below is the row where it must NOT be empty — the one this test could never have caught.
}

/// SPEC §3.3: *"A configured gate over incompletely-analyzed code MUST fail closed (exit ≠ 0); a real
/// violation (exit 1) still dominates."* The test above pins the first half; this one pins the second,
/// and the second is the half that was broken.
///
/// MEASURED BEFORE THE FIX (2026-07-28) on a report carrying two `Net` units AND a one-entry
/// `unanalyzed`: rust exited 2 and wrote `{ok:false, incomplete:true, violations: []}` — the manifest
/// branch ran first and called `write_verdict(&mut [], …)` with an EMPTY list. ts, java and swift all
/// exited 1 with both violations present. The AS-EFF-006 lines WERE printed to stderr, so a human saw
/// them; a CI consumer reading gate.json saw a fail-closed verdict with nothing in it, and the finding
/// never reached the PR.
///
/// THE ASSERTION IS ON THE VIOLATIONS BEING PRESENT, not on the exit code. The exit code was wrong too,
/// but exit 2 is still fail-closed — the violation COUNT is what regressed, and it is what a consumer
/// acts on.
#[test]
fn an_incomplete_report_does_not_swallow_the_violations_it_also_carries() {
    let f = Fixture::new("gate-incomplete-viol");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":9,"digest":"0"},
            "unanalyzed":[{"path":"src/bad.rs","reason":"source failed to parse"}],
            "functions":[{"fn":"app.netOne","inferred":["Net"],"direct":["Net"],"hosts":["a.example.com"]},
                         {"fn":"app.netTwo","inferred":["Net"],"direct":["Net"],"hosts":["b.example.com"]},
                         {"fn":"app.pure_enough","inferred":[]}]}"#,
        None,
    );
    let file = f.dir.join("verdict.json");
    let (rc, _, err) =
        run_gate(&loc, &pol(&f.dir, "denynet", "deny Net\n"), &["--gate-json", &file.to_string_lossy()]);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let fns: Vec<&str> =
        v["violations"].as_array().unwrap().iter().filter_map(|x| x["fn"].as_str()).collect();
    assert_eq!(
        fns,
        vec!["app.netOne", "app.netTwo"],
        "an incomplete manifest must not delete the findings from the verdict a CI consumer reads \
         (SPEC §3.3 — a real violation dominates):\n{v:#}\nstderr:\n{err}"
    );
    // …carried on the SAME document as the incompleteness, never instead of it.
    assert_eq!(v["ok"], false, "{v:#}");
    assert_eq!(v["incomplete"], true, "the manifest still rides the verdict:\n{v:#}");
    assert_eq!(v["unanalyzed"][0]["path"], "src/bad.rs", "{v:#}");
    assert_eq!(rc, 1, "a real violation dominates the incomplete exit 2:\n{err}");
}

/// ⟨0.24⟩ SPEC §3.1: A REPORT HANDED DIRECTLY TO THE GATE WITH `analyzed.count: 0` MAKES THE SAME CLAIM
/// AS A CHAINED ONE — it judged nothing — and must be read the same way. *"The obligation is on the
/// reading, not on the route the report arrived by."* **AS A DISCLOSURE, NOT AS AN EXIT CODE:** *"The
/// exit code and the verdict document are UNCHANGED."*
///
/// THIS TEST PINNED THE OPPOSITE UNTIL 2026-07-28, and the clause it was written from has been corrected
/// (candor-spec `0744d29`). The clause said the verb "MUST say so rather than reporting 'no violations,
/// exit 0'", which forbade exit 0 — and that contradicted §3.1's OWN byte-equality MUST, because
/// `candor-scan` over a facade package exits 0 with a clean verdict and this route must match it.
/// MEASURED on a real crate this engine's own scan judges as count-0: `candor-scan . --policy P
/// --gate-json a` exited 0 and wrote `{ok:true, analyzed:{count:0}, violations:[]}`; `gate --report er
/// --policy P --gate-json b` exited 2 and wrote NOTHING — the strongest available failure of the
/// byte-equality MUST, on a report the scan itself had just produced, on a measured 7–10% of real
/// dependency reports. Refusing also minted a THIRD exit-2 cause where §3.3 enumerates two.
///
/// SO THE ASSERTION MOVED TO THE DISCLOSURE. Deleting the refusal without checking the note is the
/// silent-regression shape this whole verb exists to prevent — the harm the corrected clause names is
/// the DELETED DISCLOSURE, not the verdict.
///
/// THE ALL-PURE ROW IS STILL THE CONTROL and the arms still differ in ONE INTEGER: `count: 2` with the
/// same empty `functions` is a legitimate all-pure package which §2 rule 3 requires the verb to BELIEVE —
/// exit 0, clean verdict, and NO note. A predicate keyed on `functions` being empty passes the floor row
/// and fails here, and over 1997 JVM dependency jars it would have hedged 104 real claims to catch 6.
#[test]
fn gate_report_discloses_a_report_that_judged_nothing_without_moving_the_verdict() {
    let f = Fixture::new("gate-judged-nothing");
    let p = pol(&f.dir, "denyfs", "deny Fs\n");
    let body = |count: &str| format!(
        r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",
            "analyzed":{{"count":{count},"digest":"0"}},
            "functions":[]}}"#);

    // THE FLOOR: judged nothing → the verb SAYS SO, names the package, and leaves everything else alone.
    let zero = gate_fixture(&f.dir, "zero", &body("0"), None);
    let vfile = f.dir.join("verdict-zero.json");
    let (rc, _, err) = run_gate(&zero, &p, &["--gate-json", &vfile.to_string_lossy()]);
    assert!(err.contains("JUDGED NOTHING") && err.contains("analyzed.count"),
            "the verb MUST say the report judged nothing — that disclosure IS the obligation, and \
             deleting the old refusal without it is the defect wearing the fix's clothes:\n{err}");
    assert!(err.contains("`app`"),
            "…and must NAME THE PACKAGE, or an adopter chaining twenty reports cannot act on it:\n{err}");
    assert_eq!(rc, 0,
               "the exit code is UNCHANGED (⟨0.24⟩): §3.3 has exactly two exit-2 causes and a \
                judged-nothing dependency is neither, so refusing here splits the verb:\n{err}");
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&vfile).expect("the verdict document is UNCHANGED too — §3.1 byte-equality \
                                                 with `scan --policy`, which writes one and exits 0")).unwrap();
    assert_eq!(v["ok"], true, "{v:#}");
    assert_eq!(v["analyzed"]["count"], 0, "the count-0 manifest rides the verdict verbatim: {v:#}");
    assert_eq!(v["violations"].as_array().unwrap().len(), 0, "{v:#}");

    // THE CONTROL: judged TWO units, found neither effectful. Believed, clean, exit 0, and NO note —
    // the note must be keyed on the integer, never on `functions` being empty.
    let allpure = gate_fixture(&f.dir, "allpure", &body("2"), None);
    let vfile = f.dir.join("verdict-allpure.json");
    let (rc, _, err) = run_gate(&allpure, &p, &["--gate-json", &vfile.to_string_lossy()]);
    assert_eq!(rc, 0, "an all-pure package's report is a CLAIM (§2 rule 3) and must still gate clean:\n{err}");
    assert!(!err.contains("JUDGED NOTHING"), "…with no ⟨0.24⟩ hedge anywhere:\n{err}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&vfile).unwrap()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["violations"].as_array().unwrap().len(), 0);
    assert!(v.get("incomplete").is_none(), "a believed all-pure verdict carries no new hedge: {v}");

    // SPEC §2's THIRD ROW on this route too: no manifest and no entries is a pre-⟨0.21⟩ producer that
    // gives the verb nothing to distinguish "judged nothing" from "judged and found nothing" — so it
    // gets the same NOTE, and the same untouched verdict.
    let legacy = gate_fixture(&f.dir, "legacy",
        r#"{"candor":{"version":"handwritten","spec":"0.20"},"package":"app","functions":[]}"#, None);
    let (rc, _, err) = run_gate(&legacy, &p, &[]);
    assert_eq!(rc, 0, "a manifest-less EMPTY report is disclosed, not refused (⟨0.24⟩ corrected):\n{err}");
    assert!(err.contains("JUDGED NOTHING"), "…but it IS disclosed:\n{err}");
    // …and the same producer WITH an entry judged something, the only way it could say so. No note.
    let legacy_full = gate_fixture(&f.dir, "legacyfull",
        r#"{"candor":{"version":"handwritten","spec":"0.20"},"package":"app",
            "functions":[{"fn":"app.pure_enough","inferred":[]}]}"#, None);
    let (rc, _, err) = run_gate(&legacy_full, &p, &[]);
    assert_eq!(rc, 0,
               "a pre-⟨0.21⟩ report that LISTS entries judged something — refusing it would withdraw \
                every manifest-less report from the verb, which is the emptiness fix wearing a \
                different hat:\n{err}");
    assert!(!err.contains("JUDGED NOTHING"), "…and it earns no hedge:\n{err}");

    // A JUDGED-NOTHING REPORT STILL GATES. The note is advisory, so the rules must still run over
    // whatever the report DOES carry — a count-0 report that nevertheless lists an effectful entry is
    // contradictory input, and the finding in it must not be swallowed by the hedge.
    let contra = gate_fixture(&f.dir, "contra",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":0,"digest":"0"},
            "functions":[{"fn":"app.reads","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/x"]}]}"#, None);
    let (rc, _, err) = run_gate(&contra, &p, &[]);
    assert_eq!(rc, 1, "the gate still evaluates what the report DOES carry:\n{err}");
    assert!(err.contains("JUDGED NOTHING"), "…and still discloses the contradiction:\n{err}");
}

/// ⟨0.24⟩ SPEC §2: *"A KEY THAT IS PRESENT BUT UNPARSEABLE IS CORRUPT INPUT, AND MUST NEVER BE COERCED
/// TO ITS EMPTY VALUE … Absent may take a documented default. Present-but-unparseable is a refusal —
/// exit 2, naming the key."*
///
/// MEASURED BEFORE THE FIX (2026-07-28), all four rows through the shipped binary:
///
///   unanalyzed: [{"unit":…,"why":…}]   exit 0 `policy ✓`   ← ts/java/swift all exited 2
///   unanalyzed: ["src/broken.rs"]      exit 0 `policy ✓`   ← all four dropped this one
///   unanalyzed: []                     exit 0              (correct — an explicit completeness claim)
///   unanalyzed absent                  exit 0              (correct — a complete scan omits the key)
///
/// `report_unanalyzed` ended in `from_value(u).ok().unwrap_or_default()`, so both corrupt shapes became
/// `[]`. **`unanalyzed` NON-EMPTINESS IS THE FAIL-CLOSED TRIGGER**, so that default does not lose a
/// hedge — it inverts the verdict, and always in the green direction.
///
/// BOTH HALVES ARE PINNED HERE ON PURPOSE. A test carrying only the corrupt rows is satisfied by an
/// engine that refuses every report without an `unanalyzed` key — which is every complete report this
/// engine writes — so the ABSENT and EMPTY rows are what keep the refusal from becoming its own defect.
#[test]
fn a_present_but_unparseable_section2_key_refuses_and_an_absent_one_does_not() {
    let f = Fixture::new("gate-corrupt-key");
    let p = pol(&f.dir, "denynet", "deny Net\n");
    // A report with NO Net anywhere: every exit code below is decided by the KEY, never by a violation.
    let body = |extra: &str| format!(
        r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",
            "analyzed":{{"count":3,"digest":"0"}}{extra},
            "functions":[{{"fn":"app.pure_enough","inferred":[]}}]}}"#);

    // THE FAIL-OPEN ROWS — corrupt `unanalyzed`, which must refuse and NAME THE KEY.
    for (name, extra) in [
        ("wrongfields", r#","unanalyzed":[{"unit":"src/broken.rs","why":"parse error"}]"#),
        ("barestrings", r#","unanalyzed":["src/broken.rs"]"#),
        ("notalist", r#","unanalyzed":"src/broken.rs""#),
    ] {
        let loc = gate_fixture(&f.dir, name, &body(extra), None);
        let vfile = f.dir.join(format!("v-{name}.json"));
        let _ = std::fs::remove_file(&vfile);
        let (rc, _out, err) = run_gate(&loc, &p, &["--gate-json", &vfile.to_string_lossy()]);
        assert_eq!(rc, 2, "a present-but-unparseable `unanalyzed` must refuse, not read as `[]`:\n{err}");
        assert!(err.contains("`unanalyzed`"), "the refusal must NAME the key it could not read:\n{err}");
        // ⟨0.24⟩ …AND WRITE THE FAIL-CLOSED DOCUMENT. This row read `assert!(!vfile.exists())` — the
        // §3.3 "no document on a config-shaped exit 2" rule — until candor-spec `1503368` (b) removed
        // that carve-out. The argument that MANDATES a document (a CI wrapper reading the path
        // unconditionally re-reads the PREVIOUS run's verdict as current) is exactly as true here: a
        // stale green does not care why this run declined to overwrite it. A report that did not load
        // has no violations to reason about, which is precisely why the document carries no
        // `violations` key — the shape already says "no claim about violations".
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&vfile).expect("a refusal document")).unwrap();
        assert_eq!(v["ok"], false, "the naive read of this document must be the fail-closed one:\n{v:#}");
        assert_eq!(v["refused"], true, "{v:#}");
        assert!(v.get("violations").is_none(), "a refusal makes NO claim about violations:\n{v:#}");
    }
    // …and the same rule on `analyzed`, including SPEC §2's live boolean row.
    for (name, extra) in [
        ("boolcount", r#""analyzed":{"count":true},"#),
        ("strmanifest", r#""analyzed":"lots","#),
    ] {
        let rep = format!(
            r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",{extra}
                "functions":[{{"fn":"app.pure_enough","inferred":[]}}]}}"#);
        let loc = gate_fixture(&f.dir, name, &rep, None);
        let (rc, _, err) = run_gate(&loc, &p, &[]);
        assert_eq!(rc, 2, "a manifest that cannot be read is not a manifest:\n{err}");
        assert!(err.contains("`analyzed`"), "the refusal must NAME the key:\n{err}");
    }

    // THE CONTROLS — without these the fix above is satisfied by refusing everything. An ABSENT key
    // takes its documented default, and so does an explicitly EMPTY one.
    for (name, extra) in [("absent", ""), ("emptylist", r#","unanalyzed":[]"#)] {
        let loc = gate_fixture(&f.dir, name, &body(extra), None);
        let vfile = f.dir.join(format!("v-{name}.json"));
        let (rc, _, err) = run_gate(&loc, &p, &["--gate-json", &vfile.to_string_lossy()]);
        assert_eq!(rc, 0, "an {name} `unanalyzed` is the documented default, never a refusal:\n{err}");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&vfile).unwrap()).unwrap();
        assert_eq!(v["ok"], true, "{v:#}");
        assert!(v.get("incomplete").is_none(), "…and no incompleteness is invented either: {v:#}");
    }
    // A digest-less `analyzed` is READABLE — `count` is the load-bearing datum, and the count must reach
    // the verdict rather than being silently zeroed (the old `.ok()` reader contributed 0 for this shape).
    let loc = gate_fixture(&f.dir, "nodigest",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app","analyzed":{"count":5},
            "functions":[{"fn":"app.pure_enough","inferred":[]}]}"#, None);
    let vfile = f.dir.join("v-nodigest.json");
    let (rc, _, err) = run_gate(&loc, &p, &["--gate-json", &vfile.to_string_lossy()]);
    assert_eq!(rc, 0, "a digest-less manifest is legible, not corrupt:\n{err}");
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&vfile).unwrap()).unwrap();
    assert_eq!(v["analyzed"]["count"], 5, "the judged count must ride the verdict, not be zeroed: {v:#}");
}

/// §3.3.1 grammar: `gate` is a QUERY verb with NO positionals, and a missing/unreadable policy is a LOUD
/// exit 2. A swallowed token is how a gate runs green over a DISCOVERED report the user never named.
#[test]
fn gate_report_grammar_is_loud() {
    let f = Fixture::new("gate-grammar");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["e.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    let p = pol(&f.dir, "denynet", "deny Net\n");
    let ps = p.to_string_lossy().into_owned();
    let missing = f.dir.join("nope.policy").to_string_lossy().into_owned();
    for (args, needle) in [
        (vec!["gate", "--report", &loc, "--policy", &ps, "stray"], "unexpected argument"),
        (vec!["gate", "--report", &loc, "--policy", &ps, "--nope"], "unknown flag"),
        (vec!["gate", "--report", &loc], "a policy is required"),
        (vec!["gate", "--report", &loc, "--policy", &missing], "could not be read"),
        (vec!["gate", "--report", &loc, "--policy"], "--policy requires"),
        (vec!["gate", "--policy", &ps, "--report"], "--report requires"),
        // `--gate-json --policy p` must not swallow the next flag and run gateless-green.
        (vec!["gate", "--report", &loc, "--gate-json", "--policy", &ps], "--gate-json requires"),
    ] {
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(2), "{args:?} must be a loud usage error:\n{err}");
        assert!(err.contains(needle), "{args:?} must say `{needle}`, got:\n{err}");
    }
    // A locator that matches no report FAILS LOUD — never a silently-empty "no violations".
    let nowhere = f.dir.join("nothing-here").to_string_lossy().into_owned();
    let (rc, _, err) = run_gate(&nowhere, &p, &[]);
    assert_eq!(rc, 2, "an empty locator must not read as a clean gate:\n{err}");
    // …and so does a report that is FOUND but corrupt (§3.1's found-but-corrupt rule).
    let bad = gate_fixture(&f.dir, "corrupt", "{ not json at all", None);
    let (rc, _, err) = run_gate(&bad, &p, &[]);
    assert_eq!(rc, 2, "a corrupt report is corrupt input, not an effect-free package:\n{err}");
}

#[test]
fn gate_flag_shaped_policy_value_is_refused_and_the_swallowed_sink_still_gets_the_document() {
    // Conformance §3.1 (b13)'s gate-verb sibling, SPEC §3.2 ⟨0.28⟩ "given no value". The loop used to
    // consume `--gate-json` as the policy FILENAME, so the verdict sink the operator named was never a
    // sink — measured on this verb as exit 2 with NOTHING on the stream ("unexpected argument `-`"),
    // where the fail-closed refusal document belongs. A flag-shaped token after a value-taking flag is
    // a usage error at exit 2, and the sinks named elsewhere in that argv are STILL SINKS: the run has
    // a broken command line, not a redefined one. BOTH halves are asserted — the exit-code half alone
    // passes against the broken behaviour, which also exited 2.
    let f = Fixture::new("gate-b13");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.egress","inferred":["Net"],"direct":["Net"],
                          "hosts":["e.com"],"netClass":["unknown-host"]}]}"#,
        None,
    );
    // The conformance rows run env-scrubbed; a CANDOR_POLICY in the harness environment must not turn
    // this into a different run (the policy ladder would resolve it and gate for real).
    let run = |args: &[&str]| -> (Option<i32>, String, String) {
        let out = Command::new(bin())
            .args(args)
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
            .output().expect("run candor-query");
        (out.status.code(),
         String::from_utf8_lossy(&out.stdout).into_owned(),
         String::from_utf8_lossy(&out.stderr).into_owned())
    };
    // The STREAM spelling: `--policy --gate-json -` — the refusal document belongs on stdout.
    let (rc, stdout, stderr) = run(&["gate", "--report", &loc, "--policy", "--gate-json", "-"]);
    assert_eq!(rc, Some(2), "a flag-shaped --policy value is a usage error:\n{stderr}");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
        panic!("the `--gate-json -` stream sink must carry the refusal document \
                (it was swallowed as the policy filename), got stdout:\n{stdout}")
    });
    assert_eq!(doc["ok"], false, "fail-closed to a naive reader: {doc}");
    assert_eq!(doc["refused"], true, "a refusal, not a verdict: {doc}");
    assert!(stderr.contains("--policy") && stderr.contains("--gate-json"),
            "stderr names the flag given no value AND the token that is not one: {stderr}");
    // The FILE spelling of the same sink: a previous run's green must not survive as current — the
    // refusal (not merely the armed placeholder) replaces it.
    let g = f.dir.join("verdict.json");
    std::fs::write(&g, "{\"ok\": true}\n").unwrap();
    let gs = g.to_string_lossy().into_owned();
    let (rc, _, stderr) = run(&["gate", "--report", &loc, "--policy", "--gate-json", &gs]);
    assert_eq!(rc, Some(2), "{stderr}");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&g).expect("sink written")).expect("valid JSON");
    assert_eq!(doc["ok"], false, "the stale green was replaced by the refusal: {doc}");
    assert_eq!(doc["refused"], true, "{doc}");
    assert!(doc["reason"].as_str().unwrap_or("").contains("--policy"),
            "the document carries the usage cause, not the armed placeholder: {doc}");
    // The boundaries: a bare `-` stays a legitimate VALUE (`--gate-json -` is the stream form), and a
    // normal `--policy <file>` is unaffected — the same argv with the mistake repaired gates for real.
    let p = pol(&f.dir, "denyfs", "deny Fs\n");
    let (rc, stdout, stderr) =
        run(&["gate", "--report", &loc, "--policy", &p.to_string_lossy(), "--gate-json", "-"]);
    assert_eq!(rc, Some(0), "deny Fs over a Net-only report passes:\n{stderr}");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim()).expect("the stream verdict");
    assert_eq!(doc["ok"], true, "a real verdict, not a refusal: {doc}");
    assert!(doc.get("refused").is_none(), "{doc}");
}

/// SPEC §3.2 ⟨0.28⟩ on the QUERY grammar (the `gate` loop's sibling — the route the b13 conformance row
/// never drives): `--report`/`--policy`/`--class` used to consume a flag-shaped next token as the value,
/// so `--report --json` ran against *locator = the file named `--json`* — exit 2 with the WRONG cause
/// ("no report files at prefix `--json`"), the "given no value" diagnostic unreachable, and the `--json`
/// the operator asked for silently gone. Usage error at exit 2, naming both tokens. These verbs write no
/// `--gate-json` sink, so there is no document half here — the boundary rows are the second half instead.
#[test]
fn query_verbs_refuse_a_flag_shaped_value_for_every_value_taking_flag() {
    let f = Fixture::new("grammar-b13");
    f.write_report();
    let rp = f.report_path();
    for (args, flag) in [
        (vec!["where", "Fs", "--report", "--json"], "--report"),
        (vec!["whatif", "outer", "Fs", "--report", rp.as_str(), "--policy", "--json"], "--policy"),
        (vec!["blindspots", "--report", rp.as_str(), "--class", "--json"], "--class"),
    ] {
        let out = Command::new(bin())
            .args(&args)
            .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
            .output().expect("run candor-query");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(out.status.code(), Some(2), "{args:?} must be a usage error:\n{stderr}");
        assert!(stderr.contains("given no value") && stderr.contains(flag) && stderr.contains("--json"),
                "{args:?} must name {flag} AND the flag-shaped token, got:\n{stderr}");
    }
    // The boundary: the same commands with the value supplied still answer. `--json` reaches the output
    // mode instead of being eaten as a locator.
    let out = Command::new(bin())
        .args(["where", "Fs", "--report", &f.report_path(), "--json"])
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "a value-shaped --report is unaffected");
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("--json reached the output mode");
    assert!(v.is_object() || v.is_array(), "{v}");
}

/// ⟨0.24⟩ **`unverified` AND `fix-gate` MUST READ THE NARROWING FILTER THE GATE READS** — SPEC §6.2,
/// the report-route half of `a_narrowed_rule_the_gate_tolerates_is_a_hole_and_the_one_it_fires_on_is_not`.
///
/// These are the verbs an agent consults BEFORE editing, and they answered from a coarser rule than the
/// one the gate applies: "does this rule NAME an effect this function has?", computed from `r.effects`
/// alone — the pre-⟨0.19⟩ question, still being asked after two rungs gave rules a filter. The two
/// symptoms run opposite ways and BOTH are measured here, on one report and one pair of policies:
///
///   - `unverified` LOST A DISCLOSURE. A hole is a function that PASSES its rule while `Unknown`, so a
///     rule the gate tolerates was read as violated, the real hole was reclassified as a
///     violation-that-isn't, and the verb answered "every function in a pure/deny layer is PROVABLY
///     clean ✓" over a function the gate had just declined to clear;
///   - `fix-gate` GAINED ONE. It named a hoist remedy for a crossing the gate does not report — a
///     boundary refactor proposed on the strength of a rule the operator narrowed to exclude it.
///
/// EVERY ROW HAS ITS MIRROR IN THE SAME RUN, because killing an over-charge is exactly where a silent
/// under-report gets introduced. `[reflect]` does not name this entry's `indirect` hole (gate exit 0);
/// `[indirect]` does (gate exit 1). The gate's own exit code is asserted on each, so the two verbs are
/// compared against the gate rather than against my expectation of it.
#[test]
fn unverified_and_fix_gate_answer_the_narrowed_rule_the_gate_actually_applies() {
    let f = Fixture::new("filter-aware");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.port","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["callback:injected port"]}]}"#,
        None,
    );
    let run = |verb: &str, p: &std::path::Path| -> (i32, String) {
        let out = Command::new(bin())
            .args([verb, "--report", &loc, "--policy", &p.to_string_lossy(), "--json"])
            .output()
            .expect("run candor-query");
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let json = |s: &str| -> serde_json::Value { serde_json::from_str(s).unwrap() };

    // ── ARM 1: the filter does NOT match (`indirect` ∉ {reflect}) — the gate TOLERATES. ──
    let miss = pol(&f.dir, "miss", "deny Unknown[reflect] app\n");
    let (rc, _, err) = run_gate(&loc, &miss, &[]);
    assert_eq!(rc, 0, "the gate tolerates a class filter this entry does not match:\n{err}");

    let (urc, uout) = run("unverified", &miss);
    let u = json(&uout);
    assert_eq!(urc, 0, "advisory");
    assert_eq!(
        u["unverified"].as_array().map(Vec::len),
        Some(1),
        "the gate DECLINED to clear `app.port` under this rule, so its purity is asserted and not \
         verified — that is the disclosure's whole subject, and it was empty:\n{uout}"
    );
    assert_eq!(u["unverified"][0]["fn"], "app.port");
    assert_eq!(
        u["unverified"][0]["rule"], "deny Unknown[reflect] app",
        "…and the rule is named WITH its filter — printing the operator's narrowed rule back as the \
         wide one is the mis-attribution this fix must not manufacture:\n{uout}"
    );
    assert_eq!(u["unverified"][0]["upgrade"], "deny Unknown app", "widen the filter, not append a second Unknown");

    let (frc, fout) = run("fix-gate", &miss);
    assert_eq!(frc, 0);
    assert_eq!(
        json(&fout)["remedies"].as_array().map(Vec::len),
        Some(0),
        "the gate reports no crossing here, so there is no boundary to hoist across:\n{fout}"
    );

    // ── ARM 2, THE MIRROR: the same rule spelled to MATCH. The gate FIRES, so `app.port` is a
    // VIOLATION — `unverified` must go silent (it is not an unproven pass) and `fix-gate` must speak
    // (there is a real crossing). Without this arm, arm 1 is satisfied by a verb that discloses
    // everything and a `fix-gate` that discloses nothing. ──
    let hit = pol(&f.dir, "hit", "deny Unknown[indirect] app\n");
    let (rc2, _, err2) = run_gate(&loc, &hit, &[]);
    assert_eq!(rc2, 1, "`callback:` classifies as `indirect`, so this filter FIRES:\n{err2}");

    let (_, uout2) = run("unverified", &hit);
    assert_eq!(
        json(&uout2)["unverified"].as_array().map(Vec::len),
        Some(0),
        "a function the gate CHARGED is a violation, not an unverified pass:\n{uout2}"
    );
    let (_, fout2) = run("fix-gate", &hit);
    assert_eq!(
        json(&fout2)["remedies"].as_array().map(Vec::len),
        Some(1),
        "…and the crossing the gate DOES report still gets its remedy — the filter-aware fix must not \
         turn `fix-gate` silent on the violations it exists for:\n{fout2}"
    );

    // ── ARM 3: NO FILTER AT ALL. Both verbs unchanged, which is what keeps conformance PARTs 12b/12c/12d
    // (four-way) from moving — the fix is confined to the rules the ⟨0.19⟩/⟨0.20⟩ rungs added. ──
    let bare = pol(&f.dir, "bare", "deny Unknown app\n");
    let (rc3, _, _) = run_gate(&loc, &bare, &[]);
    assert_eq!(rc3, 1);
    let (_, uout3) = run("unverified", &bare);
    assert_eq!(json(&uout3)["unverified"].as_array().map(Vec::len), Some(0), "a bare deny Unknown fires on every hole");
    let pure = pol(&f.dir, "pure", "pure app\n");
    let (_, uout4) = run("unverified", &pure);
    let u4 = json(&uout4);
    assert_eq!(u4["unverified"].as_array().map(Vec::len), Some(1));
    assert_eq!(u4["unverified"][0]["upgrade"], "deny Unknown app", "PART 12c's four-way form, unmoved");
}

/// ⟨0.24⟩ **`whatif` MUST NAME THE OPERATOR'S OWN RULE, AND SAY WHAT A NARROWED VERDICT RESTS ON** —
/// SPEC §6.2 for the first half, §3.1's *an unanswerable condition is DISCLOSED, never scored as a failed
/// one* for the second.
///
/// `whatif` REBUILT the rule it printed from `effects` + `scope` — and from the effect being ASKED
/// ABOUT rather than the rule's own set. MEASURED 2026-07-28:
///
///   `deny Unknown[reflect] app.nat`  printed back as  `deny Unknown app.nat`
///   `deny Net[unknown-host] app`     printed back as  `deny Net app`
///   `deny Net Db  app`               printed back as  `deny Net app`
///
/// The first two are the sharp ones: a NARROWED rule shown as the WIDE one, in the verb an agent reads
/// before editing, so the operator's own scoping is invisible at exactly the moment they are deciding
/// whether it protects them.
///
/// **AND THE ORDER MATTERED.** Printing `raw` while the verdict stayed FILTER-BLIND would have been
/// worse than the bug: the same unconditional "WOULD VIOLATE", now attributed to the narrowed line,
/// reading as though candor had evaluated a filter it did not. That is why the shared-predicate fix
/// landed first. It does not carry over to this verb, and the measurement says why: `unverified` and
/// `fix-gate` read a signature that EXISTS, while `whatif` asks about an effect not written yet, which
/// has no destination or reason class to match. The question is genuinely unanswerable, so the verdict
/// stays fail-closed (a pre-edit gate must not guess which class the edit lands in) and the CONDITION
/// rides beside it — the third arm below, which is the half that keeps `raw` from lying.
#[test]
fn whatif_names_the_operators_own_rule_and_discloses_what_a_narrowed_verdict_rests_on() {
    let f = Fixture::new("whatif-raw");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"app.nat","inferred":["Unknown"],"direct":["Unknown"],
                          "unknownWhy":["native:extern fn"]}]}"#,
        Some(r#"{"app.nat":[]}"#),
    );
    let whatif = |text: &str, effect: &str| -> serde_json::Value {
        let p = pol(&f.dir, "w", text);
        let out = Command::new(bin())
            .args(["whatif", "app.nat", effect, "--report", &loc, "--policy", &p.to_string_lossy(), "--json"])
            .output()
            .expect("run candor-query");
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap()
    };

    // ── ARM 1: the rule is named VERBATIM, comment stripped and ends trimmed — including the effects
    // the operator wrote that the question did not ask about. ──
    let v = whatif("deny Net Db  app     # keep the app layer pure\n", "Net");
    assert_eq!(v["ok"], false);
    assert_eq!(
        v["violations"][0]["rule"], "deny Net Db  app",
        "the operator's own line, not one rebuilt from the effect they happened to ask about:\n{v:#}"
    );
    let v = whatif("pure app\n", "Net");
    assert_eq!(v["violations"][0]["rule"], "pure app", "a `pure` rule reads back as itself:\n{v:#}");

    // ── ARM 2: a NARROWED rule keeps its bracket. This is the sharp one — the operator's scoping was
    // being erased in the verb they consult before editing. ──
    let v = whatif("deny Unknown[reflect] app.nat\n", "Unknown");
    assert_eq!(v["violations"][0]["rule"], "deny Unknown[reflect] app.nat", "{v:#}");
    let v2 = whatif("deny Net[unknown-host] app\n", "Net");
    assert_eq!(v2["violations"][0]["rule"], "deny Net[unknown-host] app", "{v2:#}");

    // ── ARM 3, THE HALF THAT KEEPS ARM 2 HONEST: the verdict on a narrowed rule is CONDITIONAL, and
    // says so. Without this, `raw` alone would attribute an unfiltered verdict to a filtered rule. ──
    assert_eq!(
        v["violations"][0]["conditional"], "the `Unknown` you introduce is of reason class reflect",
        "a rule that NARROWS cannot be evaluated against an effect that does not exist yet — §3.1: \
         disclose the unanswerable condition, never score it as a failed one:\n{v:#}"
    );
    assert_eq!(
        v2["violations"][0]["conditional"], "the `Net` you introduce reaches destination class unknown-host",
        "{v2:#}"
    );

    // ── THE MIRROR: an UNFILTERED rule has NO condition to disclose, so the key is absent and the
    // document is byte-identical to a pre-⟨0.24⟩ one. A `conditional` on every violation would train the
    // reader to ignore it, which is the same failure as naming a config that changed nothing. ──
    let plain = whatif("deny Unknown app.nat\n", "Unknown");
    assert_eq!(plain["violations"][0]["rule"], "deny Unknown app.nat");
    assert!(
        plain["violations"][0].get("conditional").is_none(),
        "a rule that does not narrow rests on no condition:\n{plain:#}"
    );
    let bare_net = whatif("deny Net app\n", "Net");
    assert!(bare_net["violations"][0].get("conditional").is_none(), "{bare_net:#}");
    // …and the filter must key on the effect being INTRODUCED, not merely on the rule carrying a
    // bracket: `deny Net[unknown-host] Fs app` asked about `Fs` charges `Fs` unconditionally.
    let other = whatif("deny Net[unknown-host] Fs app\n", "Fs");
    assert_eq!(other["violations"][0]["rule"], "deny Net[unknown-host] Fs app");
    assert!(
        other["violations"][0].get("conditional").is_none(),
        "the `Net` filter says nothing about an introduced `Fs`:\n{other:#}"
    );
}

/// ⟨0.24⟩ `unevaluated` IS IN THE DOCUMENT, ONE ENTRY PER RULE, RAW LINE VERBATIM — SPEC §3.1 `fc4b5f6`.
///
/// The exit-1 MUST *"disclose which rules could not be evaluated"* named no field, no shape and no
/// channel, and this engine put the disclosure on **stderr only**. Measured on `deny Fs` + `allow Fs
/// /var/data`, exit 1: java and ts carried `unevaluated` in the `--gate-json` document; rust carried
/// nothing there, so a machine consumer could not see that any rule had gone unanswered.
///
/// The `rule` field is asserted VERBATIM and PER RULE because that is where the sibling engine's defect
/// sits: java aggregates two `forbid` lines to `"forbid (× 2)"`, which answers *how many* when the
/// operator's question is *which*.
#[test]
fn unevaluated_rides_the_gate_json_document_one_entry_per_rule() {
    let f = Fixture::new("unevaldoc");
    f.write_report();
    let pol = f.dir.join("p.policy");
    let verdict = f.dir.join("v.json");
    let run = |policy: &str| -> (Option<i32>, String, serde_json::Value) {
        std::fs::write(&pol, policy).unwrap();
        let _ = std::fs::remove_file(&verdict); // never read a previous run's answer
        let out = Command::new(bin())
            .args(["gate", "--report", &f.report_path(), "--policy"])
            .arg(&pol)
            .arg("--gate-json")
            .arg(&verdict)
            .env_remove("CANDOR_POLICY")
            .env_remove("CANDOR_CONFIG")
            .output()
            .expect("run candor-query gate");
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        let doc = std::fs::read_to_string(&verdict)
            .unwrap_or_else(|e| panic!("no --gate-json document ({e}); stderr was:\n{err}"));
        (out.status.code(), err, serde_json::from_str(&doc).unwrap())
    };
    let rules_of = |v: &serde_json::Value| -> Vec<String> {
        v["unevaluated"]
            .as_array()
            .map(|a| a.iter().map(|u| u["rule"].as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    };

    // ── THE CONTROL: a policy this verb answers IN FULL carries NO `unevaluated` key. Without this arm
    // the assertions below are satisfied by a field that is simply always present. ──
    let (rc, err, v) = run("deny Fs\n");
    assert_eq!(rc, Some(1), "the control fires: {err}");
    assert!(
        v.get("unevaluated").is_none(),
        "a fully-answered policy must stay byte-identical to a pre-rung verdict: {v}"
    );

    // ── THE FINDING: a violation the verb IS sure of, beside a rule it is not. Both in the document. ──
    let (rc, err, v) = run("deny Fs\nallow Fs /var/data\n");
    assert_eq!(rc, Some(1), "the certain violation still dominates (Lemma 2): {err}");
    assert!(
        v["violations"].as_array().is_some_and(|a| !a.is_empty()),
        "the violation is not displaced by the disclosure: {v}"
    );
    assert_eq!(
        rules_of(&v),
        vec!["allow Fs /var/data".to_string()],
        "the unanswered rule reaches the MACHINE channel, with the operator's own line verbatim: {v}"
    );
    assert!(
        v["unevaluated"][0]["why"].as_str().is_some_and(|w| !w.is_empty()),
        "…and says why: {v}"
    );

    // ── ONE ENTRY PER RULE, not per KIND. Two `forbid` lines are two entries carrying two raw lines —
    // the arm that fails against an aggregate like `"forbid (× 2)"`. ──
    let (rc, err, v) = run("deny Fs\nforbid app -> infra\nforbid web -> db\n");
    assert_eq!(rc, Some(1), "still exit 1: {err}");
    assert_eq!(
        rules_of(&v),
        vec!["forbid app -> infra".to_string(), "forbid web -> db".to_string()],
        "TWO forbid lines are TWO entries — an aggregate answers `how many` when the question is \
         `which`: {v}"
    );

    // ── THE SOLE REFUSAL: nothing fired, so the document is a REFUSAL — and it carries the list too,
    // because `reason` is prose and a consumer cannot iterate prose. The MIRROR is asserted in the same
    // breath: a refusal must still have NO `violations` key, so the disclosure did not turn it into a
    // verdict claiming none were found. ──
    let (rc, err, v) = run("forbid app -> infra\nallow Fs /var/data\n");
    assert_eq!(rc, Some(2), "a policy that is nothing but unanswerable rules refuses: {err}");
    assert_eq!(v["refused"], serde_json::json!(true), "still a refusal document: {v}");
    assert!(
        v.get("violations").is_none(),
        "MIRROR: a refusal carries no `violations` key — `[]` is the claim it cannot make: {v}"
    );
    assert_eq!(
        rules_of(&v),
        vec!["forbid app -> infra".to_string(), "allow Fs /var/data".to_string()],
        "the sole refusal names every rule it could not decide: {v}"
    );

    // ── AND THE HUMAN CHANNEL IS DERIVED FROM THE SAME PAIRS, so the two cannot disagree about WHICH
    // rule went unanswered — the split that produced this family's false-disposition defect. ──
    assert!(
        err.contains("`forbid app -> infra`") && err.contains("`allow Fs /var/data`"),
        "stderr names the same rules the document does: {err}"
    );
}

/// ⟨0.24⟩ `parsepolicy` REPORTS EVERY LINE IT DID NOT HONOUR — SPEC §3.1 `195d45a` + `901f14d`.
///
/// Measured on the conformance battery before this: java 10, ts 4, **rust 0** — this verb emitted no
/// `errors` key at all. The facts existed and went to stderr, so the one verb that exists to let a
/// consumer diff what an engine made of a policy answered with the not-honoured half deleted. It also
/// contradicted this engine's own gate, which REFUSES an unrecognised class token while the parse
/// narrowed it in silence.
///
/// `kind` is asserted against the SPEC's closed set, not the reference engine's: java emits `forbid
/// form`, `allow values` and `rule kind` (space), three of which are outside `901f14d`'s four values.
#[test]
fn parsepolicy_reports_every_line_it_did_not_honour() {
    let f = Fixture::new("parsepolerr");
    let pol = f.dir.join("p.policy");
    let parse = |text: &str| -> serde_json::Value {
        std::fs::write(&pol, text).unwrap();
        let out = Command::new(bin())
            .arg("parsepolicy")
            .arg(&pol)
            .env_remove("CANDOR_CONFIG")
            .output()
            .expect("run candor-query parsepolicy");
        assert_eq!(
            out.status.code(),
            Some(0),
            "§3.1: parsepolicy MUST NOT REFUSE a policy it can read and cannot honour — it REPORTS the \
             parse. stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("parsepolicy emits one JSON document")
    };
    let errs = |v: &serde_json::Value| -> Vec<(String, String, String)> {
        v["errors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|e| {
                        (
                            e["kind"].as_str().unwrap().to_string(),
                            e["token"].as_str().unwrap().to_string(),
                            e["rule"].as_str().unwrap().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    // ── THE CONTROL: a policy honoured in full carries NO `errors` key, so a clean parse stays
    // byte-identical to the pre-rung dump and the four-way differential does not move. ──
    let v = parse("deny Fs\nallow Net github.com\nforbid app -> infra\npure core\n");
    assert!(v.get("errors").is_none(), "a clean parse emits no `errors` key: {v}");

    // ── THE FINDING: one line of each kind the parser drops, in SOURCE order. ──
    let v = parse(concat!(
        "deny notaneffect\n",          // effect-name  — a deny naming no known effect
        "allow Clock whatever\n",      // effect-name  — allow's closed effect position
        "forbid bad\n",                // rule-kind    — the arrow is not its own token
        "nonsense line\n",             // rule-kind    — no such rule keyword
        "allow Net in\n",              // rule-kind    — allow naming no values
        "deny Fs Unknown[bogus,reflect] io\n",  // reason-class/alias
        "deny Net[bogus,unknown-host] mixed\n", // Net destination-class
    ));
    let got = errs(&v);
    assert_eq!(got.len(), 7, "EVERY not-honoured line is reported, not just the fatal ones: {v}");
    let kinds: Vec<&str> = got.iter().map(|(k, _, _)| k.as_str()).collect();
    assert_eq!(
        kinds,
        vec![
            "effect-name",
            "effect-name",
            "rule-kind",
            "rule-kind",
            "rule-kind",
            "reason-class/alias",
            "Net destination-class",
        ],
        "`kind` is drawn from §3.1's CLOSED set, in source order: {v}"
    );
    // The raw line travels verbatim — it is what the operator has to go and fix.
    assert_eq!(got[0].2, "deny notaneffect");
    assert_eq!(got[6].2, "deny Net[bogus,unknown-host] mixed");
    // …and the offending TOKEN is named, not just the line.
    assert_eq!(got[5].1, "bogus", "the token is the finding: {v}");
    assert_eq!(got[3].1, "nonsense");
    // `accepted` is an ARRAY OF TOKENS, never prose (candor-ts emits a prose string, which is
    // unparseable by the consumer the field exists for).
    for e in v["errors"].as_array().unwrap() {
        assert!(
            e["accepted"].is_array(),
            "`accepted` is an array of tokens: {e}"
        );
    }
    assert_eq!(
        v["errors"][5]["accepted"],
        serde_json::json!(["reflect", "dispatch", "indirect", "native", "unresolved", "setup", "dynamic", "*"]),
        "the reason-class vocabulary is named token by token: {v}"
    );

    // ── THE MIRROR: reporting a dropped line must NOT make the gate refuse it. `errors` widened; what
    // REFUSES did not. A build that survived `nonsense line` yesterday still survives it. ──
    let v = parse("deny Fs\nnonsense line\n");
    assert_eq!(errs(&v).len(), 1, "the dropped line is reported: {v}");
    assert_eq!(
        v["deny"].as_array().map(Vec::len),
        Some(1),
        "…and the rest of the policy still parsed: {v}"
    );
    // …AND MEASURED THROUGH THE GATE, which is where the mirror would actually bite. Reporting is not
    // refusing: a policy whose only defect is a dropped line still gates, and `deny Fs` still fires.
    f.write_report();
    std::fs::write(&pol, "deny Fs\nnonsense line\n").unwrap();
    let out = Command::new(bin())
        .args(["gate", "--report", &f.report_path(), "--policy"])
        .arg(&pol)
        .env_remove("CANDOR_POLICY")
        .env_remove("CANDOR_CONFIG")
        .output()
        .expect("run candor-query gate");
    assert_eq!(
        out.status.code(),
        Some(1),
        "MIRROR: widening `errors` must not widen what REFUSES — `deny Fs` still fires beside a dropped \
         line (exit 1, not 2). stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// ⟨0.24⟩ A TYPO'D EFFECT NAME IS A POLICY ERROR, NOT A DELETED RULE — SPEC §6.2 `1e1748a`.
///
/// Measured four-way before this: `deny Nett app` → exit 0 and `allow Nett host.example` → exit 0 on
/// rust, ts, java AND swift. The rule is deleted, the gate is green, and the operator reads an armed
/// `deny Net` / `allow` that does not exist.
///
/// Two cases, and only two, because the grammar defence is real but narrower than it was taken to be:
/// `allow`'s effect position is a closed set with NO scope reading available, and a `deny` whose effect
/// list ends up EMPTY is malformed under either reading. **The ambiguous middle stays permissive by
/// design and is asserted here too** — the arm that keeps this fix from becoming its own over-reach.
#[test]
fn a_typod_effect_name_is_a_policy_error_and_the_ambiguous_middle_is_not() {
    let f = Fixture::new("typoeffect");
    f.write_report();
    let pol = f.dir.join("p.policy");
    let run = |verb: &[&str], policy: &str| -> (Option<i32>, String) {
        std::fs::write(&pol, policy).unwrap();
        let mut c = Command::new(bin());
        c.args(verb).args(["--report", &f.report_path(), "--policy"]).arg(&pol);
        c.env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG");
        let out = c.output().expect("run candor-query");
        (out.status.code(), String::from_utf8_lossy(&out.stderr).into_owned())
    };

    for (policy, why) in [
        ("deny Nett app\n", "a deny whose effect list ends up EMPTY is malformed under either reading"),
        ("deny notaneffect\n", "…including when the typo is the only token"),
        ("deny\n", "…and when there is no effect token at all"),
        ("allow Nett host.example\n", "`allow`'s effect position is closed, with no scope reading"),
        ("allow Clock whatever\n", "…and a real effect that `allow` does not cover is the same case"),
    ] {
        let (rc, err) = run(&["gate"], policy);
        assert_eq!(rc, Some(2), "gate `{}`: {why}\n{err}", policy.trim());
        assert!(
            err.contains("policy error"),
            "gate `{}` must SAY it is a policy error, not drop the rule in silence:\n{err}",
            policy.trim()
        );
        // The PRE-EDIT verbs refuse it too — answering there from a rule the gate will not apply is
        // the worse failure, since this is the verb consulted before the edit.
        let (rc, err) = run(&["whatif", "outer", "Net"], policy);
        assert_eq!(rc, Some(2), "whatif `{}`: the pre-edit verb refuses it too\n{err}", policy.trim());
    }

    // ── THE AMBIGUOUS MIDDLE, WHICH MUST STAY OPEN. `deny Net Exex app` has one valid effect and an
    // unrecognised trailing token that the parser genuinely cannot tell from a legitimate scope. Closing
    // it would make every scoped rule a coin toss, so it parses, gates, and `parsepolicy` shows which
    // reading it took by dumping the scope. ──
    let (rc, err) = run(&["gate"], "deny Net Exex app\n");
    assert_eq!(rc, Some(0), "the ambiguous middle is NOT refused — `Exex` reads as a scope:\n{err}");

    // …and the reading it took is visible, so the operator can always see it.
    std::fs::write(&pol, "deny Net Exex app\n").unwrap();
    let out = Command::new(bin())
        .arg("parsepolicy")
        .arg(&pol)
        .env_remove("CANDOR_CONFIG")
        .output()
        .expect("run parsepolicy");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["deny"][0]["scope"], serde_json::json!("Exex"), "the scope reading is shown: {v}");

    // ── THE MIRROR: a policy that is FINE must stay fine. Without this arm the assertions above are
    // satisfied by refusing everything. ──
    let (rc, err) = run(&["gate"], "deny Fs\n");
    assert_eq!(rc, Some(1), "a well-formed policy still gates:\n{err}");
    let (rc, err) = run(&["gate"], "deny Net\npure core\n");
    assert_eq!(rc, Some(0), "…and still passes when nothing violates:\n{err}");

    // ── AND `parsepolicy` STILL DOES NOT REFUSE. §3.1: it REPORTS a policy it can read and cannot
    // honour. A fatal error is exactly what an operator runs this verb to diagnose, so refusing here
    // would take the diagnosis away at the moment it is needed. ──
    std::fs::write(&pol, "deny Nett app\nallow Nett host.example\n").unwrap();
    let out = Command::new(bin())
        .arg("parsepolicy")
        .arg(&pol)
        .env_remove("CANDOR_CONFIG")
        .output()
        .expect("run parsepolicy");
    assert_eq!(out.status.code(), Some(0), "parsepolicy MUST NOT refuse a policy it can read");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rules: Vec<&str> =
        v["errors"].as_array().unwrap().iter().map(|e| e["rule"].as_str().unwrap()).collect();
    assert_eq!(rules, vec!["deny Nett app", "allow Nett host.example"], "…it reports both: {v}");
}

/// SPEC §3.2 ⟨0.24⟩ **`whatif` OVER AN INCOMPLETE REPORT OMITS `ok` — not `true`, and not `false`
/// either** (candor-spec `0075987`).
///
/// **THE DEFECT, measured here and independently in candor-java, and REPORTED rather than decided —
/// which is why it is now ruled.** Over a report declaring `unanalyzed`, this verb answered
/// `{"ok": true, …}`, exit 0, `✓ within policy`, with no disclosure on ANY channel — while
/// `candor-query gate` over the SAME report exits 2. `whatif`'s `affected` set is a reverse-reachability
/// closure over the callgraph, so a caller living in a file the scan could not parse is INVISIBLE to it:
/// the blast radius is computed over a universe the analysis knows it cannot fully see, and `true` is a
/// claim that input does not license. It is not a gate, but its `ok` reads as one.
///
/// **THE ASSERTION IS ABSENCE, AND THAT IS THE POINT.** `assert_eq!(v["ok"], false)` passes on
/// `serde_json`'s `Value::Null` comparison being false-y in neither direction — but more importantly it
/// would pass on the WRONG FIX. `ok: false` asserts a VIOLATION the analysis never found: the
/// fabrication mirror, worse than the thing it replaces. So the row asserts the key is GONE, which is
/// the only shape where `if (r.ok)` is falsy without anything being invented.
///
/// **THE MIRROR**, on the same fixture with one key removed: a COMPLETE report must still carry `ok`.
/// A "fix" that dropped `ok` unconditionally would satisfy every absence assertion above and delete the
/// verb's answer for every user who has no unanalyzed code.
#[test]
fn whatif_over_an_incomplete_report_omits_ok_rather_than_answering_either_boolean() {
    let f = Fixture::new("whatif-incomplete");
    let report = |unanalyzed: &str| {
        format!(
            r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",
                "analyzed":{{"count":2,"digest":"0"}},{unanalyzed}
                "functions":[{{"fn":"app.handler","inferred":[],"direct":[]}},
                             {{"fn":"app.leaf","inferred":[],"direct":[]}}]}}"#
        )
    };
    let whatif = |sub: &str, unanalyzed: &str, policy: &str| -> (i32, serde_json::Value, String) {
        let loc = gate_fixture(&f.dir, sub, &report(unanalyzed), Some(r#"{"app.handler":["app.leaf"]}"#));
        let p = pol(&f.dir, sub, policy);
        let out = Command::new(bin())
            .args(["whatif", "app.leaf", "Net", "--report", &loc, "--policy", &p.to_string_lossy(), "--json"])
            .output()
            .expect("run candor-query");
        (
            out.status.code().unwrap_or(-1),
            serde_json::from_slice(&out.stdout).expect("a JSON document"),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    const MANIFEST: &str = r#""unanalyzed":[{"path":"src/opaque.rs","reason":"parse error"}],"#;

    // ── ARM 1: the report declares unanalyzed source, and no rule fires. THE GREEN LIGHT IS WITHHELD. ──
    let (rc, v, _) = whatif("clean", MANIFEST, "deny Net app.elsewhere\n");
    assert!(
        v.get("ok").is_none(),
        "`ok` must be ABSENT — a consumer writing `if (r.ok)` has to get a falsy value and fail safe, \
         and `ok: false` would assert a violation the analysis never found:\n{v:#}"
    );
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v:#}");
    assert_eq!(
        v["unanalyzed"],
        serde_json::json!([{"path": "src/opaque.rs", "reason": "parse error"}]),
        "the manifest travels, so a consumer that looks past `incomplete` learns exactly what was \
         unread:\n{v:#}"
    );
    // …AND THE PARTIAL ANSWER STILL SHIPS. §3.2: *a partial answer that says it is partial is worth
    // more than a refusal, and `whatif` is consulted BEFORE an edit, where the alternative is the
    // operator guessing.* An implementation that refused here would pass the absence assert above.
    assert_eq!(
        v["affected"],
        serde_json::json!(["app.handler", "app.leaf"]),
        "the blast radius is hedged, never withheld:\n{v:#}"
    );
    assert_eq!(v["violations"], serde_json::json!([]), "{v:#}");
    assert_eq!(rc, 0, "the exit code is unchanged — §3.3's exit-2 causes are a GATE's, and this is not one");

    // ── ARM 2: the same incompleteness WITH a rule that fires. The violation is certain regardless of
    // what went unread (Lemma 2 — an unseen caller can only ADD to the affected set), so it ships; `ok`
    // still does not, because its absence is about the SET being partial, not about the verdict. ──
    let (rc2, v2, _) = whatif("firing", MANIFEST, "deny Net app\n");
    assert!(v2.get("ok").is_none(), "{v2:#}");
    assert_eq!(v2["incomplete"], serde_json::json!(true), "{v2:#}");
    assert_eq!(v2["violations"].as_array().unwrap().len(), 2, "the finding is not suppressed:\n{v2:#}");
    assert_eq!(rc2, 1, "a certain violation still fails:\n{v2:#}");

    // ── THE MIRROR: NO `unanalyzed` key ⇒ the report is complete ⇒ `ok` is BACK, and `incomplete` and
    // the manifest are both absent, so every ordinary document stays byte-identical to a pre-rung one. ──
    let (rc3, v3, _) = whatif("complete", "", "deny Net app.elsewhere\n");
    assert_eq!(
        v3["ok"],
        serde_json::json!(true),
        "a COMPLETE report must still get its answer — dropping `ok` unconditionally would satisfy every \
         absence assert above and delete the verb for everyone with no unanalyzed code:\n{v3:#}"
    );
    assert!(v3.get("incomplete").is_none(), "{v3:#}");
    assert!(v3.get("unanalyzed").is_none(), "{v3:#}");
    assert_eq!(rc3, 0);
    // …and an EMPTY manifest is the same claim as an absent one: the scan saw everything.
    let (_, v4, _) = whatif("emptymanifest", r#""unanalyzed":[],"#, "deny Net app.elsewhere\n");
    assert_eq!(v4["ok"], serde_json::json!(true), "{v4:#}");
    assert!(v4.get("incomplete").is_none(), "{v4:#}");

    // ── THE HUMAN CHANNEL CARRIES THE SAME HEDGE. `0075987` states the defect as "no disclosure on ANY
    // channel", and an operator reading an unqualified blast radius and a `✓` has been told exactly what
    // `ok: true` says. This arm exists because a MUTANT that kept the whole JSON fix and deleted only
    // the printed note survived the entire suite: absence asserts on `ok` cannot see the other channel.
    let human = |sub: &str, unanalyzed: &str| -> String {
        let loc = gate_fixture(&f.dir, sub, &report(unanalyzed), Some(r#"{"app.handler":["app.leaf"]}"#));
        let p = pol(&f.dir, sub, "deny Net app.elsewhere\n");
        let out = Command::new(bin())
            .args(["whatif", "app.leaf", "Net", "--report", &loc, "--policy", &p.to_string_lossy()])
            .output()
            .expect("run candor-query");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let h = human("human-incomplete", MANIFEST);
    assert!(h.contains("INCOMPLETE"), "the operator is told the universe was partial:\n{h}");
    assert!(h.contains("src/opaque.rs"), "…and what was unread:\n{h}");
    assert!(
        !h.contains("✓ within policy"),
        "the `✓` is withheld for the reason `ok` is — it is a claim over a set known to be partial:\n{h}"
    );
    // AND THE MIRROR ON THIS CHANNEL TOO: a complete report still gets its unqualified `✓`.
    let h2 = human("human-complete", "");
    assert!(h2.contains("✓ within policy"), "{h2}");
    assert!(!h2.contains("INCOMPLETE"), "no hedge where none is owed:\n{h2}");
}

/// SPEC §2 + §3.2 ⟨0.24⟩ — the corrupt-key arm of the row above: an `unanalyzed` key that is PRESENT
/// but is not `[{ path, reason }]`.
///
/// §2's rule is that *a key that cannot be READ is corrupt input, never its empty value* — and here the
/// empty value is exactly what licenses `ok`, so coercing it converts corrupt input into the green
/// claim. This is not hypothetical: the identical coercion was MEASURED on the gate route (2026-07-28,
/// `{"unit":…,"why":…}` — the right shape with the wrong field names, what a hand-built or
/// foreign-produced report yields) and it turned an exit 2 into `policy ✓`.
///
/// The gate REFUSES on this. `whatif` cannot: a refusal here sends the operator back to guessing, which
/// is the thing the verb exists to replace. So it takes the same fail-safe posture through the
/// DISCLOSURE instead — `ok` withheld, `incomplete: true`, and the key named on stderr.
#[test]
fn whatif_treats_an_unreadable_unanalyzed_key_as_incomplete_rather_than_as_empty() {
    let f = Fixture::new("whatif-corrupt-manifest");
    let loc = gate_fixture(
        &f.dir,
        "r",
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"app",
            "analyzed":{"count":1,"digest":"0"},
            "unanalyzed":[{"unit":"src/opaque.rs","why":"parse error"}],
            "functions":[{"fn":"app.leaf","inferred":[],"direct":[]}]}"#,
        Some(r#"{"app.leaf":[]}"#),
    );
    let p = pol(&f.dir, "w", "deny Net app.elsewhere\n");
    let out = Command::new(bin())
        .args(["whatif", "app.leaf", "Net", "--report", &loc, "--policy", &p.to_string_lossy(), "--json"])
        .output()
        .expect("run candor-query");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON document");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(v.get("ok").is_none(), "an unreadable manifest is not an empty one:\n{v:#}\n{err}");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v:#}");
    // NOTHING IS INVENTED: there is no readable manifest, so none is emitted. `incomplete` alone is the
    // whole of what the input licenses, and stderr NAMES the key — "your `unanalyzed` key is not
    // `[{path, reason}]`" is actionable where "this report did not load" sends the user to a scan they
    // may not own.
    assert!(v.get("unanalyzed").is_none(), "a manifest that could not be read is not fabricated:\n{v:#}");
    assert!(err.contains("`unanalyzed` key is PRESENT"), "the key is named on stderr: {err}");
}

/// ⟨0.24⟩ **AN ADVISORY VERB MAY BE LESS CERTAIN THAN THE GATE, NEVER MORE** — SPEC §3.2, candor-spec
/// `4fd140c`, and conformance PART 27 row R11 (`rust advisory-bound`).
///
/// **THE DEFECT, measured four-way.** Over a report carrying `hosts` and NO `netClass`, under
/// `deny Net[unknown-host] app`:
///
/// ```text
///   gate --report   exit 2   §3.1 answerability — it CANNOT judge `app.noClass`
///   unverified      exit 0   {"ok": false, "unverified": [app.nativeHole]}
/// ```
///
/// The verb whose entire job is *"your green gate is not provably green"* cleared the one function the
/// gate withheld on, and named a DIFFERENT hole — which is why R11's assertion is PER FUNCTION. A weaker
/// form ("the verb names SOMETHING") passed on all four engines while the defect stood.
///
/// **THE ASSERTIONS THAT WOULD PASS ON A WRONG FIX, and are therefore not the ones made here.** Naming
/// `app.noClass` with the DERIVED destination class as its reason would satisfy every count below while
/// restating the defect as a disclosure — this engine could floor it at `unknown-host` from `hosts` in
/// one line — so the reason is asserted to be the MISSING EVIDENCE. And `assert_eq!(fix_gate["ok"],
/// false)` would pass on the fabrication mirror (`0075987`), so the row asserts the key is GONE.
///
/// **THE MIRROR IS IN THE SAME RUN, on the same two reports.** A function the gate CAN clear must not
/// start being named, `unevaluated` must be ABSENT from an ordinary document, `ok` must come back, and
/// `fix` must still plan the remedy for a crossing the gate DOES charge. Measured across 368 OLD/NEW
/// runs over four real corpora × eight policies: zero differences on reports where every rule was
/// answerable — and on those corpora the count of `Net`-with-no-`netClass` entries is ZERO, which is
/// why the row below hand-writes the state instead of trusting a corpus to contain it.
#[test]
fn an_advisory_verb_names_what_the_gate_could_not_judge_and_stays_quiet_where_it_could() {
    let f = Fixture::new("advisory-bound");
    // R11's report, verbatim: a hole the class filter excludes, a Net entry with NO `netClass`, and a
    // plain violator so the gate has something to charge.
    let refused_loc = gate_fixture(
        &f.dir,
        "refused",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":3,"digest":"0"},
            "functions":[
              {"fn":"app.nativeHole","inferred":["Unknown"],"direct":["Unknown"],"unknownWhy":["native:dlopen"]},
              {"fn":"app.noClass","inferred":["Net"],"direct":["Net"],"hosts":["api.example.com"]},
              {"fn":"app.noClass2","inferred":["Net"],"direct":["Net"],"hosts":["b.example.com"]},
              {"fn":"app.writes","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#,
        None,
    );
    // THE MIRROR REPORT: the SAME shape with the one field the gate needs. Every rule is answerable, so
    // the gate charges `app.hasClass` and this verb must behave exactly as it did before the ruling.
    let answered_loc = gate_fixture(
        &f.dir,
        "answered",
        r#"{"candor":{"version":"handwritten","spec":"0.23"},"package":"app",
            "analyzed":{"count":3,"digest":"0"},
            "functions":[
              {"fn":"app.nativeHole","inferred":["Unknown"],"direct":["Unknown"],"unknownWhy":["native:dlopen"]},
              {"fn":"app.hasClass","inferred":["Net"],"direct":["Net"],"hosts":["api.example.com"],
               "netClass":["unknown-host"]},
              {"fn":"app.writes","inferred":["Fs"],"direct":["Fs"],"paths":["/etc/hosts"]}]}"#,
        None,
    );
    let netclass = pol(&f.dir, "netclass", "deny Net[unknown-host] app\n");
    let run = |verb: &str, loc: &str, extra: &[&str]| -> (i32, String, String) {
        let mut args: Vec<String> =
            vec![verb.into(), "--report".into(), loc.into(), "--policy".into(),
                 netclass.to_string_lossy().into_owned()];
        args.extend(extra.iter().map(|s| s.to_string()));
        let out = Command::new(bin()).args(&args).env_remove("CANDOR_POLICY").output().expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let json = |s: &str| -> serde_json::Value {
        serde_json::from_str(s).unwrap_or_else(|e| panic!("not JSON ({e}):\n{s}"))
    };

    // ── ARM 1: THE GATE REFUSES. ────────────────────────────────────────────────────────────────
    let (grc, _, gerr) = run_gate(&refused_loc, &netclass, &[]);
    assert_eq!(grc, 2, "the gate CANNOT judge `app.noClass` over this report:\n{gerr}");
    assert!(gerr.contains("app.noClass"), "…and it says which function:\n{gerr}");

    let (urc, uout, _) = run("unverified", &refused_loc, &["--json"]);
    let u = json(&uout);
    let named: Vec<&str> =
        u["unverified"].as_array().unwrap().iter().map(|h| h["fn"].as_str().unwrap()).collect();
    assert!(
        named.contains(&"app.noClass"),
        "the gate exited 2 — it could NOT clear `app.noClass` — yet the verb does not name it \
         (named: {named:?}). The advisory verb is more confident than the gate over identical bytes:\n{uout}"
    );
    // EVERY function the gate could not judge, not just the one the gate quotes as its example. The
    // gate names ONE function per rule (naming all of them would bury the rule) and a fix that inherited
    // that `break` would clear the second one in silence — the defect in miniature.
    assert!(
        named.contains(&"app.noClass2"),
        "the SECOND unjudgeable function too — the gate reports one example per rule, this verb reports \
         the functions (named: {named:?}):\n{uout}"
    );
    // The verb's OWN job is unmoved: the hole the class filter excludes is still disclosed.
    assert!(named.contains(&"app.nativeHole"), "the ordinary hole is still named:\n{uout}");
    let refusal = u["unverified"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["fn"] == "app.noClass")
        .expect("named above");
    let why = refusal["why"].as_str().unwrap_or("");
    assert!(
        why.contains("netClass") && why.contains("absent"),
        "the reason recorded is the MISSING EVIDENCE:\n{why}"
    );
    assert!(
        !why.contains("unknown-host") || why.contains("no `netClass`"),
        "…and NEVER the derived class — this engine can floor `app.noClass` at `unknown-host` from its \
         `hosts` in one line, and recording that would restate the defect as a disclosure:\n{why}"
    );
    assert!(
        refusal["upgrade"].is_null(),
        "no policy edit makes a missing field appear, so there is no upgrade to advise:\n{uout}"
    );
    // `unevaluated` is the GATE'S OWN SHAPE, not a second spelling — and the same list the gate emits.
    let verdict = f.dir.join("v.json");
    let _ = std::fs::remove_file(&verdict);
    let _ = run_gate(&refused_loc, &netclass, &["--gate-json", &verdict.to_string_lossy()]);
    let gdoc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&verdict).expect("gate document")).unwrap();
    assert_eq!(
        u["unevaluated"], gdoc["unevaluated"],
        "the disclosure is the gate's, byte for byte — inventing a second spelling is the mistake SPEC \
         §3.2 says this document has made four times:\n{uout}"
    );
    assert_eq!(u["unevaluated"].as_array().map(Vec::len), Some(1), "one entry per RULE:\n{uout}");
    // ⟨0.24⟩ **AND `ok` IS OMITTED HERE TOO, NOT `false`** — SPEC §3.2 `142740a`. `4fd140c` argued the
    // `false` deliberately and it was wrong by that clause's own reasoning one paragraph earlier: where
    // a rule is WITHHELD no hole was FOUND, the question was declined, so `false` asserts the finding
    // that did not happen — the fabrication mirror, which is exactly why the sibling INCOMPLETENESS
    // trigger omits the field. Two clauses a day apart, one shape. MEASURED on this engine before the
    // change: `{"ok": false, "unverified": [app.noClass], "unevaluated": [...]}` while `gate --report`
    // refused outright; `fix-gate` (asserted below) was already right, and the family split
    // two-against-two exactly as it had on the sibling trigger.
    assert!(
        u.get("ok").is_none(),
        "the KEY IS GONE — `assert_eq!(ok, false)` would pass on the fabrication mirror:\n{uout}"
    );
    assert_eq!(urc, 0, "advisory by default — the ruling is about the DISCLOSURE");
    let (src, _, _) = run("unverified", &refused_loc, &["--json", "--strict"]);
    assert_eq!(src, 2, "`--strict` exits 2, MATCHING the gate — answering 1 would claim it got further");

    // THE HUMAN CHANNEL, which a mutant that fixed only the JSON survived once before (`531c415`).
    let (_, utext, _) = run("unverified", &refused_loc, &[]);
    assert!(utext.contains("app.noClass"), "the operator is told too:\n{utext}");
    assert!(
        utext.contains("COULD NOT JUDGE"),
        "…and told it is a refusal, not an ordinary hole:\n{utext}"
    );

    // `fix-gate`: no remedy premised on evidence the gate refused, `ok` withheld, `unevaluated` carried.
    let (frc, fout, _) = run("fix-gate", &refused_loc, &["--json"]);
    let fg = json(&fout);
    assert_eq!(fg["remedies"].as_array().map(Vec::len), Some(0), "no hoist plan for it:\n{fout}");
    assert!(
        fg.get("ok").is_none(),
        "`ok: true` asserts there is no crossing over a boundary nothing adjudicated, and `ok: false` \
         would assert a crossing never found (the fabrication mirror, SPEC §3.2 `0075987`) — so the KEY \
         IS GONE. `assert_eq!(ok, false)` would have passed on the wrong fix:\n{fout}"
    );
    assert_eq!(fg["unevaluated"], gdoc["unevaluated"], "same shape, same list:\n{fout}");
    assert_eq!(frc, 0, "advisory by default");
    let (fsrc, _, _) = run("fix-gate", &refused_loc, &["--json", "--strict"]);
    assert_eq!(fsrc, 2, "`--strict` exits 2, matching the gate");
    let (_, ftext, ferr) = run("fix-gate", &refused_loc, &[]);
    assert!(!ftext.contains('✓'), "the tick is the same claim in prose:\n{ftext}");
    assert!(ferr.contains("app.noClass"), "the operator learns which boundary went unjudged:\n{ferr}");

    // `fix` (ONE function) was the worst of the three: it went through the filter-BLIND `denied_layer`
    // and printed a complete hoist plan — `deniedSpan`, `site`, `policyAlternative` — for exactly the
    // boundary the gate refused.
    let (xrc, xout, xerr) = {
        let out = Command::new(bin())
            .args(["fix", "app.noClass", "Net", "--report", &refused_loc, "--policy"])
            .arg(&netclass)
            .arg("--json")
            .env_remove("CANDOR_POLICY")
            .output()
            .expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let x = json(&xout);
    assert!(x.get("hoistTo").is_none() && x.get("deniedSpan").is_none(),
        "a hoist plan for a boundary the gate could not adjudicate is a confident instruction resting \
         on a guess:\n{xout}");
    assert_eq!(x["unevaluated"], gdoc["unevaluated"], "it says WHY it computed nothing:\n{xout}");
    assert_eq!(xrc, 0);
    assert!(xerr.contains("netClass"), "and on the human channel:\n{xerr}");

    // ── ARM 2, THE MIRROR: every rule answerable. Nothing new may appear. ───────────────────────
    let (grc2, _, gerr2) = run_gate(&answered_loc, &netclass, &[]);
    assert_eq!(grc2, 1, "`app.hasClass` carries the class the filter names, so the gate CHARGES:\n{gerr2}");

    let (urc2, uout2, _) = run("unverified", &answered_loc, &["--json"]);
    let u2 = json(&uout2);
    let named2: Vec<&str> =
        u2["unverified"].as_array().unwrap().iter().map(|h| h["fn"].as_str().unwrap()).collect();
    assert_eq!(
        named2,
        vec!["app.nativeHole"],
        "a function the gate CAN judge must NOT start being named — the over-report mirror, and the \
         direction a fabrication fix introduces:\n{uout2}"
    );
    assert!(
        u2.get("unevaluated").is_none(),
        "an ordinary document stays byte-identical to a pre-ruling one:\n{uout2}"
    );
    // …AND `ok` COMES BACK, with the value the findings license. The mirror for `142740a`: removing the
    // key unconditionally would satisfy the absence assert above and delete the answer for everyone
    // whose rules the gate can evaluate.
    assert_eq!(
        u2["ok"],
        serde_json::json!(false),
        "a report the gate CAN judge still gets a boolean, and here it is `false` because a hole WAS \
         found — that is a finding, not a declined question:\n{uout2}"
    );
    assert_eq!(urc2, 0);
    let (src2, _, _) = run("unverified", &answered_loc, &["--json", "--strict"]);
    assert_eq!(src2, 1, "…and `--strict` is back to 1: holes, but nothing unevaluated");
    let (_, utext2, _) = run("unverified", &answered_loc, &[]);
    assert!(
        utext2.contains("The gate still PASSES — "),
        "the unqualified sentence returns where nothing went unevaluated — 224 OLD/NEW runs over four \
         corpora showed this line was the whole of the churn until it was made conditional:\n{utext2}"
    );

    let (frc2, fout2, _) = run("fix-gate", &answered_loc, &["--json"]);
    let fg2 = json(&fout2);
    assert_eq!(fg2["ok"], false, "`ok` COMES BACK where every rule was answerable:\n{fout2}");
    assert!(fg2.get("unevaluated").is_none(), "…and nothing went unevaluated:\n{fout2}");
    assert_eq!(
        fg2["remedies"].as_array().map(Vec::len),
        Some(1),
        "…and the crossing the gate DOES charge still gets its remedy — a fix that turned `fix-gate` \
         silent on real violations would pass every absence assert above:\n{fout2}"
    );
    assert_eq!(frc2, 0);
    let out = Command::new(bin())
        .args(["fix", "app.hasClass", "Net", "--report", &answered_loc, "--policy"])
        .arg(&netclass)
        .arg("--json")
        .env_remove("CANDOR_POLICY")
        .output()
        .expect("run");
    let x2 = json(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(x2["fn"], "app.hasClass");
    assert!(x2.get("deniedSpan").is_some(), "`fix` still plans where the gate charges:\n{x2}");
}

/// SPEC §3.2 ⟨0.24⟩ **THE OMIT-`ok` RULE BINDS EVERY ADVISORY VERB, ON EVERY CHANNEL IT ANSWERS ON** —
/// candor-spec `ec1a441`.
///
/// **THE DEFECT, AND IT WAS STRUCTURAL RATHER THAN A SLIP.** `0075987` ruled this for `whatif` and this
/// engine implemented it for `whatif`, in `whatif`'s own file; `unverified.rs` and `fix.rs` contained
/// ZERO occurrences of `incomplete`. Measured on the release build over the fixture below — a report
/// declaring one `unanalyzed` unit, NO `Unknown` holes at all, and a `deny Net app` nothing violates:
///
/// ```text
///   gate --report        exit 2   ok:false, incomplete:true + manifest   ← correct
///   unverified --strict  exit 0   {"ok": true, "unverified": []}
///                        stdout   "every function in a pure/deny layer is PROVABLY clean … ✓"
///   fix-gate  --strict   exit 0   {"ok": true, "remedies": []}
///                        stdout   "no deny/pure boundary crossings in this report ✓"
/// ```
///
/// "PROVABLY clean" over a report that declares source candor could not read — and `--strict` is how CI
/// consumes both verbs.
///
/// **NO EXISTING FIXTURE HAD THIS SHAPE.** Every prior one pairs incompleteness with something else to
/// find, so the question *"nothing to report, but I could not see everything"* was never asked. That is
/// why the row's core arm has an EMPTY finding list: the verb's whole answer is then the all-clear.
///
/// **THE ASSERTION IS ABSENCE.** `assert_eq!(v["ok"], false)` would pass on the fabrication mirror —
/// on an advisory verb `false` asserts *"a hole exists, here it is"* beside an empty array, a finding
/// the analysis never made. So the row asserts the KEY IS GONE.
///
/// **THE HUMAN CHANNEL IS ASSERTED SEPARATELY** because a test that reads one channel is evidence about
/// one channel: this engine built a mutant that kept the whole JSON fix and deleted only the printed
/// line, and it SURVIVED THE ENTIRE SUITE. The prose `✓` is the prose `ok: true`. `unverified` turned
/// out to have a SECOND sentence of the same kind — *"The gate still PASSES"* — which is not merely
/// unhedged but false, since `gate --report` exits 2 over these bytes; found by reading every printed
/// sentence for the claim it makes, which is what the every-channel clause asks for.
///
/// **THE MIRROR IS IN THE SAME RUN**: a COMPLETE report must still carry `ok`, still print its `✓` and
/// its "the gate still PASSES", and still exit 0/1. A fix that omitted `ok` unconditionally would
/// satisfy every absence assert here and delete the field for everyone.
#[test]
fn an_advisory_verb_over_an_incomplete_report_omits_ok_on_every_channel_it_answers_on() {
    let f = Fixture::new("advisory-incomplete");
    // `app.reader` performs Fs (so `deny Fs app` has a real remedy to compute), `app.hole` is an
    // Unknown that PASSES `deny Net app` (so the holes branch of the human channel is reachable), and
    // `app.plain` is ordinary. Under `deny Net app` NOTHING violates and NOTHING is unanswerable — the
    // reviewer's fixture, where incompleteness is the only thing there is to say.
    let report = |unanalyzed: &str| {
        format!(
            r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",
                "analyzed":{{"count":3,"digest":"0"}},{unanalyzed}
                "functions":[{{"fn":"app.reader","inferred":["Fs"],"direct":["Fs"],"paths":["/x"]}},
                             {{"fn":"app.hole","inferred":["Unknown"],"unknownWhy":["a fn-pointer call"]}},
                             {{"fn":"app.plain","inferred":[]}}]}}"#
        )
    };
    const MANIFEST: &str = r#""unanalyzed":[{"path":"src/opaque.rs","reason":"parse error"}],"#;
    let run = |sub: &str, unanalyzed: &str, verb: &str, policy: &str, extra: &[&str]| -> (i32, String, String) {
        let loc = gate_fixture(&f.dir, sub, &report(unanalyzed), Some(r#"{"app.reader":[],"app.hole":[],"app.plain":[]}"#));
        let p = pol(&f.dir, sub, policy);
        let mut args: Vec<String> =
            vec![verb.into(), "--report".into(), loc, "--policy".into(), p.to_string_lossy().into_owned()];
        args.extend(extra.iter().map(|s| s.to_string()));
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let jrun = |sub: &str, unanalyzed: &str, verb: &str, policy: &str, extra: &[&str]| -> (i32, serde_json::Value) {
        let (rc, so, se) = run(sub, unanalyzed, verb, policy, extra);
        (rc, serde_json::from_str(&so).unwrap_or_else(|e| panic!("a JSON document ({e}):\n{so}\n{se}")))
    };
    let manifest = serde_json::json!([{"path": "src/opaque.rs", "reason": "parse error"}]);

    for verb in ["unverified", "fix-gate"] {
        let findings = if verb == "unverified" { "unverified" } else { "remedies" };

        // ── ARM 1: incomplete, and NOTHING ELSE TO SAY. The all-clear is the whole answer, and it is
        // withdrawn. ──
        let (rc, v) = jrun(&format!("{verb}-incomplete"), MANIFEST, verb, "deny Net app.elsewhere\n", &["--json", "--strict"]);
        assert!(
            v.get("ok").is_none(),
            "{verb}: `ok` must be ABSENT — a consumer writing `if (r.ok)` has to get a falsy value and \
             fail safe, and `ok: false` would assert a finding the analysis never made:\n{v:#}"
        );
        assert_eq!(v["incomplete"], serde_json::json!(true), "{verb}:\n{v:#}");
        assert_eq!(v["unanalyzed"], manifest, "{verb}: the manifest travels:\n{v:#}");
        assert_eq!(v[findings], serde_json::json!([]), "{verb}: the findings still ship:\n{v:#}");
        assert_eq!(rc, 2, "{verb} --strict exits 2 — `gate --report` does over these bytes:\n{v:#}");
        // …and WITHOUT `--strict` the verb stays advisory at 0: the ruling is about the disclosure, and
        // the document above still carries it.
        let (rc_adv, v_adv) = jrun(&format!("{verb}-advisory"), MANIFEST, verb, "deny Net app.elsewhere\n", &["--json"]);
        assert_eq!(rc_adv, 0, "{verb}:\n{v_adv:#}");
        assert_eq!(v_adv["incomplete"], serde_json::json!(true), "{verb}:\n{v_adv:#}");

        // ── THE MIRROR: no manifest ⇒ complete ⇒ `ok` is BACK and nothing is added, so every ordinary
        // document stays byte-identical to a pre-rung one. ──
        let (rc2, v2) = jrun(&format!("{verb}-complete"), "", verb, "deny Net app.elsewhere\n", &["--json", "--strict"]);
        assert_eq!(
            v2["ok"],
            serde_json::json!(true),
            "{verb}: a COMPLETE report must still get its answer — omitting `ok` unconditionally would \
             satisfy every absence assert above and delete the field for everyone:\n{v2:#}"
        );
        assert!(v2.get("incomplete").is_none(), "{verb}:\n{v2:#}");
        assert!(v2.get("unanalyzed").is_none(), "{verb}:\n{v2:#}");
        assert_eq!(rc2, 0, "{verb}:\n{v2:#}");
        // …and an EMPTY manifest says the same thing an absent one does: the scan saw everything.
        let (_, v3) = jrun(&format!("{verb}-emptymanifest"), r#""unanalyzed":[],"#, verb, "deny Net app.elsewhere\n", &["--json"]);
        assert_eq!(v3["ok"], serde_json::json!(true), "{verb}:\n{v3:#}");
        assert!(v3.get("incomplete").is_none(), "{verb}:\n{v3:#}");
    }

    // ── ARM 2: incomplete AND a certain finding. The finding is not suppressed — a partial answer that
    // says it is partial beats a refusal — but `ok` still does not ship, because its absence is about
    // the SET being partial, not about the verdict. `--strict` still exits 2, not 1: the smaller code
    // would claim this verb saw more than the gate, which refuses outright. ──
    let (rc, v) = jrun("fixgate-firing", MANIFEST, "fix-gate", "deny Fs app\n", &["--json", "--strict"]);
    assert!(v.get("ok").is_none(), "{v:#}");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v:#}");
    assert_eq!(v["remedies"].as_array().unwrap().len(), 1, "the remedy is hedged, never withheld:\n{v:#}");
    assert_eq!(rc, 2, "{v:#}");
    // THE MIRROR for that arm: complete + firing is the pre-rung document, `ok: false` and exit 1.
    let (rc2, v2) = jrun("fixgate-firing-complete", "", "fix-gate", "deny Fs app\n", &["--json", "--strict"]);
    assert_eq!(v2["ok"], serde_json::json!(false), "{v2:#}");
    assert_eq!(rc2, 1, "{v2:#}");

    // ── THE HUMAN CHANNEL, which the asserts above CANNOT SEE. ──
    //
    // FIRST the reviewer's own shape — incomplete with NOTHING to report — because that is the branch
    // where the all-clear sentence IS the verb's whole answer, and it is a DIFFERENT branch from the
    // one below. **A mutant that kept this `✓` SURVIVED the row until this arm existed**: every other
    // fixture here carries a hole, so `unverified` never reached its own all-clear line, and the
    // measured defect is the one sentence no fixture was asking about.
    let (rc_h0, h0, _) =
        run("human-incomplete-clean", MANIFEST, "unverified", "deny Net app.elsewhere\n", &["--strict"]);
    assert!(
        !h0.contains("PROVABLY clean (no Unknown holes) ✓"),
        "\"PROVABLY clean\" over a report declaring source candor could not read — the prose `ok: true`, \
         on the one branch where it is the entire answer:\n{h0}"
    );
    assert!(h0.contains("INCOMPLETE") && h0.contains("src/opaque.rs"), "{h0}");
    assert_eq!(rc_h0, 2, "…and the human channel's exit code is the JSON channel's:\n{h0}");

    let (_, h, _) = run("human-incomplete", MANIFEST, "unverified", "deny Net app\n", &[]);
    assert!(h.contains("INCOMPLETE"), "the operator is told the universe was partial:\n{h}");
    assert!(h.contains("src/opaque.rs"), "…and exactly what was unread:\n{h}");
    assert!(
        !h.contains("PROVABLY clean (no Unknown holes) ✓"),
        "the `✓` is the prose `ok: true` and is withheld for the same reason:\n{h}"
    );
    assert!(
        !h.contains("The gate still PASSES"),
        "and so is this one, which is not merely unhedged but FALSE — `gate --report` exits 2 over \
         these bytes, so the gate does not pass:\n{h}"
    );
    // …the holes are still all NAMED. Withdrawing the claim must not withdraw the disclosure.
    assert!(h.contains("app.hole"), "the finding still ships on this channel too:\n{h}");
    let (_, hf, _) = run("human-incomplete-fg", MANIFEST, "fix-gate", "deny Net app.elsewhere\n", &[]);
    assert!(hf.contains("INCOMPLETE") && !hf.contains("no deny/pure boundary crossings in this report ✓"), "{hf}");

    // AND THE MIRROR ON THIS CHANNEL: a complete report gets both sentences back, unqualified.
    let (_, h2, _) = run("human-complete", "", "unverified", "deny Net app\n", &[]);
    assert!(h2.contains("The gate still PASSES"), "no hedge where none is owed:\n{h2}");
    assert!(!h2.contains("INCOMPLETE"), "{h2}");
    let (_, h3, _) = run("human-complete-clean", "", "unverified", "deny Net app.elsewhere\n", &[]);
    assert!(h3.contains("PROVABLY clean (no Unknown holes) ✓"), "{h3}");
    let (_, h4, _) = run("human-complete-fg", "", "fix-gate", "deny Net app.elsewhere\n", &[]);
    assert!(h4.contains("no deny/pure boundary crossings in this report ✓"), "{h4}");
}

/// SPEC §3.2 ⟨0.24⟩ **AN ADVISORY VERB'S INCOMPLETENESS VERDICT IS AT LEAST AS PESSIMISTIC AS THE
/// GATE'S OVER THE SAME BYTES** — candor-spec `93cef40` — and the `fix` half of `ec1a441`.
///
/// `93cef40` states the rule as a RELATION rather than a shape, because candor-swift and candor-ts had
/// implemented the manifest reader twice with different ELEMENT rules: **skipping a malformed element
/// makes the advisory verb read a SHORTER `unanalyzed` list than the gate reads from the identical
/// file**, so a report the gate calls incomplete gets a clean advisory answer. Asserted here as the
/// relation: over inputs the gate does not clear, no advisory verb may answer clean. In this engine it
/// holds by CONSTRUCTION — one file set, one reader, shared with `load_gate_report` — and this row is
/// what would notice if a later change gave either route its own.
///
/// `fix` is not named in `ec1a441` (it answers no `ok`), but every answer it gives is a claim over the
/// report: *"does not perform E — nothing to hoist"* rests on an effect set accumulated over the
/// callgraph, and a hoist plan names the CALLERS to move the effect to. So it carries the disclosure on
/// both channels — with its EXIT CODE unchanged at 0, matching its own gate-refusal branch (`4fd140c`)
/// and candor-ts, since a second exit policy inside one verb would rank the gate's outright refusal as
/// the milder finding.
#[test]
fn an_advisory_verb_is_never_less_sensitive_to_incompleteness_than_the_gate() {
    let f = Fixture::new("advisory-gate-bound");
    let body = |unanalyzed: &str| {
        format!(
            r#"{{"candor":{{"version":"handwritten","spec":"0.24"}},"package":"app",
                "analyzed":{{"count":2,"digest":"0"}},{unanalyzed}
                "functions":[{{"fn":"app.reader","inferred":["Fs"],"direct":["Fs"],"paths":["/x"]}},
                             {{"fn":"app.plain","inferred":[]}}]}}"#
        )
    };
    // Each row is a manifest the GATE does not clear. `[{unit,why}]` is the right shape with the wrong
    // field names — what a hand-built or foreign-produced report yields, and the exact input that
    // turned this engine's gate from exit 2 into `policy ✓` in 2026-07-28's measurement. The mixed row
    // is `93cef40`'s own case: one readable element beside one that is not.
    for (sub, unanalyzed) in [
        ("readable", r#""unanalyzed":[{"path":"src/opaque.rs","reason":"parse error"}],"#),
        ("wrongfields", r#""unanalyzed":[{"unit":"src/opaque.rs","why":"parse error"}],"#),
        ("barestrings", r#""unanalyzed":["src/opaque.rs"],"#),
        ("mixed", r#""unanalyzed":[{"path":"src/a.rs","reason":"parse error"},{"unit":"src/b.rs"}],"#),
    ] {
        let loc = gate_fixture(&f.dir, sub, &body(unanalyzed), Some(r#"{"app.reader":[],"app.plain":[]}"#));
        let p = pol(&f.dir, sub, "deny Net app.elsewhere\n");
        let go = |verb: &str| -> (i32, serde_json::Value) {
            let out = Command::new(bin())
                .args([verb, "--report", &loc, "--policy", &p.to_string_lossy(), "--json", "--strict"])
                .output()
                .expect("run candor-query");
            let so = String::from_utf8_lossy(&out.stdout).into_owned();
            (out.status.code().unwrap_or(-1), serde_json::from_str(&so).unwrap_or(serde_json::Value::Null))
        };
        let (grc, gv) = go("gate");
        assert_ne!(grc, 0, "[{sub}] the control: the gate must not clear this report:\n{gv:#}");
        for verb in ["unverified", "fix-gate"] {
            let (rc, v) = go(verb);
            assert!(
                v.get("ok").is_none(),
                "[{sub}] {verb} read a manifest the gate could not — an element that cannot be read is \
                 still an element saying something was not analysed:\n{v:#}"
            );
            assert_eq!(v["incomplete"], serde_json::json!(true), "[{sub}] {verb}:\n{v:#}");
            assert_eq!(rc, 2, "[{sub}] {verb} must not exit smaller than the gate:\n{v:#}");
        }
    }
    // …and NOTHING IS FABRICATED where nothing could be read: the `unanalyzed` key travels only when a
    // manifest was actually parsed. `incomplete` alone is what an unreadable key licenses, and stderr
    // names the key, which is the actionable half.
    let loc = gate_fixture(&f.dir, "wrongfields", &body(r#""unanalyzed":[{"unit":"x","why":"y"}],"#), None);
    let p = pol(&f.dir, "wrongfields", "deny Net app.elsewhere\n");
    let out = Command::new(bin())
        .args(["unverified", "--report", &loc, "--policy", &p.to_string_lossy(), "--json"])
        .output()
        .expect("run");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON document");
    assert!(v.get("unanalyzed").is_none(), "a manifest that could not be read is not invented:\n{v:#}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("`unanalyzed` key is PRESENT"),
        "the key is NAMED: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // ── `fix`: the disclosure reaches the plan document and the prose, and the exit code does not move.
    let loc = gate_fixture(
        &f.dir,
        "fixverb",
        &body(r#""unanalyzed":[{"path":"src/opaque.rs","reason":"parse error"}],"#),
        Some(r#"{"app.reader":[],"app.plain":[]}"#),
    );
    let p = pol(&f.dir, "fixverb", "deny Fs app\n");
    let fix = |args: &[&str]| -> (i32, String, String) {
        let mut a: Vec<String> = vec!["fix".into(), "app.reader".into(), "Fs".into(), "--report".into(), loc.clone(),
                                      "--policy".into(), p.to_string_lossy().into_owned()];
        a.extend(args.iter().map(|s| s.to_string()));
        let out = Command::new(bin()).args(&a).output().expect("run");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (rc, so, se) = fix(&["--json"]);
    let v: serde_json::Value = serde_json::from_str(&so).unwrap_or_else(|e| panic!("a plan document ({e}):\n{so}"));
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v:#}");
    assert!(v.get("deniedSpan").is_some(), "the plan is hedged, never withheld:\n{v:#}");
    assert!(
        se.contains("INCOMPLETE"),
        "…and the note is on STDERR here, because stdout is a document and prose beside it would \
         corrupt it: {se}"
    );
    assert_eq!(rc, 0, "the exit code is unchanged: this verb answers no `ok` for `--strict` to follow");
    let (_, so2, _) = fix(&[]);
    assert!(so2.contains("INCOMPLETE") && so2.contains("src/opaque.rs"), "{so2}");
    // THE MIRROR: a complete report's plan document is byte-identical to a pre-rung one.
    let loc2 = gate_fixture(&f.dir, "fixverb-complete", &body(""), Some(r#"{"app.reader":[],"app.plain":[]}"#));
    let out = Command::new(bin())
        .args(["fix", "app.reader", "Fs", "--report", &loc2, "--policy", &p.to_string_lossy(), "--json"])
        .output()
        .expect("run");
    let v2: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a plan document");
    assert!(v2.get("incomplete").is_none() && v2.get("unanalyzed").is_none(), "{v2:#}");
    assert!(v2.get("deniedSpan").is_some(), "{v2:#}");
}

/// SPEC §3.2 ⟨0.24⟩ **"THE SAME BYTES" MEANS THE SAME REPORT SET** — candor-spec `142740a`.
///
/// candor-java measured the at-least-as-pessimistic relation of `93cef40` and found the two sides were
/// never reading the same input: **`gate --report <prefix>` reads the report SET the locator names,
/// while every other verb read the ONE file the prefix expansion picked.** Two siblings under one
/// prefix with the manifest in the second — the gate exits 2, `unverified --strict` comes back clean.
/// The verb was not less pessimistic than the gate; it was answering a different question, and the
/// comparison silently did not apply. **A relation between two readers is only a constraint while both
/// read the same thing.**
///
/// **MEASURED HERE: A NULL RESULT.** This engine has no such split — `report_completeness` and
/// `load_entries` both go through `glob_reports`, the same function `load_gate_report` uses, so the
/// envelope is already unioned over the whole located set. java's shape is not this engine's. The row
/// exists anyway because the property is cheap to state and a later change that gave either route its
/// own locator handling would restore exactly java's defect, silently, in the one place where silence
/// is what the whole rung is about.
///
/// The MIRROR is the single-report locator: unchanged, one unit, no invented union.
#[test]
fn the_advisory_verbs_read_the_same_report_set_the_gate_does() {
    let f = Fixture::new("advisory-report-set");
    let d = f.dir.join("twin");
    std::fs::create_dir_all(&d).unwrap();
    // Sibling reports under ONE prefix. The manifest is in the SECOND — java's exact fixture, where a
    // reader that takes "the file the prefix picked" sees a complete report and answers clean.
    std::fs::write(
        d.join("report.alpha.scan.json"),
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"alpha",
            "analyzed":{"count":1,"digest":"0"},
            "functions":[{"fn":"alpha.plain","inferred":[]}]}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("report.beta.scan.json"),
        r#"{"candor":{"version":"handwritten","spec":"0.24"},"package":"beta",
            "analyzed":{"count":1,"digest":"0"},
            "unanalyzed":[{"path":"src/opaque.rs","reason":"parse error"}],
            "functions":[{"fn":"beta.plain","inferred":[]}]}"#,
    )
    .unwrap();
    let loc = d.join("report").to_string_lossy().into_owned();
    let p = pol(&f.dir, "set", "deny Net app\n");
    // `--strict` is an ADVISORY-verb flag; the gate is always strict. Passing it there yields a usage
    // error whose exit code is also 2 — a vacuous control, and how the first draft of this row passed
    // its gate assertion while comparing against an empty document.
    let go = |verb: &str, extra: &[&str]| -> (i32, serde_json::Value) {
        let mut a: Vec<String> =
            vec![verb.into(), "--report".into(), loc.clone(), "--policy".into(),
                 p.to_string_lossy().into_owned(), "--json".into()];
        a.extend(extra.iter().map(|s| s.to_string()));
        let out = Command::new(bin()).args(&a).output().expect("run candor-query");
        let so = String::from_utf8_lossy(&out.stdout).into_owned();
        (
            out.status.code().unwrap_or(-1),
            serde_json::from_str(&so).unwrap_or_else(|e| panic!("[{verb}] a JSON document ({e}):\n{so}")),
        )
    };
    let (grc, gv) = go("gate", &[]);
    assert_eq!(grc, 2, "the control: the gate reads the SET and finds the sibling's manifest:\n{gv:#}");
    assert_eq!(gv["unanalyzed"].as_array().map(Vec::len), Some(1), "…and carries it:\n{gv:#}");
    for verb in ["unverified", "fix-gate"] {
        let (rc, v) = go(verb, &["--strict"]);
        assert_eq!(
            v["unanalyzed"], gv["unanalyzed"],
            "[{verb}] the same list off the same set — a shorter one is not less certain than the gate, \
             it is a different question:\n{v:#}"
        );
        assert!(v.get("ok").is_none(), "[{verb}]:\n{v:#}");
        assert_eq!(rc, 2, "[{verb}]:\n{v:#}");
    }
    // THE MIRROR: a locator naming ONE file reads exactly that file — the union is over what the
    // locator LOCATES, never over a directory it did not name.
    let one = d.join("report.alpha.scan.json").to_string_lossy().into_owned();
    let out = Command::new(bin())
        .args(["unverified", "--report", &one, "--policy", &p.to_string_lossy(), "--json", "--strict"])
        .output()
        .expect("run");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON document");
    assert_eq!(v["ok"], serde_json::json!(true), "the sibling's manifest is not pulled in:\n{v:#}");
    assert_eq!(out.status.code().unwrap_or(-1), 0, "{v:#}");
}

#[test]
fn gains_names_which_report_judged_nothing_on_either_side() {
    // ⟨0.28⟩ family ruling on the `gains` disclosure split (rust/swift emitted `baselineIncomplete`
    // alone; java carried `baselineJudgedNothing: [paths]`; ts a bare boolean): the key is CARRIED and
    // its shape is the ARRAY OF REPORT PATHS — the 3-of-4 majority shape `judgedNothing` already has in
    // every other verb, and the one that says WHICH report asserts nothing, which the flag alone
    // cannot. A count-0 report is the standard post-⟨0.28⟩ arming artifact AND the legitimate facade
    // package, and it carries no `unanalyzed` to name itself by — so without this key the document
    // discloses "the floor is soft" with nothing for the reviewer to go look at.
    let f = Fixture::new("gainsjudged");
    let base_pre = format!("{}.base", f.prefix);
    let cur_pre = format!("{}.cur", f.prefix);
    let judged_nothing = r#"{"candor":{"version":"t","spec":"0.28"},"package":"lib","functions":[],"analyzed":{"count":0,"digest":"0"}}"#;
    let intact = r#"{"candor":{"version":"t","spec":"0.28"},"package":"lib","functions":[{"fn":"lib::f","loc":"s:1","inferred":["Fs"],"hash":"h"}],"analyzed":{"count":1,"digest":"0"}}"#;
    let base_file = format!("{base_pre}.lib.scan.json");
    let cur_file = format!("{cur_pre}.lib.scan.json");

    // Baseline judged nothing, current intact: the BASELINE side names its report.
    std::fs::write(&base_file, judged_nothing).unwrap();
    std::fs::write(&cur_file, intact).unwrap();
    let out = Command::new(bin()).args(["gains", &cur_pre, &base_pre, "--json"]).output().expect("run");
    assert_eq!(out.status.code(), Some(0), "gains stays advisory — the manifest is a disclosure, not an exit code");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["baselineIncomplete"], serde_json::json!(true), "the flag still rides: {v}");
    assert_eq!(v["baselineJudgedNothing"], serde_json::json!([base_file.clone()]),
        "the ARRAY-OF-PATHS shape, naming WHICH baseline report judged nothing: {v}");
    assert!(v.get("incomplete").is_none() && v.get("judgedNothing").is_none(),
        "the intact CURRENT side carries no hedge — the two sides fail differently and are disclosed separately: {v}");

    // The mirror, for symmetry: a judged-nothing CURRENT names its report under the current-side key.
    std::fs::write(&base_file, intact).unwrap();
    std::fs::write(&cur_file, judged_nothing).unwrap();
    let out = Command::new(bin()).args(["gains", &cur_pre, &base_pre, "--json"]).output().expect("run");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v}");
    assert_eq!(v["judgedNothing"], serde_json::json!([cur_file]),
        "the same treatment on the current side — `incomplete: true` alone cannot name the report: {v}");
    assert!(v.get("baselineJudgedNothing").is_none(), "the intact BASELINE side stays clean: {v}");

    // Control: two intact reports carry NONE of these keys (the prior rung's byte-identical guarantee).
    std::fs::write(&cur_file, intact).unwrap();
    let out = Command::new(bin()).args(["gains", &cur_pre, &base_pre, "--json"]).output().expect("run");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    for k in ["incomplete", "judgedNothing", "baselineIncomplete", "baselineJudgedNothing"] {
        assert!(v.get(k).is_none(), "an intact pair must not carry `{k}`: {v}");
    }
}

// ── SPEC §3.3.1 (3) ⟨0.28⟩ — the gate's input guard covers what the `--report` locator EXPANDS to,
// never just the token the operator typed. MEASURED before the fix (2026-08-12), all three at the
// bytes, because every spelling here also "failed" with a plausible exit code:
//
//   gate --report r --policy P --gate-json r.<crate>.scan.json
//       → exit 2, and the operator's REPORT held the armed refusal document — the guard compared the
//         sink against the raw token `r`, the loader read the token's expansion, and the sink destroyed
//         the expansion. The diagnostic then blamed the report ("failed to parse — corrupt input") —
//         the run reporting the corpse of the file it killed.
//   the discovery spelling (no --report, sink = the discovered .candor report) — destroyed identically.
//   gate … --gate-json r.<crate>.scan.callgraph.json
//       → exit 1 and a REAL VERDICT over the §2.2 sidecar: the report loads fine, the gate runs green,
//         and the pair is destroyed one half at a time AT A SUCCESS EXIT — worse than the report case.
// ──────────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn gate_json_naming_an_expanded_report_is_refused_with_the_report_intact() {
    let f = Fixture::new("gatelocator");
    f.write_report();
    let policy = f.dir.join("policy");
    std::fs::write(&policy, "deny Fs\n").unwrap();
    let report = f.report_path();
    let before = std::fs::read(&report).unwrap();

    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", policy.to_string_lossy().as_ref(),
               "--gate-json", &report])
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
        .output().expect("run candor-query");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2),
        "a sink naming one of the locator's expanded reports is refused: {stderr}");
    assert!(stderr.contains("names a file this gate reads"),
        "the refusal names the collision, not a downstream parse failure: {stderr}");
    assert_eq!(std::fs::read(&report).unwrap(), before,
        "the report's BYTES are untouched — before the fix this file held the armed refusal document");
}

#[test]
fn gate_json_naming_a_discovered_report_is_refused_with_the_report_intact() {
    // The no-`--report` spelling: the reports this gate is about to read from the discovered
    // `.candor/` are inputs just the same — nothing in argv names them at all.
    let f = Fixture::new("gatediscovery");
    let candor = f.dir.join(".candor");
    std::fs::create_dir_all(&candor).unwrap();
    let report = candor.join("report.rpt.scan.json");
    std::fs::write(&report,
        r#"{"candor":{"version":"t","spec":"0.28"},"package":"rpt","functions":[{"fn":"inner","loc":"s:1","inferred":["Fs"],"hash":"h","paths":["/x"]}]}"#).unwrap();
    let policy = f.dir.join("policy");
    std::fs::write(&policy, "deny Fs\n").unwrap();
    let before = std::fs::read(&report).unwrap();

    let out = Command::new(bin())
        .args(["gate", "--policy", "policy", "--gate-json", ".candor/report.rpt.scan.json"])
        .current_dir(&f.dir)
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
        .output().expect("run candor-query");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(2), "the discovered report is an input: {stderr}");
    assert_eq!(std::fs::read(&report).unwrap(), before,
        "the discovered report's bytes are untouched — this spelling destroyed it before the fix");
}

#[test]
fn gate_json_naming_the_reports_sidecar_is_refused_and_a_gate_json_sibling_still_gates() {
    let f = Fixture::new("gatesidecar");
    f.write_report();
    let policy = f.dir.join("policy");
    std::fs::write(&policy, "deny Fs\n").unwrap();
    let sidecar = format!("{}.rpt.scan.callgraph.json", f.prefix);
    let side_before = std::fs::read(&sidecar).unwrap();

    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", policy.to_string_lossy().as_ref(),
               "--gate-json", &sidecar])
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2),
        "the pair's other half is part of what the locator names — before the fix this exited 1, a \
         SUCCESS, with a real verdict where the callgraph belonged");
    assert_eq!(std::fs::read(&sidecar).unwrap(), side_before, "the sidecar's bytes are untouched");

    // THE CONTROL, and it is load-bearing: `<report-stem>.gate.json` is a sibling matching
    // `<stem>.*.json` — the exact file a fix that guarded "everything sharing the stem" would refuse —
    // and it is the recommended beside-the-report verdict layout. It must still gate, with a REAL
    // verdict: over-refusal here is the plausible-but-wrong fix (the ⟨0.24⟩ count-0 lesson).
    let sink = format!("{}.rpt.scan.gate.json", f.prefix);
    let report_before = std::fs::read(f.report_path()).unwrap();
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", policy.to_string_lossy().as_ref(),
               "--gate-json", &sink])
        .env_remove("CANDOR_POLICY").env_remove("CANDOR_CONFIG").env_remove("CANDOR_REPORT")
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1),
        "deny Fs over the Fs fixture is a VIOLATION verdict, never a refusal — a guard that reddens \
         the beside-the-report layout has not implemented the rule, it has broken the default");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sink).unwrap()).expect("a real verdict document");
    assert!(v.get("refused").is_none() && v["violations"].as_array().is_some_and(|a| !a.is_empty()),
        "the sink carries the verdict, not the armed placeholder: {v}");
    assert_eq!(std::fs::read(f.report_path()).unwrap(), report_before,
        "…and the reports the gate read are byte-identical after the control run");
}

// ── ⟨0.28⟩ Phase 1, Rung A: the disclosure envelope for the two pinned shapes that cannot carry it ──

/// ⟨0.32⟩ **THE CONTROL FOR THE DESCRIPTIVE/CERTIFYING BOUNDARY — WRITTEN BEFORE THE RUNG-A CHANGE AND
/// UNCHANGED BY IT.** Rung A's substitution was widened to the unread-class cause on 2026-08-24 and then
/// ruled the wrong way round for the DESCRIPTIVE verbs (they now return the data AND the caveat, see
/// [`show_and_map_return_their_result_beside_the_caveat_when_hedging`]). The direction that must NOT move
/// is this one: a verb that answers `ok` still refuses over the same bytes. Getting that wrong re-opens
/// the cardinal sin, and both arms below are pinned by conformance PARTs 62 and 67.
///
/// The fixture is the one the ruling was measured on: a report whose `excluded` names a class the
/// producing scan never opened (`peeked: false`, no `judgedElsewhere`), under a `deny` policy.
#[test]
fn the_verbs_that_answer_ok_still_refuse_over_an_unread_class() {
    let f = Fixture::new("unread-refusal-control");
    let report = r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.32"},"package":"rpt",
        "analyzed":{"count":2,"digest":"d"},
        "excluded":[{"class":"non-library-target","count":1,"peeked":false,"reason":"tests/"}],
        "functions":[
          {"fn":"inner","loc":"s:1","inferred":["Fs"],"direct":["Fs"],"hash":"h1"},
          {"fn":"outer","loc":"s:2","inferred":["Fs"],"hash":"h2","calls":["inner"]}]}"#;
    std::fs::write(format!("{}.rpt.scan.json", f.prefix), report).unwrap();
    let pol = write_policy(&f, "deny.policy", "deny Exec\n");

    // `gate --report`: exit 2, `ok: false`, `incomplete: true`. The certifying route, unchanged.
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &pol, "--json"])
        .output()
        .expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "the gate REFUSES over a class nothing opened");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false), "{v}");
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v}");

    // The `--strict` advisory verbs: exit 2, and `ok` OMITTED rather than falsified (⟨0.24⟩).
    for argv in [vec!["unverified", "--strict"], vec!["fix-gate", "--strict"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &f.prefix, "--policy", &pol, "--json"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(2),
            "{argv:?}: an advisory verb must never be LESS sensitive than the gate over the same bytes");
        let v: serde_json::Value =
            serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
        assert!(v.get("ok").is_none(), "{argv:?}: `ok` is OMITTED, never falsified: {v}");
        assert_eq!(v["incomplete"], serde_json::json!(true), "{argv:?}: {v}");
    }

    // …and the descriptive verbs over the SAME bytes hedge WITHOUT moving an exit code. This half is what
    // makes the two arms above a boundary rather than a blanket: `show`/`map` are on the other side of it.
    for argv in [vec!["show", "inner"], vec!["map"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &f.prefix, "--json"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0),
            "{argv:?}: a descriptive verb's hedge is a DISCLOSURE, not an exit code");
        let v: serde_json::Value =
            serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
        assert_eq!(v["incomplete"], serde_json::json!(true),
            "{argv:?}: the hedge must still appear over an unread class: {v}");
    }
}

/// SPEC §2 ⟨0.28⟩ (Rung A): a verb whose pinned shape cannot carry the caveat changes SHAPE over a
/// hedging report. `show` is pinned to a TOP-LEVEL ARRAY and `map`'s top level is a USER NAMESPACE
/// (module names), so neither may carry a reserved key at its root — measured before that rung,
/// `show --json` answered the bare array with no caveat anywhere and `map --json` merged the caveat keys
/// INTO the module namespace, displacing any real module that owned the name.
///
/// ⟨0.32⟩ **AND WHAT THE HEDGING DOCUMENT CONTAINS WAS RULED AGAIN ON 2026-08-25: THE RESULT *AND* THE
/// WARNING.** Rung A's "the CAVEAT DOCUMENT INSTEAD of its result document" was written when the trigger
/// was a manifest a scan had FAILED to produce. ⟨0.32⟩'s unread-class cause then armed the same hedge on
/// approximately every no-policy report — MEASURED on a two-function crate with one `tests/` dir:
///
/// ```text
///   show wrapper --json   {"incomplete": true}      the rows are GONE
///   map          --json   {"incomplete": true}      the map is GONE
/// ```
///
/// Both verbs are DESCRIPTIVE — they certify nothing, so there is no claim for a pessimism rule to
/// protect. The result now rides under `functions` / `modules` with the caveat keys flattened beside it;
/// the loud root-type change is unchanged, no reserved-key convention is needed, and no row is displaced.
/// The verbs on the OTHER side of that boundary are pinned by
/// [`the_verbs_that_answer_ok_still_refuse_over_an_unread_class`] and must not move.
///
/// Healthy output is pinned byte-shaped by the control arm (byte-level identity vs the pre-fix binary
/// was verified out of band, on both verbs).
#[test]
fn show_and_map_return_their_result_beside_the_caveat_when_hedging() {
    // Cause 1: a non-empty `unanalyzed` manifest.
    let f = Fixture::new("runga-unanalyzed");
    let report = r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.28"},"package":"rpt",
        "analyzed":{"count":2,"digest":"d"},
        "unanalyzed":[{"path":"src/gen.rs","reason":"parse error"}],
        "functions":[
          {"fn":"inner","loc":"s:1","inferred":["Fs"],"direct":["Fs"],"hash":"h1"},
          {"fn":"outer","loc":"s:2","inferred":["Fs"],"hash":"h2","calls":["inner"]}]}"#;
    std::fs::write(format!("{}.rpt.scan.json", f.prefix), report).unwrap();
    // Cause 2: a report that judged nothing (`analyzed.count: 0`, the standard post-failure artifact).
    let j = Fixture::new("runga-judged");
    let judged = r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.28"},"package":"lib","functions":[],"analyzed":{"count":0,"digest":"0"}}"#;
    let judged_file = format!("{}.lib.scan.json", j.prefix);
    std::fs::write(&judged_file, judged).unwrap();

    // `show` hedging: an OBJECT rather than the array (the loud stop), AND the rows are still in it.
    let out = Command::new(bin()).args(["show", "inner", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "the hedge is a disclosure, not an exit code");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.is_object(),
        "show hedging must not answer the bare array — an array here is the pre-⟨0.28⟩ silent wrong \
         answer, and the root type change is the loud stop: {v}");
    assert_eq!(v["incomplete"], serde_json::json!(true));
    assert_eq!(v["unanalyzed"][0]["path"], serde_json::json!("src/gen.rs"));
    // ⟨0.32⟩ THE DEFECT ASSERTION, and it is the whole of this rung: the ANSWER is still here. The rows
    // must be the SAME rows the healthy arm would have printed — a hedge that ships an empty `functions`
    // passes "the key exists" while deleting exactly what the ruling restored.
    assert_eq!(v["functions"][0]["fn"], serde_json::json!("inner"),
        "the result travels BESIDE the warning, never instead of it: {v}");
    assert_eq!(v["functions"].as_array().map(Vec::len), Some(1), "{v}");

    // `map` hedging over the judged-nothing report: the caveat, and NOT a bare `{}` — the strongest
    // determined negative there is, asserted about code nobody examined. Here the map is legitimately
    // empty (the report judged nothing), so `modules` is `{}` INSIDE the hedged envelope: a consumer
    // reading `doc.modules` gets the same empty map the report supports, and `doc.incomplete` tells it
    // why. The two are distinguishable, which is the property `{}` at the root destroyed.
    let out = Command::new(bin()).args(["map", "--report", &j.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true), "map hedging carries the caveat: {v}");
    assert_eq!(v["judgedNothing"], serde_json::json!([judged_file]));
    assert!(v["modules"].is_object(), "…and the (empty) map is present, not withheld: {v}");

    // …and over a report that DID judge something, `map` hedging carries the module rows. They live one
    // level down, under `modules`, which is what makes the caveat keys safe at the root: `map`'s own
    // namespace is the operator's module names, and a module named `incomplete` is now a key of
    // `modules` while the boolean is a key of the root. Neither can displace the other.
    let out = Command::new(bin()).args(["map", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true), "{v}");
    let mods = v["modules"].as_object().expect("the module map rides beside the caveat");
    assert!(!mods.is_empty(), "map hedging must still ANSWER — the rows are the thing: {v}");
    assert!(v.as_object().unwrap().keys().all(|k| ["modules", "incomplete", "unanalyzed",
        "judgedNothing", "noManifest", "unread"].contains(&k.as_str())),
        "…and the ROOT carries only `modules` + the ⟨0.28⟩ caveat vocabulary — a module name at the root \
         is the merged shape whose collision the nesting removes: {v}");

    // INTACT-INPUT CONTROL: a healthy report keeps the pinned shapes exactly — `show` a top-level ARRAY,
    // `map` an object of module rows with no caveat key.
    let h = Fixture::new("runga-healthy");
    h.write_report();
    let out = Command::new(bin()).args(["show", "inner", "--report", &h.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.is_array() && v[0]["fn"] == serde_json::json!("inner"),
        "healthy show keeps its pinned top-level array: {v}");
    let out = Command::new(bin()).args(["map", "--report", &h.prefix, "--json"])
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.get("incomplete").is_none() && v.as_object().is_some_and(|o| !o.is_empty()),
        "healthy map keeps its module rows and gains no caveat key: {v}");
}

/// ⟨0.32⟩ **THE SILENT HALF OF THE SAME CLASS: `callers`, `impact` AND `path` CARRIED NO COMPLETENESS
/// READER AT ALL.** `show`/`map` OVER-hedged (the caveat replaced the data, fixed by
/// [`show_and_map_return_their_result_beside_the_caveat_when_hedging`]); these three UNDER-hedged — over
/// a report whose own `excluded` names a class the producing scan never opened they answered FLAT, at
/// exit 0, with no disclosure on the machine channel and none on the human one.
///
/// MEASURED at HEAD on a three-function crate with one `tests/` dir (`excluded: [{class:
/// "non-library-target", peeked: false}]`), scanned with no policy — reproduced identically in
/// candor-ts and candor-java:
///
/// ```text
///   callers wrapper --json   {"of":[…],"direct":["top"],"transitive":["top"]}   exit 0   no caveat
///   impact  wrapper --json   {"fn":…,"affectedCount":1,"affected":["top"],…}    exit 0   no caveat
///   path    top Fs  --json   {"fn":…,"effect":"Fs","path":[…3 steps…]}          exit 0   no caveat
/// ```
///
/// A user asks who calls a function, gets an answer, and is never told part of the codebase went unread.
/// An empty `direct` over the same bytes reads as *nothing calls this* — the determined negative SPEC §2
/// ⟨0.28⟩ binds ("a verdict, an empty RESULT SET, or a zero count"), on the three verbs the rung's own
/// enumeration skipped.
///
/// **THE REMEDY IS THE ONE THE HEDGING VERBS ALREADY USE, NOT A FOURTH SPELLING.** All three have a FIXED
/// key set at their root — `{of,direct,transitive}`, `{fn,affectedCount,affected,entryPoints}`,
/// `{fn,effect,path}` — so unlike `show`/`map` there is nothing to nest: they take
/// [`crate::completeness::ReportCompleteness::write_json`] and `print_note`, exactly as `where`,
/// `reachable`, `blindspots` and `containment` do. `must_hedge()` is the trigger, `incomplete()` and every
/// exit code are untouched, and the boundary is unmoved — the verbs that answer `ok` still REFUSE over
/// these same bytes ([`the_verbs_that_answer_ok_still_refuse_over_an_unread_class`], conformance PARTs 62
/// and 67).
///
/// The assertions are on the ROW NAMES and COUNTS, never on key presence: a hedge that shipped an empty
/// `direct`/`affected`/`path` would pass "the key exists" while deleting the answer this rung exists to
/// keep.
#[test]
fn callers_impact_and_path_disclose_an_unread_class_beside_their_answer() {
    let f = Fixture::new("cip-unread");
    // Three functions in a chain (top -> wrapper -> inner), inner is the Fs source. `excluded` names one
    // class with `peeked: false` and no `judgedElsewhere` — ⟨0.32⟩'s unread-class cause, the shape a bare
    // `candor-scan <crate-with-tests/> --out r` publishes.
    let report = r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.32"},"package":"rpt",
        "analyzed":{"count":3,"digest":"d"},
        "excluded":[{"class":"non-library-target","count":1,"peeked":false,"reason":"tests/"}],
        "functions":[
          {"fn":"inner","loc":"s:1","inferred":["Fs"],"direct":["Fs"],"hash":"h1"},
          {"fn":"wrapper","loc":"s:2","inferred":["Fs"],"hash":"h2","calls":["inner"]},
          {"fn":"top","loc":"s:3","inferred":["Fs"],"hash":"h3","calls":["wrapper"]}]}"#;
    std::fs::write(format!("{}.rpt.scan.json", f.prefix), report).unwrap();
    // The COMPLETE-graph sidecar, so `callers` takes its sidecar arm rather than the effect-only
    // fallback — the arm a real scan produces, and the one whose miss is a definitive negative.
    std::fs::write(format!("{}.rpt.scan.callgraph.json", f.prefix),
        r#"{"inner":[],"wrapper":["inner"],"top":["wrapper"]}"#).unwrap();

    // ── callers: the caveat rides at the root, and the reachers are STILL THERE ──────────────────────
    let out = Command::new(bin()).args(["callers", "wrapper", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "a descriptive verb's hedge is a DISCLOSURE, not an exit code (⟨0.24⟩)");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "callers answered flat over a class nothing opened: {v}");
    assert_eq!(v["direct"], serde_json::json!(["top"]),
        "…BESIDE the answer, never instead of it — the direct callers by NAME: {v}");
    assert_eq!(v["transitive"], serde_json::json!(["top"]), "{v}");
    assert_eq!(v["of"], serde_json::json!(["wrapper"]), "{v}");

    // The `--include-unknown` arm is a SECOND function in this engine (`callers_via_callgraph_frontier`),
    // and the file's own comment records that the last fix to this pair had to be applied twice. Driven,
    // not assumed.
    let out = Command::new(bin())
        .args(["callers", "wrapper", "--report", &f.prefix, "--json", "--include-unknown"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "the frontier arm is the OTHER site of the same verb: {v}");
    assert_eq!(v["direct"], serde_json::json!(["top"]), "{v}");

    // ── impact ──────────────────────────────────────────────────────────────────────────────────────
    let out = Command::new(bin()).args(["impact", "wrapper", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true), "impact answered flat: {v}");
    assert_eq!(v["affected"], serde_json::json!(["top"]),
        "…and the blast radius survives the hedge, by NAME: {v}");
    assert_eq!(v["affectedCount"], serde_json::json!(1), "{v}");
    assert_eq!(v["fn"], serde_json::json!("wrapper"), "{v}");

    // ── path ────────────────────────────────────────────────────────────────────────────────────────
    let out = Command::new(bin()).args(["path", "top", "Fs", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true), "path answered flat: {v}");
    let steps: Vec<&str> =
        v["path"].as_array().expect("the chain rides beside the caveat")
            .iter().map(|s| s["fn"].as_str().unwrap()).collect();
    assert_eq!(steps, ["top", "wrapper", "inner"],
        "…and the WHOLE chain is still there, in order: {v}");

    // `path`'s EMPTY-answer arm takes the same route. `{"path": []}` is the determined negative this rung
    // is about — *this function does not reach that effect* — and over an unread class it is exactly the
    // claim the report cannot support. Asserted separately because it is a different emit site.
    let out = Command::new(bin()).args(["path", "top", "Net", "--report", &f.prefix, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "an empty `path` over an unread class is a determined negative and must hedge: {v}");
    assert_eq!(v["path"], serde_json::json!([]), "{v}");

    // ── THE HUMAN CHANNEL, because a mutant that keeps the JSON fix and deletes the printed line
    //    survives an absence-assert on the document (candor-spec `ec1a441`, measured on this engine).
    for argv in [vec!["callers", "wrapper"], vec!["impact", "wrapper"], vec!["path", "top", "Fs"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &f.prefix]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.contains("INCOMPLETE") && text.contains("non-library-target"),
            "{argv:?}: the prose channel must carry the note AND name the class: {text}");
    }

    // ── THE INTACT CONTROL: nothing unread ⇒ NO hedge, and the pinned shape is untouched. Without this
    //    the rows above pass just as well from a verb that hedges unconditionally, which would make every
    //    ordinary answer read as partial — the disclosure discrediting itself.
    let h = Fixture::new("cip-healthy");
    h.write_report();
    for argv in [vec!["callers", "inner"], vec!["impact", "inner"], vec!["path", "outer", "Fs"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &h.prefix, "--json"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0));
        let v: serde_json::Value =
            serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
        assert!(v.get("incomplete").is_none(),
            "{argv:?}: a complete report gains NO caveat key: {v}");
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(keys.iter().all(|k| ["of", "direct", "transitive", "possibleViaUnknownDispatch",
                                     "fn", "affectedCount", "affected", "entryPoints", "effect",
                                     "path"].contains(k)),
            "{argv:?}: the healthy document keeps its pinned key set exactly: {keys:?}");
    }
    // …and on the human channel the healthy run prints no note either.
    for argv in [vec!["callers", "inner"], vec!["impact", "inner"], vec!["path", "outer", "Fs"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &h.prefix]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(!text.contains("INCOMPLETE"), "{argv:?}: healthy prose gains no note: {text}");
    }
}

/// SPEC §2 ⟨0.28⟩: an ADVISORY verb over a CONFIGURED policy that parsed to ZERO RULES answers with
/// the caveat document — result keys withheld, exit UNCHANGED. Measured before the fix: `whatif`,
/// `fix-gate` and `unverified` all answered `{"ok": true, …}` at exit 0 over `# no rules yet` — the
/// wire form of a `✓` from a gate that asked nothing, indistinguishable from a real all-clear. The
/// caveat is §3.1's `unevaluated` with the whole-policy entry, the same spelling both gate routes put
/// on their zero-rule REFUSAL — which stays exit 2 and now carries that entry on this route too
/// (§6.2's "one entry naming the whole policy"; this route's refusal used to carry none).
#[test]
fn advisory_verbs_answer_a_zero_rule_policy_with_the_caveat_document_at_an_unchanged_exit() {
    let f = Fixture::new("zerorule");
    f.write_report();
    let zero = write_policy(&f, "zero.policy", "# no rules yet\n");
    let entry_rule = format!("(entire policy {zero} — no rules parsed)");

    for argv in [
        vec!["whatif", "inner", "Net"],
        vec!["fix", "inner", "Fs"],
        vec!["fix-gate"],
        vec!["fix-gate", "--strict"],
        vec!["unverified"],
        vec!["unverified", "--strict"],
    ] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &f.prefix, "--policy", &zero, "--json"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0),
            "{argv:?}: the caveat is a disclosure, not an exit code — exit UNCHANGED (0)");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap())
            .unwrap_or_else(|_| panic!("{argv:?} must emit a JSON caveat document on stdout"));
        assert_eq!(v["unevaluated"][0]["rule"], serde_json::json!(entry_rule),
            "{argv:?}: the whole-policy entry, the gate routes' own spelling: {v}");
        for withheld in ["ok", "violations", "unverified", "remedies", "affected", "crossing"] {
            assert!(v.get(withheld).is_none(),
                "{argv:?}: result key `{withheld}` must be WITHHELD over a policy that asked nothing \
                 — an empty result is the prose ✓ in wire form: {v}");
        }
    }

    // The GATE over the same policy: refusal (exit 2) whose document now carries the same entry.
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &zero, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2), "the gate REFUSES a zero-rule policy");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["refused"], serde_json::json!(true));
    assert_eq!(v["unevaluated"][0]["rule"], serde_json::json!(entry_rule),
        "§6.2: the refusal's `unevaluated` carries one entry naming the whole policy — this route \
         used to write the refusal with no `unevaluated` at all: {v}");

    // INTACT-INPUT CONTROL: a policy with a real rule keeps every result key and gains no caveat.
    let real = write_policy(&f, "real.policy", "deny Net app\n");
    for argv in [vec!["whatif", "inner", "Net"], vec!["fix-gate"], vec!["unverified"]] {
        let mut args: Vec<&str> = argv.clone();
        args.extend(["--report", &f.prefix, "--policy", &real, "--json"]);
        let out = Command::new(bin()).args(&args).output().expect("run candor-query");
        assert_eq!(out.status.code(), Some(0), "{argv:?}: control exits 0");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
        assert!(v.get("ok").is_some(), "{argv:?}: control keeps `ok`: {v}");
        assert!(v.get("unevaluated").is_none(), "{argv:?}: control gains no caveat: {v}");
    }
}

/// SPEC §3.1 ⟨0.28⟩ pins `crossing` on `fix`: a boolean PRESENT EXACTLY WHEN THE VERB ANSWERED —
/// `true` beside a plan, `false` with a `reason` on the determined-negative arms, ABSENT when the verb
/// refused. Measured before the fix: this engine emitted no such key, and answered the no-crossing arm
/// as PROSE ON STDOUT under `--json` ("…the boundary isn't crossed, nothing to fix.") — the §3.3.1
/// purity violation the ruling exists to close ("stdout MUST then be pure JSON").
#[test]
fn fix_json_pins_crossing_present_iff_answered_and_stdout_stays_pure_json() {
    let f = Fixture::new("crossing");
    f.write_report();
    let deny = write_policy(&f, "deny.policy", "deny Fs\n");
    let other = write_policy(&f, "other.policy", "deny Net elsewhere\n");

    // (1) ANSWERED, crossing found: `crossing: true` beside the plan.
    let out = Command::new(bin())
        .args(["fix", "outer", "Fs", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["crossing"], serde_json::json!(true), "a plan carries crossing: true: {v}");
    assert!(v.get("deniedSpan").is_some(), "…beside the plan, not instead of it: {v}");

    // (2) ANSWERED, no crossing: a DOCUMENT (not prose) with crossing:false + the pinned reason.
    let out = Command::new(bin())
        .args(["fix", "inner", "Fs", "--report", &f.prefix, "--policy", &other, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "the no-crossing arm's exit is unchanged (0)");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|_| panic!(
        "the no-crossing arm printed PROSE on a --json stdout (the §3.3.1 purity violation this \
         ruling closes): {stdout}"));
    assert_eq!(v["crossing"], serde_json::json!(false));
    assert_eq!(v["reason"], serde_json::json!("not-forbidden"),
        "the ts/swift reason token on the false arm: {v}");

    // (3) ANSWERED, target does not perform the effect: the other false arm and its reason token.
    let out = Command::new(bin())
        .args(["fix", "inner", "Net", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["crossing"], serde_json::json!(false));
    assert_eq!(v["reason"], serde_json::json!("does-not-perform"), "{v}");

    // (4) REFUSED (an unanswerable narrowed rule): `crossing` is ABSENT — the MCP contract's
    // check-`refused`-first ordering depends on the key not existing here.
    let g = Fixture::new("crossing-refused");
    std::fs::write(format!("{}.app.scan.json", g.prefix),
        r#"{"candor":{"version":"t","toolchain":"stable","spec":"0.28"},"package":"app",
            "analyzed":{"count":1,"digest":"x"},
            "functions":[{"fn":"app::noClass","loc":"s:1","inferred":["Net"],"direct":["Net"],"hosts":["h.example"],"hash":"a#n"}]}"#).unwrap();
    let narrow = write_policy(&g, "narrow.policy", "deny Net[unknown-host] app\n");
    let out = Command::new(bin())
        .args(["fix", "app::noClass", "Net", "--report", &g.prefix, "--policy", &narrow, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.get("crossing").is_none(),
        "a refused fix must carry NO crossing key — neither boolean is a statement there: {v}");
    assert!(v.get("unevaluated").is_some(), "…and says which rule stopped it: {v}");

    // (5) fix-gate's remedies do NOT gain the key: §3.1 pins their shape separately without it.
    let out = Command::new(bin())
        .args(["fix-gate", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let remedies = v["remedies"].as_array().expect("deny Fs over the Fs fixture yields a remedy");
    assert!(!remedies.is_empty() && remedies.iter().all(|r| r.get("crossing").is_none()),
        "`crossing` is `fix`'s key — a remedies entry must not gain it: {v}");
}

/// SPEC §6.2 ⟨0.28⟩: the VERDICT document carries `ignored: [{line, text, reason}]` — the policy
/// lines the parse DROPPED — omitted when nothing was dropped. Distinct from `unevaluated` (rules
/// that PARSED and could not be answered): a consumer that sees neither is entitled to believe the
/// policy on disk is the policy that ran. Measured before the fix: all engines warned per dropped
/// line on stderr while the verdict said nothing — a 9-of-10-dropped policy was a 90%-gateless green.
/// `ok` and the exit code do not consult it (line-level leniency unchanged).
#[test]
fn gate_verdict_documents_carry_the_dropped_policy_lines_as_ignored() {
    let f = Fixture::new("ignored");
    f.write_report();
    let dropped = write_policy(&f, "dropped.policy",
        "# a comment\ndeny Fs\nfrobnicate the walrus  # typo\nforbid glued->arrow\n");

    // `gate --report … --json`: the verdict names each dropped line with its 1-based source line and
    // the VERBATIM text (comment included — the operator matches it against their file).
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &dropped, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1),
        "deny Fs still fires — a dropped line never changes ok or the exit");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let ig = v["ignored"].as_array().unwrap_or_else(|| panic!(
        "the verdict must carry the dropped lines as `ignored` — stderr is not the machine channel: {v}"));
    assert_eq!(ig.len(), 2, "two dropped lines, the comment is not one: {v}");
    assert_eq!(ig[0]["line"], serde_json::json!(3));
    assert_eq!(ig[0]["text"], serde_json::json!("frobnicate the walrus  # typo"),
        "`text` is the source line VERBATIM, before comment-stripping: {v}");
    assert!(ig[0]["reason"].as_str().unwrap().contains("unknown rule kind"), "{v}");
    assert_eq!(ig[1]["line"], serde_json::json!(4));
    assert!(!v["violations"].as_array().unwrap().is_empty(),
        "the firing rule still fires beside the disclosure");

    // gate-verdict --policy: the assembled (lint-route) verdict carries the same disclosure.
    let parts = f.dir.join("parts.ndjson");
    std::fs::write(&parts, "{\"rule\":\"AS-EFF-006\",\"fn\":\"f\",\"effects\":[\"Net\"],\"detail\":\"d\"}\n").unwrap();
    let out = Command::new(bin())
        .args(["gate-verdict", parts.to_string_lossy().as_ref(), "-", "--policy", &dropped])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ignored"].as_array().map(Vec::len), Some(2),
        "the assembled verdict route is not covered by its sibling: {v}");

    // CONTROL: a clean policy's verdict has NO `ignored` key on either route (byte-level identity vs
    // the pre-change binary was verified out of band; this pins the key's absence).
    let clean = write_policy(&f, "clean.policy", "deny Fs\n");
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &clean, "--json"])
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.get("ignored").is_none(), "a clean policy's verdict stays byte-identical: {v}");
    let out = Command::new(bin())
        .args(["gate-verdict", parts.to_string_lossy().as_ref(), "-", "--policy", &clean])
        .output().expect("run candor-query");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v.get("ignored").is_none(), "{v}");
}

/// Write a report whose envelope carries an `excluded` set, with `outOfScope` PRESENT or ABSENT.
/// `outOfScope` absent is what a scan run with NO policy writes (⟨0.29⟩): nothing was asked, so the
/// producer withholds the key — while `excluded[].peeked` is `false` on every class for that same
/// reason. Both spellings are real producer output; `candor-scan <dir> --out r` writes the second.
fn write_excluded_report(f: &Fixture, excluded: &str, out_of_scope: Option<&str>) {
    write_excluded_report_under(f, excluded, out_of_scope, None)
}

/// ⟨0.33⟩ [`write_excluded_report`] plus an explicit `scannedUnder.deny` — the canonical rules THIS
/// hand-authored report's producer HELD, as a raw JSON array fragment (e.g. `r#"["deny Exec"]"#`).
/// `None` omits the key entirely, which is the pre-⟨0.33⟩ shape every existing caller of the sibling
/// function keeps writing: `outOfScope` PRESENT with NO `scannedUnder` beside it is exactly the report
/// SPEC §2 ⟨0.33⟩ says must fail closed once a class comes back `peeked: true` — an absent
/// `scannedUnder` is the EMPTY SET for the subset test, never a licence.
fn write_excluded_report_under(
    f: &Fixture,
    excluded: &str,
    out_of_scope: Option<&str>,
    scanned_under: Option<&str>,
) {
    let oos = out_of_scope.map(|o| format!("\"outOfScope\": {o},\n  ")).unwrap_or_default();
    let su = scanned_under
        .map(|d| format!("\"scannedUnder\": {{ \"deny\": {d} }},\n  "))
        .unwrap_or_default();
    let report = format!(
        r#"{{
  "candor": {{ "version": "scan-test", "toolchain": "stable", "spec": "0.32" }},
  "package": "rpt",
  "analyzed": {{ "count": 1, "digest": "0000000000000000" }},
  "excluded": {excluded},
  {oos}{su}"functions": [
    {{ "fn": "inner", "loc": "src/lib.rs:2:1", "inferred": ["Fs"], "direct": ["Fs"], "hash": "rpt#inner", "paths": ["/x"] }}
  ]
}}"#
    );
    std::fs::write(format!("{}.rpt.scan.json", f.prefix), report).unwrap();
    std::fs::write(format!("{}.rpt.scan.callgraph.json", f.prefix), r#"{"inner":[]}"#).unwrap();
}

/// ⟨0.32⟩ A REPORT WRITTEN BY A SCAN THAT WAS NEVER ASKED CANNOT CERTIFY A DENY RULE — and the
/// ROUTE SPLIT this pins was a VERIFIED FAIL-OPEN, measured 2026-08-24 on ~/.cargo/registry:
///
///   candor-scan <crate> --out A --policy <deny Exec>   → exit 2, naming the build.rs fns spawning rustc
///   candor-scan <crate> --out B                        (no policy)
///   candor-query gate --report B --policy <deny Exec>  → exit 0, `policy ✓`, `ok: true`, NO disclosure
///
/// 90 of 795 crate×policy pairs went 2 → 0 that way. The mechanism is one conjunct: the unread-class
/// rule was gated on `outOfScope` being PRESENT, and ⟨0.29⟩ omits that key precisely when no policy was
/// configured — so the whole rule was skipped in exactly the case it exists for. The report DOES carry
/// `excluded[].peeked: false`, which is the same evidence the scan route acts on; this route simply
/// never looked at it. "Nothing to peek at" and "never looked" were being read as one answer.
///
/// The three rows are the rule and its two controls, and the controls are the point: `peeked: false`
/// has two causes and only the QUESTION decides whether it bites.
#[test]
fn gate_report_refuses_a_deny_rule_over_classes_the_producer_never_read() {
    let f = Fixture::new("unpeeked-noask");
    let deny = write_policy(&f, "deny.policy", "deny Exec\n");

    // (1) THE DEFECT. A no-policy report: `outOfScope` ABSENT, every class `peeked: false`.
    write_excluded_report(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": false, "reason": "compile time" }]"#,
        None,
    );
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2),
        "a deny rule certified over a class NOBODY READ is the fail-open this row exists for — the \
         producer's silence about the QUESTION is not an answer about the CODE. stderr: {err}");
    assert!(err.contains("did not READ") && err.contains("build-script"),
        "the refusal must NAME the unread class, or the operator cannot repair it: {err}");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false), "the DOCUMENT must agree with the exit: {v}");
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "⟨0.32⟩ unread code makes the verdict INCOMPLETE, and a machine consumer reads that key, not \
         the exit code: {v}");

    // (2) CONTROL — THE OVER-CHARGE SIDE. The same never-asked report under a policy carrying NO deny
    // rule. Nothing this policy asks depends on code outside the scan's scope, so an unread class must
    // not cost it anything: `forbid` is refused HERE for answerability (§3.1), never for want of a peek,
    // and the message is what tells the two apart.
    let forbid = write_policy(&f, "forbid.policy", "forbid app -> infra\n");
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &forbid])
        .output().expect("run candor-query");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!err.contains("did not READ"),
        "a policy with no deny rule must not be refused for an unread class — that is the over-charge \
         the scan route measured and carved out with `peek_attempted`: {err}");

    // (3) CONTROL — NOTHING WAS EXCLUDED. A no-policy report over a tree with no exclusions at all has
    // no hole to disclose, and must still gate green. Without this row the fix above is satisfied by
    // refusing every report a bare scan ever wrote.
    write_excluded_report(&f, "[]", None);
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "an empty `excluded` is \"I excluded nothing\" — there is nothing unread to refuse over. \
         stderr: {}", String::from_utf8_lossy(&out.stderr));

    // (4) CONTROL — THE PRODUCER LOOKED, UNDER THE SAME POLICY THIS GATE HOLDS. `peeked: true` beside
    // `outOfScope: []` AND a `scannedUnder` covering `deny Exec` is the asked-and-clear answer, and it
    // certifies. ⟨0.33⟩ SPEC §2: a `scannedUnder` is now required for that claim — without it the same
    // bytes are the pre-⟨0.33⟩ shape the rung fails closed over (see the row below).
    write_excluded_report_under(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }]"#,
        Some("[]"),
        Some(r#"["deny Exec"]"#),
    );
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "a class the producer READ and found clean, under THE SAME deny set this gate holds, must not \
         be re-charged as unread: {}",
        String::from_utf8_lossy(&out.stderr));

    // (4b) ⟨0.33⟩ THE SAME BYTES MINUS `scannedUnder` — the pre-rung shape, and the rung's whole point:
    // `peeked: true` is relative to a deny set the document no longer names, so this MUST now refuse
    // rather than certify (SPEC §2 ⟨0.33⟩, "an absent `scannedUnder` is the empty set for this test").
    write_excluded_report(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }]"#,
        Some("[]"),
    );
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2),
        "a peeked class with NO `scannedUnder` answers a question the document no longer names — SPEC \
         §2 ⟨0.33⟩ fails this closed rather than trusting a pre-rung producer's silence: {}",
        String::from_utf8_lossy(&out.stderr));

    // (5) THE `judgedElsewhere` CARVE-OUT still stands on this route — a DERIVED copy of already-judged
    // code is not unread code.
    write_excluded_report(
        &f,
        r#"[{ "class": "build-output", "count": 1, "peeked": false, "judgedElsewhere": true, "reason": "derived" }]"#,
        None,
    );
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "`judgedElsewhere` is the producer's statement that this class's SOURCE was judged; refusing \
         over it double-charges one body of code: {}", String::from_utf8_lossy(&out.stderr));
}

/// ⟨0.32⟩ …and `excluded` is read STRICTLY, like every other verdict-bearing §2 key on this route.
/// Present-but-unparseable coerced to `[]` is the claim "this scan excluded nothing" — the
/// safe-LOOKING value — and it deletes the rule above wholesale. ABSENT stays permissive: a
/// pre-⟨0.29⟩ report has no `excluded` key, and refusing over one would refuse every report an older
/// producer ever wrote.
#[test]
fn gate_report_refuses_a_corrupt_excluded_key_and_tolerates_an_absent_one() {
    let f = Fixture::new("unpeeked-corrupt");
    let deny = write_policy(&f, "deny.policy", "deny Exec\n");

    write_excluded_report(&f, r#""oops""#, None);
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny])
        .output().expect("run candor-query");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "a corrupt key is corrupt input, never its empty value: {err}");
    assert!(err.contains("`excluded`"), "the refusal must NAME the key it could not read: {err}");

    // CONTROL: the pre-rung report — no `excluded` key at all — still gates.
    f.write_report();
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "ABSENT is ⟨0.26⟩'s cannot-answer and takes the documented default — refusing here would \
         refuse every pre-⟨0.29⟩ report: {}", String::from_utf8_lossy(&out.stderr));
}

/// ⟨0.33⟩ **A GATE MUST NOT ANSWER FROM A PEEK THAT WAS PUT A DIFFERENT QUESTION** (SPEC §2 ⟨0.33⟩).
///
/// THE HOLE ⟨0.32⟩ LEFT: `excluded[].peeked: true` says the peek OPENED those files, so ⟨0.32⟩'s
/// unread-class rule correctly does not fire — but ⟨0.29⟩ bounds the peek to effects the PRODUCER's
/// policy denies, so a class read under `deny Net` says nothing about `Exec` in those same files. A
/// consumer gating with `deny Exec` got a definite `outOfScope: []` answer to a question nobody asked.
///
/// `scannedUnder` closes it: the report now records the deny set the peek was bounded by, so a
/// consumer whose OWN deny set is not covered by it can tell.
#[test]
fn gate_report_refuses_a_peek_bounded_by_a_different_deny_set() {
    let f = Fixture::new("cross-policy");
    let net = write_policy(&f, "net.policy", "deny Net\n");
    let exec = write_policy(&f, "exec.policy", "deny Exec\n");
    let both = write_policy(&f, "both.policy", "deny Net\ndeny Exec\n");

    // THE DEFECT: scanned under `deny Net` (peeked, clean), gated under `deny Exec` — a DIFFERENT
    // question the producer's peek never asked.
    write_excluded_report_under(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }]"#,
        Some("[]"),
        Some(r#"["deny Net"]"#),
    );
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &exec, "--json"])
        .output().expect("run candor-query");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2),
        "a peek bounded by `deny Net` answers nothing about `deny Exec` in the same excluded files — \
         certifying here is the ⟨0.33⟩ fail-open. stderr: {err}");
    assert!(err.contains("deny Exec"),
        "the refusal must NAME the unasked rule, or the operator cannot repair it: {err}");
    assert!(err.contains("THE SAME") || err.to_lowercase().contains("same policy"),
        "SPEC §2 ⟨0.33⟩: the remedy must say re-scan under THE SAME policy, not merely A policy — a \
         loose remedy is what produces this hole in the first place: {err}");
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["ok"], serde_json::json!(false), "the DOCUMENT must agree with the exit: {v}");
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "⟨0.33⟩ a cross-policy peek makes the verdict INCOMPLETE, and a machine consumer reads that \
         key, not the exit code: {v}");

    // CONTROL 1 — THE SAME POLICY BOTH ROUTES: gated under the SAME `deny Net` the producer scanned
    // under must still certify, or the scan-then-gate pipeline this rung exists to protect cannot pass.
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &net])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "the SAME policy the producer scanned under must certify — if this reddens the rung is \
         unusable: {}", String::from_utf8_lossy(&out.stderr));

    // CONTROL 2 — THE CONSUMER'S RULES A STRICT SUBSET OF THE PRODUCER'S. A producer that scanned under
    // `deny Net` AND `deny Exec` fully answers a consumer asking only `deny Net` — the control that
    // proves the key is a RULE SET, not a digest (a digest could only test equality and would refuse a
    // narrowing at the same implementation cost).
    write_excluded_report_under(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }]"#,
        Some("[]"),
        Some(r#"["deny Exec", "deny Net"]"#),
    );
    let _ = &both; // the policy text is not re-parsed here; the fixture's `scannedUnder` states its effect
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &net])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "a producer that held `deny Net` AND `deny Exec` fully answers a consumer asking only `deny \
         Net` — refusing this is the loud over-charge a careless implementation ships: {}",
        String::from_utf8_lossy(&out.stderr));

    // CONTROL 3 — NO PEEKED CLASS AT ALL, POLICIES DIFFER. Analysed code's effect sets are
    // POLICY-INDEPENDENT; only the peek was ever bounded. Refusing here would redden every
    // scan-then-gate pipeline in the family, which is why this control is written FIRST in the design.
    write_excluded_report(&f, "[]", None);
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &exec])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0),
        "no `peeked: true` class at all means nothing was ever bounded by a deny set — a policy \
         mismatch cannot matter here: {}", String::from_utf8_lossy(&out.stderr));

    // CONTROL 4 (the `pure` flattening trap, ⟨0.30⟩ one layer out): `pure` is a deny rule with an EMPTY
    // effect list. An engine deciding coverage by flattening rules to effect NAMES gets nothing from it,
    // so the STRICTEST policy in the grammar would compare equal to the empty set and certify past a
    // peek that never asked its question. THE ANALYSED FUNCTION MUST ITSELF BE PURE here: violation (1)
    // outranks incomplete (2) in this gate's precedence, so an effectful `inner` would dominate with a
    // REAL finding and the row would measure nothing about `pure`'s flattening. Written by hand rather
    // than through `write_excluded_report_under` (whose `inner` always performs `Fs`) for that reason.
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.32" },
  "package": "rpt",
  "analyzed": { "count": 1, "digest": "0000000000000000" },
  "excluded": [{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }],
  "outOfScope": [],
  "scannedUnder": { "deny": ["deny Net"] },
  "functions": []
}"#;
    std::fs::write(format!("{}.rpt.scan.json", f.prefix), report).unwrap();
    std::fs::write(format!("{}.rpt.scan.callgraph.json", f.prefix), "{}").unwrap();
    let pure = write_policy(&f, "pure.policy", "pure\n");
    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &pure])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(2),
        "`pure` denies every effect except `Unknown` — a peek bounded by `deny Net` alone must not \
         satisfy it: {}", String::from_utf8_lossy(&out.stderr));
}

/// ⟨0.32⟩ **THE ADVISORY SIBLINGS CERTIFIED WHAT THE GATE HAD JUST STARTED REFUSING** — SPEC §3.2's
/// pessimism relation, which ⟨0.24⟩ states as *"an advisory verb may be LESS certain than the gate over
/// the same bytes and NEVER MORE"* and binds explicitly to `unverified`, `fix-gate` *"and any later
/// sibling"*.
///
/// MEASURED on the release build at `ab505c0`, over the PART 62 rust fixture (an unreadable `build.rs`
/// that runs `curl`, scanned with NO policy, then gated under `deny Exec`):
///
/// ```text
///   gate --report N --policy P            exit 2   {"ok": false, "incomplete": true}
///   fix-gate   --report N --policy P -s   exit 0   {"ok": true, "remedies": []}
///   unverified --report N --policy P -s   exit 0   {"ok": true, "unverified": []}
/// ```
///
/// The `--strict` forms are how CI consumes both verbs, and the documents beside them are the agent
/// channel — the one that cannot ask a follow-up question. This is the SECOND time the ⟨0.30⟩/⟨0.32⟩
/// rung has closed a cause on the gate and left its siblings behind (`outOfScope` did it first), which
/// is why the repair is an ARM on the shared completeness value rather than a note at each verb.
///
/// **AND `unverified`'S CORRECT-LOOKING ANSWER WAS A COINCIDENCE.** Over a fixture whose functions
/// carry `Unknown`, the verb exits 1 on the holes it found and looks like it refused; over the same
/// tree with no hole in it — this row's fixture — it answers `{"ok": true, "unverified": []}` at 0.
/// A non-zero exit reached by a different finding is not this rule being satisfied.
#[test]
fn advisory_verbs_refuse_a_deny_rule_over_classes_the_producer_never_read() {
    let f = Fixture::new("unpeeked-advisory");
    let deny = write_policy(&f, "deny.policy", "deny Exec\n");
    // A no-policy producer's report: `outOfScope` ABSENT, every class `peeked: false`. Its one function
    // does `Fs` and is NOT `Unknown`, so under `deny Exec` there is no violation, no remedy and no hole
    // — every finding set is empty and the ONLY thing that can move these verbs is the unread class.
    write_excluded_report(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": false, "reason": "compile time" }]"#,
        None,
    );

    for verb in ["fix-gate", "unverified"] {
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &deny, "--strict", "--json"])
            .output().expect("run candor-query");
        let doc = String::from_utf8(out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(out.status.code(), Some(2),
            "`{verb} --strict` must answer 2 wherever `gate --report` does over the same bytes \
             (SPEC §3.2) — the gate refuses this report for an unread class. doc: {doc}");
        assert_eq!(v["incomplete"], serde_json::json!(true),
            "the DOCUMENT must carry the same finding as the exit — a CI wrapper reads the exit, an \
             agent reads this: {doc}");
        // OMITTED, never `false`. These verbs found NOTHING: `ok: false` beside an empty array would
        // assert a hole/crossing the analysis never made — the fabrication mirror ⟨0.24⟩ rules out.
        assert!(v.get("ok").is_none(),
            "`ok` must be WITHHELD over an incomplete universe, not set false: {doc}");
    }

    // CONTROL — `fix` takes the disclosure and NOT an exit code. It answers no `ok` for `--strict` to
    // follow, and a second exit policy inside one verb would say the gate's refusal is the milder
    // finding. The hedge must still reach it: every one of its answers is a claim over the report.
    let out = Command::new(bin())
        .args(["fix", "inner", "Exec", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(0), "`fix` answers no `ok`, so its exit must not move");
    let doc = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert_eq!(v["incomplete"], serde_json::json!(true),
        "`fix`'s remedy is computed over a universe it cannot fully see, and the document must say so: {doc}");
}

/// ⟨0.32⟩ THE THREE OVER-CHARGE CONTROLS for the row above, written BEFORE the fix. Each one is a way
/// the arm could be satisfied by refusing something it must not: an unread class rides almost every
/// report a bare scan writes, so a verb that hedged unconditionally would teach its reader to skip the
/// hedge — and would fail every one of these.
#[test]
fn advisory_verbs_do_not_refuse_for_a_peek_nobody_asked_for() {
    let f = Fixture::new("unpeeked-advisory-controls");
    let deny = write_policy(&f, "deny.policy", "deny Exec\n");

    // (1) THE PRODUCER LOOKED, UNDER THE SAME POLICY THESE VERBS HOLD. `peeked: true` beside
    // `outOfScope: []` AND a `scannedUnder` covering `deny Exec` is the asked-and-clear answer: both
    // verbs must certify exactly as they did before this rung. ⟨0.33⟩ makes `scannedUnder` the thing
    // that licenses the claim — see `advisory_verbs_refuse_a_deny_rule_over_classes_the_producer_never_read`
    // for the same bytes MINUS `scannedUnder`, which must now refuse.
    write_excluded_report_under(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": true, "reason": "compile time" }]"#,
        Some("[]"),
        Some(r#"["deny Exec"]"#),
    );
    for verb in ["fix-gate", "unverified"] {
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &deny, "--strict", "--json"])
            .output().expect("run candor-query");
        let doc = String::from_utf8(out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(out.status.code(), Some(0),
            "a class the producer READ and found clean, under THE SAME deny set these verbs hold, is \
             not unread code — re-charging it would refuse every complete report with an exclusion in \
             it. `{verb}` doc: {doc}");
        assert_eq!(v["ok"], serde_json::json!(true),
            "`{verb}` must still certify a report whose every class was peeked under the SAME policy: {doc}");
        assert!(v.get("incomplete").is_none(), "no hedge is owed here: {doc}");
    }

    // (2) `judgedElsewhere` — the producer's own carve-out. A derived copy of code this same scan
    // already judged hides nothing, and refusing over it double-charges one body of code.
    write_excluded_report(
        &f,
        r#"[{ "class": "build-output", "count": 1, "peeked": false, "judgedElsewhere": true, "reason": "derived" }]"#,
        None,
    );
    for verb in ["fix-gate", "unverified"] {
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &deny, "--strict", "--json"])
            .output().expect("run candor-query");
        let doc = String::from_utf8(out.stdout).unwrap();
        assert_eq!(out.status.code(), Some(0),
            "`judgedElsewhere` carves the class out on this route exactly as it does on the gate's. \
             `{verb}` doc: {doc}");
    }

    // (3) A POLICY WITH NO DENY RULE. Only a `deny`/`pure` rule's answer depends on code outside the
    // scan's scope, so an unread class must cost a `forbid`-only policy nothing.
    //
    // ASSERTED ON THE DOCUMENT, NOT THE EXIT, and that is the whole subtlety: a `forbid`-only policy
    // ALREADY exits 2 on both verbs for its OWN unanswerability (⟨0.29⟩ `whole_policy_refusals`), so an
    // exit-code assertion here would pass whether or not the arm is over-charging. `unevaluated`
    // present + `incomplete` ABSENT is the only thing that tells the two readings apart.
    write_excluded_report(
        &f,
        r#"[{ "class": "build-script", "count": 1, "peeked": false, "reason": "compile time" }]"#,
        None,
    );
    let forbid = write_policy(&f, "forbid.policy", "forbid app -> infra\n");
    for verb in ["fix-gate", "unverified"] {
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &forbid, "--strict", "--json"])
            .output().expect("run candor-query");
        let doc = String::from_utf8(out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert!(v.get("unevaluated").is_some(),
            "instrument: this row means nothing unless the forbid rule really is unanswerable here — \
             `{verb}` doc: {doc}");
        assert!(v.get("incomplete").is_none(),
            "a policy with no deny rule asks nothing of the unread class, so no incompleteness is \
             owed — this is the over-charge the scan route carved out with `peek_attempted`. \
             `{verb}` doc: {doc}");
    }

    // (4) …and `pure` IS a deny rule — an empty effect list, §2.2 ⟨0.30⟩. An engine deciding "does this
    // policy deny anything" by flattening rules into effect NAMES gets nothing from it and lets the
    // STRICTEST policy in the grammar disarm the rung. Measured four-way on the scan route once already.
    let pure = write_policy(&f, "pure.policy", "pure\n");
    for verb in ["fix-gate", "unverified"] {
        let out = Command::new(bin())
            .args([verb, "--report", &f.prefix, "--policy", &pure, "--strict", "--json"])
            .output().expect("run candor-query");
        let doc = String::from_utf8(out.stdout).unwrap();
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(out.status.code(), Some(2),
            "`pure` is a deny rule with an empty effect list — the rung must not be disarmed by the \
             strictest policy there is. `{verb}` doc: {doc}");
        // ON THE DOCUMENT TOO, because `fix-gate` already exits 1 here on a REAL finding (`inner` does
        // `Fs`, which `pure` denies) — an exit-code assertion alone would pass on that finding whether
        // or not the arm fired. `incomplete` is the only key that says this run was armed.
        assert_eq!(v["incomplete"], serde_json::json!(true),
            "`pure` must arm the unread-class cause in the DOCUMENT, not only reach a non-zero exit by \
             some other route. `{verb}` doc: {doc}");
    }
}

/// Write TWO reports under one locator, one per package, each declaring a single same-named function
/// with the given effects. This is SPEC §2's "a single analysis world" spelled as a report SET — the
/// everyday workspace shape, and the one where a bare `fn` stops being an identifier.
fn write_two_package_reports(f: &Fixture, func: &str, effects: &str) {
    for pkg in ["a", "b"] {
        let report = format!(
            r#"{{
  "candor": {{ "version": "scan-test", "toolchain": "stable", "spec": "0.32" }},
  "package": "{pkg}",
  "analyzed": {{ "count": 1, "digest": "0000000000000000" }},
  "functions": [
    {{ "fn": "{func}", "loc": "src/lib.rs:1:1", "inferred": {effects}, "direct": {effects}, "hash": "{pkg}#{func}", "cmds": ["curl"] }}
  ]
}}"#
        );
        std::fs::write(format!("{}.{pkg}.scan.json", f.prefix), report).unwrap();
        std::fs::write(format!("{}.{pkg}.scan.callgraph.json", f.prefix), format!(r#"{{"{func}":[]}}"#)).unwrap();
    }
}

/// ⟨0.32⟩ **A VERDICT ROW MUST CARRY ENOUGH IDENTITY TO TELL TWO UNITS APART** — SPEC §2, and the
/// clause names this engine's own case as its motivating example.
///
/// MEASURED at `ab505c0` on a two-member workspace whose members both violate `deny Exec`:
///
/// ```text
///   "violations": [
///     { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" },
///     { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" }
///   ]
/// ```
///
/// Two BYTE-IDENTICAL rows. No hash, no package, no loc — nothing a consumer can attribute, so a
/// reader cannot tell whether two members are broken or one is listed twice, and a consumer that
/// fingerprints on name alone (candor's own SARIF action did) hides one finding behind the other.
/// §2.2 already binds a consumer to join by `hash`; a verdict row that omits it forces exactly the
/// name join the spec forbids.
///
/// AND THE SORT KEY IS HALF THE CLAUSE. `(rule, detail)` ties on these twins, so the order between
/// them is whatever order the two routes happened to accumulate in — and §3.3.1 makes the document's
/// order part of byte-equality. Identity in the row without identity in the key leaves the two routes
/// free to disagree.
#[test]
fn a_verdict_row_names_the_unit_it_is_about() {
    let f = Fixture::new("verdict-identity");
    let deny = write_policy(&f, "deny.policy", "deny Exec\n");
    write_two_package_reports(&f, "go", r#"["Exec"]"#);

    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1), "both members violate `deny Exec`");
    let doc = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
    let rows = v["violations"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "instrument: this row means nothing without TWO violations: {doc}");
    // THE DEFECT: the two rows were identical documents.
    assert_ne!(rows[0], rows[1],
        "two units, two rows, and NOTHING in either says which unit it is about — a reader cannot tell \
         two broken members from one listed twice: {doc}");
    // …and the identity is the §2.2 JOIN KEY, not a decoration. A consumer joins a verdict row back to
    // the report entry by `hash`; anything else forces the name join §2.2 forbids.
    let hashes: Vec<&str> = rows.iter().map(|r| r["hash"].as_str().unwrap_or("")).collect();
    assert_eq!(hashes, vec!["a#go", "b#go"],
        "each row must carry its unit's `hash` — §2.2's join key — and in SORTED order, because the \
         document's order is part of §3.3.1 byte-equality and `(rule, detail)` ties on these twins: {doc}");
}

/// ⟨0.32⟩ THE OVER-CHARGE CONTROL for the row above: a SINGLE-unit verdict must not change shape
/// gratuitously. Identity is ADDED to the row; nothing that was there moves, changes value, or goes.
#[test]
fn a_single_unit_verdict_keeps_every_key_it_had() {
    let f = Fixture::new("verdict-identity-single");
    let deny = write_policy(&f, "deny.policy", "deny Fs\n");
    f.write_report();      // one package `rpt`, `inner`/`outer`, both Fs

    let out = Command::new(bin())
        .args(["gate", "--report", &f.prefix, "--policy", &deny, "--json"])
        .output().expect("run candor-query");
    assert_eq!(out.status.code(), Some(1), "`deny Fs` fires on this report");
    let doc = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
    let rows = v["violations"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "instrument: two functions violate here: {doc}");
    for r in rows {
        let o = r.as_object().unwrap();
        assert_eq!(o["rule"], serde_json::json!("AS-EFF-006"), "{doc}");
        assert!(o.contains_key("fn") && o.contains_key("effects") && o.contains_key("detail"),
            "the pre-⟨0.32⟩ keys must all survive: {doc}");
        // The KEY SET, pinned: identity is the ONLY addition. A rung that quietly grew a second field
        // would break every consumer that validates the shape, and the growth would be invisible to an
        // assertion that only reads keys by name.
        let mut keys: Vec<&str> = o.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["detail", "effects", "fn", "hash", "rule"],
            "a verdict row gains IDENTITY and nothing else: {doc}");
    }
    // …and the NAME is untouched — it is what a policy scope matches and what a human reads. Carrying
    // the hash must not turn `fn` into `pkg#fn`, which would silently stop every scoped rule matching:
    // a false green introduced by fixing a false green.
    let names: Vec<&str> = rows.iter().map(|r| r["fn"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["inner", "outer"], "the `fn` field stays the bare NAME: {doc}");
}
