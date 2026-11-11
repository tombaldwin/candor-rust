//! SOUNDNESS R1626 — `x.into()` whose target type is given by where the value goes (the tail or a `return` of a `-> T` fn,
//! `Ok(x.into())` under `-> Result<T, _>`, `Some(x.into())` under `-> Option<T>`) calls a LOCAL `impl From<Src> for T`. EXECUTED
//! EXECUTED in the lane (scratchpad `rustagent-r1626/fx/h`: each `f_*` fn's `From` read `/etc/hosts` once and each pure twin ran nothing).
//! Only the annotated `let` formed the edge before; every return-position spelling read ABSENT and `deny Fs` passed.
//! `CANDOR_R1625_OFF=into` restores the old reading. NAMED RESIDUAL: a target with SEVERAL local `From` impls is not charged (they
//! share one `T::from` key; by-name charging fabricated and hedged, 3,276 rows for one real gain, so it was not built).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SRC: &str = r#"
#![allow(dead_code)]
fn rd() { let _ = std::fs::read_to_string("/etc/hosts"); }
pub struct Src; pub struct SrcB;
pub struct TgtI; pub struct TgtP; pub struct TgtM;
impl From<Src> for TgtI { fn from(_s: Src) -> TgtI { rd(); TgtI } }
impl From<Src> for TgtP { fn from(_s: Src) -> TgtP { TgtP } }
impl From<Src> for TgtM { fn from(_s: Src) -> TgtM { TgtM } }
impl From<SrcB> for TgtM { fn from(_s: SrcB) -> TgtM { rd(); TgtM } }
// pure twins (unique names) first
pub fn p_tail() -> TgtP { Src.into() }
pub fn p_ret() -> TgtP { return Src.into(); }
pub fn p_ok() -> Result<TgtP, ()> { Ok(Src.into()) }
pub fn p_some() -> Option<TgtP> { Some(Src.into()) }
pub fn p_string() -> String { "x".into() }
pub fn p_reflexive(t: TgtI) -> TgtI { t.into() }
pub fn p_generic<T: From<Src>>() -> T { Src.into() }
// effectful
pub fn f_tail() -> TgtI { Src.into() }
pub fn f_return_only() -> TgtI { return Src.into(); }
pub fn f_ok() -> Result<TgtI, ()> { Ok(Src.into()) }
pub fn f_some() -> Option<TgtI> { Some(Src.into()) }
pub fn f_branch(b: bool) -> TgtI { if b { Src.into() } else { Src.into() } }
pub fn f_match(b: u8) -> TgtI { match b { 0 => Src.into(), _ => Src.into() } }
pub fn f_block() -> TgtI { { Src.into() } }
pub struct H; impl H { pub fn f_method(&self) -> TgtI { Src.into() } }
pub fn f_annot() -> bool { let t: TgtI = Src.into(); let _ = t; true }
pub fn m_eff() -> TgtM { SrcB.into() }
// named residual: a multi-impl target (m_eff really reads, m_pure does not)
pub fn m_pure() -> TgtM { Src.into() }
// a hand-written `into` on the receiver's own type is not std's
pub struct Own; impl Own { pub fn into(self) -> TgtQ { TgtQ } } pub struct TgtQ;
impl From<Own> for TgtQ { fn from(_o: Own) -> TgtQ { rd(); TgtQ } }
pub fn own_into() -> TgtQ { Own.into() }
"#;

const PURE: &[&str] = &["p_tail", "p_ret", "p_ok", "p_some", "p_string", "p_reflexive", "p_generic", "own_into"];
const EFF: &[&str] = &["f_tail", "f_return_only", "f_ok", "f_some", "f_branch", "f_match", "f_block", "H::f_method", "f_annot"];

fn fixture() -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1626-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), SRC).unwrap();
    d
}

fn scan(d: &Path, off: Option<&str>) -> HashMap<String, Vec<String>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(d.to_string_lossy().as_ref()).arg("--json");
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1625_OFF"] {
        cmd.env_remove(k);
    }
    if let Some(o) = off {
        cmd.env("CANDOR_R1625_OFF", o);
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let mut e: Vec<String> = f["inferred"].as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            e.sort();
            (f["fn"].as_str().unwrap().to_string(), e)
        })
        .collect()
}

fn inf(r: &HashMap<String, Vec<String>>, f: &str) -> Vec<String> {
    r.get(f).cloned().unwrap_or_default()
}

#[test]
fn r1626_a_pure_twins_stay_absent() {
    let d = fixture();
    for off in [None, Some("into")] {
        let rows = scan(&d, off);
        for f in PURE {
            assert!(inf(&rows, f).is_empty(), "{f} (off={off:?}): {:?}", inf(&rows, f));
        }
    }
}

#[test]
fn r1626_b_every_return_position_spelling_reaches_from() {
    let d = fixture();
    let on = scan(&d, None);
    for f in EFF {
        assert_eq!(inf(&on, f), vec!["Fs".to_string()], "{f}");
    }
    // the multi-impl residual is pinned, in BOTH directions: if either starts charging, the source-typed impl choice was built
    // (`m_eff` really reads: SILENT today; `m_pure` really does not: and must stay absent rather than charged as a union)
    assert!(inf(&on, "m_eff").is_empty(), "m_eff: named residual, a target with several local From impls");
    assert!(inf(&on, "m_pure").is_empty(), "m_pure: a pure source must not be charged a sibling impl's read");
    // the switch restores the old reading for the return-position spellings, and only the annotated `let` keeps its edge
    let off = scan(&d, Some("into"));
    for f in EFF.iter().filter(|f| **f != "f_annot") {
        assert!(inf(&off, f).is_empty(), "{f}: CANDOR_R1625_OFF=into must restore the old reading");
    }
    assert_eq!(inf(&off, "f_annot"), vec!["Fs".to_string()]);
}

#[test]
fn r1626_c_a_primitive_alias_target_is_not_charged() {
    // `type Wide = u128; impl From<Lm> for Wide { Lm.0.into() }`: the target is std's `u128`, the local impl is one of several
    let src = "#![allow(dead_code)]\nfn rd() { let _ = std::fs::read_to_string(\"/etc/hosts\"); }\n\
pub struct Lm(pub u64);\npub type Wide = u128;\nimpl From<Lm> for Wide { fn from(l: Lm) -> Wide { rd(); l.0 as u128 } }\n\
pub fn widen(l: Lm) -> Wide { (l.0 as u64).into() }\n";
    let d = std::env::temp_dir().join(format!("candor-r1626-alias-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    let rows = scan(&d, None);
    assert!(inf(&rows, "widen").is_empty(), "widen: {:?}", inf(&rows, "widen"));
}
