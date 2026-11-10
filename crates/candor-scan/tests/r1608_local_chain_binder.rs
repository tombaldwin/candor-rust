//! SOUNDNESS R1608 — a `let`-bound method chain over a CRATE-LOCAL root is typed by each step's DECLARED return, not by
//! its root. `let f = H::new().find().filter(1); f.all()` with `H::find` declared `-> F` typed `f` as `H`, so `f.all()`
//! formed `H::all`, which no unit declares, and a typed call with no claimant is dropped with no disclosure: the caller
//! read pure over a write (cakedome-0.1.1 `association::util::read_in`, a SELECT). Every case below was BUILT AND EXECUTED
//! in the lane (scratchpad `rustagent-r1608/fxg`): each `c_` case writes a file, `c_pure_peek` writes nothing.
//! `CANDOR_R1608_OFF=1` restores the root reading, and every `c_` defect case must then read empty again — the proof that
//! the fixtures reach the new code.
//!
//! `s_provided` reaches its step through a trait's provided method, which no declaration of `F` names: the root reading
//! stands (and is right). The other `s_` cases are steps whose declaration cannot be used (a projection return, an
//! inherent/trait same-name pair, a twin type leaf's type change). Each runs a write; the property pinned is that the
//! caller is NEVER silent: the binding is left untyped, so the call reaches R1095's floor and discloses `Unknown`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SRC: &str = r#"
#![allow(dead_code)]
fn hit(n: &str) { let _ = std::fs::write(format!("/tmp/r1608-never-run-{n}"), "x"); }
pub struct H;
pub struct F(u8);
impl H {
    pub fn new() -> Self { H }
    pub fn try_new() -> Result<Self, String> { Ok(H) }
    pub fn find(&self) -> F { F(0) }
    pub fn me(self) -> Self { self }
    pub fn conv(self) -> Self { self }
    pub fn hw(&self) { hit("H.hw") }
}
impl F {
    pub fn filter(self, x: u8) -> Self { F(self.0 + x) }
    pub fn all(self) { hit("F.all") }
    pub fn peek(&self) -> u8 { self.0 }
}
pub trait Out { type O; fn out(self) -> Self::O; }
impl Out for H { type O = F; fn out(self) -> Self::O { F(0) } }
pub trait Narrow { fn narrow(self) -> Self where Self: Sized { self } }
impl Narrow for F {}
pub trait ToF { fn to_f(self) -> F; }
impl ToF for H { fn to_f(self) -> F { F(1) } }
pub trait Conv { fn conv(self) -> F; }
impl Conv for H { fn conv(self) -> F { F(2) } }
pub fn mk_h() -> H { H }
pub trait Step { fn step(self) -> Self where Self: Sized { self } }
pub mod a { pub struct T; impl T { pub fn new() -> Self { T } pub fn step(self) -> crate::F { crate::F(3) } } }
pub mod b { pub struct T; impl T { pub fn new() -> Self { T } pub fn tw(self) { crate::hit("b.T.tw") } } impl crate::Step for T {} }

pub struct L;
impl L { pub fn refresh(&mut self) { hit("L.refresh") } }
pub static CACHE: std::sync::RwLock<L> = std::sync::RwLock::new(L);
pub fn s_static_guard() { let mut c = CACHE.write().unwrap(); c.refresh() }

pub type MyRes<T> = Result<T, String>;
impl F { pub fn app(self, x: u8) -> MyRes<Self> { Ok(F(self.0 + x)) } }
pub fn s_alias_self() -> MyRes<()> { let f = H::new().find().app(1)?; f.all(); Ok(()) }

pub fn c_let_chain() { let f = H::new().find().filter(1); f.all() }
pub fn c_let_factory() { let f = mk_h().find(); f.all() }
pub fn c_let_reassign() { let mut f = mk_h().find(); f = f.filter(2); f.all() }
pub fn c_let_plumb() -> Result<(), String> { let f = H::try_new().map_err(|e| e)?.find().filter(1); f.all(); Ok(()) }
pub fn c_let_self_step() { let f = H::new().me().find(); f.all() }
pub fn c_trait_impl_step() { let f = H::new().to_f(); f.all() }
pub fn c_inline() { H::new().find().filter(1).all() }
pub fn c_pure_peek() -> u8 { let f = H::new().find().filter(1); f.peek() }
pub fn c_self_then_hw() { let h = H::new().me(); h.hw() }
pub fn s_projection() { let f = H::new().out(); f.all() }
pub fn s_provided() { let f = H::new().find().narrow(); f.all() }
pub fn s_inherent_vs_trait() { let x = H::new().conv(); x.hw() }
pub fn s_twin() { let t = b::T::new().step(); t.tw() }
"#;

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1608-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    d
}

fn scan(dir: &Path, off: bool) -> HashMap<String, (Vec<String>, Vec<String>)> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1608_OFF"] {
        cmd.env_remove(k);
    }
    if off {
        cmd.env("CANDOR_R1608_OFF", "1");
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

fn fixture(tag: &str) -> PathBuf {
    let d = tmpdir(tag);
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"r1608fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), SRC).unwrap();
    d
}

fn inferred(rows: &HashMap<String, (Vec<String>, Vec<String>)>, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.0.clone()).unwrap_or_default()
}

const DEFECT: &[&str] =
    &["c_let_chain", "c_let_factory", "c_let_reassign", "c_let_plumb", "c_let_self_step", "c_trait_impl_step", "s_provided"];
// typed through a crate-local `Result` alias (`-> MyRes<Self>`): the alias is not a type; the root reading stands
const ALIAS: &[&str] = &["s_alias_self"];

#[test]
fn r1608_let_bound_local_chain_follows_declared_step_returns() {
    let d = fixture("steps");
    let on = scan(&d, false);
    for f in DEFECT {
        assert_eq!(inferred(&on, f), vec!["Fs".to_string()], "{f}: the terminal's edge must be formed through the declared steps");
    }
    // controls: the inline spelling was always right; a binding typed `H` through `-> Self` keeps its edge; no terminal
    // runs nothing.
    for f in ALIAS {
        assert_eq!(inferred(&on, f), vec!["Fs".to_string()], "{f}: a Result alias must not become the binding's type");
    }
    assert_eq!(inferred(&on, "c_inline"), vec!["Fs".to_string()]);
    assert_eq!(inferred(&on, "c_self_then_hw"), vec!["Fs".to_string()]);
    assert!(inferred(&on, "c_pure_peek").is_empty(), "a pure binding must not gain a charge");
    // reach: with the switch off every defect case is silent again
    let off = scan(&d, true);
    for f in DEFECT {
        assert!(inferred(&off, f).is_empty(), "{f}: CANDOR_R1608_OFF must restore the root reading (fixture reaches the change)");
    }
}

#[test]
fn r1608_an_unanswered_step_leaves_the_binding_untyped_never_silent() {
    let d = fixture("stop");
    let on = scan(&d, false);
    for f in ["s_projection", "s_inherent_vs_trait", "s_twin", "s_static_guard"] {
        let r = inferred(&on, f);
        assert!(!r.is_empty(), "{f} runs a write: the caller must not be silent (got {r:?})");
        if r == vec!["Unknown".to_string()] {
            let why = on.get(f).map(|x| x.1.join(";")).unwrap_or_default();
            assert!(why.contains("untyped receiver"), "{f}: the disclosure is the untyped-receiver floor, got {why}");
        }
    }
    // the projection step was typed as the root before, and silent
    let off = scan(&d, true);
    assert!(inferred(&off, "s_projection").is_empty(), "s_projection: pre-change reading was silent (fixture reaches the change)");
}
