//! SOUNDNESS R1540 — a method call on a STD-TYPED receiver reaches the trait impl the crate (or a chained dependency)
//! writes FOR THAT STD TYPE. `impl MyLoad for AtomicUsize { fn my_load(&self) { /* writes */ } }` was charged on its own
//! row while `let a = AtomicUsize::new(3); a.my_load()` (an annotated `let`, a `&AtomicUsize` parameter, `Path`,
//! `Duration`, `HashMap`, …) gave the caller no edge and no `Unknown`; through a chained dependency's impl the consumer
//! read pure. Every fixture here was BUILT AND EXECUTED in the lane (`~/candor-handoff/rust-r1540/lane/`): the `eff`
//! cases write a file, the `pure` cases do not, and an `inherent` case runs std's own method (the crate impl never runs).
//! `CANDOR_R1540_OFF=1` restores the published behaviour, and every `eff` case must then read empty again — the proof
//! that the fixtures reach the new code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const W: &str = "let _ = std::fs::write(\"/tmp/r1540-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1540-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_crate(dir: &Path, name: &str, deps: &str, src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps}")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), format!("#![allow(unused)]\n{}\n", src.replace("EFF", W))).unwrap();
}

/// Scan `dir` (chained over `deps` when given) and return `fn -> (inferred, unknownWhy)` for every row.
fn scan(dir: &Path, deps: Option<&Path>, off: bool) -> std::collections::HashMap<String, (Vec<String>, Vec<String>)> {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1540_OFF")
        .env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    if off {
        cmd.env("CANDOR_R1540_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let strs = |x: &serde_json::Value| -> Vec<String> {
        x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    v["functions"].as_array().unwrap().iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["unknownWhy"]))))
        .collect()
}

fn inferred(rows: &std::collections::HashMap<String, (Vec<String>, Vec<String>)>, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.0.clone()).unwrap_or_default()
}

/// One trait per std receiver type; `l_` binds by `let`, `a_` by an annotated `let`, `p_` takes a parameter.
const IN_CRATE: &[(&str, &str, &str, &str, &str)] = &[
    // (key, impl self type, use line, ctor, parameter type)
    ("atomic", "AtomicUsize", "use std::sync::atomic::AtomicUsize;", "AtomicUsize::new(3)", "&AtomicUsize"),
    ("atomicfq", "std::sync::atomic::AtomicU64", "", "std::sync::atomic::AtomicU64::new(3)", "&std::sync::atomic::AtomicU64"),
    ("pathbuf", "PathBuf", "use std::path::PathBuf;", "PathBuf::from(\"a\")", "&PathBuf"),
    ("hashmap", "HashMap<u8, u8>", "use std::collections::HashMap;", "HashMap::new()", "&HashMap<u8, u8>"),
    ("mutex", "Mutex<u8>", "use std::sync::Mutex;", "Mutex::new(1u8)", "&Mutex<u8>"),
    ("ipv4", "Ipv4Addr", "use std::net::Ipv4Addr;", "Ipv4Addr::new(1, 2, 3, 4)", "&Ipv4Addr"),
    ("ordering", "Ordering", "use std::cmp::Ordering;", "Ordering::Less", "Ordering"),
];

fn in_crate_src() -> String {
    let mut s = String::new();
    for (k, ty, us, ctor, pty) in IN_CRATE {
        s.push_str(&format!(
            "pub mod m_{k} {{ {us}\n pub trait T{k} {{ fn r_{k}(&self) -> u8; }}\n impl T{k} for {ty} {{ fn r_{k}(&self) -> u8 {{ EFF 1 }} }}\n \
             pub fn l_{k}() -> u8 {{ let x = {ctor}; x.r_{k}() }}\n pub fn a_{k}() -> u8 {{ let x: {ty} = {ctor}; x.r_{k}() }}\n \
             pub fn p_{k}(x: {pty}) -> u8 {{ x.r_{k}() }}\n}}\n"
        ));
    }
    // THE DEREF STEP: a `PathBuf` / `OsString` receiver reaches the crate's `Path` / `OsStr` impl.
    s.push_str("pub mod m_deref { use std::path::{Path, PathBuf}; use std::ffi::{OsStr, OsString};\n pub trait Tdp { fn r_dp(&self) -> u8; }\n \
                impl Tdp for Path { fn r_dp(&self) -> u8 { EFF 1 } }\n pub trait Tdo { fn r_do(&self) -> u8; }\n impl Tdo for OsStr { fn r_do(&self) -> u8 { EFF 1 } }\n \
                pub fn p_deref(x: &PathBuf) -> u8 { x.r_dp() }\n pub fn o_deref(x: &OsString) -> u8 { x.r_do() }\n}\n");
    // `Path` is unsized: annotated as a reference.
    s.push_str("pub mod m_path { use std::path::Path;\n pub trait Tpath { fn r_path(&self) -> u8; }\n impl Tpath for Path { fn r_path(&self) -> u8 { EFF 1 } }\n \
                pub fn a_path() -> u8 { let x: &Path = Path::new(\"a\"); x.r_path() }\n pub fn p_path(x: &Path) -> u8 { x.r_path() }\n}\n");
    s
}

#[test]
fn a_std_typed_receiver_reaches_the_crates_impl_for_that_std_type() {
    let d = tmpdir("incrate");
    write_crate(&d, "r1540incrate", "", &in_crate_src());
    let on = scan(&d, None, false);
    let off = scan(&d, None, true);
    let mut callers: Vec<String> = Vec::new();
    for (k, ..) in IN_CRATE {
        for sp in ["l", "a", "p"] {
            callers.push(format!("m_{k}::{sp}_{k}"));
        }
    }
    callers.push("m_path::a_path".into());
    callers.push("m_path::p_path".into());
    callers.push("m_deref::p_deref".into());
    callers.push("m_deref::o_deref".into());
    for c in &callers {
        assert_eq!(inferred(&on, c), vec!["Fs".to_string()], "{c}: a std-typed receiver's call into the crate's effectful impl must charge it");
        assert!(inferred(&off, c).is_empty(), "{c}: with CANDOR_R1540_OFF the published silence must return (the fixture reaches the code)");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn std_ctor_untyped_receiver_and_untyped_vec_binding_disclose() {
    // `Duration::from_secs(1)` does not type the binding, and its factory path used to stand the R1095 floor aside; a
    // `vec![..]` binding records only its element. Both are disclosures (`Unknown`), not edges.
    let d = tmpdir("untyped");
    write_crate(&d, "r1540untyped", "", "use std::time::Duration;\npub trait Td { fn r_d(&self) -> u8; }\nimpl Td for Duration { fn r_d(&self) -> u8 { EFF 1 } }\n\
        pub fn l_dur() -> u8 { let x = Duration::from_secs(1); x.r_d() }\n\
        pub trait Tv { fn r_v(&self) -> u8; }\nimpl Tv for Vec<u8> { fn r_v(&self) -> u8 { EFF 1 } }\npub fn l_vec() -> u8 { let x = vec![1u8]; x.r_v() }\n\
        pub trait Ta { fn r_a(&self) -> u8; }\nimpl Ta for std::sync::Arc<u8> { fn r_a(&self) -> u8 { EFF 1 } }\npub fn p_arc(x: &std::sync::Arc<u8>) -> u8 { x.r_a() }\n");
    let on = scan(&d, None, false);
    let off = scan(&d, None, true);
    for c in ["l_dur", "l_vec", "p_arc"] {
        assert_eq!(inferred(&on, c), vec!["Unknown".to_string()], "{c}: must disclose");
        assert!(inferred(&off, c).is_empty(), "{c}: off arm must be the published silence");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_macro_made_impl_on_a_std_type_discloses_macro() {
    let d = tmpdir("macro");
    write_crate(&d, "r1540macro", "", "use std::sync::atomic::{AtomicUsize, Ordering};\npub trait LoadIt { fn load_it(&self) -> usize; }\n\
        macro_rules! imp { ($t:ty) => { impl LoadIt for $t { fn load_it(&self) -> usize { EFF 1 } } }; }\nimp!(AtomicUsize);\n\
        pub fn zz(a: &AtomicUsize) -> usize { a.load_it() }\n");
    let on = scan(&d, None, false);
    assert_eq!(inferred(&on, "zz"), vec!["Unknown".to_string()]);
    assert!(on["zz"].1.iter().any(|w| w.starts_with("macro:")), "{:?}", on["zz"].1);
    assert!(inferred(&scan(&d, None, true), "zz").is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn controls_inherent_wins_and_pure_stays_pure() {
    let d = tmpdir("ctl");
    write_crate(&d, "r1540ctl", "", "use std::sync::atomic::{AtomicUsize, Ordering};\nuse std::time::Duration;\n\
        pub trait MyTr { fn load(&self, o: Ordering) -> usize; fn pure_m(&self) -> u64; }\n\
        impl MyTr for AtomicUsize { fn load(&self, _o: Ordering) -> usize { EFF 9 } fn pure_m(&self) -> u64 { 1 } }\n\
        pub trait Ext { fn get(&self) -> u64; fn pure_d(&self) -> u64; }\n\
        impl Ext for Duration { fn get(&self) -> u64 { EFF 2 } fn pure_d(&self) -> u64 { self.as_secs() } }\n\
        pub fn n_inherent(a: &AtomicUsize) -> usize { a.load(Ordering::SeqCst) }\n\
        pub fn n_pure(a: &AtomicUsize) -> u64 { a.pure_m() }\n\
        pub fn n_pure_d(d: Duration) -> u64 { d.pure_d() }\n\
        pub fn n_stdnamed(d: Duration) -> u64 { d.get() }\n\
        pub fn n_std_len(v: &Vec<u8>) -> usize { v.len() }\n");
    let on = scan(&d, None, false);
    // EXECUTED: std's inherent `AtomicUsize::load` runs, not the crate's — never charged `Fs`. The std-method table does
    // not say which std type has `load`, so the site discloses `ambiguous:` (the stated over-disclosure).
    assert_eq!(inferred(&on, "n_inherent"), vec!["Unknown".to_string()]);
    assert!(on["n_inherent"].1.iter().any(|w| w.starts_with("ambiguous:std method or crate impl")), "{:?}", on["n_inherent"].1);
    // EXECUTED: `Duration` has no `get`, the crate's impl runs and writes — disclosed, never silent.
    assert_eq!(inferred(&on, "n_stdnamed"), vec!["Unknown".to_string()]);
    for c in ["n_pure", "n_pure_d", "n_std_len"] {
        assert!(inferred(&on, c).is_empty(), "{c}: a pure impl / std method must stay pure: {:?}", on.get(c));
    }
    let _ = std::fs::remove_dir_all(&d);
}

const DEP: &str = "use std::sync::atomic::{AtomicUsize, AtomicU32, Ordering};\nuse std::time::Duration;\nuse std::path::Path;\n\
    pub trait LoadIt { fn load_it(&self) -> usize; }\nimpl LoadIt for AtomicUsize { fn load_it(&self) -> usize { EFF 1 } }\n\
    impl LoadIt for AtomicU32 { fn load_it(&self) -> usize { EFF 1 } }\n\
    pub trait PathX { fn path_x(&self) -> u64; }\nimpl PathX for Path { fn path_x(&self) -> u64 { EFF 1 } }\n\
    pub trait PrimX { fn prim_x(&self) -> u64; }\nimpl PrimX for u32 { fn prim_x(&self) -> u64 { EFF 1 } }\n\
    pub trait Marker {}\nimpl Marker for AtomicUsize {}\n\
    pub trait Blank { fn blank_x(&self) -> u64; }\nimpl<T: Marker> Blank for T { fn blank_x(&self) -> u64 { EFF 1 } }\n\
    pub trait PureIt { fn pure_it(&self) -> usize; }\nimpl PureIt for AtomicUsize { fn pure_it(&self) -> usize { 2 } }\n\
    pub trait GetIt { fn get(&self) -> u64; }\nimpl GetIt for Duration { fn get(&self) -> u64 { EFF 1 } }\n\
    pub mod prelude { pub use super::PathX; }\n";

const APP: &str = "use std::sync::atomic::AtomicUsize;\nuse std::time::Duration;\nuse std::path::Path;\n\
    pub mod n { use r1540dep::{LoadIt, PrimX, Blank, PureIt, GetIt};\n use std::sync::atomic::AtomicUsize; use std::time::Duration;\n\
      pub fn c_let() -> usize { let a = AtomicUsize::new(3); a.load_it() }\n\
      pub fn c_param(a: &AtomicUsize) -> usize { a.load_it() }\n\
      pub fn c_prim(x: u32) -> u64 { x.prim_x() }\n\
      pub fn c_blank(a: &AtomicUsize) -> u64 { a.blank_x() }\n\
      pub fn c_pure(a: &AtomicUsize) -> usize { a.pure_it() }\n\
      pub fn c_stdnamed(d: Duration) -> u64 { d.get() }\n}\n\
    pub mod g { use r1540dep::prelude::*; use std::path::Path;\n pub fn c_glob(p: &Path) -> u64 { p.path_x() }\n}\n";

#[test]
fn a_chained_dependencys_impl_for_a_std_type_is_joined() {
    let root = tmpdir("chain");
    let (lib, app, rep) = (root.join("dep"), root.join("app"), root.join("rep"));
    write_crate(&lib, "r1540dep", "", DEP);
    write_crate(&app, "r1540app", "[dependencies]\nr1540dep = { path = \"../dep\" }\n", APP);
    std::fs::create_dir_all(&rep).unwrap();
    let out = Command::new(bin()).arg(lib.to_string_lossy().as_ref()).arg("--out").arg(rep.join("r").to_string_lossy().as_ref())
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_R1540_OFF").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let on = scan(&app, Some(&rep), false);
    let off = scan(&app, Some(&rep), true);
    // The dependency's own `AtomicUsize::load_it` / `Path::path_x` / `u32::prim_x` rows are JOINED (a resolution).
    for c in ["n::c_let", "n::c_param", "n::c_prim", "g::c_glob"] {
        assert!(inferred(&on, c).contains(&"Fs".to_string()), "{c}: {:?}", on.get(c));
        assert!(!inferred(&off, c).contains(&"Fs".to_string()), "{c}: the off arm must not join");
    }
    // A blanket impl has no `AtomicUsize::blank_x` row: the in-scope trait's effectful union is disclosed.
    assert_eq!(inferred(&on, "n::c_blank"), vec!["Unknown".to_string()]);
    assert!(inferred(&off, "n::c_blank").is_empty(), "the off arm is the published silence");
    // A pure dependency impl stays pure.
    assert!(inferred(&on, "n::c_pure").is_empty(), "{:?}", on.get("n::c_pure"));
    // PINNED RESIDUAL (filed as R1543): a dependency's member whose NAME std also has (`get`), implemented for a std type
    // that lacks it (`Duration`), runs the dependency's body (EXECUTED) and is not asked — the join is formed only for
    // names std lacks. This assertion states the WRONG answer on purpose, so the fix flips it loudly.
    assert!(inferred(&on, "n::c_stdnamed").is_empty(), "R1543 changed: {:?}", on.get("n::c_stdnamed"));
    let _ = std::fs::remove_dir_all(&root);
}
