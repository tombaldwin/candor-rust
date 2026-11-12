//! SOUNDNESS R1717 (+ R1719) — THREE SILENCES THE R1657 STAGE 1b `self` TYPING EXPOSED, ONE OF WHICH WAS ON EVERY ARM.
//!
//! (1) THE `builder` ENTRY NAME IS NOT A RETURN TYPE. `sp::Proc::builder(t).build()` with `impl Proc { fn builder(..) -> ProcBuilder }`: `is_ctor`
//!     typed the root as `Proc` (written for a dependency's `Client::builder()`), so the terminal formed `Proc::build`, which no unit declares,
//!     and a typed call with no claimant is dropped with no disclosure. ABSENT on published 0.41.3 and on every switch-off arm; EXECUTED below.
//!     (opentelemetry_sdk 0.31 `TracerProviderBuilder::with_batch_exporter`, whose `BatchSpanProcessorBuilder::build` is `Clock`.) The claim
//!     "renaming the terminal charges it" in the release-bar report is FALSE: `finish`/`done`/`make`/`new`/`get` all drop too.
//! (2) A BINDING OF A CRATE-LOCAL METHOD'S RESULT THAT NO ROUTE TYPED (`let mut a = self.attrs();` where two `Entry` types declare `attrs` with
//!     different returns) and then a STD-NAMED method on it (`a.next()`): the R1095 judge stands aside for a std-named leaf (R1666), so a callee
//!     that is `Unknown` only left the caller `[]` with no `unknownWhy` (gimli 0.32 `DebuggingInformationEntry::attr`, 17 units). Before 1b an
//!     accidental `ambiguous:` hedge covered it; the pure twin crate below shows published 0.41.3's hedge was a FABRICATION as well.
//! (3) `.await` of a future whose TYPE LEAF is declared twice with only one twin a future (tokio `Notified`): `local_future_leaf` refuses a twin
//!     that is not provably a future, so no `poll` edge and no disclosure (tokio 1.25 `watch::Sender::closed`). R1719: `.await` straight on a
//!     method call's result (`n.notified().await`) read the receiver's own type, not the declared return, so even the non-twin shape was silent.
//!
//! EXECUTED GROUND TRUTH (`cargo run` of `MAIN` over `LIB`, `R1717_OUT=<dir>`): one file per `g*` unit — g1_direct, g1_bound, g1_same, g1_alias, g2_attr,
//! g3_let, g3_inline. `PURE` is the same shapes with no effect anywhere; it must stay pure. Kill switches `CANDOR_R1717_OFF`, `CANDOR_R1719_OFF`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const LIB: &str = r#"// SOUNDNESS R1717 fixture. `src/main.rs` runs every `g*` unit; each writes a file named by its tag (observable ground truth).
// Every `p*` unit is a pure twin of the same shape (the effect is absent, nothing else differs).
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
pub fn out(tag: &str) -> String { format!("{}/{}", std::env::var("R1717_OUT").unwrap_or_else(|_| "/tmp".into()), tag) }

// (1) a constructor name whose declaration says it returns ANOTHER type.
pub mod sp {
    pub struct Proc;
    pub struct ProcBuilder(pub String);
    impl Proc { pub fn builder(t: &str) -> ProcBuilder { ProcBuilder(t.to_string()) } }
    impl ProcBuilder { pub fn build(self) -> Proc { std::fs::write(crate::out(&self.0), b"x").unwrap(); Proc } }
    pub struct Same(pub String);
    impl Same { pub fn new(t: &str) -> Same { Same(t.to_string()) } pub fn go(&self) { std::fs::write(crate::out(&self.0), b"x").unwrap(); } }
}
pub fn g1_direct(t: &str) -> sp::Proc { sp::Proc::builder(t).build() }
pub fn g1_bound(t: &str) -> sp::Proc { let b = sp::Proc::builder(t); b.build() }
pub fn g1_same(t: &str) { sp::Same::new(t).go() }

// (2) a binding of a method result left untyped (the twin `Entry::attrs` returns different types), then a STD-NAMED method.
pub mod rd {
    pub struct Entry { pub tag: String, pub f: Box<dyn Fn(&str)> }
    pub struct It<'a> { pub e: &'a Entry }
    impl Entry {
        pub fn attrs<'me>(&'me self) -> It<'me> { It { e: self } }
        pub fn attr(&self) -> Option<u8> { let mut a = self.attrs(); while let Some(x) = a.next() { return Some(x); } None }
    }
    impl<'a> It<'a> { pub fn next(&mut self) -> Option<u8> { (self.e.f)(&self.e.tag); Some(1) } }
}
pub mod wr {
    pub struct Entry { pub v: Vec<u8> }
    impl Entry { pub fn attrs(&self) -> std::slice::Iter<'_, u8> { self.v.iter() } }
}

// (3) `.await` of a future whose type leaf is declared twice (only one twin is a future), and of a method call's result.
pub struct Notify { pub tag: String }
pub struct Notified<'a> { n: &'a Notify }
impl Notify { pub fn notified(&self) -> Notified<'_> { Notified { n: self } } }
impl<'a> Future for Notified<'a> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> { std::fs::write(out(&self.n.tag), b"x").unwrap(); Poll::Ready(()) }
}
pub mod task { pub struct Notified<S>(pub S); impl<S> Notified<S> { pub fn run(self) {} } }
pub async fn g3_let(n: &Notify) { let x = n.notified(); x.await; }
pub async fn g3_inline(n: &Notify) { n.notified().await; }

// CONTROL for (1): a ctor whose declared return is a type ALIAS (`RedisResult<Client>`) is the root type, not the alias (redis 1.7.1 sentinel).
pub type AliasResult<T> = Result<T, ()>;
pub struct Cl(pub String);
impl Cl {
    pub fn open(t: &str) -> AliasResult<Cl> { Ok(Cl(t.to_string())) }
    pub fn get_connection(&self) { std::fs::write(out(&self.0), b"x").unwrap(); }
}
pub fn g1_alias(t: &str) -> AliasResult<()> { let c = Cl::open(t)?; c.get_connection(); Ok(()) }
"#;

const PURE: &str = r#"// SOUNDNESS R1717 fixture, PURE TWIN CRATE: every shape of `app`, with no effect anywhere.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
pub mod sp {
    pub struct PProc;
    pub struct PProcBuilder(pub String);
    impl PProc { pub fn builder(t: &str) -> PProcBuilder { PProcBuilder(t.to_string()) } }
    impl PProcBuilder { pub fn build(self) -> PProc { let _ = self.0.len(); PProc } }
}
pub fn p1_direct(t: &str) -> sp::PProc { sp::PProc::builder(t).build() }
pub mod rd {
    pub struct PEntry { pub n: u8 }
    pub struct PIt<'a> { pub e: &'a PEntry }
    impl PEntry {
        pub fn pattrs<'me>(&'me self) -> PIt<'me> { PIt { e: self } }
        pub fn pattr(&self) -> Option<u8> { let mut a = self.pattrs(); while let Some(x) = a.next() { return Some(x); } None }
    }
    impl<'a> PIt<'a> { pub fn next(&mut self) -> Option<u8> { Some(self.e.n) } }
}
pub mod wr {
    pub struct PEntry { pub v: Vec<u8> }
    impl PEntry { pub fn pattrs(&self) -> std::slice::Iter<'_, u8> { self.v.iter() } }
}
pub struct PNotify;
pub struct PNotified<'a> { n: &'a PNotify }
impl PNotify { pub fn pnotified(&self) -> PNotified<'_> { PNotified { n: self } } }
impl<'a> Future for PNotified<'a> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> { let _ = &self.n; Poll::Ready(()) }
}
pub mod task2 { pub struct PNotified<S>(pub S); impl<S> PNotified<S> { pub fn run(self) {} } }
pub async fn p3_let(n: &PNotify) { let x = n.pnotified(); x.await; }
pub async fn p3_inline(n: &PNotify) { n.pnotified().await; }
"#;

#[allow(dead_code)]
const MAIN: &str = r#"use app::*;
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {} fn clone(_: *const ()) -> RawWaker { RawWaker::new(std::ptr::null(), &VT) }
    static VT: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let w = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
    let mut cx = Context::from_waker(&w); let mut f = Box::pin(f);
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    g1_direct("g1_direct"); g1_bound("g1_bound"); g1_same("g1_same"); g1_alias("g1_alias").unwrap();
    let e = rd::Entry { tag: "g2_attr".into(), f: Box::new(|t| { std::fs::write(out(t), b"x").unwrap(); }) };
    e.attr();
    let _ = wr::Entry { v: vec![] }.attrs().count();
    block_on(g3_let(&Notify { tag: "g3_let".into() }));
    block_on(g3_inline(&Notify { tag: "g3_inline".into() }));
    task::Notified(1u8).run();
}
"#;

fn fixture(tag: &str, src: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("candor-r1717-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    std::fs::write(root.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(root.join("app/src/lib.rs"), src).unwrap();
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
        cmd.env("CANDOR_R1717_OFF", "1").env("CANDOR_R1719_OFF", "1");
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

#[test]
fn r1717_the_three_silences_are_charged_or_disclosed() {
    let d = fixture("on", LIB);
    let (_, on) = scan(&d, None, false);
    // (1) a ctor whose declaration says another type: the terminal is edged (direct, let-bound), and a ctor that returns `Self` is unchanged.
    for u in ["g1_direct", "g1_bound", "g1_same", "g1_alias"] {
        assert!(strs(&on, u, "inferred").contains(&"Fs".to_string()), "{u}: {on:#}");
    }
    // (2) the std-named method on an untyped binding of a local method's result is disclosed, with the reason that is true.
    assert_eq!(strs(&on, "rd::Entry::attr", "inferred"), ["Unknown"], "{on:#}");
    assert!(strs(&on, "rd::Entry::attr", "unknownWhy").iter().any(|w| w.starts_with("dispatch:untyped receiver of `next`")), "{on:#}");
    // (3) `.await` of a twin-leaf future, bound and inline.
    for u in ["g3_let", "g3_inline"] {
        assert!(strs(&on, u, "inferred").contains(&"Fs".to_string()), "{u}: {on:#}");
    }
    // THE GATE FLIPS over code that really writes a file (the OFF test shows each at exit 0 before).
    for (e, u) in [("Fs", "g1_direct"), ("Fs", "g1_bound"), ("Unknown", "rd::Entry::attr"), ("Fs", "g3_let"), ("Fs", "g3_inline")] {
        let (rc, _) = scan(&d, Some(&format!("deny {e} {u}\n")), false);
        assert_eq!(rc, 1, "deny {e} {u}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1717_the_pure_twins_stay_pure() {
    let d = fixture("pure", PURE);
    let (_, on) = scan(&d, None, false);
    for r in on["functions"].as_array().unwrap() {
        let inferred = r["inferred"].as_array().unwrap();
        assert!(inferred.is_empty(), "a pure twin was charged or hedged: {r:#}");
    }
    for u in ["p1_direct", "rd::PEntry::pattr", "p3_let", "p3_inline"] {
        let (rc, _) = scan(&d, Some(&format!("deny Unknown {u}\ndeny Fs {u}\n")), false);
        assert_eq!(rc, 0, "{u}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1717_the_kill_switches_restore_the_silent_rows() {
    let d = fixture("off", LIB);
    let (_, off) = scan(&d, None, true);
    for u in ["g1_direct", "g1_bound", "rd::Entry::attr", "g3_let", "g3_inline"] {
        assert!(row(&off, u).is_none(), "the switches restore the silent row for {u}: {off:#}");
    }
    assert!(strs(&off, "g1_same", "inferred").contains(&"Fs".to_string()), "{off:#}");
    let (refused, _) = scan(&d, Some("deny Fs g1_direct\n"), true);
    assert_eq!(refused, 2, "a gate under a result-moving switch is refused (SPEC 0.42 §3.4)");
    let _ = std::fs::remove_dir_all(&d);
}
