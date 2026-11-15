//! RESOLVER STEP 1 — the namespace authority, crate side (DESIGN §4.3 step 1). One test per acceptance and precision cell,
//! each a one-variable fixture with its control, and each re-run under `CANDOR_RESOLVER_STEP1_OFF=<part>` to prove the
//! fixture reaches the change (the old reading must come back).
//!
//! Every defect program below was BUILT AND EXECUTED in the lane (scratchpad `rustagent-step1/fx`): the `unlink`/`write`/
//! `touch` really ran, so a row that is absent or `[]` is a silence over a real effect.
//!
//! | test | row | rule (DESIGN §1.2) | part |
//! |---|---|---|---|
//! | `extern_prelude_is_per_module` | R1716 / R1738 | N1 + N4 | `scope` |
//! | `a_glob_over_a_rebound_dependency` | R1711 (b) | N2 (enumerable glob) | `glob` |
//! | `a_glob_imported_unit_variant` | R1426 open half | N1 + N2 (module and block scope) | `variant` |
//! | `self_is_named_in_the_impls_scope` | R1771 | N1 (block scope) | `block` |
//! | `a_cfg_definition_beside_an_external_reexport` | R1741 | N6 | `cfg` |
//! | `a_bound_through_a_local_reexport` | R1727 | N3 | `bound` |
//! | `a_root_glob_does_not_claim_a_root_declaration_in_pass_a` | R653 / R753 | N2, pass boundary | `passa` |
//! | `a_derive_macro_import_does_not_take_the_traits_name` | R654 | N5 | `ns` |
//! | `claimants_of_one_definition_are_not_ambiguous` | R752 | identity by definition (§2.5 clause 3) | `ident` |
//! | `cfg_arm_globs_are_a_union` | R900's rule, fn names only (types: step 4) | N6 | `cfg` |

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(tag: &str, deps: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-step1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("Cargo.toml"),
        format!("[package]\nname = \"s1{tag}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n{deps}"),
    )
    .unwrap();
    for (p, t) in files {
        let f = d.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, t).unwrap();
    }
    d
}

type Rows = HashMap<String, serde_json::Value>;

fn scan_env(dir: &Path, env: &[(&str, &str)]) -> (Rows, serde_json::Value) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_candor-scan"));
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_RESOLVER_STEP1_OFF"] {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run candor-scan");
    assert!(out.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    let rows = v["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["fn"].as_str().unwrap().to_string(), f.clone()))
        .collect();
    (rows, v)
}

fn scan(dir: &Path) -> (Rows, serde_json::Value) {
    scan_env(dir, &[])
}

fn scan_off(dir: &Path, part: &str) -> Rows {
    scan_env(dir, &[("CANDOR_RESOLVER_STEP1_OFF", part)]).0
}

fn strs(rows: &Rows, f: &str, field: &str) -> Vec<String> {
    rows.get(f)
        .and_then(|r| r[field].as_array())
        .map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn inferred(rows: &Rows, f: &str) -> Vec<String> {
    strs(rows, f, "inferred")
}

fn has(rows: &Rows, f: &str, e: &str) -> bool {
    inferred(rows, f).iter().any(|x| x == e)
}

const LIBC: &str = "libc = \"0.2\"\n";
const FAKE_FFI: &str = "pub mod ffi { pub unsafe fn unlink(_p: *const std::os::raw::c_char) -> i32 { 0 } }";

/// R1716 (a) + R1738 (b): an inline module that imports nothing sees the EXTERN PRELUDE, not its file's `use` of the same
/// name (a) nor the file's own `mod ffi` (b). Controls: a child that GLOB-imports its parent does see the parent's binding
/// (rustc looks globs up before the extern prelude), and `use super::ffi` in a child names the parent's binding.
#[test]
fn extern_prelude_is_per_module() {
    let root = "#![allow(unused_imports, dead_code)]\nextern crate libc as ffi;\n\
        pub mod other { pub mod ffi { pub unsafe fn unlink(_p: *const std::os::raw::c_char) -> i32 { 0 } } }\n\
        pub mod host; pub mod hostb; pub mod hostg; pub mod hosts;\n";
    let z = "pub unsafe fn z(p: *const std::os::raw::c_char) { ffi::unlink(p); }";
    let host = format!("use crate::other::ffi;\npub mod child {{ {z} }}\n");
    let hostb = format!("{FAKE_FFI}\npub mod child {{ {z} }}\n");
    let hostg = format!("use crate::other::ffi;\npub mod child {{ use super::*; {z} }}\n");
    let hosts = format!("use crate::other::ffi;\npub mod child {{ use super::ffi; {z} }}\n");
    let d = fixture(
        "prelude",
        LIBC,
        &[("src/lib.rs", root), ("src/host.rs", &host), ("src/hostb.rs", &hostb), ("src/hostg.rs", &hostg), ("src/hosts.rs", &hosts)],
    );
    let (on, v) = scan(&d);
    assert!(has(&on, "host::child::z", "Fs"), "R1716: the child's `ffi` is the extern prelude's libc\n{v:#}");
    assert!(has(&on, "hostb::child::z", "Fs"), "R1738: the parent's `mod ffi` does not shadow the prelude in the child\n{v:#}");
    assert!(inferred(&on, "hostg::child::z").is_empty(), "control: `use super::*` brings the parent's pure `ffi`\n{v:#}");
    assert!(inferred(&on, "hosts::child::z").is_empty(), "control: `use super::ffi` names the parent's pure `ffi`\n{v:#}");
    let off = scan_off(&d, "scope");
    assert!(!has(&off, "host::child::z", "Fs") && !has(&off, "hostb::child::z", "Fs"), "part `scope` off restores the silence");
}

/// R1711 (b): `mod n { pub use libc as sys; }` then `use crate::n::*; sys::unlink(p)` — the crate-local glob is enumerable,
/// so `sys` is `n`'s binding, the dependency. The named-import spelling (a) is the control.
#[test]
fn a_glob_over_a_rebound_dependency() {
    let src = "pub mod n { pub use libc as sys; }\n\
        pub mod ua { use crate::n::sys; pub unsafe fn za(p: *const std::os::raw::c_char) { sys::unlink(p); } }\n\
        pub mod ub { use crate::n::*; pub unsafe fn zb(p: *const std::os::raw::c_char) { sys::unlink(p); } }\n";
    let d = fixture("glob", LIBC, &[("src/lib.rs", src)]);
    let (on, v) = scan(&d);
    assert!(has(&on, "ua::za", "Fs"), "control (a)\n{v:#}");
    assert!(has(&on, "ub::zb", "Fs"), "R1711 (b): the glob brings `n`'s `sys`\n{v:#}");
    assert!(!has(&scan_off(&d, "glob"), "ub::zb", "Fs"), "part `glob` off restores the silence");
}

/// R1426, the open half: a UNIT variant imported by a glob of its enum, at module scope and at block scope.
#[test]
fn a_glob_imported_unit_variant() {
    let src = "pub enum Kd { Aa, Bb }\n\
        impl Kd { pub fn go(&self) { std::fs::write(\"/tmp/candor-step1-kd\", \"x\").unwrap(); } }\n\
        pub mod ug { use crate::Kd::*; pub fn zg() { Aa.go() } }\n\
        pub mod ul { pub fn zl() { use crate::Kd::*; Aa.go() } }\n\
        pub mod uq { use crate::Kd; pub fn zq() { Kd::Aa.go() } }\n";
    let d = fixture("variant", "", &[("src/lib.rs", src)]);
    let (on, v) = scan(&d);
    assert!(has(&on, "uq::zq", "Fs"), "control: the qualified spelling\n{v:#}");
    assert!(has(&on, "ug::zg", "Fs"), "R1426: module-scope enum glob\n{v:#}");
    assert!(has(&on, "ul::zl", "Fs"), "R1426: block-scope enum glob\n{v:#}");
    let off = scan_off(&d, "variant");
    assert!(!has(&off, "ug::zg", "Fs") && !has(&off, "ul::zl", "Fs"), "part `variant` off restores the silence");
}

/// R1771: a body-local `use` that rebinds the impl's self-type NAME does not change what `self` is.
#[test]
fn self_is_named_in_the_impls_scope() {
    let src = "pub mod other { pub struct Foo; impl Foo { pub fn run(&self) {} } }\n\
        pub struct Foo;\n\
        impl Foo {\n\
          pub fn run(&self) { std::fs::write(\"/tmp/candor-step1-run\", \"x\").unwrap(); }\n\
          pub fn go_aa(&self) { use crate::other::Foo; let _f = Foo; self.run(); }\n\
          pub fn go_dd(&self) { use crate::other::Foo; self.run(); }\n\
          pub fn go_cc(&self) { self.run(); }\n\
        }\n\
        pub fn pure_ctl() { use crate::other::Foo; Foo.run(); }\n";
    let d = fixture("block", "", &[("src/lib.rs", src)]);
    let (on, v) = scan(&d);
    assert!(has(&on, "Foo::go_cc", "Fs"), "control\n{v:#}");
    assert!(has(&on, "Foo::go_aa", "Fs"), "R1771\n{v:#}");
    assert!(has(&on, "Foo::go_dd", "Fs"), "R1771\n{v:#}");
    assert!(inferred(&on, "pure_ctl").is_empty(), "control: the body-local `Foo` VALUE is the other, pure type\n{v:#}");
    let off = scan_off(&d, "block");
    assert!(!has(&off, "Foo::go_aa", "Fs") && !has(&off, "Foo::go_dd", "Fs"), "part `block` off restores the silence");
}

/// R1741: a name declared under one `#[cfg]` arm and re-exported from a dependency under another is the UNION of the arms.
#[test]
fn a_cfg_definition_beside_an_external_reexport() {
    let src = "pub mod sha {\n\
          #[cfg(not(target_os = \"none\"))] pub fn digest(t: &str) { std::process::Command::new(\"touch\").arg(t).status().unwrap(); }\n\
          #[cfg(target_os = \"none\")] pub use libc::abs as digest;\n\
        }\n\
        use crate::sha::digest;\n\
        pub fn e_twin_zq(t: &str) { digest(t) }\n";
    let d = fixture("cfgdef", LIBC, &[("src/lib.rs", src)]);
    let (on, v) = scan(&d);
    assert!(has(&on, "e_twin_zq", "Exec"), "R1741: the declared arm's spawn is charged\n{v:#}");
    assert!(strs(&on, "e_twin_zq", "invisible").contains(&"libc".to_string()), "the external arm stays disclosed\n{v:#}");
    assert!(!has(&scan_off(&d, "cfg"), "e_twin_zq", "Exec"), "part `cfg` off restores the silence");
}

/// R1727: `T: crate::io::BufRead` through `pub mod io { pub use fio::AsyncBufRead as BufRead; }` keys the dependency's member
/// exactly as `use crate::io::BufRead; T: BufRead` does (`fio#AsyncBufRead::poll_fill_buf`), so a chained consumer's own
/// implementor is joined. The bare spelling is the control.
#[test]
fn a_bound_through_a_local_reexport() {
    let src = "pub mod io { pub use fio::AsyncBufRead as BufRead; }\n\
        pub fn alias_zq<T: crate::io::BufRead>(t: &mut T) -> usize { t.poll_fill_buf() }\n\
        pub mod b { use crate::io::BufRead; pub fn bare_zq<T: BufRead>(t: &mut T) -> usize { t.poll_fill_buf() } }\n";
    let d = fixture(
        "bound",
        "fio = { path = \"fio\" }\n",
        &[
            ("src/lib.rs", src),
            ("fio/Cargo.toml", "[package]\nname = \"fio\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            ("fio/src/lib.rs", "pub trait AsyncBufRead { fn poll_fill_buf(&mut self) -> usize; }\n"),
        ],
    );
    let key = |rows: &Rows, f: &str| strs(rows, f, "dispatchesOn");
    let (on, v) = scan(&d);
    assert_eq!(key(&on, "b::bare_zq"), vec!["fio#AsyncBufRead::poll_fill_buf".to_string()], "control\n{v:#}");
    assert_eq!(key(&on, "alias_zq"), key(&on, "b::bare_zq"), "R1727: both spellings key the dependency's member\n{v:#}");
    assert!(key(&scan_off(&d, "bound"), "alias_zq").is_empty(), "part `bound` off restores the missing key");
}

/// R653 / R753: the crate root's own external glob does not claim a `use crate::<root decl>::…` in Pass A, so the trait's
/// impls are not keyed under the dependency (`extdep#types::sink::Sink::emit`).
#[test]
fn a_root_glob_does_not_claim_a_root_declaration_in_pass_a() {
    let src = "pub mod sink;\n#[allow(unused_imports)] use libc::*;\nuse crate::sink::Sink;\npub struct W;\n\
        impl Sink for W { fn emit(&self) { std::fs::write(\"/tmp/candor-step1-w\", \"y\").unwrap(); } }\n\
        pub fn run(s: &dyn Sink) { s.emit() }\n";
    let d = fixture("passa", LIBC, &[("src/lib.rs", src), ("src/sink.rs", "pub trait Sink { fn emit(&self); }\n")]);
    let phantom = |v: &serde_json::Value| {
        v["functions"].as_array().unwrap().iter().any(|f| f["hash"].as_str().is_some_and(|h| h.starts_with("libc#")))
    };
    let (on, v) = scan(&d);
    assert!(!phantom(&v), "R653: no row is keyed under the glob's crate\n{v:#}");
    assert!(has(&on, "run", "Fs"), "the dispatch through the crate's own trait is still charged\n{v:#}");
    let (_, voff) = scan_env(&d, &[("CANDOR_RESOLVER_STEP1_OFF", "passa")]);
    assert!(phantom(&voff), "part `passa` off restores the phantom key (fixture reaches the change)");
}

/// R654: `pub use dep::Name;` beside `pub trait Name` binds another namespace (E0255 rules out a type), so `impl Name for W`
/// is this crate's trait and mints no dependency key. The dependency exports only a MACRO of that name — the derive-named-
/// after-its-trait idiom (diesel `AsChangeset`, zeroize `Zeroize`).
#[test]
fn a_derive_macro_import_does_not_take_the_traits_name() {
    let src = "pub use extdep::AsChangeset;\npub trait AsChangeset { fn build(&self); }\npub struct W;\n\
        impl AsChangeset for W { fn build(&self) { std::fs::write(\"/tmp/candor-step1-b\", \"y\").unwrap(); } }\n\
        pub fn run(s: &dyn AsChangeset) { s.build() }\n";
    let d = fixture(
        "ns",
        "extdep = { path = \"extdep\" }\n",
        &[
            ("src/lib.rs", src),
            ("extdep/Cargo.toml", "[package]\nname = \"extdep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            ("extdep/src/lib.rs", "#[macro_export] macro_rules! AsChangeset { () => {}; }\n"),
        ],
    );
    let phantom = |v: &serde_json::Value| {
        v["functions"].as_array().unwrap().iter().any(|f| f["hash"].as_str().is_some_and(|h| h.starts_with("extdep#")))
    };
    let (on, v) = scan(&d);
    assert!(!phantom(&v), "R654: no `extdep#AsChangeset::build`\n{v:#}");
    assert!(has(&on, "run", "Fs"), "{v:#}");
    let (_, voff) = scan_env(&d, &[("CANDOR_RESOLVER_STEP1_OFF", "ns")]);
    assert!(phantom(&voff), "part `ns` off restores the phantom key (fixture reaches the change)");
}

/// R752: two modules claiming one tail2 key, both standing for ONE definition (`a::fs` re-exports it explicitly; `fs` globs
/// `a::fs`), are not ambiguous. Control: two claimants with DIFFERENT definitions are still refused and disclosed.
#[test]
fn claimants_of_one_definition_are_not_ambiguous() {
    let src = "pub mod a { pub mod fs { mod check { pub fn probe(p: &str) { std::fs::remove_file(p).ok(); } } \
               pub(crate) use check::probe; } }\n\
        pub mod fs { pub(crate) use crate::a::fs::*; }\n\
        pub mod b { pub mod fs { mod k { pub fn other(p: &str) { std::fs::remove_file(p).ok(); } } pub(crate) use k::other; } }\n\
        pub mod c { pub mod fs { mod k { pub fn other(_p: &str) {} } pub(crate) use k::other; } }\n\
        pub fn same_zq(p: &str) { fs::probe(p) }\n\
        pub fn twin_zq(p: &str) { crate::b::fs::other(p); crate::c::fs::other(p); }\n";
    let d = fixture("ident", "", &[("src/lib.rs", src)]);
    let (on, v) = scan(&d);
    assert!(has(&on, "same_zq", "Fs"), "R752: one definition behind two claimants\n{v:#}");
    assert!(!strs(&on, "same_zq", "unknownWhy").iter().any(|w| w.contains("claimed by two modules")), "{v:#}");
    assert!(strs(&on, "twin_zq", "unknownWhy").iter().any(|w| w.contains("claimed by two modules")),
        "control: claimants with DIFFERENT definitions stay refused and disclosed\n{v:#}");
    let off = scan_off(&d, "ident");
    assert!(strs(&off, "same_zq", "unknownWhy").iter().any(|w| w.contains("claimed by two modules")),
        "part `ident` off restores the refusal (fixture reaches the change)");
}

/// R900's rule (cfg arms are a set) for a FUNCTION reached through `pub(crate) use self::inner::*;` over two `#[cfg]` `#[path]`
/// arms of `mod inner`: the union of the arms. A TYPE reached that way (`zi::Database::from_env`, jiff's shape) keeps the
/// refusal and its disclosure: a member call through an arm set of types is resolved per arm, and an arm whose type gets the
/// member from a trait's provided body names no unit there (flate2, corpus A/B) — that is member resolution, step 4.
#[test]
fn cfg_arm_globs_are_a_union() {
    let src = "pub mod zi;\npub fn get_zq() -> u32 { zi::from_env() }\npub fn ty_zq() -> u32 { zi::Database::from_env() }\n\
        pub mod co { pub fn from_env() -> u32 { 1 } pub struct Database; impl Database { pub fn from_env() -> u32 { 1 } } }\n";
    let zi = "pub(crate) use self::inner::*;\n\
        #[cfg(not(feature = \"zi\"))] #[path = \"disabled.rs\"] mod inner;\n\
        #[cfg(feature = \"zi\")] #[path = \"enabled.rs\"] mod inner;\n";
    let dis = "pub fn from_env() -> u32 { 0 }\npub struct Database;\nimpl Database { pub fn from_env() -> u32 { 0 } }\n";
    let en = "pub fn from_env() -> u32 { std::env::var(\"TZDIR\").map(|v| v.len() as u32).unwrap_or(0) }\n\
        pub struct Database;\nimpl Database { pub fn from_env() -> u32 { std::env::var(\"TZDIR\").map(|v| v.len() as u32).unwrap_or(0) } }\n";
    let d = fixture(
        "arms",
        "[features]\nzi = []\n",
        &[("src/lib.rs", src), ("src/zi/mod.rs", zi), ("src/zi/disabled.rs", dis), ("src/zi/enabled.rs", en)],
    );
    let (on, v) = scan(&d);
    assert!(has(&on, "get_zq", "Env"), "a fn through the arm set: the enabled arm's env read is charged\n{v:#}");
    assert!(!has(&on, "get_zq", "Unknown"), "a fn through the arm set is a union, not an ambiguity\n{v:#}");
    assert!(has(&on, "ty_zq", "Unknown"), "control: a type through the arm set keeps the refusal's disclosure\n{v:#}");
    // Reach: under the fan-out cap the re-export index already unions a fn's cfg arms, so this fn is answered with the
    // part off too; the shape the part alone answers is R190(e)'s thirteen arms (`tests.rs`, `go` gains the live `Exec`).
    let off = scan_off(&d, "cfg");
    assert!(has(&off, "get_zq", "Env") && has(&off, "ty_zq", "Unknown"), "part `cfg` off: the legacy reading is unchanged here");
}

/// Fable's step-1 review §2: a `static` (or `const`) reached through a `#[cfg]` arm set is NOT unioned. `zi::PDB.pq()` is a
/// member call on each arm's declared type, and where arm b's type gets `pq` from a trait's PROVIDED body the arm loop's
/// tail match drops that arm (the same per-arm member resolution a type answer needs, step 4). A `mod` and a `fn` arm set
/// still are. Read off the namespace's own reach counter, because the unit rows are the same either way: `static_prov_zq`
/// is ABSENT on 0.42.0, main and this lane (a pre-existing residual, filed by the coordinator), and `static_zq` keeps the
/// hedge it had.
#[test]
fn a_static_through_a_cfg_arm_set_is_not_unioned() {
    let lib = "pub mod zi;\npub fn modarm_zq() -> u32 { zi::sys::f() }\npub fn static_zq() -> u32 { zi::DB.q() }\n\
        pub fn fnarm_zq() -> u32 { zi::from_env() }\n";
    let zi = "pub(crate) use self::inner::*;\n\
        #[cfg(not(feature = \"zi\"))] #[path = \"a.rs\"] mod inner;\n\
        #[cfg(feature = \"zi\")] #[path = \"b.rs\"] mod inner;\n";
    let a = "pub mod sys { pub fn f() -> u32 { 0 } }\npub fn from_env() -> u32 { 0 }\n\
        pub struct Database; impl Database { pub fn q(&self) -> u32 { 0 } }\npub static DB: Database = Database;\n";
    let b = "pub mod sys { pub fn f() -> u32 { std::env::var(\"X\").map(|v| v.len() as u32).unwrap_or(0) } }\n\
        pub fn from_env() -> u32 { std::env::var(\"T\").map(|v| v.len() as u32).unwrap_or(0) }\n\
        pub struct Database; impl Database { pub fn q(&self) -> u32 { std::env::var(\"Y\").map(|v| v.len() as u32).unwrap_or(0) } }\n\
        pub static DB: Database = Database;\n";
    let d = fixture(
        "statarm",
        "[features]\nzi = []\n",
        &[("src/lib.rs", lib), ("src/zi/mod.rs", zi), ("src/zi/a.rs", a), ("src/zi/b.rs", b)],
    );
    let out = Command::new(env!("CARGO_BIN_EXE_candor-scan"))
        .arg(d.to_string_lossy().as_ref())
        .arg("--json")
        .env_remove("CANDOR_RESOLVER_STEP1_OFF")
        .env("CANDOR_STEP1_INSTR", "1")
        .output()
        .expect("run candor-scan");
    let err = String::from_utf8_lossy(&out.stderr);
    let armed = |n: &str| err.lines().any(|l| l.starts_with("STEP1ARMS\t") && l.split('\t').nth(1) == Some(n));
    assert!(armed("zi::sys") && armed("zi::from_env"), "a mod / fn arm set is unioned:\n{err}");
    assert!(!armed("zi::DB"), "a static arm set is not unioned (member resolution per arm is step 4):\n{err}");
    let (on, v) = scan(&d);
    assert!(has(&on, "modarm_zq", "Env") && has(&on, "fnarm_zq", "Env"), "{v:#}");
    assert!(has(&on, "static_zq", "Unknown"), "control: the static keeps its disclosure\n{v:#}");
}

/// Step 1 round 4 (part `assoc`): a dependency dispatch on an ASSOCIATED-TYPE value (`s.seq().end()`, serde's
/// `s.serialize_seq(..)?.end()`) is keyed under the PARENT trait (`fio#Ser::end`), while a consumer's implementor of the
/// associated type is keyed under ITS trait (`SeqT::end`) and nothing joins the two: EXECUTED with real serde (fixture `assoc`),
/// a consumer whose `SerializeSeq` writes a file read `[]` + `invisible` on 0.42.0 and main. The site now discloses. A chained
/// `Result`/`Option` method on a direct call (`seed.deserialize(d).map(..)`, R549 drift) is not this shape and is left alone.
#[test]
fn a_dependency_dispatch_on_an_associated_type_value_is_disclosed() {
    let src = "pub fn named_zq<S: fio::Ser>(s: S) { let mut q = s.seq(); q.push_elem(); q.end() }\n\
        pub fn chained_zq<S: fio::Ser>(s: S) -> Result<(), ()> { s.try_seq()?.push_elem(); Ok(()) }\n\
        pub fn end_only_zq<S: fio::Ser>(s: S) { let q = s.seq(); q.end() }\n\
        pub fn bare_zq<S: fio::Ser>(s: S) { s.seq().push_elem() }\n\
        pub fn direct_zq<S: fio::Ser>(s: S) { s.done() }\n";
    let fio = "pub trait SeqT { fn push_elem(&mut self); fn end(self); }\n\
        pub trait Ser { type Seq: SeqT; fn seq(self) -> Self::Seq; fn try_seq(self) -> Result<Self::Seq, ()>; fn done(self); }\n";
    let d = fixture(
        "assocv",
        "fio = { path = \"fio\" }\n",
        &[
            ("src/lib.rs", src),
            ("fio/Cargo.toml", "[package]\nname = \"fio\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
            ("fio/src/lib.rs", fio),
        ],
    );
    let why = |rows: &Rows, f: &str| strs(rows, f, "unknownWhy").iter().any(|w| w.contains("returns, whose type is not known here"));
    let (on, v) = scan(&d);
    assert!(has(&on, "named_zq", "Unknown") && why(&on, "named_zq"), "binding spelling discloses\n{v:#}");
    assert!(has(&on, "chained_zq", "Unknown") && why(&on, "chained_zq"), "chained spelling discloses\n{v:#}");
    assert!(!why(&on, "direct_zq"), "control: a direct member of the bound trait is not this shape\n{v:#}");
    // the stated residual: a member that shares a std method name (`end` is `RangeInclusive::end`) is excluded with the R549
    // drift it is indistinguishable from here
    assert!(!why(&on, "end_only_zq"), "residual: an `end()`-only body is not disclosed by this site\n{v:#}");
    assert!(!why(&on, "bare_zq"), "residual: a bare `s.seq().m()` chain (the drift shape) is not disclosed by this site\n{v:#}");
    let off = scan_off(&d, "assoc");
    assert!(!why(&off, "named_zq") && !why(&off, "chained_zq"), "part `assoc` off restores the old reading");
}
