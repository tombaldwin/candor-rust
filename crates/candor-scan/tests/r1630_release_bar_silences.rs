//! SOUNDNESS R1630 / R1631 / R1636 / R1637 / R1638, and the R1624 twin-element follow-up — the silences R1624's
//! own-declaration field typing uncovered, each of which a WRONG-TWIN charge had been standing in front of. Every effectful
//! shape has a pure twin beside it that must stay absent. Every switch is shown to restore the silence, so each assertion
//! is a reading of the code it names rather than of the fixture.
//!
//! * R1630: `self.io.send(b)` with `io: DontDrop<UdpSocket>` and `impl<T> Deref for DontDrop<T> { type Target = T }`. A
//!   non-field receiver of such a wrapper (`try_lock()`'s guard) discloses instead.
//! * R1631: `self.inner.get_mut().read(buf)`, where the accessor returns the owner's bounded parameter.
//! * R1636: a receiver type declared only inside an unexpanded item-position macro (tokio's `cfg_io_driver!`).
//! * R1624 twin element: `for c in self.m.borrow().values() { c.go() }` over `m: RefCell<HashMap<_, C>>` in a same-leaf
//!   twin.
//! * R1637: a chained dependency called through a SUBMODULE re-export path (`pd::io::w()`).
//!
//! Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, name: &str, deps: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1630-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"),
    )
    .unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

const SWITCHES: &[&str] = &[
    "CANDOR_R1630_OFF",
    "CANDOR_R1631_OFF",
    "CANDOR_R1636_OFF",
    "CANDOR_R1637_OFF",
    "CANDOR_R1638_OFF",
    "CANDOR_R1624ELEM_OFF",
    "CANDOR_R1624_OFF",
];

fn scan(dir: &Path, deps: Option<&Path>, off: &[&str]) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    for s in SWITCHES {
        cmd.env_remove(s);
    }
    for s in off {
        cmd.env(s, "1");
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("report json")
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    let mut out: Vec<String> = row
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    let row = v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f));
    row.and_then(|r| r["unknownWhy"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const DEREF: &str = "\
use std::ops::Deref;\n\
use std::net;\n\
pub struct DontDrop<T>(Option<T>);\n\
impl<T> Deref for DontDrop<T> { type Target = T; fn deref(&self) -> &T { self.0.as_ref().unwrap() } }\n\
impl<T> DontDrop<T> { pub fn own(&self) -> bool { self.0.is_some() } }\n\
pub struct Solo { io: DontDrop<net::UdpSocket> }\n\
impl Solo {\n\
    pub fn solo_send_zq(&self, b: &[u8]) -> std::io::Result<usize> { self.io.send(b) }\n\
    pub fn solo_own_zq(&self) -> bool { self.io.own() }\n\
}\n\
pub struct Raw { r#type: DontDrop<net::UdpSocket> }\n\
pub fn raw_send_zq(r: &Raw, b: &[u8]) -> std::io::Result<usize> { r.r#type.send(b) }\n\
pub struct Lock<T>(std::cell::RefCell<T>);\n\
pub struct Guard<'a, T>(std::cell::RefMut<'a, T>);\n\
impl<'a, T> Deref for Guard<'a, T> { type Target = T; fn deref(&self) -> &T { &self.0 } }\n\
impl<T> Lock<T> { pub fn try_lock(&self) -> Option<Guard<'_, T>> { self.0.try_borrow_mut().ok().map(Guard) } }\n\
pub struct Drv;\n\
impl Drv { pub fn shutdown(&self) { let _ = std::fs::remove_file(\"/tmp/candor-r1630-drv\"); } }\n\
pub struct Park { d: Lock<Drv> }\n\
impl Park { pub fn park_shut_zq(&self) { if let Some(g) = self.d.try_lock() { g.shutdown() } } }\n";

#[test]
fn r1630_generic_deref_field_reaches_its_instantiation() {
    let d = fixture("deref", "dd", "", &[("lib.rs", DEREF)]);
    let on = scan(&d, None, &[]);
    assert_eq!(inferred(&on, "Solo::solo_send_zq"), vec!["Net"], "auto-deref to UdpSocket::send");
    assert!(inferred(&on, "Solo::solo_own_zq").is_empty(), "the wrapper's own member stays the wrapper's (pure twin)");
    // A raw-identifier field: the file must be analysed at all (a re-built `Ident::new("r#type")` aborted the whole file).
    assert_eq!(inferred(&on, "raw_send_zq"), vec!["Net"], "raw identifier field, owner not proven (leaf route)");
    assert_eq!(inferred(&on, "Park::park_shut_zq"), vec!["Unknown"], "a non-field receiver of a generic Deref discloses");
    assert!(why(&on, "Park::park_shut_zq").iter().any(|w| w.starts_with("dispatch:generic payload receiver")));
    let off = scan(&d, None, &["CANDOR_R1630_OFF"]);
    assert!(inferred(&off, "Solo::solo_send_zq").is_empty(), "the switch restores the silence: the assertion reads the rule");
    assert!(inferred(&off, "Park::park_shut_zq").is_empty());
    let _ = std::fs::remove_dir_all(&d);
}

const ACCESSOR: &str = "\
use std::io::{self, Read, Write};\n\
pub struct Writer<W: Write> { obj: Option<W> }\n\
impl<W: Write> Writer<W> { pub fn get_mut(&mut self) -> &mut W { self.obj.as_mut().unwrap() } }\n\
pub struct Dec<W: Write> { inner: Writer<W> }\n\
impl<W: Read + Write> Read for Dec<W> { fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> { self.inner.get_mut().read(buf) } }\n\
impl<W: Read + Write> Dec<W> { pub fn dec_read_zq(&mut self, b: &mut [u8]) -> io::Result<usize> { self.inner.get_mut().read(b) } }\n\
pub struct Calm;\n\
impl Calm { pub fn read(&mut self, _b: &mut [u8]) -> io::Result<usize> { Ok(0) } }\n\
pub struct Hold { inner: Calm }\n\
impl Hold { pub fn hold_read_zq(&mut self, b: &mut [u8]) -> io::Result<usize> { self.inner.read(b) } }\n";

#[test]
fn r1631_accessor_returning_the_owners_parameter_dispatches() {
    let d = fixture("acc", "gw", "", &[("lib.rs", ACCESSOR)]);
    let on = scan(&d, None, &[]);
    assert_eq!(inferred(&on, "Dec::dec_read_zq"), vec!["Unknown"]);
    assert!(why(&on, "Dec::dec_read_zq").iter().any(|w| w == "dispatch:Read.read"), "{:?}", why(&on, "Dec::dec_read_zq"));
    assert!(inferred(&on, "Hold::hold_read_zq").is_empty(), "a concrete pure field stays pure");
    let off = scan(&d, None, &["CANDOR_R1631_OFF"]);
    assert!(inferred(&off, "Dec::dec_read_zq").is_empty(), "the switch restores the silence");
    let _ = std::fs::remove_dir_all(&d);
}

const MACRO_LIB: &str = "\
#[macro_use]\nmod macros;\npub mod drv;\n";
const MACRO_DEFS: &str = "\
macro_rules! cfg_io { ($($item:item)*) => { $( #[cfg(feature = \"io\")] $item )* } }\n\
macro_rules! cfg_not_io { ($($item:item)*) => { $( #[cfg(not(feature = \"io\"))] $item )* } }\n";
const MACRO_DRV: &str = "\
cfg_io! {\n\
    pub(crate) struct Stack(std::net::UdpSocket);\n\
    impl Stack { pub(crate) fn park(&mut self) { let _ = self.0.send(b\"x\"); } }\n\
}\n\
cfg_not_io! {\n\
    pub(crate) struct Stack(());\n\
    impl Stack { pub(crate) fn park(&mut self) {} }\n\
}\n\
pub struct Driver { park: Stack }\n\
impl Driver { pub fn drv_park_zq(&mut self) { self.park.park() } }\n\
pub struct Seen;\n\
impl Seen { pub fn quiet(&self) {} }\n\
pub struct Other { s: Seen }\n\
impl Other { pub fn other_zq(&self) { self.s.quiet() } }\n";

#[test]
fn r1636_a_receiver_type_declared_only_in_macro_text_discloses() {
    let d = fixture("macro", "mh", "", &[("lib.rs", MACRO_LIB), ("macros.rs", MACRO_DEFS), ("drv.rs", MACRO_DRV)]);
    let on = scan(&d, None, &[]);
    assert_eq!(inferred(&on, "drv::Driver::drv_park_zq"), vec!["Unknown"]);
    assert!(why(&on, "drv::Driver::drv_park_zq").iter().any(|w| w.starts_with("macro:")));
    assert!(inferred(&on, "drv::Other::other_zq").is_empty(), "a type candor read stays pure (twin)");
    let off = scan(&d, None, &["CANDOR_R1636_OFF"]);
    assert!(inferred(&off, "drv::Driver::drv_park_zq").is_empty(), "the switch restores the silence");
    let _ = std::fs::remove_dir_all(&d);
}

const TWIN_ELEM: &str = "\
use std::cell::RefCell;\n\
use std::collections::HashMap;\n\
pub trait Tr { fn go(&self) -> bool; }\n\
pub struct X;\n\
impl Tr for X { fn go(&self) -> bool { std::fs::read(\"/etc/hosts\").is_ok() } }\n\
pub mod a {\n\
    use super::*;\n\
    pub struct W<C> { m: RefCell<HashMap<u8, C>>, v: Vec<C> }\n\
    impl<C: Tr> W<C> {\n\
        pub fn ge_map_zq(&self) -> bool { for c in self.m.borrow().values() { if !c.go() { return false; } } true }\n\
        pub fn ge_vec_zq(&self) -> bool { for c in self.v.iter() { if !c.go() { return false; } } true }\n\
    }\n\
}\n\
pub mod b {\n\
    pub struct Calm;\n\
    impl Calm { pub fn go(&self) -> bool { true } }\n\
    pub struct W { m: std::cell::RefCell<std::collections::HashMap<u8, Calm>> }\n\
    impl W { pub fn b_map_zq(&self) -> bool { for c in self.m.borrow().values() { if !c.go() { return false; } } true } }\n\
}\n";

#[test]
fn r1624_twin_element_typed_by_the_owners_parameter_dispatches_on_its_bound() {
    let d = fixture("elem", "ge", "", &[("lib.rs", TWIN_ELEM)]);
    let on = scan(&d, None, &[]);
    assert_eq!(inferred(&on, "a::W::ge_map_zq"), vec!["Fs"]);
    assert_eq!(inferred(&on, "a::W::ge_vec_zq"), vec!["Fs"]);
    assert!(inferred(&on, "b::W::b_map_zq").is_empty(), "the pure twin is not charged its sibling's bound");
    let off = scan(&d, None, &["CANDOR_R1624ELEM_OFF"]);
    assert!(inferred(&off, "a::W::ge_map_zq").is_empty(), "the switch restores the silence");
    let _ = std::fs::remove_dir_all(&d);
}

const PD: &str = "\
mod inner;\n\
pub use inner::eff;\n\
pub mod io { mod rw { pub fn w() -> usize { std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0) } pub fn calm() -> usize { 0 } } pub use rw::{w, calm}; }\n\
pub mod jo { mod rw { pub fn w() -> usize { 0 } } pub use rw::*; }\n\
pub mod io2 { pub struct Bx(pub u8); pub fn mk(n: u8) -> Bx { Bx(n) } impl Bx { pub fn mk(&self) -> usize { std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0) } } }\n";
const PD_INNER: &str = "pub fn eff() -> usize { std::fs::read(\"/etc/hosts\").map(|v| v.len()).unwrap_or(0) }\n";
const PC: &str = "\
pub fn c_root_zq() -> usize { pd::eff() }\n\
pub fn c_io_zq() -> usize { pd::io::w() }\n\
pub fn c_calm_zq() -> usize { pd::io::calm() }\n\
pub fn c_jo_zq() -> usize { pd::jo::w() }\n\
pub fn c_mk_free_zq() -> u8 { pd::io2::mk(3).0 }\n";

#[test]
fn r1637_a_submodule_reexport_path_joins_the_chained_producer() {
    let p = fixture("pd", "pd", "", &[("lib.rs", PD), ("inner/mod.rs", PD_INNER)]);
    let out = std::env::temp_dir().join(format!("candor-r1630-pdrep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    // The producer's report on disk (`--out` writes it; `--json` alone does not).
    let st = Command::new(bin()).arg(p.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"))
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").output().expect("producer scan");
    assert!(st.status.code().is_some_and(|c| c == 0 || c == 1), "{}", String::from_utf8_lossy(&st.stderr));
    let rep = std::fs::read_dir(&out).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .find(|f| f.to_string_lossy().ends_with(".scan.json") && !f.to_string_lossy().contains("callgraph"))
        .expect("producer report");
    let c = fixture("pc", "pc", &format!("pd = {{ path = \"{}\" }}\n", p.display()), &[("lib.rs", PC)]);
    let on = scan(&c, Some(&rep), &[]);
    assert_eq!(inferred(&on, "c_root_zq"), vec!["Fs"], "a crate-root re-export (worked before)");
    assert_eq!(inferred(&on, "c_io_zq"), vec!["Fs"], "the submodule re-export joins the unit beneath it");
    assert!(inferred(&on, "c_calm_zq").is_empty(), "the pure twin re-exported the same way stays pure");
    // `jo::rw::w` is pure and so absent from the producer's `functions[]`; the only `w` the report shows is `io::rw::w`,
    // which is not beneath `jo`. Picking it would charge `Fs` over a pure body: the call discloses instead.
    assert_eq!(inferred(&on, "c_jo_zq"), vec!["Unknown"], "a re-export the report cannot place discloses, never picks");
    // SOUNDNESS R1637 (rustagent-r1641) — a PURE free fn beside an effectful inherent METHOD of the same leaf on a type in
    // that module (dylint_internal's `cargo::build` / `cargo::Builder::build`): a `pub use` never re-exports a method, so
    // the method is neither a join target nor a reason to disclose. The first cut charged the free fn the method's `Fs`.
    assert!(inferred(&on, "c_mk_free_zq").is_empty(), "{:?}", inferred(&on, "c_mk_free_zq"));
    let off = scan(&c, Some(&rep), &["CANDOR_R1637_OFF"]);
    assert!(inferred(&off, "c_io_zq").is_empty(), "the switch restores the silence");
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::remove_dir_all(&c);
    let _ = std::fs::remove_dir_all(&out);
}

const WRAPDEP: &str = "\
pub struct Wrap<S>(pub S);\n\
impl<S: std::fmt::Debug> std::fmt::Debug for Wrap<S> { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple(\"W\").field(&self.0).finish() } }\n";
const WRAPUSE: &str = "\
use std::fmt;\n\
pub struct Tls<S>(dw::Wrap<S>);\n\
impl<S: fmt::Debug> fmt::Debug for Tls<S> { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(&self.0, f) } }\n\
impl<S: fmt::Debug> Tls<S> { pub fn tls_fmt_zq(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(&self.0, f) } }\n\
pub struct Plain(dw::Wrap<u8>);\n\
impl Plain { pub fn plain_fmt_zq(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(&self.0, f) } }\n";

#[test]
fn r1638_a_dependency_wrapper_over_this_scopes_bounded_parameter_dispatches() {
    let p = fixture("dw", "dw", "", &[("lib.rs", WRAPDEP)]);
    let c = fixture("dwu", "dwu", &format!("dw = {{ path = \"{}\" }}\n", p.display()), &[("lib.rs", WRAPUSE)]);
    let on = scan(&c, None, &[]);
    assert_eq!(inferred(&on, "Tls::tls_fmt_zq"), vec!["Unknown"]);
    assert!(why(&on, "Tls::tls_fmt_zq").iter().any(|w| w == "dispatch:Debug.fmt"), "{:?}", why(&on, "Tls::tls_fmt_zq"));
    assert!(!inferred(&on, "Plain::plain_fmt_zq").contains(&"Unknown".to_string()), "a concrete instantiation is not hedged");
    let off = scan(&c, None, &["CANDOR_R1638_OFF"]);
    assert!(!inferred(&off, "Tls::tls_fmt_zq").contains(&"Unknown".to_string()), "the switch removes the hedge");
    let _ = std::fs::remove_dir_all(&p);
    let _ = std::fs::remove_dir_all(&c);
}
