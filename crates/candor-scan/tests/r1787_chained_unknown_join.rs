//! SOUNDNESS R1787 — with a dependency CHAINED, a joined report row that still carries `Unknown` withdrew the consumer's
//! CURATED charge on the path the join answered. Field case: sqlx-postgres `raw_sql(..).fetch_all(..)` over sqlx_core's real
//! report (`RawSql::fetch_all` is `Unknown` + `dispatchesOn: Executor::fetch_all`, and `Db` is curated with no platform leaf):
//! `deny Db` exited 1 over the dependency unchained and 0 chained, while `deny Db Unknown` stayed 1.
//!
//! The dependency here is a stub named `sqlx_core` (the classifier calibrates that crate by NAME), whose `RawSql*` types have
//! three shapes of `fetch_all`:
//!   * `RawSql`  — only a dispatch on a trait nothing in the dependency implements: `Unknown`, nothing concrete. The case.
//!   * `RawSql2` — a concrete effect (`Fs`), no `Unknown`: no `Unknown`. A PIN of today's untyped-route answer, UNCHANGED by
//!     R1787: the row wins and no `Db` is added. (The typed route answers the same row the other way, curated wins; an open
//!     mismatch this test does not endorse.)
//!   * `RawSql3` — a concrete effect AND a dispatch: keeps its `Fs` and gains the curated `Db` beside it.
//!
//! Every effectful case loses its `Db` with `CANDOR_R1787_OFF=1` (the proof the fixture reaches the change); each has a pure twin
//! with a unique name. Fixture fns end `_zq` because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1787-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn pkg(root: &Path, name: &str, deps: &[&str], src: &str) -> PathBuf {
    let d = root.join(name);
    let deps: String = deps.iter().map(|x| format!("{x} = {{ path = \"../{x}\" }}\n")).collect();
    write(&d.join("Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"));
    write(&d.join("src/lib.rs"), src);
    d
}

const SWITCHES: [&str; 5] = ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1787_OFF", "CANDOR_R1787_INSTR"];

fn scan_cmd(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref());
    for v in SWITCHES {
        cmd.env_remove(v);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// `deny <effects> <f>` over `dir` (chained on `deps`), with `env` set: the exit code.
fn deny(dir: &Path, effects: &str, f: &str, deps: Option<&Path>, env: &[(&str, &str)]) -> i32 {
    let pol = dir.join(format!("policy-{}.txt", effects.replace(' ', "-")));
    write(&pol, &format!("deny {effects} {f}\n"));
    let mut cmd = scan_cmd(dir, deps, env);
    cmd.arg("--json").arg("--policy").arg(&pol);
    let o = cmd.output().expect("run candor-scan");
    let c = o.status.code().unwrap_or(-1);
    assert!(c == 0 || c == 1, "scan failed ({c}): {}", String::from_utf8_lossy(&o.stderr));
    c
}

/// The `inferred` effects of `f`'s row over `dir` chained on `deps`, with `env` set (no policy: a result-moving switch makes a
/// GATE refuse, so the switched arm is read from the report). An absent row is no effects.
fn infers(dir: &Path, f: &str, deps: Option<&Path>, env: &[(&str, &str)]) -> Vec<String> {
    let mut cmd = scan_cmd(dir, deps, env);
    cmd.arg("--json");
    let o = cmd.output().expect("run candor-scan");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("report json");
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"] == f)
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn has(v: &[String], e: &str) -> bool {
    v.iter().any(|x| x == e)
}

fn stderr_of(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> String {
    let mut cmd = scan_cmd(dir, deps, env);
    cmd.arg("--json");
    String::from_utf8_lossy(&cmd.output().expect("run candor-scan").stderr).into_owned()
}

fn world(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = tmpdir(tag);
    let dep = pkg(
        &root,
        "sqlx_core",
        &[],
        "pub trait Executor { fn fetch_all(&self); }\n\
         pub mod raw_sql {\n\
          pub struct RawSql(pub String);\n\
          pub fn raw_sql(s: &str) -> RawSql { RawSql(s.to_string()) }\n\
          impl RawSql { pub fn fetch_all<E: crate::Executor>(self, e: E) { e.fetch_all() } }\n\
          pub struct RawSql2(pub String);\n\
          pub fn raw_sql2(s: &str) -> RawSql2 { RawSql2(s.to_string()) }\n\
          impl RawSql2 { pub fn fetch_all(self, p: &str) { std::fs::write(p, self.0).unwrap(); } }\n\
          pub struct RawSql3(pub String);\n\
          pub fn raw_sql3(s: &str) -> RawSql3 { RawSql3(s.to_string()) }\n\
          impl RawSql3 { pub fn fetch_all<E: crate::Executor>(self, p: &str, e: E) { std::fs::write(p, self.0).unwrap(); e.fetch_all() } }\n\
          pub struct Quiet(pub String);\n\
          pub fn quiet(s: &str) -> Quiet { Quiet(s.to_string()) }\n\
          impl Quiet { pub fn fetch_all(self) -> usize { self.0.len() } }\n\
         }\n",
    );
    let reps = root.join("reps");
    std::fs::create_dir_all(&reps).unwrap();
    let o = scan_cmd(&dep, None, &[]).arg("--out").arg(reps.join("sqlx_core")).output().unwrap();
    assert!(o.status.code().is_some_and(|c| (0..=2).contains(&c)), "{}", String::from_utf8_lossy(&o.stderr));
    let app = pkg(
        &root,
        "app",
        &["sqlx_core"],
        "use sqlx_core::raw_sql::{quiet, raw_sql, raw_sql2, raw_sql3};\n\
         use sqlx_core::Executor;\n\
         pub fn f_unknown_only_zq<E: Executor>(e: E) { raw_sql(\"select 1\").fetch_all(e) }\n\
         pub fn f_complete_zq(p: &str) { raw_sql2(\"select 1\").fetch_all(p) }\n\
         pub fn f_mixed_zq<E: Executor>(p: &str, e: E) { raw_sql3(\"select 1\").fetch_all(p, e) }\n\
         pub fn f_twin_zq(s: &str) -> usize { quiet(s).fetch_all() }\n",
    );
    // a lock, so the dependency's lock-pinned source is the stub's (the unchained typed arm) and the chain joins
    write(
        &app.join("Cargo.lock"),
        "version = 3\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx_core\"]\n\n[[package]]\nname = \"sqlx_core\"\nversion = \"0.1.0\"\n",
    );
    (root, app, reps)
}

#[test]
fn a_chained_unknown_only_row_does_not_withdraw_the_curated_charge() {
    let (_root, app, reps) = world("unknown-only");
    // the field case's own reading: the chained row is `Unknown` alone, and the curated `Db` is charged beside it
    assert_eq!(deny(&app, "Db", "f_unknown_only_zq", Some(&reps), &[]), 1, "chained: the curated Db stands beside the join's Unknown");
    assert_eq!(deny(&app, "Db Unknown", "f_unknown_only_zq", Some(&reps), &[]), 1, "the Unknown disclosure is kept");
    // the proof the fixture reaches the change: without it the chained arm reads `Unknown` only
    let off = infers(&app, "f_unknown_only_zq", Some(&reps), &[("CANDOR_R1787_OFF", "1")]);
    assert!(!has(&off, "Db") && has(&off, "Unknown"), "calibration: R1787 off loses the Db and keeps the Unknown: {off:?}");
    let on = stderr_of(&app, Some(&reps), &[("CANDOR_R1787_INSTR", "1")]);
    assert!(on.lines().any(|l| l.starts_with("R1787TYPED\t") && l.contains("unknown_only=true")), "the route must fire:\n{on}");
    // the pure twin stays pure
    // (the twin's receiver is a type with no non-pure member, so it is `Unknown` today by the untyped-receiver floor: the claim
    // is only that no `Db` is fabricated onto it)
    assert_eq!(deny(&app, "Db", "f_twin_zq", Some(&reps), &[]), 0, "the pure twin gains no Db");
    assert_eq!(infers(&app, "f_twin_zq", Some(&reps), &[]), infers(&app, "f_twin_zq", Some(&reps), &[("CANDOR_R1787_OFF", "1")]), "and the switch moves nothing on it");
}

#[test]
fn a_chained_row_with_concrete_effects_and_no_unknown_is_unchanged_by_r1787() {
    let (_root, app, reps) = world("complete");
    assert_eq!(deny(&app, "Fs", "f_complete_zq", Some(&reps), &[]), 1, "the chained row's own Fs wins");
    assert_eq!(deny(&app, "Db", "f_complete_zq", Some(&reps), &[]), 0, "unchanged by R1787: the untyped route lets this row win and adds no Db (the typed route answers the same row the other way)");
    let off = infers(&app, "f_complete_zq", Some(&reps), &[("CANDOR_R1787_OFF", "1")]);
    assert!(has(&off, "Fs") && !has(&off, "Db"), "and that is today's answer, byte for byte: {off:?}");
    assert_eq!(off, infers(&app, "f_complete_zq", Some(&reps), &[]), "the switch moves nothing here");
}

#[test]
fn a_chained_row_with_concrete_effects_and_unknown_keeps_them_and_gains_the_curated_charge() {
    let (_root, app, reps) = world("mixed");
    assert_eq!(deny(&app, "Fs", "f_mixed_zq", Some(&reps), &[]), 1, "the row's concrete effect stays");
    assert_eq!(deny(&app, "Db", "f_mixed_zq", Some(&reps), &[]), 1, "the curated Db stands beside the row's Unknown");
    let off = infers(&app, "f_mixed_zq", Some(&reps), &[("CANDOR_R1787_OFF", "1")]);
    assert!(has(&off, "Fs") && has(&off, "Unknown") && !has(&off, "Db"), "calibration: off keeps Fs + Unknown and loses the Db: {off:?}");
}
