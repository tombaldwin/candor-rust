//! SOUNDNESS R1625 — a call whose return type is a generic parameter of the callee is typed by where its value goes, not
//! only by a turbofish. EXECUTED in the lane (scratchpad `rustagent-r1625/fx/g`, run per spelling: each `*_f_*` target's
//! impl read `/etc/hosts` once; each pure twin ran nothing). Before this, only `s.parse::<L>()` formed the edge to
//! `L::from_str`; `let o: L = s.parse()?`, `let o: Result<L, _> = s.parse()`, the tail or `return` of a `-> Result<L, _>` fn,
//! and the same shape for `try_into`, `Default::default()`, `collect` read ABSENT; return-position `into` is measured and FILED (R1626), not fixed.
//!
//! The pure twins (a pure `FromStr`, `u32`, `String`, a generic target, a hand-written `parse` method) are FIRST in the
//! file and must stay absent. `CANDOR_R1625_OFF` restores the old reading, the proof that the fixture reaches the change.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const LOCAL: &str = r#"
#![allow(dead_code)]
use std::str::FromStr;
fn rd() { let _ = std::fs::read_to_string("/etc/hosts"); }

// ---- pure controls (unique names) -------------------------------------------------------------------------------
pub struct P;
impl FromStr for P { type Err = (); fn from_str(_s: &str) -> Result<P, ()> { Ok(P) } }
pub fn p_turbo(s: &str) -> bool { s.parse::<P>().is_ok() }
pub fn p_annot(s: &str) -> bool { let o: Result<P, ()> = s.parse(); o.is_ok() }
pub fn p_annot_q(s: &str) -> Result<P, ()> { let o: P = s.parse()?; Ok(o) }
pub fn p_ret(s: &str) -> Result<P, ()> { s.parse() }
pub fn p_ret_unwrap(s: &str) -> P { s.parse().unwrap() }
pub fn p_u32_annot(s: &str) -> bool { let o: u32 = s.parse().unwrap(); o > 1 }
pub fn p_u32_q(s: &str) -> Result<u32, std::num::ParseIntError> { let o: u32 = s.parse()?; Ok(o) }
pub fn p_u32_ret(s: &str) -> Result<u32, std::num::ParseIntError> { s.parse() }
pub fn p_string(s: &str) -> bool { let o: String = s.parse().unwrap(); !o.is_empty() }
// the caller's own generic: its instantiation answers it, nothing is charged here
pub fn p_generic<T: FromStr>(s: &str) -> Option<T> { s.parse().ok() }
// a hand-written `parse` method is the call, not `str::parse`
pub struct Pz;
impl Pz { pub fn parse(&self) -> Result<L, ()> { Err(()) } }
pub fn p_own_parse(z: &Pz) -> Result<L, ()> { z.parse() }
pub struct Tz;
impl Tz { pub fn try_into(&self) -> Result<TfE, ()> { Err(()) } }
pub fn p_own_try_into(z: &Tz) -> Result<TfE, ()> { z.try_into() }
pub struct Src; pub struct TgtP;
impl From<Src> for TgtP { fn from(_s: Src) -> TgtP { TgtP } }
pub fn p_into_ret() -> TgtP { Src.into() }
pub struct TfP;
impl TryFrom<u8> for TfP { type Error = (); fn try_from(_v: u8) -> Result<TfP, ()> { Ok(TfP) } }
pub fn p_try_into_ret() -> Result<TfP, ()> { 3u8.try_into() }
#[derive(Default)] pub struct DfP;
pub fn p_default_ret() -> DfP { Default::default() }
pub struct CoP;
impl FromIterator<u8> for CoP { fn from_iter<I: IntoIterator<Item = u8>>(_i: I) -> CoP { CoP } }
pub fn p_collect_ret(v: Vec<u8>) -> CoP { v.into_iter().collect() }
pub fn p_collect_std(v: Vec<u8>) -> Vec<u8> { v.into_iter().collect() }

// ---- the defect: an effectful local FromStr, every spelling ------------------------------------------------------
pub struct L;
impl FromStr for L { type Err = (); fn from_str(_s: &str) -> Result<L, ()> { rd(); Ok(L) } }
pub fn f_l_turbo(s: &str) -> bool { s.parse::<L>().is_ok() }
pub fn f_l_annot(s: &str) -> bool { let o: Result<L, ()> = s.parse(); o.is_ok() }
pub fn f_l_annot_q(s: &str) -> Result<L, ()> { let o: L = s.parse()?; Ok(o) }
pub fn f_l_ret(s: &str) -> Result<L, ()> { s.parse() }
pub fn f_l_ret_unwrap(s: &str) -> L { s.parse().unwrap() }
pub fn f_l_ret_expect(s: &str) -> L { s.parse().expect("x") }
pub fn f_l_ret_maperr(s: &str) -> Result<L, String> { s.parse().map_err(|_| "e".to_string()) }
pub fn f_l_let_unwrap(s: &str) -> bool { let o: L = s.parse().unwrap(); let _ = o; true }
pub fn f_l_ok(s: &str) -> bool { let o: Option<L> = s.parse().ok(); o.is_some() }
pub fn f_l_return_kw(s: &str) -> Result<L, ()> { if s.is_empty() { return s.parse(); } Err(()) }
pub fn f_l_branch(s: &str) -> Result<L, ()> { if s.is_empty() { Err(()) } else { s.parse() } }
pub fn f_l_match(s: &str) -> Result<L, ()> { match s.len() { 0 => Err(()), _ => s.parse() } }
pub fn f_l_uwor(s: &str) -> L { let o: L = s.parse().unwrap_or(L); o }

// ---- the same rule, other generic returns ------------------------------------------------------------------------
pub struct TgtI;
impl From<Src> for TgtI { fn from(_s: Src) -> TgtI { rd(); TgtI } }
pub fn i_r_ret() -> TgtI { Src.into() }
pub struct TfE;
impl TryFrom<u8> for TfE { type Error = (); fn try_from(_v: u8) -> Result<TfE, ()> { rd(); Ok(TfE) } }
pub fn t_f_annot() -> bool { let t: Result<TfE, ()> = 3u8.try_into(); t.is_ok() }
pub fn t_f_annot_q() -> Result<TfE, ()> { let t: TfE = 3u8.try_into()?; Ok(t) }
pub fn t_f_ret() -> Result<TfE, ()> { 3u8.try_into() }
pub struct DfE;
impl Default for DfE { fn default() -> DfE { rd(); DfE } }
pub fn d_f_annot() -> bool { let d: DfE = Default::default(); let _ = d; true }
pub fn d_f_ret() -> DfE { Default::default() }
pub struct CoE;
impl FromIterator<u8> for CoE { fn from_iter<I: IntoIterator<Item = u8>>(_i: I) -> CoE { rd(); CoE } }
pub fn c_f_annot(v: Vec<u8>) -> bool { let c: CoE = v.into_iter().collect(); let _ = c; true }
pub fn c_f_ret(v: Vec<u8>) -> CoE { v.into_iter().collect() }

// ---- still a residual (the context is an ARGUMENT, a FIELD, or a LATER use): filed, not fixed ----------------------
pub fn r_l_arg(s: &str) -> bool { take_l(s.parse().unwrap()) }
fn take_l(_l: L) -> bool { true }
"#;

const PURE: &[&str] = &[
    "p_turbo", "p_annot", "p_annot_q", "p_ret", "p_ret_unwrap", "p_u32_annot", "p_u32_q", "p_u32_ret", "p_string",
    "p_generic", "p_own_parse", "p_own_try_into", "p_try_into_ret", "p_default_ret", "p_collect_ret", "p_collect_std",
];
const PARSE: &[&str] = &[
    "f_l_turbo", "f_l_annot", "f_l_annot_q", "f_l_ret", "f_l_ret_unwrap", "f_l_ret_expect", "f_l_ret_maperr",
    "f_l_let_unwrap", "f_l_ok", "f_l_return_kw", "f_l_branch", "f_l_match", "f_l_uwor",
];
const OTHER: &[(&str, &str)] = &[
    ("t_f_annot", "try_into"), ("t_f_annot_q", "try_into"), ("t_f_ret", "try_into"),
    ("d_f_annot", "default"), ("d_f_ret", "default"), ("c_f_annot", "collect"), ("c_f_ret", "collect"),
];

fn fixture(tag: &str, deps: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1625-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"),
    )
    .unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, off: Option<&str>) -> HashMap<String, Vec<String>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
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

fn inf(rows: &HashMap<String, Vec<String>>, f: &str) -> Vec<String> {
    rows.get(f).cloned().unwrap_or_default()
}

fn fs() -> Vec<String> {
    vec!["Fs".to_string()]
}

#[test]
fn r1625_a_pure_twins_stay_absent() {
    let d = fixture("pure", "", LOCAL);
    for off in [None, Some("all")] {
        let rows = scan(&d, off);
        for f in PURE {
            assert!(inf(&rows, f).is_empty(), "{f} (off={off:?}): a pure target must not gain a charge: {:?}", inf(&rows, f));
        }
    }
}

#[test]
fn r1625_b_every_parse_spelling_reaches_from_str() {
    let d = fixture("parse", "", LOCAL);
    let on = scan(&d, None);
    for f in PARSE {
        assert_eq!(inf(&on, f), fs(), "{f}: `L::from_str` reads a file");
    }
    // reach: the parse consumer alone restores the old reading, only the turbofish spelling keeps its edge
    let off = scan(&d, Some("parse"));
    for f in PARSE.iter().filter(|f| **f != "f_l_turbo") {
        assert!(inf(&off, f).is_empty(), "{f}: CANDOR_R1625_OFF=parse must restore the old reading");
    }
    assert_eq!(inf(&off, "f_l_turbo"), fs());
}

#[test]
fn r1625_c_the_same_rule_for_try_into_default_and_collect() {
    let d = fixture("other", "", LOCAL);
    let on = scan(&d, None);
    for (f, _) in OTHER {
        assert_eq!(inf(&on, f), fs(), "{f}");
    }
    // each consumer has its own switch, and switching ONE off leaves the others
    for (kind, name) in [("try_into", "t_f_ret"), ("default", "d_f_ret"), ("collect", "c_f_ret")] {
        let off = scan(&d, Some(kind));
        for (f, k) in OTHER {
            if *k == kind {
                assert!(inf(&off, f).is_empty(), "{f}: CANDOR_R1625_OFF={kind} must restore the old reading");
            } else {
                assert_eq!(inf(&off, f), fs(), "{f}: switching `{kind}` off must leave `{k}`");
            }
        }
        let _ = name;
    }
}

#[test]
fn r1625_d_a_context_that_is_an_argument_is_still_a_residual() {
    // return-position `into` is R1626 (closed in `r1626_return_position_into.rs`), not a residual
    // PINNED RESIDUAL, not a claim of soundness: `take_l(s.parse().unwrap())` is typed by the CALLEE's parameter.
    let d = fixture("arg", "", LOCAL);
    let on = scan(&d, None);
    assert!(inf(&on, "r_l_arg").is_empty(), "r_l_arg: if this starts charging, the residual is closed - move it to PARSE");
}

// ---- a DEPENDENCY target (EXECUTED in the R1623 lane: sqlx 0.9.0 `PgConnectOptions::from_str` reads the `PG*`
// variables and stats the socket directories). Lock-pinned, so the classifier judges the dependency type.
const DEP_LOCK: &str = "version = 3\n\n\
[[package]]\nname = \"sqlx\"\nversion = \"0.9.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n\
[[package]]\nname = \"fx\"\nversion = \"0.1.0\"\ndependencies = [\"sqlx\"]\n";

const DEP_SRC: &str = "\
pub fn f_pg_ret(u: &str) -> sqlx::postgres::PgConnectOptions { u.parse().unwrap() }\n\
pub fn f_pg_ret_q(u: &str) -> Result<sqlx::postgres::PgConnectOptions, sqlx::Error> { u.parse() }\n\
pub fn f_pg_annot_q(u: &str) -> Result<(), sqlx::Error> { let o: sqlx::postgres::PgConnectOptions = u.parse()?; let _ = o; Ok(()) }\n\
pub fn f_pg_turbo(u: &str) -> bool { u.parse::<sqlx::postgres::PgConnectOptions>().is_ok() }\n\
pub fn c_u32(u: &str) -> u32 { u.parse().unwrap() }\n";

#[test]
fn r1625_e_a_dependency_target_reaches_the_classifier() {
    let d = std::env::temp_dir().join(format!("candor-r1625-dep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nsqlx = { version = \"=0.9.0\", default-features = false, features = [\"postgres\"] }\n",
    )
    .unwrap();
    std::fs::write(d.join("Cargo.lock"), DEP_LOCK).unwrap();
    std::fs::write(d.join("src/lib.rs"), DEP_SRC).unwrap();
    let on = scan(&d, None);
    let envfs = vec!["Env".to_string(), "Fs".to_string()];
    for f in ["f_pg_ret", "f_pg_ret_q", "f_pg_annot_q", "f_pg_turbo"] {
        assert_eq!(inf(&on, f), envfs, "{f}");
    }
    assert!(inf(&on, "c_u32").is_empty(), "a std target must not be charged: {:?}", inf(&on, "c_u32"));
    let off = scan(&d, Some("parse"));
    for f in ["f_pg_ret", "f_pg_ret_q", "f_pg_annot_q"] {
        assert!(inf(&off, f).is_empty(), "{f}: CANDOR_R1625_OFF=parse must restore the old reading");
    }
    assert_eq!(inf(&off, "f_pg_turbo"), envfs);
}
