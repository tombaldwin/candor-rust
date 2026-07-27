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
    assert_eq!(v["spec"], "0.24");
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
        std::fs::write(
            format!("{}.app.hierarchy.json", f.prefix),
            r#"{"mod.Sub":["mod.Base"]}"#,
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.24" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.24" },
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
