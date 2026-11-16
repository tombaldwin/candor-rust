//! SOUNDNESS R1541 (I1) — units a `macro_rules!` makes. Lane `rust-i1`, stage 1.
//!
//! Step 1, SCOPE-AWARE BINDING: an item-position invocation binds to a crate-local `macro_rules!` by NAME, so a definition
//! rustc does not pick — a `#[cfg]`-gated stub beside a `use dep::name` import of the same name (deranged-0.5.8's
//! `#[cfg(docsrs)] macro_rules! int` beside `pub use deranged_macros::int`), or any local definition of a name the
//! invoking file imports from outside the crate (redox_syscall's `use bitflags::bitflags` beside a local wrapper) — minted
//! the WRONG items. Measured on the lane's executed fixture (`fx/bind`): the compiled-out stub's empty `bnd_f` became a unit
//! and the caller read pure over the dependency macro's write. Refused now; `CANDOR_I1_BIND_OFF=1` restores the old binding,
//! and each case below asserts the OFF arm still mints the wrong unit (so the fixture reaches the code).
//!
//! Steps 2+ (trait impls and declarations spliced, the one "expanded" fact) are pinned further down.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

const W: &str = "let _ = std::fs::write(\"/tmp/i1-never-run\", b\"x\");";

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-i1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_crate(dir: &Path, name: &str, deps: &str, src: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps}"),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rs"), format!("#![allow(unused)]\n{}\n", src.replace("EFF", W))).unwrap();
}

/// One scan. Returns (fn -> (inferred, unknownWhy)) over the report rows and the SET of every unit in the call-graph
/// sidecar (which holds pure units too, so it can say a unit was MINTED).
type Rows = BTreeMap<String, (Vec<String>, Vec<String>)>;
fn scan(dir: &Path, deps: Option<&Path>, env: &[(&str, &str)]) -> (Rows, BTreeSet<String>) {
    let out = dir.join("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r"));
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_I1_OFF", "CANDOR_I1_BIND_OFF", "CANDOR_I1_RESPAN_OFF"] {
        cmd.env_remove(k);
    }
    if let Some(d) = deps {
        cmd.env("CANDOR_DEPS", d);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run candor-scan");
    assert!(o.status.code().is_some_and(|c| c == 0 || c == 1), "scan failed: {}", String::from_utf8_lossy(&o.stderr));
    let mut rows = Rows::new();
    let mut units = BTreeSet::new();
    for e in std::fs::read_dir(&out).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        if n.ends_with(".callgraph.json") {
            units.extend(v.as_object().unwrap().keys().cloned());
        } else if n.ends_with(".json") && !n.ends_with(".locs.json") && !n.ends_with(".hierarchy.json") {
            let strs = |x: &serde_json::Value| -> Vec<String> {
                x.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(str::to_string)).collect()).unwrap_or_default()
            };
            for f in v["functions"].as_array().unwrap() {
                rows.insert(f["fn"].as_str().unwrap().to_string(), (strs(&f["inferred"]), strs(&f["unknownWhy"])));
            }
        }
    }
    (rows, units)
}

fn eff(rows: &Rows, f: &str) -> Vec<String> {
    rows.get(f).map(|r| r.0.clone()).unwrap_or_default()
}

/// Step 1 — a `#[cfg]`-gated sole definition of a name the file also imports: rustc binds the import.
#[test]
fn i1_step1_cfg_gated_stub_beside_an_import_is_not_expanded() {
    let d = tmpdir("bindcfg");
    write_crate(
        &d,
        "bindcfg",
        "[dependencies]\ndmlib = { path = \"../dmlib\" }\n",
        "#[cfg(any())]\nmacro_rules! make_bnd { () => { pub fn bnd_f() { EFF } }; }\nuse dmlib::make_bnd;\nmake_bnd!();\n\
         pub fn b_call() { bnd_f() }\n",
    );
    let (on_rows, on_units) = scan(&d, None, &[]);
    assert!(!on_units.contains("bnd_f"), "the compiled-out stub must not be expanded: {on_units:?}");
    assert!(!eff(&on_rows, "b_call").contains(&"Fs".to_string()), "b_call must not be charged the stub's body");
    let (off_rows, off_units) = scan(&d, None, &[("CANDOR_I1_BIND_OFF", "1")]);
    assert!(off_units.contains("bnd_f"), "OFF must mint the stub's unit (the fixture reaches the code): {off_units:?}");
    assert_eq!(eff(&off_rows, "b_call"), vec!["Fs".to_string()], "OFF binds the stub (the old, wrong binding)");
}

/// Step 1 — an UNGATED local definition of a name the invoking file imports from outside the crate is refused too; a
/// cfg-gated definition WITHOUT a competing import is still expanded (its invocations are gated with it).
#[test]
fn i1_step1_import_clash_refused_and_gated_def_without_import_kept() {
    let d = tmpdir("bindimp");
    write_crate(
        &d,
        "bindimp",
        "",
        "macro_rules! wrap { () => { pub fn wr_f() { EFF } }; }\nmod a { use other::wrap; wrap!(); pub fn a_call() { wr_f() } }\n\
         #[cfg(feature = \"x\")]\nmacro_rules! gated { () => { pub fn gt_f() { EFF } }; }\n#[cfg(feature = \"x\")]\ngated!();\n\
         pub fn g_call() { gt_f() }\n",
    );
    let (on_rows, on_units) = scan(&d, None, &[]);
    assert!(!on_units.contains("a::wr_f"), "an import clash must refuse the local binding: {on_units:?}");
    assert!(on_units.contains("gt_f"), "a gated definition with no competing import is still expanded: {on_units:?}");
    assert_eq!(eff(&on_rows, "g_call"), vec!["Fs".to_string()]);
    let (_, off_units) = scan(&d, None, &[("CANDOR_I1_BIND_OFF", "1")]);
    assert!(off_units.contains("a::wr_f"), "OFF binds the local wrapper: {off_units:?}");
}

/// The lane's executed fixture `fx/inc`: a `macro_rules!` that stamps `impl LoadIt for $t` onto a std type and a local
/// struct, every spelling of the call, and a pure twin macro with uniquely named members.
const INC: &str = "use std::sync::atomic::{AtomicU16, AtomicU32};\n\
pub trait LoadIt { fn load_it(&self) -> u32; }\n\
pub trait QuietIt { fn quiet_it(&self) -> u32; }\n\
pub struct Loc(pub u32);\npub struct Qloc(pub u32);\n\
macro_rules! impl_load { ($t:ty) => { impl LoadIt for $t { fn load_it(&self) -> u32 { EFF 1 } } }; }\n\
macro_rules! impl_quiet { ($t:ty) => { impl QuietIt for $t { fn quiet_it(&self) -> u32 { 2 } } }; }\n\
impl_load!(AtomicU32);\nimpl_load!(Loc);\nimpl_quiet!(AtomicU16);\nimpl_quiet!(Qloc);\n\
pub fn gen_load<T: LoadIt>(t: &T) -> u32 { t.load_it() }\n\
pub fn s_method(a: &AtomicU32) -> u32 { a.load_it() }\n\
pub fn s_ufcs(a: &AtomicU32) -> u32 { <AtomicU32 as LoadIt>::load_it(a) }\n\
pub fn s_tpath(a: &AtomicU32) -> u32 { LoadIt::load_it(a) }\n\
pub fn s_generic(a: &AtomicU32) -> u32 { gen_load(a) }\n\
pub fn s_dyn(a: &AtomicU32) -> u32 { let d: &dyn LoadIt = a; d.load_it() }\n\
pub fn l_method(a: &Loc) -> u32 { a.load_it() }\n\
pub fn l_ufcs(a: &Loc) -> u32 { <Loc as LoadIt>::load_it(a) }\n\
pub fn qs_method(a: &AtomicU16) -> u32 { a.quiet_it() }\n\
pub fn ql_ufcs(a: &Qloc) -> u32 { <Qloc as QuietIt>::quiet_it(a) }\n";

/// Step 2 — a macro-made TRAIT impl is a unit, in-crate, on a std self type (R1541's own `s_ufcs` cell, silent on
/// published 0.42.0 and main) and on a local one; the R828 hedge for the fully expanded macro is retired, so the
/// `dyn` and generic spellings read the body rather than `Unknown`; pure twins read pure. OFF restores the old answers.
#[test]
fn i1_step2_macro_made_trait_impl_is_a_unit_in_crate() {
    let d = tmpdir("inc");
    write_crate(&d, "inc", "", INC);
    let (rows, units) = scan(&d, None, &[]);
    for f in ["s_method", "s_ufcs", "s_tpath", "s_generic", "s_dyn", "l_method", "l_ufcs"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f} must be charged the macro-made impl's write");
    }
    for f in ["qs_method", "ql_ufcs"] {
        assert!(eff(&rows, f).is_empty(), "pure twin {f} must read pure: {:?}", rows.get(f));
    }
    assert!(units.contains("AtomicU32::load_it") && units.contains("Loc::load_it"), "{units:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_OFF", "1")]);
    assert!(eff(&off, "s_ufcs").is_empty(), "OFF: s_ufcs is the R1541 silence again");
    assert_eq!(eff(&off, "s_dyn"), vec!["Unknown".to_string()], "OFF: the R828 hedge is back");
}

/// Step 2, chained — the producer publishes the macro-made impl, so a consumer joins it. `ch0` is the shape whose
/// dependency has NO hand-written implementor of the trait (F2's macro_rules instance): on main the std-receiver call
/// read ABSENT under `deny Fs` and `deny Unknown`.
#[test]
fn i1_step2_chained_consumer_joins_the_macro_made_impl() {
    let root = tmpdir("ch");
    let lib = root.join("chlib");
    let app = root.join("chapp");
    write_crate(
        &lib,
        "chlib",
        "",
        "use std::sync::atomic::AtomicU32;\npub trait LoadIt { fn load_it(&self) -> u32; }\npub struct Loc(pub u32);\n\
         macro_rules! impl_load { ($t:ty) => { impl LoadIt for $t { fn load_it(&self) -> u32 { EFF 1 } } }; }\n\
         impl_load!(AtomicU32);\nimpl_load!(Loc);\npub fn hand_writer() { EFF }\n",
    );
    write_crate(
        &app,
        "chapp",
        "[dependencies]\nchlib = { path = \"../chlib\" }\n",
        "use std::sync::atomic::AtomicU32;\nuse chlib::{Loc, LoadIt};\npub fn c_live() { chlib::hand_writer() }\n\
         pub fn c_s_method(a: &AtomicU32) -> u32 { a.load_it() }\npub fn c_l_method(a: &Loc) -> u32 { a.load_it() }\n\
         pub fn c_l_ufcs(a: &Loc) -> u32 { <Loc as LoadIt>::load_it(a) }\n",
    );
    let deps = root.join("deps");
    for (arm, env) in [("on", vec![]), ("off", vec![("CANDOR_I1_OFF", "1")])] {
        let _ = std::fs::remove_dir_all(&deps);
        let o = Command::new(bin())
            .arg(lib.to_string_lossy().as_ref())
            .arg("--out")
            .arg(deps.join("r"))
            .env_remove("CANDOR_DEPS")
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(o.status.code().is_some_and(|c| c == 0 || c == 1));
        let (rows, _) = scan(&app, Some(&deps), &[]);
        assert_eq!(eff(&rows, "c_live"), vec!["Fs".to_string()], "{arm}: the join must be LIVE");
        if arm == "on" {
            for f in ["c_s_method", "c_l_method", "c_l_ufcs"] {
                assert!(eff(&rows, f).contains(&"Fs".to_string()), "{f} must join the producer's macro-made impl: {rows:?}");
            }
        } else {
            assert!(!rows.contains_key("c_s_method"), "OFF: c_s_method is ABSENT again (the fixture reaches the code)");
        }
    }
}

/// Step 2 — ONE "expanded" fact. A fully expanded invocation no longer counts as hidden module text, so R128's
/// `macro:` hedge on a path into that module is retired (`via_a` names a fn the module does not declare; it is the probe
/// that the module's evidence is gone, and OFF restores the hedge). A PARTIAL expansion — it also emits a `mod`, a
/// stage-2 kind — keeps the invocation and with it the hedge (`via_b2`).
#[test]
fn i1_step2_expanded_fact_retires_module_evidence_partial_keeps_it() {
    let d = tmpdir("partial");
    write_crate(
        &d,
        "partial",
        "",
        "pub mod a { macro_rules! full { () => { pub struct Fa; impl Fa { pub fn fa(&self) {} } }; } full!(); }\n\
         pub mod b { macro_rules! part { () => { pub mod inner { pub fn hid() { EFF } } pub fn pb() {} }; } part!(); }\n\
         pub fn via_a() { crate::a::nope() }\npub fn via_b2() { crate::b::hid2() }\n",
    );
    let (rows, units) = scan(&d, None, &[]);
    assert!(units.contains("a::Fa::fa") && units.contains("b::pb"), "{units:?}");
    assert!(!rows.contains_key("via_a"), "module `a` was expanded in full; its R128 evidence is retired: {rows:?}");
    assert_eq!(eff(&rows, "via_b2"), vec!["Unknown".to_string()], "a partial expansion keeps R128's disclosure: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_OFF", "1")]);
    assert_eq!(eff(&off, "via_a"), vec!["Unknown".to_string()], "OFF: the kept invocation is evidence again");
}

/// Step 2 — a declaration the macro makes is spliced WITH its impls (Fable §7.3): the impl's units are keyed under the
/// macro-made type, and a same-leaf hand-written type in another module does not claim them (R1678's class).
#[test]
fn i1_step2_macro_made_struct_owns_its_impl() {
    let d = tmpdir("decl");
    write_crate(
        &d,
        "decl",
        "",
        "pub trait Go { fn go(&self); }\n\
         pub mod m { macro_rules! mk { ($n:ident) => { pub struct $n; impl crate::Go for $n { fn go(&self) { EFF } } }; } mk!(Twin); }\n\
         pub mod o { pub struct Twin; impl crate::Go for Twin { fn go(&self) {} } }\n\
         pub fn call_m(t: &m::Twin) { crate::Go::go(t) }\npub fn call_o(t: &o::Twin) { crate::Go::go(t) }\n",
    );
    let (rows, units) = scan(&d, None, &[]);
    assert!(units.contains("m::Twin::go"), "{units:?}");
    assert_eq!(eff(&rows, "call_m"), vec!["Fs".to_string()], "{rows:?}");
}

/// Step 2 — the retirement is CRATE-WIDE and per macro: R828's hedge for a `macro_rules!` implementor stays whenever ANY
/// invocation of that macro is left unexpanded (here one sits in a fn body, where the item splice does not reach), so a
/// dispatch over the trait still discloses rather than reading the visible pure implementor alone.
#[test]
fn i1_step2_an_unexpanded_invocation_keeps_the_r828_hedge() {
    let d = tmpdir("keep828");
    write_crate(
        &d,
        "keep828",
        "",
        "pub trait T2 { fn go(&self); }\npub struct Pure; impl T2 for Pure { fn go(&self) {} }\n\
         macro_rules! mk2 { ($t:ident) => { impl T2 for $t { fn go(&self) { EFF } } } }\n\
         pub struct M2; mk2!(M2);\npub fn setup() { struct Hid; mk2!(Hid); }\n\
         pub fn l2(t: &dyn T2) { t.go() }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    assert!(eff(&rows, "l2").contains(&"Unknown".to_string()), "the R828 hedge must stay: {rows:?}");
    assert!(eff(&rows, "l2").contains(&"Fs".to_string()), "and the expanded M2 impl is charged: {rows:?}");
}

/// Step 3 (Fable §7.7) — a SAME-FILE expansion is re-spanned to its invocation, so N expansions of one macro publish N
/// units at N invocation `loc`s rather than all at one line inside the `macro_rules!` body (hickory-resolver's eleven
/// `*_lookup` units at `resolver.rs:51:9`, which the census mapper could not tell apart — F1).
#[test]
fn i1_step3_same_file_expansions_carry_the_invocation_loc() {
    let d = tmpdir("respan");
    write_crate(
        &d,
        "respan",
        "",
        "pub struct R;\nmacro_rules! lookup_fn {\n    ($p:ident) => {\n        pub fn $p(&self) { EFF }\n    };\n}\n\
         impl R {\n    lookup_fn!(a_lookup);\n    lookup_fn!(b_lookup);\n}\n",
    );
    let locs = |env: &[(&str, &str)]| -> BTreeMap<String, String> {
        let out = d.join("out");
        let _ = std::fs::remove_dir_all(&out);
        let mut cmd = Command::new(bin());
        cmd.arg(d.to_string_lossy().as_ref()).arg("--out").arg(out.join("r")).env_remove("CANDOR_I1_RESPAN_OFF");
        for (k, v) in env {
            cmd.env(k, v);
        }
        assert!(cmd.output().unwrap().status.code().is_some_and(|c| c == 0 || c == 1));
        let mut m = BTreeMap::new();
        for e in std::fs::read_dir(&out).unwrap() {
            let p = e.unwrap().path();
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if n.ends_with(".scan.json") {
                let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                for f in v["functions"].as_array().unwrap() {
                    m.insert(f["fn"].as_str().unwrap().to_string(), f["loc"].as_str().unwrap_or("").to_string());
                }
            }
        }
        m
    };
    let on = locs(&[]);
    // the `lookup_fn!` path token of each invocation (lib.rs line 1 is the `#![allow]` the helper writes)
    assert_eq!(on.get("R::a_lookup").map(String::as_str), Some("src/lib.rs:9:5"), "{on:?}");
    assert_eq!(on.get("R::b_lookup").map(String::as_str), Some("src/lib.rs:10:5"), "{on:?}");
    let off = locs(&[("CANDOR_I1_RESPAN_OFF", "1")]);
    assert_eq!(off.get("R::a_lookup"), off.get("R::b_lookup"), "OFF: both at the definition span: {off:?}");
}

/// Step 2b — a NESTED invocation an expansion could not expand (here `crate::mk3!`, invoked by path inside `outer!`) stays
/// unexpanded program text: it must still count as an unexpanded invocation of `mk3`, so R828's hedge for `mk3`'s impls
/// stands even though another, bare invocation of `mk3` was expanded in full. Dropped, it made the crate-wide retirement
/// retire the evidence (bitflags 2.9.4: `__impl_public_bitflags` invoked by `$crate::` path inside `__impl_internal_bitflags`).
#[test]
fn i1_step2b_a_nested_unexpanded_invocation_keeps_the_hedge() {
    let d = tmpdir("nested");
    write_crate(
        &d,
        "nested",
        "",
        "pub trait T3 { fn go(&self); }\npub struct Pure; impl T3 for Pure { fn go(&self) {} }\n\
         #[macro_export]\nmacro_rules! mk3 { ($t:ident) => { impl crate::T3 for $t { fn go(&self) { EFF } } } }\n\
         macro_rules! outer { ($t:ident) => { pub struct $t; crate::mk3!($t); } }\n\
         pub struct M3; mk3!(M3);\nouter!(Hid);\npub fn l3(t: &dyn T3) { t.go() }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    assert!(eff(&rows, "l3").contains(&"Unknown".to_string()), "the R828 hedge must stay: {rows:?}");
    assert!(eff(&rows, "l3").contains(&"Fs".to_string()), "and the expanded M3 impl is charged: {rows:?}");
}

/// Step 4 — a free fn's generic-return position WITHDRAWN by a same-leaf contributor (`parser::from_str<B: Flags>` beside a
/// `FromStr::from_str` method) is not an absent one: `from_str::<B>(s)?.bits()` dispatches through `B`'s bounds and keeps
/// the `Flags::bits` key. Pre-existing for the hand-written shape (ABSENT on main), exposed in bitflags by I1's spliced
/// `impl FromStr`. `CANDOR_I1_RETGENW_OFF=1` restores the silence.
#[test]
fn i1_step4_withdrawn_generic_return_still_dispatches() {
    let d = tmpdir("retgenw");
    write_crate(
        &d,
        "retgenw",
        "",
        "pub trait Flags: Sized { fn bits(&self) -> u32; }\n\
         pub mod parser {\n    pub fn from_str<B: crate::Flags>(_s: &str) -> Result<B, ()> { Err(()) }\n\
             pub fn from_str_truncate<B: crate::Flags>(s: &str) -> Result<u32, ()> { Ok(from_str::<B>(s)?.bits()) }\n}\n\
         pub struct Field0(u32);\nimpl core::str::FromStr for Field0 { type Err = (); fn from_str(s: &str) -> Result<Self, ()> { \
         crate::parser::from_str::<Pub>(s).map(|_| Field0(0)) } }\n\
         pub struct Pub(u32);\nimpl Flags for Pub { fn bits(&self) -> u32 { EFF self.0 } }\n",
    );
    let (rows, units) = scan(&d, None, &[]);
    assert_eq!(eff(&rows, "parser::from_str_truncate"), vec!["Fs".to_string()], "{rows:?} {units:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_RETGENW_OFF", "1")]);
    // OFF: with an EFFECTFUL implementor the untyped-receiver hedge discloses (`Unknown`, no charge); with only pure
    // implementors (the bitflags shape) the row was ABSENT and the `Flags::bits` key gone — the silence this closes.
    assert!(!eff(&off, "parser::from_str_truncate").contains(&"Fs".to_string()), "OFF reads no resolution: {off:?}");
}

/// Step 5 — a PATH call through a type alias of a PRIMITIVE the crate implements a trait for (`type Cursor = u32;` beside
/// `impl TryParse for u32`) was skipped as "std/core's assoc fn" and read PURE (pre-existing; x11rb-protocol was covered
/// only by the `macro:` evidence of its unexpanded `implement_try_parse!`, which I1 retires). Disclosed at the lost site.
#[test]
fn i1_step5_alias_to_a_primitive_with_a_local_impl_discloses() {
    let d = tmpdir("primalias");
    write_crate(
        &d,
        "primalias",
        "",
        "pub trait TryParse: Sized { fn try_parse(v: &[u8]) -> Result<(Self, &[u8]), ()>; }\n\
         impl TryParse for u32 { fn try_parse(v: &[u8]) -> Result<(Self, &[u8]), ()> { EFF Ok((0, v)) } }\n\
         pub type Cursor = u32;\n\
         pub fn tp(v: &[u8]) -> Result<u32, ()> { let (c, _) = Cursor::try_parse(v)?; Ok(c) }\n\
         pub fn tp_direct(v: &[u8]) -> Result<u32, ()> { let (c, _) = u32::try_parse(v)?; Ok(c) }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    assert_eq!(eff(&rows, "tp"), vec!["Unknown".to_string()], "{rows:?}");
    assert_eq!(eff(&rows, "tp_direct"), vec!["Fs".to_string()], "control: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_PRIMALIAS_OFF", "1")]);
    assert!(!off.contains_key("tp"), "OFF: the pre-existing silence: {off:?}");
}

/// Step 6 (the R1804 exposure guard) — where a crate holds a non-std derive, an attribute macro or an unexpanded item
/// invocation, a trait may have an implementor the scan cannot see, and R828's `macro_rules!`-body hedge was the only
/// disclosure on a dispatch over it (wasm-bindgen 0.2.130 `RefFromWasmAbi::try_ref_from_abi`, palette 0.7.7
/// `Alpha::set_hue`; both read `Unknown` in the rustc-expanded program, pure on the first stage-1 cut). The retirement
/// keeps that hedge there. `CANDOR_I1_HIDDEN_OFF=1` retires it anyway.
#[test]
fn i1_step6_hidden_impl_surface_keeps_the_r828_hedge() {
    let d = tmpdir("hidden");
    write_crate(
        &d,
        "hidden",
        "",
        "pub trait Go { fn go(&self); }\npub struct Pure; impl Go for Pure { fn go(&self) {} }\n\
         macro_rules! mk { ($t:ident) => { impl Go for $t { fn go(&self) {} } } }\npub struct M; mk!(M);\n\
         #[derive(other::GoDerive)] pub struct Hid;\n\
         pub fn d(t: &dyn Go) { t.go() }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    assert_eq!(eff(&rows, "d"), vec!["Unknown".to_string()], "the hedge must stand: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_HIDDEN_OFF", "1")]);
    assert!(eff(&off, "d").is_empty(), "OFF retires it: {off:?}");
}
