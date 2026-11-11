//! SOUNDNESS R1692 — AN `<untyped>` MARKER THAT NO DEPENDENCY WILL CONSUME MUST NOT STAND THE R1095 UNTYPED-RECEIVER FLOOR ASIDE.
//!
//! `let t = T::mk(k)?; t.write_out(k)` over a generic `T: Mk` (a crate-local trait whose implementor `Disk::write_out` writes a
//! file and `Other::write_out` reads the environment): the callee path is multi-segment, so the binding keeps a dependency
//! provenance and the call files a `T::<untyped>::mk::write_out` marker. The collector read that marker as a call the site formed
//! and so never handed `write_out` to the R1095 judge; `scan.rs` then dropped the marker because `T` is no declared dependency.
//! The row read `[]` with no disclosure, while `.map_err(..)?` (which files no marker) hedged `Unknown`. The same for `Self::mk`
//! in a trait default, `<T as Mk>::mk`, a closure body, and the dependency-trait spellings (`M::new_from_slice(k)?` over
//! `M: backoff::Backoff`). EXECUTED (lane scratch `rustagent-r1692/fxF`; `MAIN` below is the driver, and `cargo run` writes
//! one file per `g_*`/`l_*` unit and none for a `p_*` one): `deny Unknown l_bound` and `deny Fs l_bound` were both 0 on
//! aacd3880 and on published 0.41.3. The `.unwrap()`/`.expect()` spellings read `Unknown` on 0.41.3 and `[]` on aacd3880.
//!
//! The marker is still consumed exactly as before when its root IS a declared dependency (`<M as Backoff>::new_from_slice(k)?`
//! roots at `backoff`, κ `invisible` unchained), so that spelling is unchanged here. Kill switch `CANDOR_R1692_OFF`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const DEP: &str = r#"pub mod backoff {
    pub trait Backoff {
        fn new_from_slice(k: &[u8]) -> Result<Self, ()> where Self: Sized;
        fn update(&mut self, k: &[u8]);
    }
    pub trait Quiet {
        fn q_new(k: &[u8]) -> Result<Self, ()> where Self: Sized;
        fn quiet_touch(&mut self, k: &[u8]);
    }
}
"#;

const LIB: &str = r#"// SOUNDNESS R1692 fixture: `let x = Root::f(..)?; x.m()` where `Root` is no dependency (a generic parameter, `Self`, a
// crate-local trait or module). `src/main.rs` runs every `g_*`/`l_*` unit with an implementor whose `m` writes a file
// (Loud / Disk); every `p_*` unit is a pure control (no body of its method writes or reads anything).
use backoff::backoff::{Backoff, Quiet};
fn out(tag: &str) -> String { format!("{}/{}", std::env::var("R1692_OUT").unwrap_or_else(|_| "/tmp".into()), tag) }

pub struct Loud(pub String);
impl Backoff for Loud {
    fn new_from_slice(k: &[u8]) -> Result<Self, ()> { Ok(Loud(String::from_utf8_lossy(k).into_owned())) }
    fn update(&mut self, k: &[u8]) { std::fs::write(out(&self.0), k).unwrap(); }
}
pub struct Loud2;
impl Backoff for Loud2 {
    fn new_from_slice(_k: &[u8]) -> Result<Self, ()> { Ok(Loud2) }
    fn update(&mut self, _k: &[u8]) { let _ = std::env::var("R1692_NEVER"); }
}
pub struct Hush;
impl Quiet for Hush { fn q_new(_k: &[u8]) -> Result<Self, ()> { Ok(Hush) } fn quiet_touch(&mut self, _k: &[u8]) {} }
pub struct Hush2;
impl Quiet for Hush2 { fn q_new(_k: &[u8]) -> Result<Self, ()> { Ok(Hush2) } fn quiet_touch(&mut self, k: &[u8]) { let _ = k.len(); } }

pub trait Mk: Sized {
    fn mk(k: &[u8]) -> Result<Self, ()>;
    fn write_out(&mut self, k: &[u8]);
    fn l_trait_default(k: &[u8]) -> Result<(), ()> { let mut s = Self::mk(k)?; s.write_out(k); Ok(()) }
}
pub struct Disk(pub String);
impl Mk for Disk {
    fn mk(k: &[u8]) -> Result<Self, ()> { Ok(Disk(String::from_utf8_lossy(k).into_owned())) }
    fn write_out(&mut self, k: &[u8]) { std::fs::write(out(&self.0), k).unwrap(); }
}
pub struct Other;
impl Mk for Other { fn mk(_k: &[u8]) -> Result<Self, ()> { Ok(Other) } fn write_out(&mut self, _k: &[u8]) { let _ = std::env::var("R1692_NEVER"); } }
impl Disk {
    pub fn l_self_root(k: &[u8]) -> Result<(), ()> { let mut s = Self::mk(k)?; s.write_out(k); Ok(()) }
}
pub trait MkQ: Sized { fn mkq(k: &[u8]) -> Result<Self, ()>; fn q_read(&mut self, k: &[u8]) -> usize; }
impl MkQ for Hush { fn mkq(_k: &[u8]) -> Result<Self, ()> { Ok(Hush) } fn q_read(&mut self, k: &[u8]) -> usize { k.len() } }
impl MkQ for Hush2 { fn mkq(_k: &[u8]) -> Result<Self, ()> { Ok(Hush2) } fn q_read(&mut self, _k: &[u8]) -> usize { 0 } }

pub mod util {
    use backoff::backoff::{Backoff, Quiet};
    pub fn make<M: Backoff>(k: &[u8]) -> Result<M, ()> { M::new_from_slice(k) }
    pub fn qmake<M: Quiet>(k: &[u8]) -> Result<M, ()> { M::q_new(k) }
    pub fn mkl<T: super::Mk>(k: &[u8]) -> Result<T, ()> { T::mk(k) }
}

// already disclosed on main: the `.map_err` spelling files no marker
pub fn g_maperr<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = <M as Backoff>::new_from_slice(k).map_err(|_| ())?; m.update(k); Ok(()) }
// generic root over a DEPENDENCY trait
pub fn g_bound<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = M::new_from_slice(k)?; m.update(k); Ok(()) }
pub fn g_unwrap<M: Backoff>(k: &[u8]) { let mut m = M::new_from_slice(k).unwrap(); m.update(k); }
pub fn g_expect<M: Backoff>(k: &[u8]) { let mut m = M::new_from_slice(k).expect("k"); m.update(k); }
pub fn g_where<M>(k: &[u8]) -> Result<(), ()> where M: Backoff { let mut m = M::new_from_slice(k)?; m.update(k); Ok(()) }
pub fn g_ok<M: Backoff>(k: &[u8]) -> Option<()> { let mut m = M::new_from_slice(k).ok()?; m.update(k); Some(()) }
pub fn g_closure<M: Backoff>(k: &[u8]) -> Result<(), ()> {
    let f = |k: &[u8]| -> Result<(), ()> { let mut m = M::new_from_slice(k)?; m.update(k); Ok(()) };
    f(k)
}
pub fn g_qself<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = <M as Backoff>::new_from_slice(k)?; m.update(k); Ok(()) }
pub fn g_trait<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m: M = Backoff::new_from_slice(k)?; m.update(k); Ok(()) }
pub fn g_mod<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = util::make::<M>(k)?; m.update(k); Ok(()) }
pub fn g_mod_crate<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = crate::util::make::<M>(k)?; m.update(k); Ok(()) }
pub fn g_mod_infer<M: Backoff>(k: &[u8]) -> Result<(), ()> { let mut m = util::make(k)?; let _: &M = &m; m.update(k); Ok(()) }
pub fn g_mod_unwrap<M: Backoff>(k: &[u8]) { let mut m = util::make::<M>(k).unwrap(); m.update(k); }
pub struct Gen<M>(pub std::marker::PhantomData<M>);
impl<M: Backoff> Gen<M> {
    pub fn g_assoc(k: &[u8]) -> Result<(), ()> { let mut m = M::new_from_slice(k)?; m.update(k); Ok(()) }
}
// generic root over a CRATE-LOCAL trait
pub fn l_bound<T: Mk>(k: &[u8]) -> Result<(), ()> { let mut t = T::mk(k)?; t.write_out(k); Ok(()) }
pub fn l_unwrap<T: Mk>(k: &[u8]) { let mut t = T::mk(k).unwrap(); t.write_out(k); }
pub fn l_qself<T: Mk>(k: &[u8]) -> Result<(), ()> { let mut t = <T as Mk>::mk(k)?; t.write_out(k); Ok(()) }
pub fn l_closure<T: Mk>(k: &[u8]) -> Result<(), ()> { let f = |k: &[u8]| -> Result<(), ()> { let mut t = T::mk(k)?; t.write_out(k); Ok(()) }; f(k) }
pub fn l_mod<T: Mk>(k: &[u8]) -> Result<(), ()> { let mut t = util::mkl::<T>(k)?; t.write_out(k); Ok(()) }
pub fn l_mod_infer<T: Mk>(k: &[u8]) -> Result<(), ()> { let mut t = util::mkl(k)?; let _: &T = &t; t.write_out(k); Ok(()) }
pub fn l_ann<T: Mk>(k: &[u8]) -> Result<(), ()> { let mut t: T = T::mk(k)?; t.write_out(k); Ok(()) }
// PURE controls, same spellings
pub fn p_bound<M: Quiet>(k: &[u8]) -> Result<(), ()> { let mut m = M::q_new(k)?; m.quiet_touch(k); Ok(()) }
pub fn p_unwrap<M: Quiet>(k: &[u8]) { let mut m = M::q_new(k).unwrap(); m.quiet_touch(k); }
pub fn p_mod<M: Quiet>(k: &[u8]) -> Result<(), ()> { let mut m = util::qmake::<M>(k)?; m.quiet_touch(k); Ok(()) }
pub fn p_local<T: MkQ>(k: &[u8]) -> Result<usize, ()> { let mut t = T::mkq(k)?; Ok(t.q_read(k)) }
pub fn p_qself<T: MkQ>(k: &[u8]) -> Result<usize, ()> { let mut t = <T as MkQ>::mkq(k)?; Ok(t.q_read(k)) }
// a std PRELUDE root keeps R1540's std-name rule: `len` is std's on a `String`, whatever this crate's `Disk::len` does
impl Disk { pub fn len(&self) -> usize { let _ = std::fs::read(out("never")); 0 } }
pub fn p_prelude(k: &[u8]) -> usize { let s = String::from_utf8(k.to_vec()).unwrap(); s.len() }
// …and a PRIMITIVE root: `abs_diff` on a `u8` is std's inherent method, whatever this crate's `Disk::abs_diff` does
impl Disk { pub fn abs_diff(&self, _o: u8) -> u8 { let _ = std::fs::read(out("never")); 0 } }
pub fn p_prim(k: &[u8]) -> Result<u8, ()> { let n = u8::try_from(k.len()).map_err(|_| ())?; let m = u8::try_from(k.len()).unwrap(); Ok(m.abs_diff(n)) }
"#;

/// The executed driver. Not scanned (the fixture declares no `[[bin]]`); kept so the program the assertions describe is on record.
#[allow(dead_code)]
const MAIN: &str = r#"use app::*;
fn main() {
    g_maperr::<Loud>(b"g_maperr").unwrap();
    g_bound::<Loud>(b"g_bound").unwrap();
    g_unwrap::<Loud>(b"g_unwrap");
    g_expect::<Loud>(b"g_expect");
    g_where::<Loud>(b"g_where").unwrap();
    g_ok::<Loud>(b"g_ok").unwrap();
    g_closure::<Loud>(b"g_closure").unwrap();
    g_qself::<Loud>(b"g_qself").unwrap();
    g_trait::<Loud>(b"g_trait").unwrap();
    g_mod::<Loud>(b"g_mod").unwrap();
    g_mod_crate::<Loud>(b"g_mod_crate").unwrap();
    g_mod_infer::<Loud>(b"g_mod_infer").unwrap();
    g_mod_unwrap::<Loud>(b"g_mod_unwrap");
    Gen::<Loud>::g_assoc(b"g_assoc").unwrap();
    l_bound::<Disk>(b"l_bound").unwrap();
    l_unwrap::<Disk>(b"l_unwrap");
    l_qself::<Disk>(b"l_qself").unwrap();
    l_closure::<Disk>(b"l_closure").unwrap();
    l_mod::<Disk>(b"l_mod").unwrap();
    l_mod_infer::<Disk>(b"l_mod_infer").unwrap();
    l_ann::<Disk>(b"l_ann").unwrap();
    Disk::l_self_root(b"l_self_root").unwrap();
    <Disk as Mk>::l_trait_default(b"l_trait_default").unwrap();
    p_bound::<Hush>(b"p_bound").unwrap();
    p_unwrap::<Hush>(b"p_unwrap");
    p_mod::<Hush>(b"p_mod").unwrap();
    p_local::<Hush>(b"p_local").unwrap();
    p_qself::<Hush>(b"p_qself").unwrap();
    p_prelude(b"p_prelude");
    p_prim(b"p_prim").unwrap();
}
"#;

fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("candor-r1692-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("backoff/src")).unwrap();
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    std::fs::write(root.join("backoff/Cargo.toml"), "[package]\nname = \"backoff\"\nversion = \"0.4.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(root.join("backoff/src/lib.rs"), DEP).unwrap();
    std::fs::write(
        root.join("app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nbackoff = { path = \"../backoff\" }\n",
    )
    .unwrap();
    std::fs::write(root.join("app/src/lib.rs"), LIB).unwrap();
    root
}

fn scan(root: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
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
    if off {
        cmd.env("CANDOR_R1692_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn strs(v: &serde_json::Value, f: &str, key: &str) -> Vec<String> {
    row(v, f).and_then(|r| r[key].as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// Every executed unit whose only typing was an inert marker, and the method its receiver calls.
const HEDGED: &[(&str, &str)] = &[
    ("l_bound", "write_out"),
    ("l_unwrap", "write_out"),
    ("l_qself", "write_out"),
    ("l_closure", "write_out"),
    ("Mk::l_trait_default", "write_out"),
    ("g_bound", "update"),
    ("g_unwrap", "update"),
    ("g_expect", "update"),
    ("g_where", "update"),
    ("g_closure", "update"),
    ("Gen::g_assoc", "update"),
];

#[test]
fn r1692_an_inert_marker_root_reaches_the_untyped_receiver_floor() {
    let d = fixture("on");
    let (_, on) = scan(&d, None, false);
    for (u, m) in HEDGED {
        assert_eq!(strs(&on, u, "inferred"), ["Unknown"], "{u}: {on:#}");
        let want = format!("dispatch:untyped receiver of `{m}`");
        assert!(strs(&on, u, "unknownWhy").iter().any(|w| w.starts_with(&want)), "{u}: {on:#}");
    }
    // The fixture reaches the effect: the typed spellings over the same trait are charged, and `.map_err` already hedged.
    for u in ["l_ann", "l_mod", "l_mod_infer", "Disk::l_self_root"] {
        assert!(strs(&on, u, "inferred").contains(&"Fs".to_string()), "{u}: {on:#}");
    }
    assert_eq!(strs(&on, "g_maperr", "inferred"), ["Unknown"], "{on:#}");
    // PURE controls, same spellings: no candidate of `quiet_touch` / `q_read` carries an effect, so nothing is disclosed.
    for u in ["p_bound", "p_unwrap", "p_mod", "p_local", "p_qself"] {
        assert!(row(&on, u).is_some(), "{u} is a dispatching row: {on:#}");
        assert!(strs(&on, u, "inferred").is_empty(), "{u}: {on:#}");
    }
    // A std PRELUDE root keeps R1540's std-name rule: `s.len()` on a `String` is std's, whatever `Disk::len` does.
    assert!(row(&on, "p_prelude").is_none(), "{on:#}");
    // …and a PRIMITIVE root (`u8::try_from(..).unwrap()`): `abs_diff` is std's inherent method on `u8`.
    assert!(row(&on, "p_prim").is_none(), "{on:#}");
    // A DEPENDENCY root is consumed by the marker's own route and not judged here: unchanged (κ `invisible`, no `Unknown`).
    assert!(strs(&on, "g_qself", "inferred").is_empty(), "{on:#}");
    assert_eq!(strs(&on, "g_qself", "invisible"), ["backoff"], "{on:#}");
    // THE GATE FLIP over code that writes a file, and its pure twin.
    let (rc, _) = scan(&d, Some("deny Unknown l_bound\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let (rc, _) = scan(&d, Some("deny Unknown Mk::l_trait_default\n"), false);
    assert_eq!(rc, 1);
    let (rc_pure, _) = scan(&d, Some("deny Unknown p_local\n"), false);
    assert_eq!(rc_pure, 0);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1692_the_kill_switch_restores_the_silent_row() {
    let d = fixture("off");
    let (_, off) = scan(&d, None, true);
    for (u, _) in HEDGED {
        assert!(row(&off, u).is_some(), "{u}: {off:#}");
        assert!(!strs(&off, u, "inferred").contains(&"Unknown".to_string()), "CANDOR_R1692_OFF restores {u} {off:#}");
    }
    let (refused, _) = scan(&d, Some("deny Unknown l_bound\n"), true);
    assert_eq!(refused, 2, "a gate under a result-moving switch is refused (SPEC 0.42 §3.4)");
    let _ = std::fs::remove_dir_all(&d);
}
