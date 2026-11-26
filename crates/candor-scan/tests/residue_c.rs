//! rust lane residue-C — six small-reach rows from the `Self`/std binding, path-macro and cfg-twin areas. Every `p_*` probe below
//! performs the effect it is asserted to carry when the fixture is compiled and run (executed by the lane against the same bytes,
//! `rustagent-resC/fx`), and every `q_*` probe is pure. On v0.43.0 (`5b001dbf`) and main (`ed33afa7`) each `p_*` here read ABSENT or
//! lacked the asserted effect, and each `q_*` read a fabricated `Fs`.
//!
//! - R1846: `Self::m` in a trait DEFAULT body naming a SUPERTRAIT member (local provided, local required, two levels, a std
//!   supertrait with a local implementor).
//! - R1853: a PATH invocation of a crate macro (`crate::m!`, `self::m!`, `super::m!`, `$crate::m!`) at item, impl-item and body
//!   position; at trait-item position it stays hidden text (named residual).
//! - R1850: `Self` in an impl whose self type is a dependency module path reached only through globs.
//! - R1859 / R1860 / R1902: `Self::m` in an impl of a local trait for `std::fs::File`.
//! - R1906 / R1908: the cfg-twin matrix's section N cells (`soundness/cfgtwin/fx/n*`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// fn -> (inferred, unknownWhy)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn tmp(tag: &str) -> PathBuf {
    // unique per call: tests run as parallel threads of one process
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("candor-residueC-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Write a crate (`files`: path under the crate dir -> text) and scan it.
fn crate_with(tag: &str, deps: &[(&str, &str)], files: &[(&str, &str)]) -> PathBuf {
    let d = tmp(tag);
    let mut toml = format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n", tag.replace('-', "_"));
    for (n, lib) in deps {
        toml.push_str(&format!("{n} = {{ path = \"{n}\" }}\n"));
        let dd = d.join(n);
        std::fs::create_dir_all(dd.join("src")).unwrap();
        std::fs::write(dd.join("Cargo.toml"), format!("[package]\nname = \"{n}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")).unwrap();
        std::fs::write(dd.join("src/lib.rs"), lib).unwrap();
    }
    toml.push_str("\n[workspace]\n");
    std::fs::write(d.join("Cargo.toml"), toml).unwrap();
    for (rel, txt) in files {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    d
}

fn scan_env(dir: &Path, env: &[(&str, &str)]) -> Rows {
    let work = tmp("out");
    let out = work.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_R1846_OFF", "CANDOR_R1850_OFF", "CANDOR_R1853_OFF",
              "CANDOR_R1860_OFF", "CANDOR_R1902_OFF", "CANDOR_R1906_OFF", "CANDOR_R1908_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.env("CANDOR_CACHE_DIR", work.join("c"));
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut r = Rows::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".json") && !n.ends_with(".callgraph.json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                let s = |k: &str| f[k].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
                r.insert(f["fn"].as_str().unwrap().to_string(), (s("inferred"), s("unknownWhy")));
            }
        }
    }
    r
}

fn scan(dir: &Path) -> Rows {
    scan_env(dir, &[])
}

fn inferred<'a>(r: &'a Rows, probe: &str) -> Option<&'a Vec<String>> {
    r.get(probe).map(|(i, _)| i)
}

fn has(r: &Rows, probe: &str, eff: &str) -> bool {
    inferred(r, probe).is_some_and(|v| v.iter().any(|e| e == eff))
}

fn pure(r: &Rows, probe: &str) -> bool {
    inferred(r, probe).is_none_or(|v| v.is_empty())
}

const R1846: &str = r#"#![allow(unused)]
fn mark() -> String { std::env::var("MARK").unwrap() }
pub trait LSup { fn ls_w() { std::fs::write(mark(), b"x").unwrap(); } fn ls_pure() -> u8 { 1 } }
pub trait Qa: LSup {
    fn qa_path() { Self::ls_w() }
    fn qa_val() { let f = Self::ls_w; f() }
    fn qa_pure() -> u8 { Self::ls_pure() }
}
pub trait Mid: LSup { fn mid_x() {} }
pub trait Qb: Mid { fn qb_path() { Self::ls_w() } }
pub trait VT { fn connect(p: &str) -> u8; }
pub trait CVT: VT {
    fn create_path(p: &str) -> u8 { Self::connect(p) }
    fn create_qual(p: &str) -> u8 { <Self as VT>::connect(p) }
}
pub struct Tag(pub u8);
pub trait Cl: Sized + TryFrom<Tag> { fn cl_mk() -> Option<Self> { Self::try_from(Tag(0)).ok() } }
pub struct X;
impl LSup for X {}
impl Qa for X {}
impl Mid for X {}
impl Qb for X {}
impl VT for X { fn connect(_p: &str) -> u8 { std::fs::write(mark(), b"x").unwrap(); 1 } }
impl CVT for X {}
impl TryFrom<Tag> for X { type Error = (); fn try_from(_t: Tag) -> Result<X, ()> { std::fs::write(mark(), b"x").unwrap(); Ok(X) } }
impl Cl for X {}
pub fn p_qa_path() { X::qa_path() }
pub fn p_qa_val() { X::qa_val() }
pub fn q_qa_pure() -> u8 { X::qa_pure() }
pub fn p_qb_path() { X::qb_path() }
pub fn p_create_path() -> u8 { X::create_path("") }
pub fn p_create_qual() -> u8 { X::create_qual("") }
pub fn p_cl_mk() { let _ = X::cl_mk(); }
fn main() {}
"#;

/// R1846 — every supertrait spelling now reaches the member: the local provided body (`Fs`), the local REQUIRED member through
/// its implementors, two levels up, and the explicit `<Self as VT>::` spelling of a required member (ABSENT before too). A std
/// supertrait's member under a `pub` trait reaches the local implementor AND is disclosed (`dispatch:Cl.try_from`): a downstream
/// implementor's `try_from` is out of sight and no key can name a std member. The pure supertrait member stays pure.
#[test]
fn r1846_self_member_of_a_supertrait_in_a_default_body() {
    let r = scan(&crate_with("r1846", &[], &[("src/main.rs", R1846)]));
    for p in ["p_qa_path", "p_qa_val", "p_qb_path", "p_create_path", "p_create_qual", "p_cl_mk"] {
        assert!(has(&r, p, "Fs"), "{p} must read Fs over its executed write: {:?}", r.get(p));
    }
    assert!(has(&r, "p_cl_mk", "Unknown"), "a pub trait's std-supertrait member is disclosed: {:?}", r.get("p_cl_mk"));
    // the reason is the default body's own (`unknownWhy` is per function and direct)
    assert!(r.get("Cl::cl_mk").is_some_and(|(_, w)| w.iter().any(|x| x == "dispatch:Cl.try_from")), "{:?}", r.get("Cl::cl_mk"));
    assert!(!has(&r, "q_qa_pure", "Fs") && !has(&r, "q_qa_pure", "Unknown"), "pure supertrait member: {:?}", r.get("q_qa_pure"));
    // the switch restores the old (silent) reading — the fixture reaches the change
    let off = scan_env(&crate_with("r1846-off", &[], &[("src/main.rs", R1846)]), &[("CANDOR_R1846_OFF", "1")]);
    assert!(pure(&off, "p_qa_path") && pure(&off, "p_create_path"), "{:?} {:?}", off.get("p_qa_path"), off.get("p_create_path"));
}

const R1853: &str = r#"#![allow(unused)]
#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
#[macro_export] macro_rules! mkm { ($n:ident) => { pub fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
#[macro_export] macro_rules! wrapfn { ($n:ident) => { $crate::mkfn! { $n } } }
#[macro_export] macro_rules! mkimpl { ($t:ident, $n:ident) => { impl $t { pub fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }
#[macro_export] macro_rules! mkpure { ($n:ident) => { pub fn $n() {} } }
pub struct S; pub struct B;
crate::mkfn! { i_crate }
self::mkfn! { i_self }
wrapfn! { i_dollar }
pub mod sub { super::mkfn! { i_super } }
crate::mkpure! { i_pure }
impl S { crate::mkm! { m_crate } }
pub fn host_path() { crate::mkimpl!(B, go_path); }
pub fn p_i_crate() { i_crate() }
pub fn p_i_self() { i_self() }
pub fn p_i_dollar() { i_dollar() }
pub fn p_i_super() { sub::i_super() }
pub fn p_m_crate() { S.m_crate() }
pub fn p_body_path() { B.go_path() }
pub fn q_pure() { i_pure() }
fn main() {}
"#;

/// R1853 — a path-spelled invocation of the crate's own macro is expanded at item, impl-item and body position, so the fns it
/// writes are units and their callers read the write; `crate::mkpure!` mints a pure fn and its caller stays pure.
#[test]
fn r1853_path_invoked_crate_macro_is_expanded() {
    let r = scan(&crate_with("r1853", &[], &[("src/main.rs", R1853)]));
    for p in ["p_i_crate", "p_i_self", "p_i_dollar", "p_i_super", "p_m_crate", "p_body_path"] {
        assert!(has(&r, p, "Fs"), "{p} must read Fs over its executed write: {:?}", r.get(p));
    }
    assert!(pure(&r, "q_pure"), "{:?}", r.get("q_pure"));
    let off = scan_env(&crate_with("r1853-off", &[], &[("src/main.rs", R1853)]), &[("CANDOR_R1853_OFF", "1")]);
    assert!(!has(&off, "p_i_crate", "Fs") && !has(&off, "p_body_path", "Fs"), "{:?} {:?}", off.get("p_i_crate"), off.get("p_body_path"));
}

const R1850: &str = r#"#![allow(dead_code, unused)]
pub trait Gl { fn gl(p: &str) -> u8; }
mod g1 {
    use depx::*; use depy::*;
    impl crate::Gl for d::Lone { fn gl(p: &str) -> u8 { Self::m(p) } }
}
mod reexp { pub use depx::*; }
mod g2 {
    use crate::reexp::*;
    pub trait Gl2 { fn gl2(p: &str) -> u8; }
    impl Gl2 for d::Lone { fn gl2(p: &str) -> u8 { Self::m(p) } }
}
mod g7 {
    use super::*; use depx::*;
    pub trait Gl7 { fn gl7(p: &str) -> u8; }
    impl Gl7 for d::Lone { fn gl7(p: &str) -> u8 { Self::m(p) } }
}
fn main() {}
"#;
const DEPX: &str = "pub mod d { pub struct Lone; impl Lone { pub fn m(p: &str) -> u8 { std::fs::write(p, b\"x\").unwrap(); 1 } } }\n";
const DEPY: &str = "pub mod e { pub struct Other; impl Other { pub fn m(_p: &str) -> u8 { 0 } } }\n";

/// R1850 — two external globs and an external glob beside `use super::*` give `Self::m` the written path's candidates (unchained:
/// the dependency disclosure `invisible: [depx]`, which a chained scan answers); a crate-local glob of a globbing module, which
/// this scan does not follow, is disclosed at the call. Each read pure with no disclosure before.
#[test]
fn r1850_self_through_ambiguous_globs() {
    let d = crate_with("r1850", &[("depx", DEPX), ("depy", DEPY)], &[("src/main.rs", R1850)]);
    let out = tmp("r1850json");
    let o = Command::new(env!("CARGO_BIN_EXE_candor-scan")).arg(d.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"))
        .env("CANDOR_CACHE_DIR", out.join("c")).env_remove("CANDOR_DEPS").output().unwrap();
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1));
    let mut inv: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unk: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        if n.ends_with(".scan.json") {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            for f in v["functions"].as_array().unwrap() {
                let s = |k: &str| f[k].as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default();
                inv.insert(f["fn"].as_str().unwrap().to_string(), s("invisible"));
                unk.insert(f["fn"].as_str().unwrap().to_string(), s("unknownWhy"));
            }
        }
    }
    let row = |k: &str| inv.iter().find(|(f, _)| f.ends_with(k)).map(|(_, v)| v.clone());
    assert!(row("Lone::gl").is_some_and(|v| v.iter().any(|c| c == "depx")), "g1: {inv:?}");
    assert!(row("Lone::gl7").is_some_and(|v| v.iter().any(|c| c == "depx")), "g7: {inv:?}");
    let g2 = unk.iter().find(|(f, _)| f.ends_with("Lone::gl2")).map(|(_, v)| v.clone());
    assert!(g2.is_some_and(|v| v.iter().any(|w| w.starts_with("ambiguous:self type"))), "g2: {unk:?}");
}

const STDSELF: &str = r#"#![allow(unused)]
use std::fs::File;
fn w() { std::fs::write(std::env::var("MARKF").unwrap(), b"x").unwrap(); }
fn e() { std::env::set_var(std::env::var("MARKE").unwrap(), "1"); }
pub trait Ra { fn read_at(&self) -> u8; fn s_path(&self) -> u8; }
impl Ra for File { fn read_at(&self) -> u8 { e(); w(); 1 } fn s_path(&self) -> u8 { Self::read_at(self) } }
pub trait Ow { fn oenv() -> u8; fn opure() -> u8; fn g_env() -> u8; fn g_pure() -> u8; fn g_env_q() -> u8; fn g_pure_val() -> u8; }
impl Ow for File {
    fn oenv() -> u8 { e(); 1 }
    fn opure() -> u8 { 2 }
    fn g_env() -> u8 { Self::oenv() }
    fn g_pure() -> u8 { Self::opure() }
    fn g_env_q() -> u8 { <Self>::oenv() }
    fn g_pure_val() -> u8 { let f = Self::opure; f() }
}
pub trait Sb { fn sib() -> u8; fn sibpure() -> u8; }
impl Sb for File { fn sib() -> u8 { e(); 1 } fn sibpure() -> u8 { 3 } }
pub trait Sa { fn s3() -> u8; fn s3p() -> u8; }
impl Sa for File { fn s3() -> u8 { Self::sib() } fn s3p() -> u8 { Self::sibpure() } }
pub trait Cr { fn create(p: &str) -> u8; fn c_go() -> u8; }
impl Cr for File { fn create(_p: &str) -> u8 { e(); 0 } fn c_go() -> u8 { let _ = Self::create("x"); 1 } }
pub fn p_s_path(f: &File) -> u8 { f.s_path() }
pub fn p_g_env() -> u8 { <File as Ow>::g_env() }
pub fn q_g_pure() -> u8 { <File as Ow>::g_pure() }
pub fn p_g_env_q() -> u8 { <File as Ow>::g_env_q() }
pub fn q_g_pure_val() -> u8 { <File as Ow>::g_pure_val() }
pub fn p_s3() -> u8 { <File as Sa>::s3() }
pub fn q_s3p() -> u8 { <File as Sa>::s3p() }
pub fn p_c_go() -> u8 { <File as Cr>::c_go() }
fn main() {}
"#;

/// R1859 / R1860 / R1902 — under `use std::fs::File`, `Self::m` with no PUBLIC std inherent `m` and no std trait fn `m` runs the
/// crate's own member: this impl's (`oenv`: Env only, so the std path's `Fs` was a fabrication), another impl's of the same type
/// (`sib`: its `Env` was lost), or a pure one (`opure`: its caller read `Fs`). `read_at` (the private `sys::File`'s, which the table
/// held) is the impl's own member, read with its `Env`. A name std DOES have inherently (`create`) keeps std's reading.
#[test]
fn r1859_r1860_r1902_self_on_a_std_type_reads_the_crates_member() {
    let r = scan(&crate_with("stdself", &[], &[("src/main.rs", STDSELF)]));
    assert!(has(&r, "p_s_path", "Env") && has(&r, "p_s_path", "Fs"), "R1859: {:?}", r.get("p_s_path"));
    for p in ["p_g_env", "p_g_env_q", "p_s3"] {
        assert!(has(&r, p, "Env"), "{p}: {:?}", r.get(p));
        assert!(!has(&r, p, "Fs"), "{p} performs no file write: {:?}", r.get(p));
    }
    for q in ["q_g_pure", "q_g_pure_val", "q_s3p"] {
        assert!(pure(&r, q), "R1902 {q} is pure: {:?}", r.get(q));
    }
    assert!(has(&r, "p_c_go", "Fs"), "std's inherent File::create still runs: {:?}", r.get("p_c_go"));
    // second fixture written first: the switch restores the fabrication, so the fixture reaches the change
    let off = scan_env(&crate_with("stdself-off", &[], &[("src/main.rs", STDSELF)]), &[("CANDOR_R1902_OFF", "1"), ("CANDOR_R1860_OFF", "1")]);
    assert!(has(&off, "q_g_pure", "Fs") && !has(&off, "p_s3", "Env"), "{:?} {:?}", off.get("q_g_pure"), off.get("p_s3"));
}

fn cfgtwin(cell: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../soundness/cfgtwin/fx").join(cell)
}

/// R1906 — the hand-written arm cover runs in a crate that defines no `macro_rules!` at all (inline twins `n1`, `#[path]` twins
/// `n2`): `m_n1::go_n1` held behind a dependency macro read ABSENT; the `_ctl` twins (one unrelated local macro) already read U.
#[test]
fn r1906_arm_cover_runs_without_a_local_macro() {
    let r = scan(&cfgtwin("n1"));
    assert!(has(&r, "n1_e", "Unknown"), "{:?}", r.get("n1_e"));
    let r = scan(&cfgtwin("n2"));
    assert!(has(&r, "pure_n2::go_n2", "Unknown") || r.iter().any(|(k, v)| k.ends_with("go_n2") && v.0.iter().any(|e| e == "Unknown")), "{r:?}");
    let off = scan_env(&cfgtwin("n1"), &[("CANDOR_R1906_OFF", "1")]);
    assert!(!has(&off, "n1_e", "Unknown"), "{:?}", off.get("n1_e"));
}

/// R1908 — a glob of a module this file declares cannot rebind `thread_local!`, so the hand-written gated pair beside it is not
/// held (`n3`: E, no U); a dependency glob still may (`n3d`: the hold stays).
#[test]
fn r1908_local_glob_does_not_rebind_a_toolchain_macro() {
    let r = scan(&cfgtwin("n3"));
    assert!(has(&r, "n3_e", "Fs") && !has(&r, "n3_e", "Unknown"), "{:?}", r.get("n3_e"));
    let r = scan(&cfgtwin("n3d"));
    assert!(has(&r, "n3d_e", "Fs") && has(&r, "n3d_e", "Unknown"), "{:?}", r.get("n3d_e"));
    // a crate-wide re-export of the name through a local module keeps the hold (the disclosure the narrowing might delete)
    let d = crate_with("r1908-reexp", &[("ldep", "#[macro_export] macro_rules! thread_local { ($($t:tt)*) => {}; }\n")], &[
        ("src/main.rs", "#![allow(unused)]\nmacro_rules! unrelated { () => {} }\nmod helpers;\nuse helpers::*;\nuse std::cell::Cell;\n\
            thread_local! { static X: Cell<u8> = Cell::new(0); }\n\
            #[cfg(unix)] pub fn gated() { std::fs::write(\"x\", b\"x\").unwrap(); }\n#[cfg(not(unix))] pub fn gated() {}\n\
            pub fn p_gated() { gated() }\nfn main() {}\n"),
        ("src/helpers.rs", "pub use ldep::thread_local;\npub fn h() {}\n"),
    ]);
    let r = scan(&d);
    assert!(has(&r, "p_gated", "Unknown"), "re-exported through the globbed module: {:?}", r.get("p_gated"));
}
