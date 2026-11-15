//! SOUNDNESS R1541 (I1) — units a `macro_rules!` makes. Lane `rust-i1`, stage 1.
//!
//! Step 1, SCOPE-AWARE BINDING: an item-position invocation binds to a crate-local `macro_rules!` by NAME, so a definition
//! rustc does not pick — a `#[cfg]`-gated stub beside a `use dep::name` import of the same name (deranged-0.5.8's
//! `#[cfg(docsrs)] macro_rules! int` beside `pub use deranged_macros::int`), or any local definition of a name the
//! invoking file imports from outside the crate (redox_syscall's `use bitflags::bitflags` beside a local wrapper) — minted
//! the WRONG items. Measured on the lane's executed fixture (`fx/bind`): the compiled-out stub's empty `bnd_f` became a unit
//! and the caller read pure over the dependency macro's write. Refused now; `CANDOR_I1_BIND_OFF=1` restores the old binding,
//! and each case below asserts the OFF arm still mints the wrong unit (so the fixture reaches the code).
//!
//! Steps 2+ (trait impls and declarations spliced, the one "expanded" fact) are pinned further down.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const W: &str = "let _ = std::fs::write(\"/tmp/i1-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-i1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_crate(dir: &Path, name: &str, deps: &str, src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps}"),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), format!("#![allow(unused)]\n{}\n", src.replace("EFF", W))).unwrap();
}

/// One scan. Returns (fn -> (inferred, unknownWhy)) over the report rows and the SET of every unit in the call-graph
/// sidecar (which holds pure units too, so it can say a unit was MINTED).
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;
fn scan(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> (Rows, BTreeSet<String>) {
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_I1_OFF", "CANDOR_I1_BIND_OFF", "CANDOR_I1_RESPAN_OFF"] {
        cmd.env_remove(k);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut rows = Rows::new();
    let mut units = BTreeSet::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        if n.ends_with(".callgraph.json") {
            units.extend(v.as_object().unwrap().keys().cloned());
        } else if n.ends_with(".json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let strs = |x: &serde_json::Value| -> Vec<String> {
                x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
            };
            for f in v["functions"].as_array().unwrap() {
                rows.insert(f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["unknownWhy"])));
            }
        }
    }
    (rows, units)
}

fn eff(rows: &Rows, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.0.clone()).unwrap_or_default()
}

/// Step 1 — a `#[cfg]`-gated sole definition of a name the file also imports: rustc binds the import.
#[test]
fn i1_step1_cfg_gated_stub_beside_an_import_is_not_expanded() {
    let d = tmpdir("bindcfg");
    write_crate(
        &d,
        "bindcfg",
        "[dependencies]\ndmlib = { path = \"../dmlib\" }\n",
        "#[cfg(any())]\nmacro_rules! make_bnd { () => { pub fn bnd_f() { EFF } }; }\nuse dmlib::make_bnd;\nmake_bnd!();\n\
         pub fn b_call() { bnd_f() }\n",
    );
    let (on_rows, on_units) = scan(&d, None, &[]);
    assert!(!on_units.contains("bnd_f"), "the compiled-out stub must not be expanded: {on_units:?}");
    assert!(!eff(&on_rows, "b_call").contains(&"Fs".to_string()), "b_call must not be charged the stub's body");
    let (off_rows, off_units) = scan(&d, None, &[("CANDOR_I1_BIND_OFF", "1")]);
    assert!(off_units.contains("bnd_f"), "OFF must mint the stub's unit (the fixture reaches the code): {off_units:?}");
    assert_eq!(eff(&off_rows, "b_call"), vec!["Fs".to_string()], "OFF binds the stub (the old, wrong binding)");
}

/// Step 1 — an UNGATED local definition of a name the invoking file imports from outside the crate is refused too; a
/// cfg-gated definition WITHOUT a competing import is still expanded (its invocations are gated with it).
#[test]
fn i1_step1_import_clash_refused_and_gated_def_without_import_kept() {
    let d = tmpdir("bindimp");
    write_crate(
        &d,
        "bindimp",
        "",
        "macro_rules! wrap { () => { pub fn wr_f() { EFF } }; }\nmod a { use other::wrap; wrap!(); pub fn a_call() { wr_f() } }\n\
         #[cfg(feature = \"x\")]\nmacro_rules! gated { () => { pub fn gt_f() { EFF } }; }\n#[cfg(feature = \"x\")]\ngated!();\n\
         pub fn g_call() { gt_f() }\n",
    );
    let (on_rows, on_units) = scan(&d, None, &[]);
    assert!(!on_units.contains("a::wr_f"), "an import clash must refuse the local binding: {on_units:?}");
    assert!(on_units.contains("gt_f"), "a gated definition with no competing import is still expanded: {on_units:?}");
    assert_eq!(eff(&on_rows, "g_call"), vec!["Fs".to_string()]);
    let (_, off_units) = scan(&d, None, &[("CANDOR_I1_BIND_OFF", "1")]);
    assert!(off_units.contains("a::wr_f"), "OFF binds the local wrapper: {off_units:?}");
}
