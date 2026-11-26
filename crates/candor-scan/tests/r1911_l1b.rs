//! rust-l1b-r1911 — two things, both measured on the checked-in macropath matrix (`soundness/macropath`, `gen.py`, executed ground
//! truth in `expect.tsv`):
//!
//! * SOUNDNESS R1911: a dependency EXPRESSION macro (`D::mk!()`, `use D::mk; mk!()`) whose expansion calls D's effectful fn read
//!   `[] invisible:[D]` scanned alone and ABSENT, "nothing hidden", once D's real report was chained (a macro is not a unit of the
//!   report, so the join was never asked and its silence answered nothing). The writer and its pure twin carry the SAME bytes, so
//!   the disclosure is all the engine can restore: the chained reading must equal the standalone one, and a pure dependency FN
//!   called beside it must stay answered (absent), so the mark is per UNIT.
//! * The L1(b) counter (`CANDOR_L1B_INSTR`): measurement only. Default output is byte-identical with and without it, and it fires on
//!   the shapes it names and not on the controls (a body-local closure, a std-prelude fn, an expanded bare macro).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fx(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/macropath/fx").join(cell)
}

fn tmp(tag: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("candor-r1911-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Run candor-scan on `dir`; returns (report json, stderr). `deps` = a directory of dependency reports to chain.
fn scan(dir: &Path, work: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> (serde_json::Value, String) {
    let out = work.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_L1B_INSTR"] {
        cmd.env_remove(k);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CANDOR_CACHE_DIR", work.join(format!("c{}", env.len())));
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut found = None;
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".scan.json") {
            found = Some(serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap());
        }
    }
    (found.expect("a report"), String::from_utf8_lossy(&o.stderr).to_string())
}

/// fn -> (inferred, invisible)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn rows(v: &serde_json::Value) -> Rows {
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    v["functions"].as_array().unwrap().iter().map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["invisible"])))).collect()
}

fn row<'a>(r: &'a Rows, probe: &str) -> Option<&'a (Vec<String>, Vec<String>)> {
    r.iter().find(|(k, _)| *k == probe || k.ends_with(&format!("::{probe}"))).map(|(_, v)| v)
}

/// The cell's dependency scanned alone into a report directory, then the consumer scanned standalone and chained.
fn arms(cell: &str, dep: &str) -> (Rows, Rows, serde_json::Value) {
    let w = tmp(cell);
    let depdir = w.join("deps");
    std::fs::create_dir_all(&depdir).unwrap();
    let (dv, _) = scan(&fx(cell).join(dep), &w.join("dep"), None, &[]);
    std::fs::write(depdir.join(format!("r.{dep}.scan.json")), serde_json::to_vec(&dv).unwrap()).unwrap();
    let (alone, _) = scan(&fx(cell), &w.join("alone"), None, &[]);
    let (chained, _) = scan(&fx(cell), &w.join("chained"), Some(&depdir), &[]);
    (rows(&alone), rows(&chained), chained)
}

#[test]
fn r1911_path_macro_keeps_its_invisible_disclosure_when_the_dependency_is_chained() {
    let (alone, chained, doc) = arms("s1path", "fxdeps1path");
    // standalone: the κ ledger discloses both callers (writer and its byte-identical twin)
    for p in ["go_s1path", "qgo_s1path", "via_s1path", "qvia_s1path"] {
        assert_eq!(row(&alone, p).map(|r| r.1.clone()), Some(vec!["fxdeps1path".to_string()]), "standalone {p}");
        // chained: the SAME disclosure (v0.43.0 / main: absent from functions[])
        assert_eq!(row(&chained, p).map(|r| r.1.clone()), Some(vec!["fxdeps1path".to_string()]), "chained {p}");
        // a disclosure, never a charge and never `Unknown`
        assert!(row(&chained, p).unwrap().0.is_empty(), "chained {p} must stay [] (no charge, no hedge)");
    }
    // the envelope names the crate again, as the standalone scan does
    assert!(doc["coverage"]["uncovered"].as_array().is_some_and(|a| a.iter().any(|e| e["name"] == "fxdeps1path")), "{doc}");
}

#[test]
fn r1911_use_imported_macro_keeps_its_disclosure_chained() {
    let (alone, chained, _) = arms("s1use", "fxdeps1use");
    for p in ["go_s1use", "qgo_s1use"] {
        assert_eq!(row(&alone, p).map(|r| r.1.clone()), Some(vec!["fxdeps1use".to_string()]), "standalone {p}");
        assert_eq!(row(&chained, p).map(|r| r.1.clone()), Some(vec!["fxdeps1use".to_string()]), "chained {p}");
    }
}

#[test]
fn r1911_a_pure_dependency_fn_call_is_still_answered_by_the_chained_report() {
    // the control that keeps the mark per UNIT: `fnonly` calls only a pure fn of the same chained crate, so the join answers it
    // (ABSENT) while `go` (the macro caller) in the same crate is disclosed. A crate-level mark would disclose both.
    let (alone, chained, _) = arms("s1fn", "fxdeps1fn");
    assert!(row(&alone, "fnonly_s1fn").is_some_and(|r| r.1 == ["fxdeps1fn"]), "standalone discloses the fn caller (unchained dependency)");
    assert!(row(&chained, "fnonly_s1fn").is_none(), "chained: the join answered the pure fn: {chained:?}");
    assert_eq!(row(&chained, "go_s1fn").map(|r| r.1.clone()), Some(vec!["fxdeps1fn".to_string()]));
}

const CAL: &str = r#"
macro_rules! mk { ($n:ident) => { pub fn $n() { std::fs::write("/tmp/x", "y").unwrap(); } } }
pub mod b1 { self::mk! { hid_one } pub fn caller_b1() { hid_one() } }
pub mod b0 { crate::mk! { hid_two } pub fn caller_b0() { not_declared_here() } }
pub mod b2 { depcrate::items! { } pub fn caller_b2() { from_dep() } }
pub mod gl { use std::collections::*; depcrate::items! { } pub fn caller_gl() { from_dep_glob() } }
macro_rules! mk3 { ($n:ident) => { pub fn $n() {} } }
pub mod i3 { mk3! { vis_three } pub fn caller_i3() { vis_three() } }
pub mod ctl { depcrate::items! { } pub fn caller_ctl() { let f = || 1; let _ = f(); } }
pub mod pre { depcrate::items! { } pub fn caller_pre(x: Option<u8>) -> Option<u8> { drop(1); Some(x.unwrap_or(0)) } }
"#;

#[test]
fn l1b_counter_fires_on_its_shapes_not_its_controls_and_changes_no_output() {
    let w = tmp("l1bcal");
    let src = w.join("crate");
    std::fs::create_dir_all(src.join("src")).unwrap();
    std::fs::write(src.join("Cargo.toml"), "[package]\nname = \"l1bcal\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n").unwrap();
    std::fs::write(src.join("src/lib.rs"), CAL).unwrap();
    let (plain, _) = scan(&src, &w.join("plain"), None, &[]);
    let (instr, err) = scan(&src, &w.join("instr"), None, &[("CANDOR_L1B_INSTR", "1")]);
    assert_eq!(plain, instr, "the counter must not change a byte of the report");
    let sites: Vec<(&str, &str)> = err
        .lines()
        .filter_map(|l| l.strip_prefix("L1BSITE\t"))
        .map(|l| {
            let t: Vec<&str> = l.split('\t').collect();
            (t[0], t[1])
        })
        .collect();
    let has = |b: &str, u: &str| sites.iter().any(|(bb, uu)| *bb == b && *uu == u);
    assert!(has("b1-hedged", "b1::caller_b1"), "{sites:?}");
    assert!(has("b0-local-undeclared", "b0::caller_b0"), "{sites:?}");
    assert!(has("b2", "b2::caller_b2"), "{sites:?}");
    assert!(has("glob", "gl::caller_gl"), "{sites:?}");
    for ctl in ["i3::caller_i3", "ctl::caller_ctl", "pre::caller_pre"] {
        assert!(!sites.iter().any(|(_, u)| *u == ctl), "control {ctl} must not count: {sites:?}");
    }
}
