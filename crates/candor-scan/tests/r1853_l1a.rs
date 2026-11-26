//! rust-macropath L1(a) — SOUNDNESS R1853: the CALLER side of unexpanded LOCAL item / impl-item / trait-item text.
//!
//! When a local item-position invocation is left unexpanded (a PATH invocation is never bound by `mbe::Ctx::local_name`),
//! R128 discloses only the caller that spells the hidden module's `crate::` path. Every other spelling of a name that text
//! declares read ABSENT over an executed write on v0.43.0 and main: a bare same-module call, a `use`-imported call (named,
//! renamed, glob, from another file), a method on a type whose INHERENT impl holds an unexpanded impl-item invocation (method
//! and UFCS spellings), and a method on a trait holding an unexpanded trait-item invocation. Ground truth for each is EXECUTED
//! in `soundness/macropath` (cells l1bare l1use l1ren l1glob l1file l1impl l1ufcs l1trait l1traitm); these fixtures pin the
//! scan-level reading and carry a discriminator beside every hedge that must NOT fire.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// fn -> (inferred, unknownWhy)
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;

fn tmp(tag: &str) -> PathBuf {
    // unique per call: tests run as parallel threads of one process
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("candor-r1853l1a-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn crate_with(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = tmp(tag);
    let toml = format!("[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n", tag.replace('-', "_"));
    std::fs::write(d.join("Cargo.toml"), toml).unwrap();
    for (rel, txt) in files {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    d
}

fn scan(dir: &Path) -> Rows {
    let work = tmp("out");
    let out = work.join("out");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY"] {
        cmd.env_remove(k);
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

fn unknown(r: &Rows, f: &str) -> bool {
    r.get(f).is_some_and(|(i, _)| i.iter().any(|e| e == "Unknown"))
}

fn why(r: &Rows, f: &str, reason: &str) -> bool {
    r.get(f).is_some_and(|(_, w)| w.iter().any(|x| x == reason))
}

const MODULE_WHY: &str = "macro:module items hidden by an unexpanded macro";
const MEMBER_WHY: &str = "macro:impl members hidden by an unexpanded macro";

const FREE: &str = r#"#![allow(unused)]
mod defs {
    macro_rules! mkfn { ($n:ident) => { pub fn $n() { std::fs::write(std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
    pub(crate) use mkfn;
}
mod inner {
    use crate::defs::mkfn;
    self::mkfn! { go }
    pub fn visible() {}
    pub fn bare() { go() }
    pub fn callvis() { visible() }
}
mod other { pub fn elsewhere() {} pub fn callelse() { elsewhere() } }
use inner::go;
use inner::go as renamed;
pub fn viause() { go() }
pub fn viaren() { renamed() }
pub fn viaother() { other::elsewhere() }
fn main() {}
"#;

/// A bare call inside the module, a `use`-imported call and a renamed one each name `go`, which the unexpanded `self::mkfn! { go }`
/// declares: disclosed with the module reason. A VISIBLE name in the same module, and a module with nothing hidden, do not hedge.
#[test]
fn r1853_free_callers_of_a_name_unexpanded_local_item_text_declares() {
    let r = scan(&crate_with("free", &[("src/main.rs", FREE)]));
    for f in ["inner::bare", "viause", "viaren"] {
        assert!(unknown(&r, f) && why(&r, f, MODULE_WHY), "{f} must read Unknown with the module reason: {:?}", r.get(f));
    }
    for f in ["inner::callvis", "other::callelse", "viaother"] {
        assert!(!unknown(&r, f), "{f} calls a VISIBLE name and must not hedge: {:?}", r.get(f));
    }
}

const GLOB: &str = r#"#![allow(unused)]
mod defs {
    macro_rules! mkfn { ($n:ident) => { pub fn $n() { std::fs::write(std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
    pub(crate) use mkfn;
}
mod inner { use crate::defs::mkfn; self::mkfn! { goglob } }
mod viaglob {
    use crate::inner::*;
    pub fn caller() { goglob() }
    pub fn callvis() { visible_g() }
    pub fn visible_g() {}
}
mod foreign {
    fxdepx::mkfn! { gofar }
    pub fn farcall() { gofar() }
}
fn main() {}
"#;

/// A glob importer of the module is disclosed (the closure is R145's). A DEPENDENCY's macro whose leaf equals a local macro's name
/// binds the dependency: its names are unknowable and are not read from the local body, so that caller stays as on v0.43.0 (the
/// opaque arm is a separate question, not this one).
#[test]
fn r1853_a_glob_importer_is_disclosed_and_a_foreign_macro_of_the_same_leaf_is_not_recorded() {
    let r = scan(&crate_with("glob", &[("src/main.rs", GLOB)]));
    assert!(unknown(&r, "viaglob::caller") && why(&r, "viaglob::caller", MODULE_WHY), "glob importer: {:?}", r.get("viaglob::caller"));
    assert!(!unknown(&r, "viaglob::callvis"), "a visible name in the importer must not hedge: {:?}", r.get("viaglob::callvis"));
    assert!(!why(&r, "foreign::farcall", MODULE_WHY), "a foreign macro's name is not read from a same-leaf local body: {:?}", r.get("foreign::farcall"));
}

const MEMBER: &str = r#"#![allow(unused)]
mod defs {
    macro_rules! mkm { ($n:ident) => { pub fn $n(&self) { std::fs::write(std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
    pub(crate) use mkm;
    macro_rules! mktr { ($n:ident) => { fn $n(&self) { std::fs::write(std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
    pub(crate) use mktr;
}
mod inner {
    use crate::defs::{mkm, mktr};
    pub struct S;
    impl S { self::mkm! { go } pub fn visible(&self) {} }
    pub struct U;
    pub trait T { self::mktr! { tgo } }
    impl T for U {}
}
pub fn viamethod(s: &inner::S) { s.go() }
pub fn viavis(s: &inner::S) { s.visible() }
pub fn viaufcs() { inner::S::go(&inner::S) }
pub fn viatrait(u: &inner::U) { use inner::T; u.tgo() }
fn main() {}
"#;

/// A method on a type whose INHERENT impl holds an unexpanded impl-item invocation, the UFCS spelling of it, and a method of a
/// trait holding an unexpanded trait-item invocation, each naming the declared member. A VISIBLE member beside the hidden text
/// does not hedge.
#[test]
fn r1853_typed_method_callers_of_a_name_unexpanded_impl_item_or_trait_item_text_declares() {
    let r = scan(&crate_with("member", &[("src/main.rs", MEMBER)]));
    for f in ["viamethod", "viaufcs", "viatrait"] {
        assert!(unknown(&r, f), "{f} must read Unknown: {:?}", r.get(f));
    }
    assert!(why(&r, "viamethod", MEMBER_WHY) && why(&r, "viatrait", MEMBER_WHY) && why(&r, "viaufcs", MEMBER_WHY), "the member reason: {:?} {:?} {:?}", r.get("viamethod"), r.get("viatrait"), r.get("viaufcs"));
    assert!(!unknown(&r, "viavis"), "a VISIBLE member must not hedge: {:?}", r.get("viavis"));
}

/// The caller in ANOTHER file, and the invocation in another file's module: the module qual comes from the file's place in the
/// tree, not from the text.
#[test]
fn r1853_the_invocation_and_its_caller_in_other_files() {
    let r = scan(&crate_with("files", &[
        ("src/main.rs", "#![allow(unused)]\nmod defs;\nmod hid;\nmod user;\nfn main() {}\n"),
        ("src/defs.rs", "macro_rules! mkfn { ($n:ident) => { pub fn $n() { std::fs::write(std::env::var(\"MARK\").unwrap(), b\"x\").unwrap(); } } }\npub(crate) use mkfn;\n"),
        ("src/hid.rs", "use crate::defs::mkfn;\nself::mkfn! { gofile }\npub fn visible_f() {}\n"),
        ("src/user.rs", "use crate::hid::gofile;\nuse crate::hid::visible_f;\npub fn hit() { gofile() }\npub fn miss() { visible_f() }\n"),
    ]));
    assert!(unknown(&r, "user::hit"), "a `use`d name another file's hidden text declares: {:?}", r.get("user::hit"));
    assert!(!unknown(&r, "user::miss"), "a visible name of that module: {:?}", r.get("user::miss"));
}
