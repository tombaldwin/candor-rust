//! SOUNDNESS R1657 (bound projection), R1793, R1795, R1796 and R1794 — the method PROBE over a GENERIC receiver.
//!
//! A bound was stored as its trait LEAF, so a receiver typed by a generic lost what the bound says about it: the `Deref` TARGET
//! (`C: DerefMut<Target = Conn>`, tokio-rustls' `self.session.read_tls()`), a conversion's written ARGUMENT (`P: AsRef<Path>`,
//! `p.as_ref().exists()`, R1793), and the bounds on a PROJECTION (`<C as Has>::A`, `where C::Target: Tr`, R1795). Every unit
//! below whose name ends `_eff` performed its effect when EXECUTED (handoff `rust-typer-impl/`, fixtures `fxproj`/`fxb`/`fxl`), and
//! read ABSENT on 0.42.0 and on main; each pure twin reaches a pure body under a UNIQUE member name.
//!
//! Two SEEDED C3s are pinned (Fable review §2.1): the plotters shape (`Target = C`, `C` generic) and its conversion twin
//! (`C: AsRef<X>`, `X` generic). A string-valued design typed the generic's NAME as a type and deleted the hedge; here both
//! dispatch on the bound with the trait's key and charge the effectful implementor.
//!
//! R1794: a supertrait member's dispatch key names the DECLARING trait, so a chained consumer's implementor joins it.
//!
//! Kill switches: `CANDOR_R1657BP_OFF` (the probe and the `let` binder), `CANDOR_R1794_OFF` (the key).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const SRC: &str = r#"#![allow(dead_code)]
use std::borrow::Borrow;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
fn mark(n: &str) -> usize { std::fs::write(n, "x").map(|_| 1).unwrap_or(0) }
pub struct Conn;
impl Conn { pub fn m_a1(&mut self) -> usize { mark("a1") } pub fn m_a3(&self) -> usize { mark("a3") } pub fn m_h1(&self) -> usize { mark("h1") } }
pub struct PConn;
impl PConn { pub fn q_a1(&mut self) -> usize { 1 } pub fn q_a3(&self) -> usize { 3 } }
// DEREF: the Target, inline / where / generic field / `?Sized`
pub fn a1_eff<C: DerefMut + Deref<Target = Conn>>(c: &mut C) -> usize { c.m_a1() }
pub fn a1_pure<C: DerefMut + Deref<Target = PConn>>(c: &mut C) -> usize { c.q_a1() }
pub fn a3_eff<C>(c: &C) -> usize where C: Deref<Target = Conn> { c.m_a3() }
pub fn a3_pure<C>(c: &C) -> usize where C: Deref<Target = PConn> { c.q_a3() }
pub fn a4_eff<C: ?Sized + Deref<Target = Conn>>(c: &C) -> usize { c.m_a3() }
pub struct Stream<'a, C> { pub session: &'a mut C }
impl<'a, C> Stream<'a, C> where C: DerefMut + Deref<Target = Conn> { pub fn a2_eff(&mut self) -> usize { self.session.m_a1() } }
pub fn a5_eff<C: DerefMut<Target = std::fs::File>>(c: &mut C) -> usize { use std::io::Write; c.write_all(b"y").map(|_| 1).unwrap_or(0) }
pub fn h1_eff<C: Deref<Target = Conn>>(c: &C) -> usize { c.deref().m_h1() }
// CONV (R1793): the conversion's written argument
pub fn b3_eff<C: Borrow<Path>>(c: &C) -> usize { c.borrow().exists() as usize }
pub fn b4_eff(p: impl AsRef<Path>) -> usize { p.as_ref().exists() as usize }
pub fn b4_pure(p: impl AsRef<Path>) -> usize { p.as_ref().is_relative() as usize }
pub fn b5_eff<P: AsRef<Path>>(p: P) -> usize { p.as_ref().metadata().map(|m| m.len() as usize).unwrap_or(0) }
pub fn l1_eff<P: AsRef<Path>>(p: P) -> usize { let q = p.as_ref(); q.exists() as usize }
pub fn l1_pure<P: AsRef<Path>>(p: P) -> usize { let q = p.as_ref(); q.is_absolute() as usize }
pub fn l2_eff(p: impl AsRef<Path>) -> usize { let p = p.as_ref(); p.exists() as usize }
pub fn l5_eff<P: Into<PathBuf>>(p: P) -> usize { p.into().exists() as usize }
// generic shadowing (Fable §2.2): `Loud` names the PARAMETER in s1_shadow and the struct in s1_ctl
pub struct Loud;
impl Loud { pub fn same_s1(&self) -> usize { mark("s1") } }
pub struct Soft;
pub trait QuietTr { fn same_s1(&self) -> usize; }
impl QuietTr for Soft { fn same_s1(&self) -> usize { 0 } }
pub fn s1_shadow<Loud: QuietTr, C: Deref<Target = Loud>>(c: &C) -> usize { c.same_s1() }
pub fn s1_ctl<C: Deref<Target = crate::Loud>>(c: &C) -> usize { c.same_s1() }
// seeded C3 pair: Target = generic; AsRef argument = generic
pub trait DoerD { fn d_run(&self) -> usize; }
pub struct LC;
impl DoerD for LC { fn d_run(&self) -> usize { mark("d") } }
pub trait Tr5 { fn m_s5(&self) -> usize; }
impl Tr5 for LC { fn m_s5(&self) -> usize { mark("s5") } }
impl<C: Tr5, T: Deref<Target = C>> Tr5 for T { fn m_s5(&self) -> usize { self.deref().m_s5() } }
pub fn s12_genarg<X: DoerD, C: AsRef<X>>(c: &C) -> usize { c.as_ref().d_run() }
// generic `self` in a blanket impl (R1796), and a `dyn` Target
pub trait Tr9 { fn m_s9(&self) -> usize; }
impl Loud { pub fn m_s9i(&self) -> usize { mark("s9") } }
impl<T: Deref<Target = Loud>> Tr9 for T { fn m_s9(&self) -> usize { self.m_s9i() } }
pub fn s4_dyntarget<C: Deref<Target = dyn DoerD>>(c: &C) -> usize { c.d_run() }
// PROJECTION (R1795)
pub trait Has { type A: DoerD; fn get(&self) -> Self::A; }
pub fn d1_eff<C: Has>(c: &C) -> usize { c.get().d_run() }
pub fn d4_eff<C: Has>(a: <C as Has>::A) -> usize { a.d_run() }
pub trait DoerD3 { fn d3_run(&self) -> usize; }
impl DoerD3 for LC { fn d3_run(&self) -> usize { mark("d3") } }
pub fn d5_eff<C: Deref>(c: &C) -> usize where C::Target: DoerD3 { c.d3_run() }
// R-A: a Target whose ARGUMENTS decide the member stays disclosed
pub struct Wrap<T>(pub T);
impl<T> Deref for Wrap<T> { type Target = T; fn deref(&self) -> &T { &self.0 } }
pub fn g1_eff<C: Deref<Target = Wrap<Conn>>>(c: &C) -> usize { c.m_a3() }
// a closure parameter shadowing the bounded parameter
pub struct QW(pub Soft);
impl AsRef<Soft> for QW { fn as_ref(&self) -> &Soft { &self.0 } }
impl Soft { pub fn same_l4(&self) -> usize { 0 } }
impl Loud { pub fn same_l4(&self) -> usize { mark("l4") } }
pub fn l12_shadow<P: AsRef<Loud>>(p: &P, v: Vec<QW>) -> usize { v.iter().map(|p| p.as_ref().same_l4()).sum() }
// R1794: a supertrait member called through the sub-trait bound
pub trait BaseS { fn s_go(&self) -> usize; }
pub trait SubS: BaseS {}
pub fn run_sup<T: SubS>(t: &T) -> usize { t.s_go() }
"#;

const EFF: [&str; 20] = [
    "a1_eff", "a3_eff", "a4_eff", "Stream::a2_eff", "a5_eff", "h1_eff", "b3_eff", "b4_eff", "b5_eff", "l1_eff", "l2_eff",
    "l5_eff", "s1_ctl", "T::m_s5", "s12_genarg", "T::m_s9", "s4_dyntarget", "d1_eff", "d4_eff", "d5_eff",
];
const PURE: [&str; 6] = ["a1_pure", "a3_pure", "b4_pure", "l1_pure", "s1_shadow", "l12_shadow"];

fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("candor-r1657bp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    std::fs::write(root.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(root.join("app/src/lib.rs"), SRC).unwrap();
    root
}

fn scan(root: &Path, policy: Option<&str>, env: &[(&str, &str)]) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(root.join("app").to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = root.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    for (k, _) in std::env::vars() {
        if k.starts_with("CANDOR_") {
            cmd.env_remove(k);
        }
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    let refused_by_switch = !env.is_empty() && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("matched NO function"), "a policy rule bound nothing, so its exit says nothing: {err}");
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f) && r.get("interfaceUnion").is_none())
}

fn field(v: &serde_json::Value, f: &str, k: &str) -> Vec<String> {
    row(v, f)
        .and_then(|r| r[k].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

#[test]
fn r1657bp_effects_through_a_generic_receiver_are_charged_and_pure_twins_are_not() {
    let d = fixture("on");
    let (_, v) = scan(&d, None, &[]);
    for u in EFF {
        assert!(field(&v, u, "inferred").contains(&"Fs".to_string()), "{u} must carry Fs: {v:#}");
    }
    for u in PURE {
        assert!(!field(&v, u, "inferred").contains(&"Fs".to_string()), "{u} is a pure twin: {v:#}");
    }
    for u in ["a1_eff", "b4_eff", "l2_eff", "d5_eff"] {
        assert_eq!(scan(&d, Some(&format!("deny Fs  {u}\n")), &[]).0, 1, "deny Fs {u}");
    }
}

#[test]
fn r1657bp_the_seeded_c3_pair_dispatches_on_the_bound_with_its_key() {
    let d = fixture("c3");
    let (_, v) = scan(&d, None, &[]);
    assert!(field(&v, "T::m_s5", "dispatchesOn").iter().any(|k| k.ends_with("#Tr5::m_s5")), "{v:#}");
    assert!(field(&v, "s12_genarg", "dispatchesOn").iter().any(|k| k.ends_with("#DoerD::d_run")), "{v:#}");
    // the shadowing generic dispatches on its OWN bound (QuietTr), never on the struct named like it
    assert!(field(&v, "s1_shadow", "dispatchesOn").iter().any(|k| k.ends_with("#QuietTr::same_s1")), "{v:#}");
}

#[test]
fn r1657bp_a_target_whose_arguments_decide_the_member_stays_disclosed() {
    let d = fixture("ra");
    let (_, v) = scan(&d, None, &[]);
    assert!(field(&v, "g1_eff", "inferred").contains(&"Unknown".to_string()), "{v:#}");
}

#[test]
fn r1657bp_switch_off_reproduces_the_silence() {
    let d = fixture("off");
    let (_, v) = scan(&d, None, &[("CANDOR_R1657BP_OFF", "1")]);
    for u in ["a1_eff", "b4_eff", "l1_eff", "d4_eff", "d5_eff", "T::m_s9"] {
        assert!(!field(&v, u, "inferred").contains(&"Fs".to_string()), "{u} was charged with the switch off: {v:#}");
    }
    assert_eq!(scan(&d, Some("deny Fs  a1_eff\n"), &[("CANDOR_R1657BP_OFF", "1")]).0, 2);
}

#[test]
fn r1794_a_supertrait_members_key_names_the_declaring_trait() {
    let d = fixture("r1794");
    let (_, v) = scan(&d, None, &[]);
    let k = field(&v, "run_sup", "dispatchesOn");
    assert!(k.iter().any(|k| k.ends_with("#BaseS::s_go")), "{k:?}");
    assert!(!k.iter().any(|k| k.ends_with("#SubS::s_go")), "SubS does not declare s_go: {k:?}");
    let (_, off) = scan(&d, None, &[("CANDOR_R1794_OFF", "1")]);
    assert!(field(&off, "run_sup", "dispatchesOn").iter().any(|k| k.ends_with("#SubS::s_go")), "{off:#}");
}
