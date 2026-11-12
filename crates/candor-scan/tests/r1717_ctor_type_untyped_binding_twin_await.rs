//! SOUNDNESS R1717 (+ R1719) — THREE SILENCES THE R1657 STAGE 1b `self` TYPING EXPOSED, ONE OF WHICH WAS ON EVERY ARM.
//!
//! (1) THE `builder` ENTRY NAME IS NOT A RETURN TYPE. `sp::Proc::builder(t).build()` with `impl Proc { fn builder(..) -> ProcBuilder }`: `is_ctor`
//!     typed the root as `Proc` (written for a dependency's `Client::builder()`), so the terminal formed `Proc::build`, which no unit declares,
//!     and a typed call with no claimant is dropped with no disclosure. ABSENT on published 0.41.3 and on every switch-off arm; EXECUTED below.
//!     (opentelemetry_sdk 0.31 `TracerProviderBuilder::with_batch_exporter`, whose `BatchSpanProcessorBuilder::build` is `Clock`.) The claim
//!     "renaming the terminal charges it" in the release-bar report is FALSE: `assemble`/`done`/`make`/`new`/`get` all drop too.
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

// CONTROL for (1) (x11rb-protocol): a value constructor returning `Result<Option<Entries>>` is filed under the WRAPPER's leaf `Option`, which this
// crate also declares as a struct elsewhere; it must keep the root reading (the A/B lost `XAuthorityEntries::next` when it did not).
pub mod shadow { pub struct Option(pub u8); }
pub struct Entries(pub String);
impl Entries {
    pub fn new(t: &str) -> Result<std::option::Option<Entries>, ()> { Ok(Some(Entries(t.to_string()))) }
    pub fn run(&self) { std::fs::write(out(&self.0), b"x").unwrap(); }
}
pub fn g1_option(t: &str) { let e = Entries::new(t).unwrap().unwrap(); e.run(); }
pub fn g1_option_match(t: &str) { match Entries::new(t).unwrap() { Some(e) => e.run(), None => {} } }

// (4) `Client::builder().build()?.get(u)` over TWIN `Client`/`ClientBuilder` types and a STD-NAMED `get` (reqwest `blocking::get`): the typed tail
// names no unit, and the std-name exclusion of the R1663 hedge made it vanish. Now disclosed AT the `get` call.
pub type Res<T> = Result<T, ()>;
pub mod web {
    pub struct Client(pub String);
    pub struct ClientBuilder(pub String);
    impl Client {
        pub fn builder(t: &str) -> ClientBuilder { ClientBuilder(t.to_string()) }
        pub fn get(&self) -> u8 { std::fs::write(crate::out(&self.0), b"x").unwrap(); 1 }
    }
    impl ClientBuilder { pub fn assemble(self) -> crate::Res<Client> { Ok(Client(self.0)) } }
    pub mod blocking {
        pub struct Client(pub String);
        pub struct ClientBuilder(pub String);
        impl Client {
            pub fn builder(t: &str) -> ClientBuilder { ClientBuilder(t.to_string()) }
            pub fn get(&self) -> u8 { std::fs::write(crate::out(&self.0), b"x").unwrap(); 1 }
        }
        impl ClientBuilder { pub fn assemble(self) -> crate::Res<Client> { Ok(Client(self.0)) } }
        pub fn g4_get(t: &str) -> crate::Res<u8> { Ok(Client::builder(t).assemble()?.get()) }
    }
}
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

/// Fix 4's own fixture: BUILDER types are distinct, so no other site in `g4b_get` is ambiguous and the hop hedge is the ONLY disclosure of `get`.
const LIB2: &str = r#"pub fn out(tag: &str) -> String { format!("{}/{}", std::env::var("R1717_OUT").unwrap_or_else(|_| "/tmp".into()), tag) }
pub type Res<T> = Result<T, ()>;// ONLY disclosure of `get`.
pub mod web2 {
    pub struct Client(pub String);
    pub struct BuilderA(pub String);
    impl Client {
        pub fn builder(t: &str) -> BuilderA { BuilderA(t.to_string()) }
        pub fn get(&self) -> u8 { std::fs::write(crate::out(&self.0), b"x").unwrap(); 1 }
    }
    impl BuilderA { pub fn assemble_a(self) -> crate::Res<Client> { Ok(Client(self.0)) } }
    pub mod inner {
        pub struct Client(pub String);
        impl Client {
            pub fn get(&self) -> u8 { std::fs::write(crate::out(&self.0), b"x").unwrap(); 1 }
        }
        pub fn g4b_get(t: &str) -> crate::Res<u8> { Ok(super::Client::builder(t).assemble_a()?.get()) }
    }
}
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
    g1_direct("g1_direct"); g1_bound("g1_bound"); g1_same("g1_same"); g1_alias("g1_alias").unwrap(); g1_option("g1_option"); g1_option_match("g1_option_match"); web::blocking::g4_get("g4_get").unwrap();
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
    for u in ["g1_direct", "g1_bound", "g1_same", "g1_alias", "g1_option", "g1_option_match"] {
        assert!(strs(&on, u, "inferred").contains(&"Fs".to_string()), "{u}: {on:#}");
    }
    // (2) the std-named method on an untyped binding of a local method's result is disclosed, with the reason that is true.
    assert_eq!(strs(&on, "rd::Entry::attr", "inferred"), ["Unknown"], "{on:#}");
    assert!(strs(&on, "rd::Entry::attr", "unknownWhy").iter().any(|w| w.starts_with("dispatch:untyped receiver of `next`")), "{on:#}");
    // (4) the twin-type chain over a std-named tail: disclosed AT the `get` call (the reason names it).
    assert!(strs(&on, "web::blocking::g4_get", "unknownWhy").iter().any(|w| w.starts_with("dispatch:untyped receiver of `get`")), "{on:#}");
    // (3) `.await` of a twin-leaf future, bound and inline.
    for u in ["g3_let", "g3_inline"] {
        assert!(strs(&on, u, "inferred").contains(&"Fs".to_string()), "{u}: {on:#}");
    }
    // THE GATE FLIPS over code that really writes a file (the OFF test shows each at exit 0 before).
    for (e, u) in [("Fs", "g1_direct"), ("Fs", "g1_bound"), ("Unknown", "rd::Entry::attr"), ("Fs", "g3_let"), ("Fs", "g3_inline"), ("Unknown", "web::blocking::g4_get")] {
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

/// CALIBRATION of fix 4 ALONE (the std-named-tail hedge after a hop). `CANDOR_R1717_OFF` also turns fix 1 off, which puts the root reading back, so it
/// cannot show the hedge is what makes the assertion pass; `CANDOR_R1717_HOP_OFF` isolates it. Executed ground truth: `g4b_get` writes a file.
/// A gate under a result-moving switch is refused (exit 2), so the "exit 0" half is read off the report: no row at all for `g4b_get`.
#[test]
fn r1717_the_hop_hedge_alone_is_what_discloses_g4b_get() {
    let d = fixture("hop", LIB2);
    let (_, on) = scan(&d, None, false);
    assert_eq!(strs(&on, "web2::inner::g4b_get", "inferred"), ["Unknown"], "{on:#}");
    assert!(strs(&on, "web2::inner::g4b_get", "unknownWhy").iter().any(|w| w.starts_with("dispatch:untyped receiver of `get`")), "{on:#}");
    let (rc, _) = scan(&d, Some("deny Unknown web2::inner::g4b_get\n"), false);
    assert_eq!(rc, 1, "the hop hedge reaches the gate");
    let mut cmd = Command::new(bin());
    cmd.arg(d.join("app").to_string_lossy().as_ref()).arg("--json");
    for (k, _) in std::env::vars() {
        if k.starts_with("CANDOR_") {
            cmd.env_remove(k);
        }
    }
    cmd.env("CANDOR_R1717_HOP_OFF", "1");
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code() == Some(0) || out.status.code() == Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let off: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(row(&off, "web2::inner::g4b_get").is_none(), "fix 4 off: g4b_get reads silent over a real write: {off:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A DEPENDENCY type that shares its leaf with a type this crate declares must not read the local declaration (lambda_runtime_api_client: `hyper::Client::builder()`
/// beside a local `Client::builder() -> ClientBuilder` whose `build` reads the environment). The scan of the whole corpus found it: a fabricated `Env`.
#[test]
fn r1717_a_dependency_type_sharing_a_local_leaf_is_not_retyped() {
    let root = std::env::temp_dir().join(format!("candor-r1717-dep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("hyp/src")).unwrap();
    std::fs::create_dir_all(root.join("app/src")).unwrap();
    std::fs::write(root.join("hyp/Cargo.toml"), "[package]\nname = \"hyp\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(root.join("hyp/src/lib.rs"), "pub struct Client; pub struct B; impl Client { pub fn builder() -> B { B } } impl B { pub fn build(self) -> Client { Client } }\n").unwrap();
    std::fs::write(
        root.join("app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\nhyp = { path = \"../hyp\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/src/lib.rs"),
        "pub struct Client; pub struct ClientBuilder;\n\
         impl Client { pub fn builder() -> ClientBuilder { ClientBuilder } }\n\
         impl ClientBuilder { pub fn build(self) -> Client { let _ = std::env::var(\"X\"); Client } }\n\
         pub fn dep_chain() { let _c = hyp::Client::builder().build(); }\n\
         pub fn local_chain() { let _c = Client::builder().build(); }\n",
    )
    .unwrap();
    let (_, v) = scan(&root, None, false);
    assert!(!strs(&v, "dep_chain", "inferred").contains(&"Env".to_string()), "a dependency's builder was read as the local one: {v:#}");
    assert!(strs(&v, "local_chain", "inferred").contains(&"Env".to_string()), "control: the local chain is charged: {v:#}");
    let _ = std::fs::remove_dir_all(&root);
}

/// R1719 must not read a TWIN receiver's declared return (reqwest 0.11 `wasm::RequestBuilder::send`): `a::Client::exec -> Pending` (a future whose `poll`
/// writes a file) beside `w::Client::exec -> impl Future` (not filed, so no disagreement is recorded). `w::send` awaits ITS OWN future and must not be
/// charged `a::Pending::poll`'s `Fs`.
#[test]
fn r1719_a_twin_receivers_declared_return_is_not_read() {
    let src = "use std::future::Future;\nuse std::pin::Pin;\nuse std::task::{Context, Poll};\n\
        pub mod a {\n\
            use super::*;\n\
            pub struct Client;\n\
            pub struct Pending(pub u8);\n\
            impl Future for Pending { type Output = u8; fn poll(self: Pin<&mut Self>, _c: &mut Context<'_>) -> Poll<u8> { std::fs::write(\"/tmp/r1719\", b\"x\").unwrap(); Poll::Ready(1) } }\n\
            impl Client { pub fn exec(&self) -> Pending { Pending(1) } }\n\
        }\n\
        pub mod w {\n\
            use super::*;\n\
            pub struct Client;\n\
            impl Client { pub fn exec(&self) -> impl Future<Output = u8> { async { 1 } } }\n\
            pub async fn send(c: &Client) -> u8 { c.exec().await }\n\
        }\n";
    let d = fixture("r1719twin", src);
    let (_, on) = scan(&d, None, false);
    assert!(!strs(&on, "w::send", "inferred").contains(&"Fs".to_string()), "w::send awaited its own future: {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}
