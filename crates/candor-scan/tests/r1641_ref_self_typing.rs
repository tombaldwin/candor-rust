//! SOUNDNESS R1641 — `self` in a NON-operator trait impl on a REFERENCE to a nominal type was untyped.
//!
//! object 0.32.2's `impl<'a, R: Read + Seek> ReadRef<'a> for &'a ReadCache<R>` seeks and reads the caller's stream
//! (`cache.read.seek(..)`), and `StringTable::get` — which bounded CHA does edge to that impl's unit — read pure, so
//! `SectionTable::section_by_name` read `[]`. The impl is keyed at module level (R1034, by design), and `self` had no
//! type, so every call through it formed no edge and no disclosure. Executed fixture: rustagent-r1641/fx/rr (each
//! variant seeks and reads a real file; the concrete `RefCell<File>` variant was ABSENT on published 0.41.3 too).
//!
//! Each variant sits in its own module because every `impl Tr for &X` of one module shares one `{mod}::at` unit
//! (R1034's union), and a pure twin in the same module would read the union. Fixture fns carry a unique `_zq`
//! terminator because `deny <E> <fn>` matches by PREFIX.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, src: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1641-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"rr\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(d.join("src/lib.rs"), src).unwrap();
    d
}

fn scan(dir: &Path, off: bool) -> serde_json::Value {
    scan_sw(dir, if off { Some("CANDOR_R1641_OFF") } else { None })
}

fn scan_sw(dir: &Path, off: Option<&str>) -> serde_json::Value {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY");
    cmd.env_remove("CANDOR_R1641_OFF").env_remove("CANDOR_R1643_OFF");
    if let Some(sw) = off {
        cmd.env(sw, "1");
    }
    let o = cmd.output().expect("run candor-scan");
    let code = o.status.code().unwrap_or(-1);
    assert!(code == 0 || code == 1, "scan failed ({code}): {}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("report json")
}

fn row<'a>(v: &'a serde_json::Value, f: &str) -> Option<&'a serde_json::Value> {
    v["functions"].as_array().unwrap().iter().find(|r| r["fn"].as_str() == Some(f))
}

fn inferred(v: &serde_json::Value, f: &str) -> Vec<String> {
    let mut out: Vec<String> = row(v, f)
        .and_then(|r| r["inferred"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn why(v: &serde_json::Value, f: &str) -> Vec<String> {
    row(v, f)
        .and_then(|r| r["unknownWhy"].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

const SRC: &str = "\
use std::cell::RefCell;\n\
use std::io::{Read, Seek, SeekFrom};\n\
pub trait Rr<'a>: Clone + Copy { fn at(self, off: u64) -> Result<u8, ()>; }\n\
pub mod ga {\n\
    use super::*;\n\
    pub struct Cache<R: Read + Seek> { cache: RefCell<Inner<R>> }\n\
    struct Inner<R: Read + Seek> { read: R }\n\
    impl<'a, R: Read + Seek> Rr<'a> for &'a Cache<R> {\n\
        fn at(self, off: u64) -> Result<u8, ()> {\n\
            let cache = &mut *self.cache.borrow_mut();\n\
            cache.read.seek(SeekFrom::Start(off)).map_err(|_| ())?;\n\
            let mut b = [0u8; 1];\n\
            cache.read.read_exact(&mut b).map_err(|_| ())?;\n\
            Ok(b[0])\n\
        }\n\
    }\n\
    pub fn ga_get_zq<R: Read + Seek>(c: &Cache<R>) -> Result<u8, ()> { c.at(0) }\n\
}\n\
pub mod fc {\n\
    use super::*;\n\
    pub struct FileCache { f: RefCell<std::fs::File> }\n\
    impl<'a> Rr<'a> for &'a FileCache {\n\
        fn at(self, off: u64) -> Result<u8, ()> {\n\
            let mut f = self.f.borrow_mut();\n\
            f.seek(SeekFrom::Start(off)).map_err(|_| ())?;\n\
            let mut b = [0u8; 1];\n\
            f.read_exact(&mut b).map_err(|_| ())?;\n\
            Ok(b[0])\n\
        }\n\
    }\n\
}\n\
pub mod mc {\n\
    use super::*;\n\
    pub struct MemCache { m: RefCell<Vec<u8>> }\n\
    impl<'a> Rr<'a> for &'a MemCache {\n\
        fn at(self, off: u64) -> Result<u8, ()> { self.m.borrow().get(off as usize).copied().ok_or(()) }\n\
    }\n\
}\n\
pub mod gp {\n\
    use super::*;\n\
    pub struct CacheP<R: Clone> { cache: RefCell<InnerP<R>> }\n\
    struct InnerP<R: Clone> { read: R }\n\
    impl<'a, R: Clone> Rr<'a> for &'a CacheP<R> {\n\
        fn at(self, _off: u64) -> Result<u8, ()> { let cache = &mut *self.cache.borrow_mut(); let _c = cache.read.clone(); Ok(7) }\n\
    }\n\
}\n\
pub mod bv {\n\
    use super::*;\n\
    pub struct ByVal<'a, R: Read + Seek> { c: &'a RefCell<R> }\n\
    impl<'a, R: Read + Seek> Clone for ByVal<'a, R> { fn clone(&self) -> Self { *self } }\n\
    impl<'a, R: Read + Seek> Copy for ByVal<'a, R> {}\n\
    impl<'a, R: Read + Seek> Rr<'a> for ByVal<'a, R> {\n\
        fn at(self, off: u64) -> Result<u8, ()> { self.c.borrow_mut().seek(SeekFrom::Start(off)).map(|_| 0).map_err(|_| ()) }\n\
    }\n\
}\n";

#[test]
fn r1641_reference_self_impl_types_self_as_its_referent() {
    let d = fixture("refself", SRC);
    let on = scan(&d, false);
    // generic stream: the dispatch on the caller-chosen `R: Read + Seek` is disclosed at the site that lost it
    assert_eq!(inferred(&on, "ga::at"), vec!["Unknown"], "{:?}", why(&on, "ga::at"));
    assert!(why(&on, "ga::at").iter().any(|w| w == "dispatch:Seek.seek"), "{:?}", why(&on, "ga::at"));
    assert_eq!(inferred(&on, "ga::ga_get_zq"), vec!["Unknown"], "the caller inherits through the CHA edge");
    // concrete stream: a RESOLUTION, the real effect
    assert_eq!(inferred(&on, "fc::at"), vec!["Fs"]);
    // pure twins: a concrete in-memory buffer, and a generic parameter only cloned
    assert!(row(&on, "mc::at").is_none() || inferred(&on, "mc::at").is_empty(), "{:?}", inferred(&on, "mc::at"));
    assert!(row(&on, "gp::at").is_none() || inferred(&on, "gp::at").is_empty(), "{:?}", inferred(&on, "gp::at"));
    // a by-value self type was already typed: unchanged by the switch
    let off = scan(&d, true);
    assert_eq!(inferred(&on, "bv::ByVal::at"), inferred(&off, "bv::ByVal::at"));
    assert_eq!(inferred(&off, "bv::ByVal::at"), vec!["Unknown"]);
    // the switch restores the silence, so each assertion above reads the rule
    assert!(inferred(&off, "ga::at").is_empty(), "{:?}", inferred(&off, "ga::at"));
    assert!(inferred(&off, "fc::at").is_empty(), "{:?}", inferred(&off, "fc::at"));
    let _ = std::fs::remove_dir_all(&d);
}

const OWN_PARAM: &str = "\
pub trait Go { fn go(&self) -> u8; }\n\
pub struct Loud;\n\
impl Go for Loud { fn go(&self) -> u8 { let _ = std::fs::read(\"/x\"); 1 } }\n\
impl<T: Go> Go for &T { fn go(&self) -> u8 { (**self).go() } }\n\
pub fn own_param_zq(l: &Loud) -> u8 { (&l).go() }\n";

#[test]
fn r1641_own_parameter_referent_is_not_retyped() {
    // `impl<T: Go> Go for &T` is R1208/R1292's blanket forwarder: its referent is the impl's own parameter, which names no
    // type, so the new typing must not touch it. Compare the whole report with and without the switch.
    let d = fixture("ownparam", OWN_PARAM);
    let on = scan(&d, false);
    let off = scan(&d, true);
    assert_eq!(on["functions"], off["functions"]);
    let _ = std::fs::remove_dir_all(&d);
}

const HANDLE: &str = "\
use std::fs::File;\n\
use std::io::{self, Read};\n\
pub struct Ntf { file: File }\n\
impl Ntf {\n\
    pub fn as_file(&self) -> &File { &self.file }\n\
    pub fn as_file_mut(&mut self) -> &mut File { &mut self.file }\n\
    pub fn acc_mut_zq(&mut self, b: &mut [u8]) -> io::Result<usize> { self.as_file_mut().read(b) }\n\
    pub fn field_zq(&mut self, b: &mut [u8]) -> io::Result<usize> { self.file.read(b) }\n\
}\n\
impl<'a> Read for &'a Ntf {\n\
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.as_file().read(b) }\n\
}\n\
pub fn free_param_zq(n: &mut Ntf, b: &mut [u8]) -> io::Result<usize> { n.as_file_mut().read(b) }\n\
pub struct Hold { s: String }\n\
impl Hold { pub fn as_s(&self) -> &String { &self.s } pub fn twin_pure_zq(&self) -> usize { self.as_s().len() } }\n";

/// SOUNDNESS R1643 — a method called on a local accessor's DECLARED std I/O handle return (`self.as_file_mut().read(b)`)
/// was ABSENT (executed rustagent-r1641/fx/tf); it is what made tempfile's `impl Read for &NamedTempFile` read pure once
/// R1641 typed its `self` (the old `untyped receiver` hedge had been standing in front of the silence).
#[test]
fn r1643_accessor_returning_a_std_handle_is_typed_by_it() {
    let d = fixture("handle", HANDLE);
    let on = scan_sw(&d, None);
    assert_eq!(inferred(&on, "Ntf::acc_mut_zq"), vec!["Fs"]);
    assert_eq!(inferred(&on, "free_param_zq"), vec!["Fs"]);
    assert_eq!(inferred(&on, "read"), vec!["Fs"], "the `&Ntf` impl, R1641 + R1643 together");
    assert_eq!(inferred(&on, "Ntf::field_zq"), vec!["Fs"], "control: the field spelling always charged");
    assert!(inferred(&on, "Hold::twin_pure_zq").is_empty(), "pure twin: a non-handle accessor return");
    let off = scan_sw(&d, Some("CANDOR_R1643_OFF"));
    assert!(inferred(&off, "Ntf::acc_mut_zq").is_empty(), "the switch restores the silence");
    assert!(inferred(&off, "free_param_zq").is_empty());
    assert_eq!(inferred(&off, "Ntf::field_zq"), vec!["Fs"]);
    let _ = std::fs::remove_dir_all(&d);
}

const SHAPES: &str = "\
use std::fs::File;\n\
use std::io::{self, Read};\n\
use std::path::{Path, PathBuf};\n\
pub mod gen_arg {\n\
    use super::*;\n\
    pub struct Ntf<F> { file: F }\n\
    impl<F> Ntf<F> { pub fn as_file(&self) -> &F { &self.file } }\n\
    impl Read for &Ntf<File> { fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.as_file().read(b) } }\n\
}\n\
pub trait Src { fn resolve(self) -> usize; }\n\
impl Src for PathBuf { fn resolve(self) -> usize { std::fs::read_dir(&self).map(|d| d.count()).unwrap_or(0) } }\n\
pub mod std_ref {\n\
    use super::*;\n\
    impl<'s> Src for &'s Path { fn resolve(self) -> usize { self.to_owned().resolve() } }\n\
}\n";

/// The two shapes the first R1641 cut got wrong on the corpus, each pinned: a CONCRETE argument of the referent
/// (`&NamedTempFile<File>`, tempfile >= 3.5) answers a generic accessor return, and a std referent (`&'s Path`,
/// sqlx-core's `MigrationSource`) keeps its untyped `self`, whose hedge names the local `PathBuf::resolve` that reads
/// the filesystem (typing `self` as `Path` made the row ABSENT).
#[test]
fn r1641_concrete_referent_argument_and_std_referent() {
    let d = fixture("shapes", SHAPES);
    let on = scan_sw(&d, None);
    assert_eq!(inferred(&on, "gen_arg::read"), vec!["Fs"], "{:?}", why(&on, "gen_arg::read"));
    assert!(!inferred(&on, "std_ref::resolve").is_empty(), "the std referent must not lose its disclosure");
    let off = scan_sw(&d, Some("CANDOR_R1643_OFF"));
    assert!(inferred(&off, "gen_arg::read").is_empty(), "{:?}", inferred(&off, "gen_arg::read"));
    let _ = std::fs::remove_dir_all(&d);
}
