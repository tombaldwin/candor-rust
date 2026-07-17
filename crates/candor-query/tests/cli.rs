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
        r#"{{"candor":{{"version":"scan-test","toolchain":"stable","spec": "0.19"}},"package":"cnt","functions":[
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
    assert_eq!(v["spec"], "0.19");
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
            r#"{{"candor":{{"version":"scan-test","toolchain":"stable","spec": "0.19"}},"functions":[
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
    let base_report = r#"{"candor":{"version":"t","spec":"0.19"},"package":"lib","functions":[{"fn":"lib::f","loc":"s:1","inferred":["Fs"],"hash":"h"}]}"#;
    let cur_report = r#"{"candor":{"version":"t","spec":"0.19"},"package":"lib","functions":[{"fn":"lib::f","loc":"s:1","inferred":["Fs","Net"],"hash":"h"}]}"#;
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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

/// Write a report + callgraph (+ optionally a hierarchy sidecar) for the frontier scenario:
/// confirmed chain `mod.Sub.handle → mod.Target.work`, plus three Unknown-dispatch carriers whose
/// disclosure depends on the hierarchy gate.
fn write_frontier_fixture(f: &Fixture, with_hierarchy: bool) {
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
  "package": "app",
  "functions": [
    { "fn": "mod.Target.work", "inferred": ["Fs"], "direct": ["Fs"] },
    { "fn": "mod.Sub.handle", "inferred": ["Fs"], "calls": ["mod.Target.work"] },
    { "fn": "mod.Caller.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Base.handle"] },
    { "fn": "mod.Other.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Unrelated.frob"] },
    { "fn": "mod.NotSub.run", "inferred": ["Unknown"], "unknownWhy": ["dispatch:mod.Elsewhere.handle"] }
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
    assert_eq!(poss.len(), 1, "exactly the hierarchy-confirmed overrider's dispatch: {v}");
    assert_eq!(poss[0]["fn"], "mod.Caller.run");
    assert_eq!(poss[0]["viaDispatchOn"], "handle");
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
    assert_eq!(fns, vec!["mod.Caller.run", "mod.NotSub.run"],
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

// ── blindspots: the Unknown sources ranked by blast radius (SPEC §3.1 ⟨0.6⟩ — was conformance-only) ─

#[test]
fn blindspots_ranks_sources_by_unknown_blast_radius() {
    // Two SOURCES (entries carrying their own unknownWhy): src_a smears Unknown up a two-hop caller
    // chain (reaches 2), src_b reaches nobody. Ranked most-smearing first; `affected` is the sorted
    // transitive caller set; totalUnknown counts every Unknown-carrying fn (sources + inheritors).
    let f = Fixture::new("blindspots");
    let report = r#"{
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
  "candor": { "version": "scan-test", "toolchain": "stable", "spec": "0.19" },
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
