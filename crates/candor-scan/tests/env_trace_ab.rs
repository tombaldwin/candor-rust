//! THE MEASURED HALF OF THE `trace` CLAIM (SOUNDNESS R1634). `tests/env_lint.rs` proves each `trace` read sits in a position
//! that can only choose whether something is printed, by the SHAPE of its guarded body. That is structural, not a proof — a body
//! can call a method the lint does not know mutates. This is the other instrument: set EVERY variable the source reads through
//! `candor_report::env::trace` at once, scan real code, and require the report to be byte for byte what an unset environment
//! produces. A `trace` read that moves a result would be a switch the engine fails to name; here it shows as a differing byte.
//!
//! The set of names is READ FROM THE SOURCE, never listed here, so a new `trace` variable is measured the day it is added.
//!
//! CALIBRATED in both directions inside this file: the arm must be LIVE (the all-set scan writes markers to stderr that the
//! unset scan does not, or it measured nothing) and the comparison must be able to FAIL (a real kill switch, which does move a
//! row, is run through the same comparison and must be reported as different).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `candor_report::env::trace("NAME")` literal in production source (the lint test guarantees there is no other spelling).
fn trace_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for rel in ["crates/candor-report/src", "crates/candor-classify/src", "crates/candor-query/src", "crates/candor-scan/src", "src"] {
        for e in walkdir::WalkDir::new(root().join(rel)).into_iter().filter_map(Result::ok) {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "rs") || p.file_name().is_some_and(|n| n == "tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(p).unwrap();
            let mut rest = text.as_str();
            while let Some(i) = rest.find("env::trace(\"") {
                rest = &rest[i + "env::trace(\"".len()..];
                if let Some(j) = rest.find('"') {
                    let n = &rest[..j];
                    if n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && !n.is_empty() {
                        names.insert(n.to_string());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

/// Scan `dir`, return (exit, stderr, the report files' bytes concatenated in name order).
fn scan(dir: &Path, out: &Path, env: &[(String, String)]) -> (i32, String, Vec<u8>) {
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap();
    let mut cmd = Command::new(bin());
    cmd.arg(dir).arg("--out").arg(out.join("r"));
    for (k, _) in std::env::vars() {
        if k.starts_with("CANDOR_") {
            cmd.env_remove(k);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(out).unwrap().filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    let mut bytes = Vec::new();
    for f in files {
        bytes.extend_from_slice(f.file_name().unwrap().to_string_lossy().as_bytes());
        bytes.push(0);
        bytes.extend(std::fs::read(&f).unwrap());
    }
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stderr).into_owned(), bytes)
}

fn synthetic(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-trace-ab-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"abfx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    // A spread of the shapes the engine has special cases for: a let-bound builder chain, trait objects and generics, a
    // closure, a macro, an alias, an `impl Trait for std::fs::File`, a dependency-shaped call, a Drop impl.
    std::fs::write(
        d.join("src/lib.rs"),
        r#"use std::collections::HashMap;
use std::io::Write;
fn hit() { let _ = std::fs::write("/tmp/candor-trace-ab-never-run", "x"); }
pub struct H; pub struct F(u8);
impl H { pub fn new() -> Self { H } pub fn find(&self) -> F { F(0) } }
impl F { pub fn filter(self, x: u8) -> Self { F(self.0 + x) } pub fn all(self) { hit() } }
pub fn c_let_chain() { let f = H::new().find().filter(1); f.all() }
pub trait Wr { fn w(&self); }
impl Wr for std::fs::File { fn w(&self) { let _ = std::fs::remove_file("/tmp/x"); } }
pub struct Noisy; impl Wr for Noisy { fn w(&self) { println!("hi"); } }
pub fn dynamic(x: &dyn Wr) { x.w() }
pub fn generic<T: Wr>(x: T) { x.w() }
pub fn boxed() -> Box<dyn Wr> { Box::new(Noisy) }
pub fn closure() { let f = |p: &str| std::fs::read_to_string(p); let _ = f("/etc/hosts"); }
macro_rules! mk { ($n:ident) => { pub fn $n() { hit() } }; }
mk!(from_macro);
type Alias = HashMap<String, Vec<u8>>;
pub fn alias_user(a: &mut Alias) { a.entry("k".into()).or_default().push(1); }
pub struct D; impl Drop for D { fn drop(&mut self) { hit() } }
pub fn drops() { let _d = D; }
pub fn writer(w: &mut dyn Write) { let _ = w.write_all(b"x"); }
pub fn view(s: &String) -> usize { s.as_bytes().len() }
pub fn clock() -> std::time::SystemTime { std::time::SystemTime::now() }
pub fn env_read() -> Option<String> { std::env::var("HOME").ok() }
pub fn pure_fn(a: u8) -> u8 { a.wrapping_add(1) }
"#,
    )
    .unwrap();
    d
}

#[test]
fn setting_every_trace_variable_at_once_changes_no_report_byte() {
    let names = trace_names();
    assert!(names.len() >= 100, "expected well over a hundred trace variables in the source, found {}: the name census is broken", names.len());
    let all: Vec<(String, String)> = names.iter().map(|n| (n.clone(), "1".to_string())).collect();
    let work = std::env::temp_dir().join(format!("candor-trace-ab-out-{}", std::process::id()));
    let fixtures: Vec<(&str, PathBuf)> = vec![
        ("synthetic", synthetic("a")),
        ("candor-report", root().join("crates/candor-report")),
        ("candor-classify", root().join("crates/candor-classify")),
    ];
    let mut live_somewhere = false;
    for (name, dir) in &fixtures {
        let (rc0, err0, base) = scan(dir, &work.join(format!("{name}-unset")), &[]);
        let (rc1, err1, all_set) = scan(dir, &work.join(format!("{name}-all")), &all);
        assert!(rc0 == 0 || rc0 == 1, "{name}: unset scan failed ({rc0}): {err0}");
        assert_eq!(rc0, rc1, "{name}: setting diagnostics changed the exit code");
        assert!(!base.is_empty());
        // THE ARM MUST BE LIVE: the diagnostics print to stderr when set. If they never fired, byte equality means nothing.
        if err1.len() > err0.len() && err1.lines().count() > err0.lines().count() + 3 {
            live_somewhere = true;
        }
        assert!(
            base == all_set,
            "{name}: setting every `trace` variable CHANGED the report — one of them moves a result and must be a `switch`.\n\
             Bisect by setting the names one at a time. unset stderr {} bytes, all-set stderr {} bytes",
            err0.len(),
            err1.len()
        );
    }
    assert!(live_somewhere, "no all-set scan printed any diagnostic the unset scan did not: the trace variables never fired, so this measured nothing");
    for (_, dir) in &fixtures {
        if dir.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn the_comparison_can_fail_a_real_kill_switch_is_reported_as_different() {
    // CALIBRATION: the same comparison over a variable that DOES move a result. If this passed as "identical" the test above
    // would be vacuous.
    let d = synthetic("b");
    let work = std::env::temp_dir().join(format!("candor-trace-ab-cal-{}", std::process::id()));
    let (_, _, base) = scan(&d, &work.join("unset"), &[]);
    let (_, _, moved) = scan(&d, &work.join("off"), &[("CANDOR_R1608_OFF".to_string(), "1".to_string())]);
    assert!(base != moved, "CANDOR_R1608_OFF must change the bytes on this fixture, or the A/B cannot tell a moving variable from a quiet one");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&work);
}
