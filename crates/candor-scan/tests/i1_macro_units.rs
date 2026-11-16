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
    for k in ["CANDOR_DEPS", "CANDOR_CONFIG", "CANDOR_POLICY", "CANDOR_I1_SEED_DROP", "CANDOR_I1_BIND_OFF", "CANDOR_I1_RESPAN_OFF", "CANDOR_I1S2_MODUSE_OFF", "CANDOR_I1S2_XCFG_OFF"] {
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
    // OFF arm (stage 2 deleted the allowlist and its switch): the oracle seed leaves `impl_load!` unexpanded.
    let (off, _) = scan(&d, None, &[("CANDOR_I1_SEED_DROP", "impl_load")]);
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
    for (arm, env) in [("on", vec![]), ("off", vec![("CANDOR_I1_SEED_DROP", "impl_load")])] {
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
/// that the module's evidence is gone, and the OFF arm — the invocation left unexpanded — restores the hedge). A PARTIAL
/// expansion — it also emits a nested `macro_rules!` definition, which stays refused until R1784's stage — keeps the
/// invocation and with it the hedge (`via_b2`). Stage 2: an expansion that emits a MODULE is now full, and the module's
/// fn is a unit (`via_c` is charged through it).
#[test]
fn i1_step2_expanded_fact_retires_module_evidence_partial_keeps_it() {
    let d = tmpdir("partial");
    write_crate(
        &d,
        "partial",
        "",
        "pub mod a { macro_rules! full { () => { pub struct Fa; impl Fa { pub fn fa(&self) {} } }; } full!(); }\n\
         pub mod b { macro_rules! part { () => { macro_rules! inner_m { () => {} } pub fn pb() {} }; } part!(); }\n\
         pub mod c { macro_rules! modm { () => { pub mod inner { pub fn hid() { EFF } } pub fn pc() {} }; } modm!(); }\n\
         pub fn via_a() { crate::a::nope() }\npub fn via_b2() { crate::b::hid2() }\npub fn via_c() { crate::c::inner::hid() }\n",
    );
    let (rows, units) = scan(&d, None, &[]);
    assert!(units.contains("a::Fa::fa") && units.contains("b::pb") && units.contains("c::inner::hid"), "{units:?}");
    assert!(!rows.contains_key("via_a"), "module `a` was expanded in full; its R128 evidence is retired: {rows:?}");
    assert_eq!(eff(&rows, "via_b2"), vec!["Unknown".to_string()], "a partial expansion keeps R128's disclosure: {rows:?}");
    assert_eq!(eff(&rows, "via_c"), vec!["Fs".to_string()], "a macro-made module is a module: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1_SEED_DROP", "full")]);
    assert_eq!(eff(&off, "via_a"), vec!["Unknown".to_string()], "OFF: the kept invocation is evidence again");
    // The stage-1 arm: the module is refused, and a call into a module NESTED in the invocation's module names a module R128
    // never marked, so it read ABSENT (R1535's class) — the silence stage 2 closes.
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_MODUSE_OFF", "1")]);
    assert!(!eff(&off, "via_c").contains(&"Fs".to_string()), "MODUSE_OFF: the module is refused again: {off:?}");
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

/// Step 6b (Fable merge review §4) — the hidden-impl surface is read at EVERY position an attribute or derive can add an
/// `impl` from, with `cfg_attr` read recursively. One one-variable crate per position; each shape was BUILT AND EXECUTED in
/// the lane with a real proc macro (`fx/pos`, `pmac2::{GoDerive, add_go}`): the hidden implementor ran and wrote. The first
/// step-6 cut held the hedge for the plain derive and the attributed impl only, and read `d` pure for the rest.
#[test]
fn i1_step6b_every_position_keeps_the_hedge() {
    const POSITIONS: &[(&str, &str)] = &[
        ("plain_derive", "#[derive(other::GoDerive)] pub struct Hid;"),
        ("cfgattr_derive", "#[cfg_attr(feature = \"x\", derive(other::GoDerive))] pub struct Hid;"),
        ("cfgattr_attr", "#[cfg_attr(feature = \"x\", other::add_go(Hid))] pub struct Other1;"),
        ("nested_cfgattr", "#[cfg_attr(unix, cfg_attr(feature = \"x\", derive(other::GoDerive)))] pub struct Hid;"),
        ("attr_impl", "pub struct Other1; #[other::add_go(Hid)] impl Other1 {}"),
        ("attr_extern_block", "#[other::add_go(Hid)] extern \"C\" {}"),
        ("attr_foreign_item", "extern \"C\" { #[other::add_go(Hid)] fn ff(); }"),
        ("attr_mod", "#[other::add_go(Hid)] pub mod hm {}"),
        ("attr_const", "#[other::add_go(Hid)] pub const C: u8 = 0;"),
        ("attr_static", "#[other::add_go(Hid)] pub static ST: u8 = 0;"),
        ("attr_type_alias", "#[other::add_go(Hid)] pub type T = u8;"),
        ("attr_trait", "#[other::add_go(Hid)] pub trait Tx {}"),
        ("attr_fn", "#[other::add_go(Hid)] pub fn f0() {}"),
        ("attr_enum", "#[other::add_go(Hid)] pub enum E0 { A }"),
        ("derive_in_fn_body", "pub fn setup() { #[derive(other::GoDerive)] struct Inner; }"),
        ("derive_in_inline_mod", "pub mod inner { #[derive(other::GoDerive)] pub struct Hid; }"),
        ("cfg_if_body", "cfg_if::cfg_if! { if #[cfg(unix)] { #[derive(other::GoDerive)] pub struct Hid; } }"),
        ("attr_impl_item", "pub struct Other1; impl Other1 { #[other::add_go(Hid)] pub fn x(&self) {} }"),
    ];
    for (name, shape) in POSITIONS {
        let d = tmpdir(&format!("pos-{name}"));
        write_crate(
            &d,
            &format!("pos_{name}"),
            "",
            &format!(
                "pub trait Go {{ fn go(&self); }}\npub struct Pure; impl Go for Pure {{ fn go(&self) {{}} }}\n\
                 macro_rules! mk {{ ($t:ident) => {{ impl Go for $t {{ fn go(&self) {{}} }} }} }}\npub struct M; mk!(M);\n\
                 pub struct Hid;\npub fn d(t: &dyn Go) {{ t.go() }}\n{shape}\n"
            ),
        );
        let (rows, _) = scan(&d, None, &[]);
        assert_eq!(eff(&rows, "d"), vec!["Unknown".to_string()], "{name}: the hedge must stand: {rows:?}");
        let (off, _) = scan(&d, None, &[("CANDOR_I1_HIDDEN_OFF", "1")]);
        assert!(eff(&off, "d").is_empty(), "{name}: OFF retires it (the fixture reaches the code): {off:?}");
    }
}

/// Step 1b (Fable merge review §6k) — a `#[cfg]`-gated stub beside a GLOB `use dep::*` or a `#[macro_use] extern crate dep`
/// (either can bring the macro's name into scope) is not bound either; pre-existing on main.
#[test]
fn i1_step1b_glob_and_macro_use_refuse_a_gated_stub() {
    for (name, import) in [("glob", "use dmlib::*;"), ("macuse", "#[macro_use] extern crate dmlib;")] {
        let d = tmpdir(&format!("bind-{name}"));
        write_crate(
            &d,
            &format!("bind_{name}"),
            "",
            &format!("{import}\n#[cfg(any())]\nmacro_rules! make_bnd {{ () => {{ pub fn bnd_f() {{ EFF }} }}; }}\nmake_bnd!();\npub fn b_call() {{ bnd_f() }}\n"),
        );
        let (_, units) = scan(&d, None, &[]);
        assert!(!units.contains("bnd_f"), "{name}: the compiled-out stub must not be bound: {units:?}");
        let (_, off) = scan(&d, None, &[("CANDOR_I1_BIND_OFF", "1")]);
        assert!(off.contains("bnd_f"), "{name}: OFF binds it: {off:?}");
    }
}

/// Fable's merge re-check RC2 — a bang macro this scan does not expand, at a STATEMENT or EXPRESSION position inside a fn
/// body, can emit the hidden implementor. Executed in the lane (`stmt/`: proc macro `pmfn::make_go!`, dependency
/// `dmlib2::make_go_dep!`): the implementor wrote through `d(&Hid)` while the step-6b binary read `d = []`. Main covered it
/// only through the unexpanded `mk!`. A toolchain macro and a LOCAL `macro_rules!` in the body hold nothing new.
#[test]
fn i1_rc2_a_foreign_bang_macro_in_a_body_keeps_the_hedge() {
    const SHAPES: &[(&str, &str, bool)] = &[
        ("stmt_path", "pub fn setup() { other::make_go!(Hid); }", true),
        ("stmt_dep_path", "pub fn setup() { dmlib::make_go_dep!(Hid); }", true),
        ("expr_tail", "pub fn setup() -> u32 { other::make_go!(Hid) }", true),
        ("expr_let", "pub fn setup() { let _x = other::make_go!(Hid); }", true),
        ("imported_bare", "use other::make_go;\npub fn setup() { make_go!(Hid); }", true),
        ("macro_use_bare", "pub fn setup() { make_go!(Hid); }", true),
        ("closure_body", "pub fn setup() { let f = || { other::make_go!(Hid); }; f() }", true),
        ("method_body", "pub struct S0; impl S0 { pub fn s(&self) { other::make_go!(Hid); } }", true),
        // Fable re-check 2, RC2-1/RC2-2: the text the scan never parses, each shape executed in the lane (`stmt2/`).
        ("local_wrap_in_body", "pub mod re { pub use other::make_go; }\nmacro_rules! wrap { ($t:ident) => { $crate::re::make_go!($t); } }\npub fn setup() { wrap!(Hid); }", true),
        ("macro_use_beside_empty_gated_stub", "#[macro_use] extern crate other;\n#[cfg(any())] macro_rules! make_go { ($t:ident) => {}; }\npub fn setup() { make_go!(Hid); }", true),
        ("macro_use_toolchain_name", "#[macro_use] extern crate other;\npub fn setup() { let _v = vec!(Hid); }", true),
        ("impl_in_toolchain_args", "pub fn setup() { println!(\"{}\", { impl Go for Hid { fn go(&self) {} } 1 }); }", true),
        ("missing_module_file", "mod gen;", true),
        ("unspliced_include", "include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\"));", true),
        ("toolchain_imports", "use core::ptr;\nuse core::arch::asm;\npub fn setup() { let x = 0u8; let _p = ptr::addr_of!(x); }\npub fn n() { unsafe { asm!(\"nop\") } }", false),
        ("builtin_attr_in_args", "thread_local! { #[allow(unused)] static X: u8 = 0; }\npub fn setup() { let _v = vec![1]; }", false),
        ("toolchain_only", "pub fn setup() { println!(\"x\"); let _v = vec![1]; assert!(true); std::format!(\"y\"); }", false),
        ("local_macro_only", "macro_rules! twice { ($e:expr) => { $e + $e } }\npub fn setup() -> u32 { twice!(1) }", false),
    ];
    for (name, shape, holds) in SHAPES {
        let d = tmpdir(&format!("rc2-{name}"));
        write_crate(
            &d,
            &format!("rc2_{name}"),
            "",
            &format!(
                "pub trait Go {{ fn go(&self); }}\npub struct Pure; impl Go for Pure {{ fn go(&self) {{}} }}\n\
                 macro_rules! mk {{ ($t:ident) => {{ impl Go for $t {{ fn go(&self) {{}} }} }} }}\npub struct M; mk!(M);\n\
                 pub struct Hid;\npub fn d(t: &dyn Go) {{ t.go() }}\n{shape}\n"
            ),
        );
        let (rows, _) = scan(&d, None, &[]);
        if *holds {
            assert_eq!(eff(&rows, "d"), vec!["Unknown".to_string()], "{name}: the hedge must stand: {rows:?}");
            let (off, _) = scan(&d, None, &[("CANDOR_I1_HIDDEN_OFF", "1")]);
            assert!(eff(&off, "d").is_empty(), "{name}: OFF retires it (the fixture reaches the code): {off:?}");
        } else {
            assert!(eff(&rows, "d").is_empty(), "{name}: nothing foreign, so `mk!` retires as before: {rows:?}");
        }
    }
}

/// Fable re-check 2 — a local macro invoked only inside another local macro's body that is itself invoked unexpanded (in a fn
/// body) is not fully expanded: `mk3!`'s macro-implementor hedge stands although `mk3!(M3)` also expands at item position.
#[test]
fn i1_rc2_a_local_macro_reached_through_an_unexpanded_body_keeps_its_hedge() {
    let d = tmpdir("rc2-via-body");
    write_crate(
        &d,
        "rc2_via_body",
        "",
        "pub trait Go { fn go(&self); }\npub struct Pure; impl Go for Pure { fn go(&self) {} }\n\
         macro_rules! mk3 { ($t:ident) => { impl Go for $t { fn go(&self) {} } } }\npub struct M3; mk3!(M3);\n\
         macro_rules! wrap2 { ($t:ident) => { mk3!($t); } }\npub struct Hid;\npub fn setup() { wrap2!(Hid); }\n\
         pub fn d(t: &dyn Go) { t.go() }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    assert_eq!(eff(&rows, "d"), vec!["Unknown".to_string()], "mk3's hedge must stand: {rows:?}");
    // The control: without the body invocation, mk3 is fully expanded and retires.
    let c = tmpdir("rc2-via-body-ctl");
    write_crate(
        &c,
        "rc2_via_body_ctl",
        "",
        "pub trait Go { fn go(&self); }\npub struct Pure; impl Go for Pure { fn go(&self) {} }\n\
         macro_rules! mk3 { ($t:ident) => { impl Go for $t { fn go(&self) {} } } }\npub struct M3; mk3!(M3);\n\
         pub struct Hid;\npub fn d(t: &dyn Go) { t.go() }\n",
    );
    let (rows, _) = scan(&c, None, &[]);
    assert!(eff(&rows, "d").is_empty(), "control: mk3 fully expanded retires: {rows:?}");
}

/// Fable re-check 2 (the lane's own sweep of unparsed-text positions) — the control for `missing_module_file`: the same
/// `mod gen;` with its file present is parsed text, so `mk!` retires as before.
#[test]
fn i1_rc2_a_present_module_file_holds_nothing() {
    let d = tmpdir("rc2-modfile");
    write_crate(
        &d,
        "rc2_modfile",
        "",
        "pub trait Go { fn go(&self); }\npub struct Pure; impl Go for Pure { fn go(&self) {} }\n\
         macro_rules! mk { ($t:ident) => { impl Go for $t { fn go(&self) {} } } }\npub struct M; mk!(M);\n\
         pub struct Hid;\npub fn d(t: &dyn Go) { t.go() }\nmod gen;\n",
    );
    std::fs::write(d.join("src/gen.rs"), "pub fn g() {}\n").unwrap();
    let (rows, _) = scan(&d, None, &[]);
    assert!(eff(&rows, "d").is_empty(), "the module file is read, nothing hidden: {rows:?}");
}

/// Fable re-check 3, RC3-2 — a dependency glob's names reach a child module through `use super::*`, and a `#[cfg]`-gated
/// local stub of the same name does not outrank them. Executed in the lane (`stmt2/x_super_glob_stub`): rustc binds the
/// dependency's `make_go_dep!`, whose impl wrote; 4b52cf1c read `d = []`. The no-stub control held before and still does.
#[test]
fn i1_rc3_a_parent_glob_reaches_a_child_beside_a_gated_stub() {
    for (name, stub) in [("stub", "#[cfg(any())] macro_rules! make_go_dep { ($t:ident) => {}; }\n"), ("nostub", "")] {
        let d = tmpdir(&format!("rc3-{name}"));
        write_crate(
            &d,
            &format!("rc3_{name}"),
            "",
            &format!(
                "use dmlib::*;\npub trait Go {{ fn go(&self); }}\npub struct Pure; impl Go for Pure {{ fn go(&self) {{}} }}\n\
                 macro_rules! mk {{ ($t:ident) => {{ impl Go for $t {{ fn go(&self) {{}} }} }} }}\npub struct M; mk!(M);\n\
                 pub struct Hid;\npub fn d(t: &dyn Go) {{ t.go() }}\n{stub}mod inner;\npub use inner::setup;\n"
            ),
        );
        std::fs::write(d.join("src/inner.rs"), "use super::*;\npub fn setup() { make_go_dep!(Hid); }\n").unwrap();
        let (rows, _) = scan(&d, None, &[]);
        assert_eq!(eff(&rows, "d"), vec!["Unknown".to_string()], "{name}: the hedge must stand: {rows:?}");
        let (off, _) = scan(&d, None, &[("CANDOR_I1_HIDDEN_OFF", "1")]);
        assert!(eff(&off, "d").is_empty(), "{name}: OFF retires it: {off:?}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// I1 STAGE 2 — modules, `use`s and `extern crate`s an expansion yields are spliced (the allowlist is gone), the cross-file
// cfg refusal is lifted, and a cfg-gated fn whose twin stayed in unexpanded macro text keeps a hedge.
// ---------------------------------------------------------------------------------------------------------------------

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body.replace("EFF", W)).unwrap();
}

/// R1535 (a macro-made module named like a type that implements a local trait), R1688 (a `use` arm a macro makes, cfg
/// paired), a glob re-export a macro makes, and an aliased std module: each read ABSENT or `[]` over a write on published
/// 0.42.0 and main. Pure twins have unique names and stay pure; the MODUSE_OFF arm (the stage-1 refusal list) re-silences.
#[test]
fn i1s2_macro_made_modules_and_uses_are_spliced() {
    let d = tmpdir("s2mod");
    write_crate(
        &d,
        "s2mod",
        "",
        "pub trait Tr { fn act(&self); fn go(&self) { self.act() } }\nimpl Tr for bool { fn act(&self) {} }\n\
         macro_rules! mk { ($n:ident) => { pub mod $n { pub fn go(_b: &$n) { EFF } } } }\nmk!(bool);\n\
         pub fn r1535_e() { bool::go(&true) }\n\
         pub mod tu { pub fn f() { EFF } }\npub mod tw { pub fn f() {} }\npub mod tq { pub fn g() {} }\n\
         macro_rules! sys { ($m:ident) => { use crate::$m as sys; } }\n#[cfg(unix)] sys!(tu);\n#[cfg(windows)] sys!(tw);\n\
         macro_rules! sysq { ($m:ident) => { use crate::$m as sysq_alias; } }\nsysq!(tq);\n\
         pub fn usearm_e() { sys::f() }\npub fn usearmq_x() { sysq_alias::g() }\n\
         pub mod impl1 { pub fn w2() { EFF } }\nmacro_rules! reexp { ($m:ident) => { pub use crate::$m::*; } }\n\
         pub mod facade { reexp!(impl1); }\npub fn glob_e() { facade::w2() }\n\
         macro_rules! alias_fs { () => { use std::fs as ffm; } }\nalias_fs!();\n\
         pub fn usestd_e() { let _ = ffm::write(\"/tmp/i1-never-run\", b\"x\"); }\n",
    );
    let (rows, units) = scan(&d, None, &[]);
    for f in ["r1535_e", "usearm_e", "glob_e", "usestd_e"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f} must be charged through the macro-made item: {rows:?}");
    }
    assert!(eff(&rows, "usearmq_x").is_empty() && units.contains("usearmq_x"), "the pure twin stays pure: {rows:?}");
    assert!(units.contains("bool::go") && units.contains("tq::g"), "{units:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_MODUSE_OFF", "1")]);
    for f in ["r1535_e", "usearm_e", "usestd_e"] {
        assert!(!eff(&off, f).contains(&"Fs".to_string()), "MODUSE_OFF: {f} is the stage-1 silence again: {off:?}");
    }
}

/// A configuration macro defined in another file (tokio's `cfg_rt!` shape) that stamps `#[cfg]` on what it wraps is expanded
/// by default since the stage 2b flip: with `use`s spliced the twin arm (`cfg_unix! { pub use crate::real::leaf2; }`) stands
/// beside the stub, so the union is whole and the call is charged. Under the opt-out `CANDOR_I1S2_XCFG_OFF` it is refused
/// and the call discloses (R128), as it did by default through stage 2a.
#[test]
fn i1s2_cross_file_cfg_expansion_is_spliced() {
    let d = tmpdir("s2xcfg");
    write_crate(
        &d,
        "s2xcfg",
        "",
        "#[macro_use]\nmod macros;\npub mod a;\npub mod b;\npub mod real { pub fn leaf2() { EFF } }\n\
         pub fn xcfg_e() { a::leaf() }\npub fn xcfguse_e() { b::leaf2() }\npub fn xcfgq_x() { a::leafq() }\n",
    );
    write_file(
        &d,
        "src/macros.rs",
        "macro_rules! cfg_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! cfg_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n",
    );
    write_file(
        &d,
        "src/a.rs",
        "cfg_unix! { pub fn leaf() { EFF } }\ncfg_not_unix! { pub fn leaf() {} }\n\
         cfg_unix! { pub fn leafq() {} }\ncfg_not_unix! { pub fn leafq() {} }\n",
    );
    write_file(&d, "src/b.rs", "cfg_unix! { pub use crate::real::leaf2; }\ncfg_not_unix! { pub fn leaf2() {} }\n");
    let (rows, units) = scan(&d, None, &[]);
    for f in ["xcfg_e", "xcfguse_e"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "default (lift on): {f}: {rows:?}");
    }
    assert!(eff(&rows, "xcfgq_x").is_empty() && units.contains("a::leafq"), "the pure twin stays pure: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["xcfg_e", "xcfguse_e"] {
        assert_eq!(eff(&off, f), vec!["Unknown".to_string()], "XCFG_OFF: {f} is R128's disclosure: {off:?}");
    }
}

/// The exposure the lift made, and its cover. A cfg twin whose wrapper this pass will not expand (defined twice, here
/// with different text) stays as unexpanded text while the other arm's stub is spliced, so the call RESOLVES to the stub.
/// Executed (fixture `s2tw`): every effectful probe wrote its file; `deny Unknown` was 1 on published 0.42.0 and main for
/// the cross-file shapes and 0 on the lift alone. The visible stub now carries a hedge with a true reason: free fn
/// (cross-file and same-file), a `use` arm, an impl-item arm and a whole hidden `impl` arm. Must NOT hold: a once-defined
/// pair (both arms spliced, `twctl_e` charged), and an UNGATED fn whose name the unexpanded text also writes.
#[test]
fn i1s2_a_hidden_cfg_twin_keeps_a_hedge() {
    let d = tmpdir("s2tw");
    write_crate(
        &d,
        "s2tw",
        "[features]\nx = []\n",
        "#[macro_use]\nmod macros;\npub mod a;\npub mod b;\npub mod d;\npub mod e;\npub mod f;\n\
         pub trait Go { fn go(&self); }\npub struct PureGo;\nimpl Go for PureGo { fn go(&self) {} }\n\
         pub fn tw_e() { a::leaf() }\npub fn twctl_e() { b::leafb() }\npub fn twii_e(s: &d::S) { s.m() }\n\
         pub fn twuse_e() { e::leafe() }\npub fn twty_e(s: &f::S2) { s.go() }\npub fn ungated_x() { a::plain() }\n\
         pub mod c {\n\
         #[cfg(feature = \"x\")]\nmacro_rules! s_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! s_unix { ($($it:item)*) => { $( #[cfg(unix)] $it )* } }\n\
         macro_rules! s_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         s_unix! { pub fn leafc() { EFF } }\ns_not_unix! { pub fn leafc() {} }\n}\npub fn twsame_e() { c::leafc() }\n",
    );
    write_file(
        &d,
        "src/macros.rs",
        "#[cfg(feature = \"x\")]\nmacro_rules! cfg_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! cfg_unix { ($($it:item)*) => { $( #[cfg(unix)] $it )* } }\n\
         macro_rules! cfg_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         macro_rules! on_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! off_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         #[cfg(feature = \"x\")]\nmacro_rules! ii_unix { ($($t:tt)*) => { #[cfg(unix)] $($t)* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! ii_unix { ($($tt:tt)*) => { #[cfg(unix)] $($tt)* } }\n\
         macro_rules! ii_not_unix { ($($t:tt)*) => { #[cfg(not(unix))] $($t)* } }\n",
    );
    write_file(
        &d,
        "src/a.rs",
        "cfg_unix! { pub fn leaf() { EFF } pub fn plain() { EFF } }\ncfg_not_unix! { pub fn leaf() {} }\npub fn plain() {}\n",
    );
    write_file(&d, "src/b.rs", "on_unix! { pub fn leafb() { EFF } }\noff_unix! { pub fn leafb() {} }\n");
    write_file(
        &d,
        "src/d.rs",
        "pub struct S;\nimpl S {\n    ii_unix! { pub fn m(&self) { EFF } }\n    ii_not_unix! { pub fn m(&self) {} }\n}\n",
    );
    write_file(
        &d,
        "src/e.rs",
        "pub mod real { pub fn leafe() { EFF } }\ncfg_unix! { pub use self::real::leafe; }\ncfg_not_unix! { pub fn leafe() {} }\n",
    );
    write_file(
        &d,
        "src/f.rs",
        "pub struct S2;\ncfg_unix! { impl crate::Go for S2 { fn go(&self) { EFF } } }\n\
         cfg_not_unix! { impl crate::Go for S2 { fn go(&self) {} } }\n",
    );
    // Since the stage 2b flip the cross-file cfg expansion is LIFTED by default and the cover below is the default reading;
    // under the opt-out `CANDOR_I1S2_XCFG_OFF` it is refused and the call discloses (R128) as on main — the teeth.
    let (def, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["tw_e", "twsame_e", "twuse_e", "twty_e", "twctl_e"] {
        assert!(eff(&def, f).contains(&"Unknown".to_string()), "XCFG_OFF: {f} discloses (R128): {def:?}");
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["tw_e", "twsame_e", "twii_e", "twuse_e", "twty_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the hidden arm: {rows:?}");
    }
    assert_eq!(eff(&rows, "twctl_e"), vec!["Fs".to_string()], "a once-defined pair is spliced whole and charged: {rows:?}");
    assert!(!eff(&rows, "ungated_x").contains(&"Unknown".to_string()), "an ungated fn is the only one: {rows:?}");
    assert_eq!(rows["a::leaf"].1, vec!["macro:another cfg arm of this fn is hidden by an unexpanded macro".to_string()]);
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_TWIN_OFF", "1")]);
    for f in ["tw_e", "twsame_e", "twii_e", "twuse_e", "twty_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "TWIN_OFF: {f} resolves to the stub alone: {off:?}");
    }
}

/// Stage 2 exposed a pre-existing resolver hole: `use ::dep::prelude::*;` (a LEADING `::`, the extern crate) beside a module
/// the crate also names `dep` resolved the glob against the LOCAL module, so a bare call through it named nothing and lost
/// `invisible:[dep]`. time-0.3.55's benchmarks declare `mod rand09;` inside `mods![..]`, which stage 2 splices, and 129 rows
/// lost `invisible:[rand09]` that way. The hand-written shape below read ABSENT on main. LCGLOB_OFF restores that reading.
#[test]
fn i1s2_a_leading_colon_glob_names_the_extern_crate() {
    let root = tmpdir("s2lc");
    let dep = root.join("dep1");
    let app = root.join("app");
    write_crate(&dep, "dep1", "", "pub mod prelude { pub fn g() { EFF } }\n");
    write_crate(
        &app,
        "app",
        "[dependencies]\ndep1 = { path = \"../dep1\" }\n",
        "mod dep1;\npub fn lc_glob() { use ::dep1::prelude::*; g() }\n",
    );
    write_file(&app, "src/dep1.rs", "pub fn local() {}\n");
    let inv = |dir: &Path, env: &[(&str, &str)]| -> Vec<String> {
        let out = dir.join("out2");
        let _ = std::fs::remove_dir_all(&out);
        let mut cmd = Command::new(bin());
        cmd.arg(dir.to_string_lossy().as_ref()).arg("--out").arg(out.join("r")).env_remove("CANDOR_DEPS");
        for (k, v) in env {
            cmd.env(k, v);
        }
        assert!(cmd.output().unwrap().status.code().is_some_and(|c| c == 0 || c == 1));
        for e in std::fs::read_dir(&out).unwrap() {
            let p = e.unwrap().path();
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            if n.ends_with(".scan.json") {
                let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                for f in v["functions"].as_array().unwrap() {
                    if f["fn"] == "lc_glob" {
                        return f["invisible"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    }
                }
            }
        }
        Vec::new()
    };
    assert_eq!(inv(&app, &[]), vec!["dep1".to_string()], "the glob names the extern crate");
    assert!(inv(&app, &[("CANDOR_I1S2_LCGLOB_OFF", "1")]).is_empty(), "LCGLOB_OFF: the local module answers, nothing named");
}

/// Fable's stage-2 review, item 1: the twin cover reached fns and methods only. A spliced `use` stub beside a hidden arm of
/// the same name (a `use` or a `fn`) resolved to the stub, and a spliced module beside a hidden module twin was read alone.
/// Executed (fixture `fxm`): `useuse_e`, `fnuse_e`, `useother_e` read `deny Unknown` 1 on 0.42.0 and main, 0 on the first
/// stage-2 cut; `modtw_e` was silent on every arm. Must NOT hold: a once-defined pair of module arms (`modctl_e`) is charged.
#[test]
fn i1s2_a_hidden_use_or_module_twin_keeps_a_hedge() {
    let d = tmpdir("s2twu");
    write_crate(
        &d,
        "s2twu",
        "[features]\nx = []\n",
        "#[macro_use]\nmod macros;\npub mod m1;\npub mod m1c;\npub mod m3;\npub mod m4;\npub mod m5;\n\
         pub fn modtw_e() { m1::sys::f() }\npub fn modctl_e() { m1c::sysc::fc() }\npub fn useuse_e() { m3::leaf3() }\n\
         pub fn fnuse_e() { m4::leaf4() }\npub fn useother_e() { m5::via5() }\n",
    );
    write_file(
        &d,
        "src/macros.rs",
        "#[cfg(feature = \"x\")]\nmacro_rules! cfg_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! cfg_unix { ($($it:item)*) => { $( #[cfg(unix)] $it )* } }\n\
         macro_rules! cfg_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         macro_rules! on_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! off_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n",
    );
    write_file(&d, "src/m1.rs", "cfg_unix! { pub mod sys { pub fn f() { EFF } } }\ncfg_not_unix! { pub mod sys { pub fn f() {} } }\n");
    write_file(&d, "src/m1c.rs", "on_unix! { pub mod sysc { pub fn fc() { EFF } } }\noff_unix! { pub mod sysc { pub fn fc() {} } }\n");
    write_file(
        &d,
        "src/m3.rs",
        "pub mod real { pub fn leaf3() { EFF } }\npub mod stub { pub fn leaf3() {} }\n\
         cfg_unix! { pub use self::real::leaf3; }\ncfg_not_unix! { pub use self::stub::leaf3; }\n",
    );
    write_file(
        &d,
        "src/m4.rs",
        "pub mod stub4 { pub fn leaf4() {} }\ncfg_unix! { pub fn leaf4() { EFF } }\ncfg_not_unix! { pub use self::stub4::leaf4; }\n\
         pub mod real5 { pub fn leaf5() { EFF } }\npub mod stub5 { pub fn leaf5() {} }\n\
         cfg_unix! { pub use self::real5::leaf5; }\ncfg_not_unix! { pub use self::stub5::leaf5; }\n",
    );
    write_file(&d, "src/m5.rs", "use crate::m4::leaf5;\npub fn via5() { leaf5() }\n");
    // Since the stage 2b flip the cross-file cfg expansion is LIFTED by default and the cover below is the default reading;
    // under the opt-out `CANDOR_I1S2_XCFG_OFF` it is refused and the call discloses (R128) as on main — the teeth.
    let (def, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["useuse_e", "fnuse_e", "useother_e"] {
        assert!(eff(&def, f).contains(&"Unknown".to_string()), "XCFG_OFF: {f} discloses (R128): {def:?}");
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["modtw_e", "useuse_e", "fnuse_e", "useother_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the hidden arm: {rows:?}");
    }
    assert_eq!(eff(&rows, "modctl_e"), vec!["Fs".to_string()], "a once-defined module pair is charged: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_TWIN_OFF", "1")]);
    for f in ["modtw_e", "useuse_e", "fnuse_e", "useother_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "TWIN_OFF: {f} resolves to the stub alone: {off:?}");
    }
}

/// Fable's stage-2 re-check 1: the twin cover was keyed on the NAMES the hidden text writes, and missed every arm that binds a
/// name it does not write: a glob arm (visible or hidden), a name handed to a nested macro, and a hidden fn SHADOWING a
/// complete glob pair (an item beats a glob import). Executed (fixture `fxn` and this lane's `globshadow_e`): each read
/// `deny Unknown` 1 on 0.42.0 and main and 0 on the name-keyed cover. Must NOT hold: once-defined pairs with no hidden text
/// in their module stay charged, named and glob.
#[test]
fn i1s2_glob_and_nested_name_twins_keep_a_hedge() {
    let d = tmpdir("s2twg");
    write_crate(
        &d,
        "s2twg",
        "[features]\nx = []\n",
        "#[macro_use]\nmod macros;\npub mod mc;\npub mod md;\npub mod mk;\npub mod mh;\npub mod mq;\npub mod mctl;\npub mod mg2;\n\
         pub fn glob_e() { mc::leaf_g() }\npub fn globuse_e() { md::leaf_g2() }\npub fn hidglob_e() { mk::leaf_h() }\n\
         pub fn deepname_e() { mh::leaf_dn() }\npub fn globshadow_e() { mq::leaf_q() }\n\
         pub fn usectl_x() { mctl::leaf_ctl() }\npub fn globctl2_x() { mg2::leaf_gc2() }\n",
    );
    write_file(
        &d,
        "src/macros.rs",
        "#[cfg(feature = \"x\")]\nmacro_rules! cfg_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! cfg_unix { ($($it:item)*) => { $( #[cfg(unix)] $it )* } }\n\
         macro_rules! cfg_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         macro_rules! on_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! off_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         macro_rules! mk_w { ($n:ident) => { pub fn $n() { EFF } } }\nmacro_rules! mk_s { ($n:ident) => { pub fn $n() {} } }\n",
    );
    write_file(&d, "src/mc.rs", "pub mod stubg { pub fn leaf_g() {} }\ncfg_unix! { pub fn leaf_g() { EFF } }\ncfg_not_unix! { pub use self::stubg::*; }\n");
    write_file(
        &d,
        "src/md.rs",
        "pub mod realg2 { pub fn leaf_g2() { EFF } }\npub mod stubg2 { pub fn leaf_g2() {} }\n\
         cfg_unix! { pub use self::realg2::*; }\ncfg_not_unix! { pub use self::stubg2::*; }\n",
    );
    write_file(&d, "src/mk.rs", "pub mod realh { pub fn leaf_h() { EFF } }\ncfg_unix! { pub use self::realh::*; }\ncfg_not_unix! { pub fn leaf_h() {} }\n");
    write_file(&d, "src/mh.rs", "cfg_unix! { mk_w!(leaf_dn); }\ncfg_not_unix! { mk_s!(leaf_dn); }\n");
    write_file(
        &d,
        "src/mq.rs",
        "pub mod realq { pub fn leaf_q() {} }\npub mod stubq { pub fn leaf_q() {} }\n\
         on_unix! { pub use self::realq::*; }\noff_unix! { pub use self::stubq::*; }\ncfg_unix! { pub fn leaf_q() { EFF } }\n",
    );
    write_file(
        &d,
        "src/mctl.rs",
        "pub mod real { pub fn leaf_ctl() { EFF } }\npub mod stub { pub fn leaf_ctl() {} }\n\
         on_unix! { pub use self::real::leaf_ctl; }\noff_unix! { pub use self::stub::leaf_ctl; }\n",
    );
    write_file(
        &d,
        "src/mg2.rs",
        "pub mod realgc2 { pub fn leaf_gc2() { EFF } }\npub mod stubgc2 { pub fn leaf_gc2() {} }\n\
         on_unix! { pub use self::realgc2::*; }\noff_unix! { pub use self::stubgc2::*; }\n",
    );
    // Since the stage 2b flip the cross-file cfg expansion is LIFTED by default and the cover below is the default reading;
    // under the opt-out `CANDOR_I1S2_XCFG_OFF` it is refused and the call discloses (R128) as on main — the teeth.
    let (def, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["glob_e", "globuse_e", "hidglob_e", "deepname_e", "globshadow_e", "usectl_x", "globctl2_x"] {
        assert!(eff(&def, f).contains(&"Unknown".to_string()), "XCFG_OFF: {f} discloses (R128): {def:?}");
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["glob_e", "globuse_e", "hidglob_e", "deepname_e", "globshadow_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the hidden arm: {rows:?}");
    }
    for f in ["usectl_x", "globctl2_x"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f}: a once-defined pair with no hidden text is charged: {rows:?}");
    }
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_TWIN_OFF", "1")]);
    for f in ["glob_e", "globuse_e", "hidglob_e", "deepname_e", "globshadow_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "TWIN_OFF: {f} resolves to one arm: {off:?}");
    }
}

/// Fable's stage-2 re-check 2: the hidden side was read by a hand token scanner that disagreed with `syn` about `use` groups,
/// toolchain globs, traits and item-level impls. Both sides are now read by ONE reader (`item_keys` over `syn`-parsed
/// expansions). Executed (fixture `fxo`): each shape below read `deny Unknown` 1 on 0.42.0 and main and 0 on the cut before
/// this one. Must NOT hold: a hidden `std::fs` glob beside a produced arm whose name `std::fs` does not export, a glob pair
/// beside hidden text whose names its target does not hold, and a hand-written gated pair beside forwarding impls whose
/// expansion is long.
#[test]
fn i1s2_one_reader_covers_groups_std_globs_traits_and_item_impls() {
    let d = tmpdir("s2one");
    write_crate(
        &d,
        "s2one",
        "[features]\nx = []\n",
        "#[macro_use]\nmod macros;\npub mod m_hg;\npub mod m_gg;\npub mod m_sg;\npub mod m_sf;\npub mod m_td;\npub mod m_im;\n\
         pub mod m_sgc;\npub mod m_gn;\npub mod m_bu;\nuse m_td::TrD;\n\
         pub fn hidgroup_e() { m_hg::leaf_hg() }\npub fn hidgroupglob_e() { m_gg::leaf_gg() }\n\
         pub fn stdglob_e() { let _ = m_sg::write(\"/tmp/i1-never-run\", b\"x\"); }\n\
         pub fn stdglobty_e() { let _ = m_sf::File::create(\"/tmp/i1-never-run\"); }\n\
         pub fn traitdef_e() { m_td::SD.go() }\npub fn implmix_e() { m_im::SM::go() }\n\
         pub fn stdglobctl_x() { m_sgc::leaf_sgc() }\npub fn globnamedctl_x() { m_gn::leaf_gn() }\n\
         pub fn budgetctl_x() { m_bu::SB.go() }\n",
    );
    write_file(
        &d,
        "src/macros.rs",
        "#[cfg(feature = \"x\")]\nmacro_rules! cfg_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! cfg_unix { ($($it:item)*) => { $( #[cfg(unix)] $it )* } }\n\
         macro_rules! cfg_not_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         macro_rules! on_unix { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
         macro_rules! off_unix { ($($i:item)*) => { $( #[cfg(not(unix))] $i )* } }\n\
         #[cfg(feature = \"x\")]\nmacro_rules! fwd { ($($f:ident)*) => { $( fwd_h!{$f} )* } }\n\
         #[cfg(not(feature = \"x\"))]\nmacro_rules! fwd { ($($g:ident)*) => { $( fwd_h!{$g} )* } }\n\
         macro_rules! fwd_h { ($f:ident) => { fwd_m!{$f} } }\nmacro_rules! fwd_m { ($f:ident) => { pub fn $f(&self) {} } }\n",
    );
    write_file(
        &d,
        "src/m_hg.rs",
        "pub mod real_hg { pub fn leaf_hg() { EFF } pub fn other_hg() {} }\n\
         cfg_unix! { pub use self::real_hg::{leaf_hg, other_hg}; }\ncfg_not_unix! { pub fn leaf_hg() {} pub fn other_hg() {} }\n",
    );
    write_file(&d, "src/m_gg.rs", "pub mod real_gg { pub fn leaf_gg() { EFF } }\ncfg_unix! { pub use self::{real_gg::*}; }\ncfg_not_unix! { pub fn leaf_gg() {} }\n");
    write_file(
        &d,
        "src/m_sg.rs",
        "cfg_unix! { pub use std::fs::*; }\n\
         cfg_not_unix! { pub fn write<P: AsRef<std::path::Path>, C: AsRef<[u8]>>(_p: P, _c: C) -> std::io::Result<()> { Ok(()) } }\n",
    );
    write_file(
        &d,
        "src/m_sf.rs",
        "cfg_unix! { pub use std::fs::*; }\n\
         cfg_not_unix! { pub struct File; impl File { pub fn create<P: AsRef<std::path::Path>>(_p: P) -> std::io::Result<File> { Ok(File) } } }\n",
    );
    write_file(
        &d,
        "src/m_td.rs",
        "cfg_unix! { pub trait TrD { fn go(&self) { EFF } } }\ncfg_not_unix! { pub trait TrD { fn go(&self) {} } }\npub struct SD;\nimpl TrD for SD {}\n",
    );
    write_file(&d, "src/m_im.rs", "pub struct SM;\ncfg_unix! { impl SM { pub fn go() { EFF } } }\nimpl SM { cfg_not_unix! { pub fn go() {} } }\n");
    write_file(&d, "src/m_sgc.rs", "cfg_unix! { pub use std::fs::*; }\non_unix! { pub fn leaf_sgc() { EFF } }\n");
    write_file(
        &d,
        "src/m_gn.rs",
        "pub mod real_gn { pub fn leaf_gn() { EFF } }\npub mod stub_gn { pub fn leaf_gn() {} }\n\
         on_unix! { pub use self::real_gn::*; }\noff_unix! { pub use self::stub_gn::*; }\ncfg_unix! { pub fn unrelated_gn() {} }\n",
    );
    write_file(
        &d,
        "src/m_bu.rs",
        "pub struct SB;\nimpl SB {\n    #[cfg(unix)]\n    pub fn go(&self) { EFF }\n    #[cfg(not(unix))]\n    pub fn go(&self) {}\n}\n\
         pub struct SB1;\nimpl SB1 { fwd!{ a1 a2 a3 a4 a5 a6 a7 a8 } }\npub struct SB2;\nimpl SB2 { fwd!{ a1 a2 a3 a4 a5 a6 a7 a8 } }\n",
    );
    // Since the stage 2b flip the cross-file cfg expansion is LIFTED by default and the cover below is the default reading;
    // under the opt-out `CANDOR_I1S2_XCFG_OFF` it is refused and the call discloses (R128) as on main — the teeth.
    let (def, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["hidgroup_e", "hidgroupglob_e", "stdglob_e", "stdglobty_e", "traitdef_e", "implmix_e", "stdglobctl_x", "globnamedctl_x"] {
        assert!(eff(&def, f).contains(&"Unknown".to_string()), "XCFG_OFF: {f} discloses (R128): {def:?}");
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["hidgroup_e", "hidgroupglob_e", "stdglob_e", "stdglobty_e", "traitdef_e", "implmix_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the hidden arm: {rows:?}");
    }
    for f in ["stdglobctl_x", "globnamedctl_x", "budgetctl_x"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f}: must not hold: {rows:?}");
    }
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_TWIN_OFF", "1")]);
    for f in ["hidgroup_e", "hidgroupglob_e", "stdglob_e", "stdglobty_e", "traitdef_e", "implmix_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "TWIN_OFF: {f} resolves to one arm: {off:?}");
    }
}

/// UPDATE 4a-2: stage 1 (I1 step 2, `97c23112`) splices macro-made TRAIT impls, and with that a cfg-gated trait impl whose twin
/// stays as unexpanded text in the same file resolved to its own pure member, or through it to the trait's pure default. Each
/// shape read `deny Unknown` 1 on 0.42.0 and 0 on main (fixture `fxq`, executed): the twin is the hidden TRAIT's default (`a`),
/// a hidden empty impl inheriting a visible default (`a2`), and a hidden impl overriding a default the visible arm inherits
/// (`h`, where the cover stands a marked member). Must NOT hold: hidden text naming an unrelated trait and type, and a hidden
/// impl of the same trait for ANOTHER type.
#[test]
fn i1s2_a_hidden_trait_arm_keeps_a_hedge() {
    let d = tmpdir("s2ti");
    let hdr = |s: &str| {
        format!(
            "#[cfg(feature = \"x\")]\nmacro_rules! hid_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n\
             #[cfg(not(feature = \"x\"))]\nmacro_rules! hid_{s} {{ ($($it:item)*) => {{ $( #[cfg(unix)] $it )* }} }}\n\
             macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n\
             macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n"
        )
    };
    write_crate(
        &d,
        "s2ti",
        "[features]\nx = []\n",
        "pub mod a;\npub mod a2;\npub mod h;\npub mod x1;\npub mod x2;\n\
         pub fn ta_e() { use crate::a::TrA; crate::a::SA.go() }\n\
         pub fn ta2_e() { use crate::a2::TrA2; crate::a2::SA2.go() }\n\
         pub fn th_e() { use crate::h::TrH; crate::h::SH.go() }\n\
         pub fn tx1_x() { use crate::x1::TrX1; crate::x1::SX1.go() }\n\
         pub fn tx2_x() { use crate::x2::TrX2; crate::x2::SX2.go() }\n",
    );
    write_file(
        &d,
        "src/a.rs",
        &(hdr("a")
            + "hid_a! { pub trait TrA { fn go(&self) { EFF } } }\nvis_a! { pub trait TrA { fn go(&self); } }\npub struct SA;\n\
               on_a! { impl TrA for SA {} }\nvis_a! { impl TrA for SA { fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/a2.rs",
        &(hdr("a2")
            + "#[cfg(unix)] pub trait TrA2 { fn go(&self) { EFF } }\n#[cfg(not(unix))] pub trait TrA2 { fn go(&self); }\n\
               pub struct SA2;\nhid_a2! { impl TrA2 for SA2 {} }\nvis_a2! { impl TrA2 for SA2 { fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/h.rs",
        &(hdr("h")
            + "pub trait TrH { fn go(&self) {} }\npub struct SH;\nhid_h! { impl TrH for SH { fn go(&self) { EFF } } }\n\
               vis_h! { impl TrH for SH {} }\n"),
    );
    write_file(
        &d,
        "src/x1.rs",
        &(hdr("x1")
            + "pub trait TrX1 { fn go(&self); }\npub struct SX1;\npub struct SZ1;\n\
               hid_x1! { pub trait TrZ1 { fn go(&self) {} } impl TrZ1 for SZ1 { fn go(&self) {} } }\n\
               on_x1! { impl TrX1 for SX1 { fn go(&self) { EFF } } }\nvis_x1! { impl TrX1 for SX1 { fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/x2.rs",
        &(hdr("x2")
            + "pub trait TrX2 { fn go(&self) { EFF } }\npub struct SX2;\npub struct OX2;\n\
               hid_x2! { impl TrX2 for OX2 { fn go(&self) {} } }\non_x2! { impl TrX2 for SX2 {} }\nvis_x2! { impl TrX2 for SX2 {} }\n"),
    );
    let (rows, units) = scan(&d, None, &[]);
    for f in ["ta_e", "ta2_e", "th_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the hidden trait arm: {rows:?}");
    }
    assert!(units.contains("h::SH::go"), "the hidden override's member is stood on the visible arm: {units:?}");
    assert_eq!(rows["h::SH::go"].1, vec!["macro:another cfg arm of this fn is hidden by an unexpanded macro".to_string()]);
    for f in ["tx1_x", "tx2_x"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f}: must not hold: {rows:?}");
    }
    // Stage 2c's crate-wide check also reaches these same-module twins, so the arm is shown alone with both off.
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_TRAITARM_OFF", "1"), ("CANDOR_I1S2C_XMOD_OFF", "1")]);
    for f in ["ta_e", "ta2_e", "th_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "TRAITARM_OFF: {f} resolves to the visible arm alone: {off:?}");
    }
}

/// UPDATE 4a-3 (Fable merge review 2a `rp_e`/`rp2_e`/`rp3_e`): the twin is wrapped by ANOTHER crate's macro in the same
/// module. Its input parses as items and was read, but filed under `loose` only, where no qualified lookup (`Tr`, `S as Tr`,
/// `S::m`) finds it, so the produced gated arm resolved alone: `deny Unknown` 1 on 0.42.0 and 0 on main for the trait and
/// trait-impl twins (stage 1), 0 everywhere for the inherent one. The items passed are now also read on the keys path.
/// Must NOT hold: a once-defined gated pair beside a foreign wrapper whose items name something else.
#[test]
fn i1s2_a_twin_wrapped_by_another_crates_macro_keeps_a_hedge() {
    let root = tmpdir("s2fw");
    let dep = root.join("fwdep");
    write_crate(&dep, "fwdep", "", "#[macro_export]\nmacro_rules! passthru { ($($i:item)*) => { $($i)* } }\n");
    let d = root.join("app");
    let hdr = |s: &str| {
        format!(
            "macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n\
             macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n"
        )
    };
    write_crate(
        &d,
        "s2fw",
        "[dependencies]\nfwdep = { path = \"../fwdep\" }\n",
        "pub mod p;\npub mod p2;\npub mod p3;\npub mod px;\n\
         pub fn fw_e() { use crate::p::TrP; crate::p::SP.go() }\n\
         pub fn fw2_e() { use crate::p2::TrP2; crate::p2::SP2.go() }\n\
         pub fn fw3_e() { crate::p3::SP3.go() }\n\
         pub fn fwx_x() { use crate::px::TrPX; crate::px::SPX.go() }\n",
    );
    write_file(
        &d,
        "src/p.rs",
        &(hdr("p")
            + "fwdep::passthru! { #[cfg(unix)] pub trait TrP { fn go(&self) { EFF } } }\nvis_p! { pub trait TrP { fn go(&self); } }\n\
               pub struct SP;\non_p! { impl TrP for SP {} }\nvis_p! { impl TrP for SP { fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/p2.rs",
        &(hdr("p2")
            + "pub trait TrP2 { fn go(&self); }\npub struct SP2;\n\
               fwdep::passthru! { #[cfg(unix)] impl TrP2 for SP2 { fn go(&self) { EFF } } }\nvis_p2! { impl TrP2 for SP2 { fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/p3.rs",
        &(hdr("p3")
            + "pub struct SP3;\nfwdep::passthru! { #[cfg(unix)] impl SP3 { pub fn go(&self) { EFF } } }\n\
               vis_p3! { impl SP3 { pub fn go(&self) {} } }\n"),
    );
    write_file(
        &d,
        "src/px.rs",
        &(hdr("px")
            + "pub trait TrPX { fn go(&self); }\npub struct SPX;\npub struct OPX;\n\
               fwdep::passthru! { pub trait TrOther { fn go(&self) {} } impl TrOther for OPX {} }\n\
               on_px! { impl TrPX for SPX { fn go(&self) { EFF } } }\nvis_px! { impl TrPX for SPX { fn go(&self) {} } }\n"),
    );
    let (rows, _) = scan(&d, None, &[]);
    for f in ["fw_e", "fw2_e", "fw3_e"] {
        assert_eq!(eff(&rows, f), vec!["Unknown".to_string()], "{f} must disclose the foreign-wrapped arm: {rows:?}");
    }
    // Stage 2b: another crate's macro is OPAQUE (it can emit any name), so the once-defined pair beside it keeps its `Fs` and now
    // also carries the hedge, the stated cost of that superset. The narrower reading (`CANDOR_I1S2B_FOREIGN_LOOSE=1`) holds it.
    assert!(eff(&rows, "fwx_x").contains(&"Fs".to_string()), "the pair is still charged: {rows:?}");
    let (loose, _) = scan(&d, None, &[("CANDOR_I1S2B_FOREIGN_LOOSE", "1")]);
    assert_eq!(eff(&loose, "fwx_x"), vec!["Fs".to_string()], "must not hold under the narrower reading: {loose:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// I1 stage 2c (Fable merge review 2a `ra_e`/`rb_e`/`rc_e`): an IMPL's cfg twin can sit in any module of the crate, and a
/// trait's default reaches every impl of it wherever written, so the per-module cover missed a twin one module away. Each
/// shape read `deny Unknown` 1 on 0.42.0 and 0 on main (stage 1 spliced the visible arm). Now every gated impl member is
/// checked against the hidden keys every module exports: the trait twin in a sibling module (`a`), the trait-impl twin in a
/// sibling module (`b`), the hidden trait inside a nested hidden module (`c`), the trait re-exported under another name
/// (`r`), an inherent twin in a sibling module (`i`), and a hidden override whose visible gated impl lacks the member, hedged
/// at the trait's default (`h`). Must NOT hold: a once-defined cross-module pair beside unrelated hidden text.
#[test]
fn i1s2c_a_cross_module_impl_twin_keeps_a_hedge() {
    let d = tmpdir("s2c");
    let hdr = |s: &str| {
        format!(
            "#[cfg(feature = \"x\")]\nmacro_rules! hid_{s} {{ ($($t:tt)*) => {{ #[cfg(unix)] $($t)* }} }}\n\
             #[cfg(not(feature = \"x\"))]\nmacro_rules! hid_{s} {{ ($($u:tt)*) => {{ #[cfg(unix)] $($u)* }} }}\n\
             macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n\
             macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n"
        )
    };
    let files: &[(&str, &str)] = &[
        ("a_t", "hid_a_t! { pub trait TrA { fn go(&self) { EFF } } }\nvis_a_t! { pub trait TrA { fn go(&self); } }\n"),
        ("a", "pub struct SA;\non_a! { impl crate::a_t::TrA for SA {} }\nvis_a! { impl crate::a_t::TrA for SA { fn go(&self) {} } }\n"),
        ("b_t", "pub trait TrB { fn go(&self); }\n"),
        ("b", "pub struct SB;\nhid_b! { impl crate::b_t::TrB for SB { fn go(&self) { EFF } } }\n"),
        ("b_v", "vis_b_v! { impl crate::b_t::TrB for crate::b::SB { fn go(&self) {} } }\n"),
        (
            "c",
            "hid_c! { pub mod inner { pub trait TrC { fn go(&self) { EFF } } } }\nvis_c! { pub mod inner { pub trait TrC { fn go(&self); } } }\n\
             pub struct SC;\non_c! { impl inner::TrC for SC {} }\nvis_c! { impl inner::TrC for SC { fn go(&self) {} } }\n",
        ),
        ("r_t", "hid_r_t! { pub trait TrR { fn go(&self) { EFF } } }\nvis_r_t! { pub trait TrR { fn go(&self); } }\n"),
        ("r_r", "pub use crate::r_t::TrR as TrAlias;\n"),
        ("r", "pub struct SR;\non_r! { impl crate::r_r::TrAlias for SR {} }\nvis_r! { impl crate::r_r::TrAlias for SR { fn go(&self) {} } }\n"),
        ("i", "pub struct SI;\nvis_i! { impl SI { pub fn go(&self) {} } }\n"),
        ("i_h", "hid_i_h! { impl crate::i::SI { pub fn go(&self) { EFF } } }\n"),
        ("h_t", "pub trait TrH { fn go(&self) {} }\npub struct SH;\n"),
        ("h", "hid_h! { impl crate::h_t::TrH for crate::h_t::SH { fn go(&self) { EFF } } }\n"),
        ("h_v", "vis_h_v! { impl crate::h_t::TrH for crate::h_t::SH {} }\n"),
        ("x_t", "pub trait TrX { fn go(&self); }\nhid_x_t! { pub trait TrOther { fn go(&self) {} } }\n"),
        ("x", "pub struct SX;\non_x! { impl crate::x_t::TrX for SX { fn go(&self) { EFF } } }\nvis_x! { impl crate::x_t::TrX for SX { fn go(&self) {} } }\n"),
    ];
    let mut lib = String::new();
    for (n, _) in files {
        lib.push_str(&format!("pub mod {n};\n"));
    }
    lib.push_str(
        "pub fn ca_e() { use crate::a_t::TrA; crate::a::SA.go() }\npub fn cb_e() { use crate::b_t::TrB; crate::b::SB.go() }\n\
         pub fn cc_e() { use crate::c::inner::TrC; crate::c::SC.go() }\npub fn cr_e() { use crate::r_t::TrR; crate::r::SR.go() }\n\
         pub fn ci_e() { crate::i::SI.go() }\npub fn ch_e() { use crate::h_t::TrH; crate::h_t::SH.go() }\n\
         pub fn cx_x() { use crate::x_t::TrX; crate::x::SX.go() }\n",
    );
    write_crate(&d, "s2c", "[features]\nx = []\n", &lib);
    for (n, body) in files {
        write_file(&d, &format!("src/{n}.rs"), &(hdr(n) + body));
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["ca_e", "cb_e", "cc_e", "cr_e", "ci_e", "ch_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f} must disclose the twin in another module: {rows:?}");
    }
    assert_eq!(eff(&rows, "cx_x"), vec!["Fs".to_string()], "must not hold: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2C_XMOD_OFF", "1")]);
    for f in ["ca_e", "cb_e", "cc_e", "cr_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "XMOD_OFF: {f} is main's silence again: {off:?}");
    }
}

/// UPDATE 2c-2 (Fable merge review 2c): three more ways a gated impl member's twin was missed, each executed in fixture
/// `fxt`. (m) the impl pair sits inside PRODUCED cfg-gated modules: the module's cfg gates its members, which carry none of
/// their own (`deny Unknown` 1 on 0.42.0 and on main, 0 on 2a: the module splice's own regression). (g) the hidden twin is
/// a BLANKET impl `impl<T: Mk> Tr for T`, keyed under its parameter's name (1 on 0.42.0, 0 on main). (n) the trait is
/// reached through a `use … as` rename hidden on both arms (1 on 0.42.0, 0 on main). Must NOT hold: a once-defined pair
/// inside once-defined produced modules beside unrelated hidden text.
#[test]
fn i1s2c_gated_modules_blanket_impls_and_hidden_renames_keep_a_hedge() {
    let d = tmpdir("s2c2");
    let hdr = |s: &str| {
        format!(
            "#[cfg(feature = \"x\")]\nmacro_rules! hid_{s} {{ ($($t:tt)*) => {{ #[cfg(unix)] $($t)* }} }}\n\
             #[cfg(not(feature = \"x\"))]\nmacro_rules! hid_{s} {{ ($($u:tt)*) => {{ #[cfg(unix)] $($u)* }} }}\n\
             macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n\
             macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n"
        )
    };
    let files: &[(&str, &str)] = &[
        (
            "m",
            "hid_m! { pub trait TrM { fn go(&self) { EFF } } }\nvis_m! { pub trait TrM { fn go(&self); } }\npub struct SM;\n\
             on_m! { pub mod imp { impl super::TrM for super::SM {} } }\nvis_m! { pub mod imp { impl super::TrM for super::SM { fn go(&self) {} } } }\n",
        ),
        (
            "g",
            "pub trait TrG { fn go(&self); }\npub trait MkG {}\npub struct SG;\nimpl MkG for SG {}\n\
             hid_g! { impl<T: MkG> TrG for T { fn go(&self) { EFF } } }\nvis_g! { impl TrG for SG { fn go(&self) {} } }\n",
        ),
        ("n_t", "hid_n_t! { pub trait TrN { fn go(&self) { EFF } } }\nvis_n_t! { pub trait TrN { fn go(&self); } }\n"),
        ("n_r", "hid_n_r! { pub use crate::n_t::TrN as TrNAlias; }\n"),
        ("n_r2", "hid_n_r2! { pub use crate::n_t::TrN as TrNAlias2; }\n"),
        ("n", "pub struct SN;\non_n! { impl crate::n_t::TrN for SN {} }\nvis_n! { impl crate::n_r::TrNAlias for SN { fn go(&self) {} } }\n"),
        (
            "x",
            "pub trait TrX { fn go(&self); }\npub struct SX;\nhid_x! { pub trait TrOther { fn go(&self) {} } }\n\
             on_x! { pub mod imp { impl super::TrX for super::SX { fn go(&self) { EFF } } } }\n\
             vis_x! { pub mod imp { impl super::TrX for super::SX { fn go(&self) {} } } }\n",
        ),
    ];
    let mut lib = String::new();
    for (n, _) in files {
        lib.push_str(&format!("pub mod {n};\n"));
    }
    lib.push_str(
        "pub fn dm_e() { use crate::m::TrM; crate::m::SM.go() }\npub fn dg_e() { use crate::g::TrG; crate::g::SG.go() }\n\
         pub fn dn_e() { use crate::n_t::TrN; crate::n::SN.go() }\npub fn dx_x() { use crate::x::TrX; crate::x::SX.go() }\n",
    );
    write_crate(&d, "s2c2", "[features]\nx = []\n", &lib);
    for (n, body) in files {
        write_file(&d, &format!("src/{n}.rs"), &(hdr(n) + body));
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["dm_e", "dg_e", "dn_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f} must disclose the hidden twin: {rows:?}");
    }
    assert_eq!(eff(&rows, "dx_x"), vec!["Fs".to_string()], "must not hold: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2C_XMOD_OFF", "1")]);
    for f in ["dm_e", "dg_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "XMOD_OFF: {f} resolves to the visible arm alone: {off:?}");
    }
}

/// I1 stage 2d (Fable re-check 2c-2 RC4-1/RC4-2/RC4.3, fixtures `fxu`/`fxw`/`fxv`, executed). (r) a hidden blanket over a
/// NON-PATH self type (`impl<T: Mk> Tr for &T`) was keyed `& T`, which no candidate asked for (`deny Unknown` 1 on 0.42.0,
/// 0 on main); now it is a blanket over that SHAPE (`& as Tr`), as are `&mut T`, tuples and slices of `T`. (a) a visible gated impl whose
/// self type is a `type` alias of the type the hidden impl names (1 on 0.42.0, 0 on main), and the reverse orientation.
/// (o) the OVER-HOLD removed: a same-named `macro_rules!` in a SIBLING cfg module was read as the hidden invocation's body,
/// so a member that invocation never defines was marked; hidden bodies now come from textual scope. Must NOT hold: a hidden
/// `&T` blanket of ANOTHER trait beside a once-defined pair, and the over-hold shape `o`.
#[test]
fn i1s2d_ref_blankets_aliases_and_scoped_bodies() {
    let d = tmpdir("s2d");
    let hdr = |s: &str| {
        format!(
            "#[cfg(feature = \"x\")]\nmacro_rules! hid_{s} {{ ($($t:tt)*) => {{ #[cfg(unix)] $($t)* }} }}\n\
             #[cfg(not(feature = \"x\"))]\nmacro_rules! hid_{s} {{ ($($u:tt)*) => {{ #[cfg(unix)] $($u)* }} }}\n\
             macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n\
             macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n"
        )
    };
    let files: &[(&str, &str)] = &[
        (
            "r",
            "pub trait TrR { fn go(&self); }\npub trait MkR {}\npub struct SR;\nimpl MkR for SR {}\n\
             hid_r! { impl<T: MkR> TrR for &T { fn go(&self) { EFF } } }\nvis_r! { impl TrR for &SR { fn go(&self) {} } }\n",
        ),
        (
            "t",
            "pub trait TrT { fn go(&self); }\npub trait MkT {}\npub struct ST;\nimpl MkT for ST {}\n\
             hid_t! { impl<T: MkT> TrT for (T,) { fn go(&self) { EFF } } }\nvis_t! { impl TrT for (ST,) { fn go(&self) {} } }\n",
        ),
        (
            "a",
            "pub trait TrA { fn go(&self); }\npub struct SA;\npub type AA = SA;\n\
             hid_a! { impl TrA for SA { fn go(&self) { EFF } } }\nvis_a! { impl TrA for AA { fn go(&self) {} } }\n",
        ),
        (
            "b",
            "pub trait TrB { fn go(&self); }\npub struct SB;\npub type AB = SB;\n\
             hid_b! { impl TrB for AB { fn go(&self) { EFF } } }\nvis_b! { impl TrB for SB { fn go(&self) {} } }\n",
        ),
        (
            "x",
            "pub trait TrX { fn go(&self); }\npub trait OtherX { fn go(&self); }\npub trait MkX {}\npub struct SX;\n\
             hid_x! { impl<T: MkX> OtherX for &T { fn go(&self) {} } }\n\
             on_x! { impl TrX for SX { fn go(&self) { EFF } } }\nvis_x! { impl TrX for SX { fn go(&self) {} } }\n",
        ),
        (
            "o",
            "pub trait FdO { fn fd(&self); }\npub trait HandleO { fn handle(&self); }\npub struct SO;\n\
             #[cfg(unix)]\nmod sys {\n    macro_rules! ito { ($t:ty) => { impl super::FdO for $t { fn fd(&self) { EFF } } } }\n    ito!(super::SO);\n}\n\
             #[cfg(any(windows, docsrs))]\nmod win {\n    #[cfg(not(docsrs))]\n    macro_rules! ito { ($t:ty) => { impl super::HandleO for $t { fn handle(&self) {} } } }\n\
                 #[cfg(docsrs)]\n    macro_rules! ito { ($t:ty) => { impl super::HandleO for $t { fn handle(&self) {} } } }\n    ito!(super::SO);\n}\n",
        ),
    ];
    let mut lib = String::new();
    for (n, _) in files {
        lib.push_str(&format!("pub mod {n};\n"));
    }
    lib.push_str(
        "pub fn er_e() { use crate::r::TrR; let s = crate::r::SR; (&s).go() }\n\
         pub fn et_e() { use crate::t::TrT; (crate::t::ST,).go() }\n\
         pub fn ea_e() { use crate::a::TrA; crate::a::SA.go() }\npub fn eb_e() { use crate::b::TrB; crate::b::SB.go() }\n\
         pub fn ex_x() { use crate::x::TrX; crate::x::SX.go() }\npub fn eo_x() { use crate::o::FdO; crate::o::SO.fd() }\n",
    );
    write_crate(&d, "s2d", "[features]\nx = []\n", &lib);
    for (n, body) in files {
        let text = if *n == "o" { body.to_string() } else { hdr(n) + body };
        write_file(&d, &format!("src/{n}.rs"), &text);
    }
    let (rows, _) = scan(&d, None, &[]);
    for f in ["er_e", "et_e", "ea_e", "eb_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f} must disclose the hidden twin: {rows:?}");
    }
    for f in ["ex_x", "eo_x"] {
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f}: must not hold: {rows:?}");
    }
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2D_REFBLANKET_OFF", "1"), ("CANDOR_I1S2D_ALIAS_OFF", "1"), ("CANDOR_I1S2D_SCOPEBODY_OFF", "1")]);
    for f in ["er_e", "ea_e"] {
        assert!(!eff(&off, f).contains(&"Unknown".to_string()), "switches off: {f} is main's silence again: {off:?}");
    }
    assert!(eff(&off, "eo_x").contains(&"Unknown".to_string()), "SCOPEBODY_OFF: the sibling body is bound again: {off:?}");
}

/// UPDATE 2d-2 (Fable merge review 2d D-1, fixture `fxx`, executed): textual scope binds an UNQUALIFIED invocation only. A
/// hidden PATH invocation (`crate::q::defs::m!` through `pub(crate) use m;`, a `#[macro_export]` macro invoked as
/// `crate::m!`) beside a textual same-name `macro_rules!` at the root read the textual body and dropped the one the path
/// names (`deny Unknown` 1 on 2c, 0 on 2d). Must NOT hold: the unqualified invocation beside a sibling's same-named body.
#[test]
fn i1s2d_a_path_invocation_is_not_bound_by_textual_scope() {
    let d = tmpdir("s2d2");
    write_crate(
        &d,
        "s2d2",
        "",
        "pub mod b;\npub mod c;\n\
         pub fn pb_e() { use crate::b::TrB; crate::b::SB.go() }\npub fn pc_e() { use crate::c::TrC; crate::c::SC.go() }\n",
    );
    write_file(
        &d,
        "src/b.rs",
        "pub trait TrB { fn go(&self); }\npub struct SB;\nmacro_rules! mb { ($t:ty) => { impl $t { pub fn noop_b(&self) {} } } }\n\
         pub mod defs {\n    macro_rules! mb { ($t:ty) => { impl crate::b::TrB for $t { fn go(&self) { EFF } } } }\n    pub(crate) use mb;\n}\n\
         #[cfg(unix)]\nmod u { crate::b::defs::mb!(crate::b::SB); }\n\
         #[cfg(windows)]\nmod w { impl crate::b::TrB for crate::b::SB { fn go(&self) {} } }\n",
    );
    write_file(
        &d,
        "src/c.rs",
        "pub trait TrC { fn go(&self); }\npub struct SC;\nmacro_rules! mc { ($t:ty) => { impl $t { pub fn noop_c(&self) {} } } }\n\
         pub mod defs {\n    #[macro_export]\n    macro_rules! mc { ($t:ty) => { impl crate::c::TrC for $t { fn go(&self) { EFF } } } }\n}\n\
         #[cfg(unix)]\nmod u { crate::mc!(crate::c::SC); }\n\
         #[cfg(windows)]\nmod w { impl crate::c::TrC for crate::c::SC { fn go(&self) {} } }\n",
    );
    let (rows, _) = scan(&d, None, &[]);
    for f in ["pb_e", "pc_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f}: the path-named body must be read: {rows:?}");
    }
}

/// I1 stage 2b — every fallback arm of the hidden reader reads a SUPERSET of what the text can bind, each pinned by a probe whose
/// name appears ONLY in the part of the text the arm used to skip (fixture `fxb`, executed: each probe wrote; `deny Unknown` 0
/// on 0.42.0 and main for all four). "noexpand" (the matcher cannot take `'de`): the name is only in a NESTED macro's body.
/// "limit" (deeper than the reader's depth): only in the base case's nested macro body. "foreign" / "foreign-items": only in
/// another crate's macro body. Must NOT hold: an unrelated name in the nested body beside a once-defined pair.
#[test]
fn i1s2b_every_fallback_arm_reads_a_superset() {
    let root = tmpdir("s2b");
    let dep = root.join("bdep");
    write_crate(
        &dep,
        "bdep",
        "",
        "#[macro_export]\nmacro_rules! mint { () => { #[cfg(unix)] pub fn leaf_f() { EFF } } }\n\
         #[macro_export]\nmacro_rules! wrap_items { ($($i:item)*) => { $($i)* #[cfg(unix)] pub fn leaf_i() { EFF } } }\n",
    );
    let d = root.join("app");
    let vis = |s: &str| format!("macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n");
    let a = vec!["a"; 140].join(" ");
    write_crate(
        &d,
        "s2b",
        "[dependencies]\nbdep = { path = \"../bdep\" }\n",
        "#![recursion_limit = \"512\"]\npub mod n;\npub mod nx;\npub mod l;\npub mod f;\npub mod i;\n\
         pub fn bn_e() { crate::n::leaf_n() }\npub fn bnx_x() { crate::nx::leaf_nx() }\npub fn bl_e() { crate::l::leaf_l() }\n\
         pub fn bf_e() { crate::f::leaf_f() }\npub fn bi_e() { crate::i::leaf_i() }\n",
    );
    write_file(
        &d,
        "src/n.rs",
        &(vis("n")
            + "macro_rules! inner_n { () => { #[cfg(unix)] pub fn leaf_n() { EFF } } }\n\
               macro_rules! outer_n { (go<$l:tt>) => { inner_n!{} } }\nouter_n!{ go<'de> }\nvis_n! { pub fn leaf_n() {} }\n"),
    );
    write_file(
        &d,
        "src/nx.rs",
        &(vis("nx")
            + "macro_rules! inner_nx { () => { #[cfg(unix)] pub fn other_nx() {} } }\n\
               macro_rules! outer_nx { (go<$l:tt>) => { inner_nx!{} } }\nouter_nx!{ go<'de> }\n\
               macro_rules! on_nx { ($($i:item)*) => { $( #[cfg(unix)] $i )* } }\n\
               on_nx! { pub fn leaf_nx() { EFF } }\nvis_nx! { pub fn leaf_nx() {} }\n"),
    );
    write_file(
        &d,
        "src/l.rs",
        &(vis("l")
            + "macro_rules! base_l { () => { #[cfg(unix)] pub fn leaf_l() { EFF } } }\n\
               macro_rules! rec_l { ($h:ident $($t:ident)*) => { rec_l!{ $($t)* } }; () => { base_l!{} } }\n"
            + &format!("rec_l!{{ {a} }}\nvis_l! {{ pub fn leaf_l() {{}} }}\n")),
    );
    write_file(&d, "src/f.rs", &(vis("f") + "bdep::mint!();\nvis_f! { pub fn leaf_f() {} }\n"));
    write_file(&d, "src/i.rs", &(vis("i") + "bdep::wrap_items! { pub fn passed_i() {} }\nvis_i! { pub fn leaf_i() {} }\n"));
    let (rows, _) = scan(&d, None, &[]);
    for f in ["bn_e", "bl_e", "bf_e", "bi_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f}: the arm must read the skipped text: {rows:?}");
    }
    assert_eq!(eff(&rows, "bnx_x"), vec!["Fs".to_string()], "must not hold: {rows:?}");
    let (loose, _) = scan(&d, None, &[("CANDOR_I1S2B_FOREIGN_LOOSE", "1")]);
    for f in ["bf_e", "bi_e"] {
        assert!(!eff(&loose, f).contains(&"Unknown".to_string()), "FOREIGN_LOOSE: {f} is the narrower reading's silence: {loose:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// UPDATE A1 (Fable's 2b review, fixture `fxc`, executed): three lookups the fallback arms made narrower than the text.
/// (ci) a `cfg_if!` branch holding a nested LOCAL invocation: the exception read only cfg_if's input. (mv) the closure read a
/// `$m!{}` as the local macro named `m`. (ps) the closure read a path-invoked macro as its textual-scope namesake. Each read
/// `deny Unknown` 0 on 0.42.0, main and the first 2b cut. Must NOT hold: a nested macro naming an unrelated item beside a
/// once-defined pair (`cix`), and a noexpand body holding `a != b` (`ne`: `!=` is not an invocation).
#[test]
fn i1s2b_closure_lookups_are_supersets() {
    let d = tmpdir("s2ba1");
    let vis = |s: &str| format!("macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n");
    let on = |s: &str| format!("macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n");
    write_crate(
        &d,
        "s2ba1",
        "",
        "pub mod ci;\npub mod mv;\npub mod ps_other;\npub mod ps;\npub mod cix;\npub mod ne;\n\
         pub fn ci_e() { crate::ci::leaf_ci() }\npub fn mv_e() { crate::mv::leaf_mv() }\npub fn ps_e() { crate::ps::leaf_ps() }\n\
         pub fn cix_x() { crate::cix::leaf_cix() }\npub fn ne_x() { crate::ne::leaf_ne() }\n",
    );
    write_file(
        &d,
        "src/ci.rs",
        &(vis("ci")
            + "macro_rules! mint_ci { () => { pub fn leaf_ci() { EFF } } }\n\
               cfg_if::cfg_if! { if #[cfg(unix)] { mint_ci!{} } else { } }\nvis_ci! { pub fn leaf_ci() {} }\n"),
    );
    write_file(
        &d,
        "src/mv.rs",
        &(vis("mv")
            + "macro_rules! m { () => { pub fn unrelated_mv() {} } }\n\
               macro_rules! real_mv { () => { #[cfg(unix)] pub fn leaf_mv() { EFF } } }\n\
               macro_rules! call_mv { ($m:ident go<$l:tt>) => { $m!{} } }\ncall_mv!{ real_mv go<'de> }\nvis_mv! { pub fn leaf_mv() {} }\n"),
    );
    write_file(
        &d,
        "src/ps_other.rs",
        "macro_rules! inner_ps { () => { #[cfg(unix)] pub fn leaf_ps() { EFF } } }\npub(crate) use inner_ps;\n",
    );
    write_file(
        &d,
        "src/ps.rs",
        &(vis("ps")
            + "macro_rules! inner_ps { () => { pub fn unrelated_ps() {} } }\n\
               macro_rules! outer_ps { (go<$l:tt>) => { crate::ps_other::inner_ps!{} } }\nouter_ps!{ go<'de> }\nvis_ps! { pub fn leaf_ps() {} }\n"),
    );
    write_file(
        &d,
        "src/cix.rs",
        &(vis("cix")
            + &on("cix")
            + "macro_rules! other_cix { () => { pub fn unrelated_cix() {} } }\n\
               cfg_if::cfg_if! { if #[cfg(unix)] { other_cix!{} } }\non_cix! { pub fn leaf_cix() { EFF } }\nvis_cix! { pub fn leaf_cix() {} }\n"),
    );
    write_file(
        &d,
        "src/ne.rs",
        &(vis("ne")
            + &on("ne")
            + "macro_rules! outer_ne { (go<$l:tt>) => { pub fn unrelated_ne(a: u8, b: u8) -> bool { a != b } } }\n\
               outer_ne!{ go<'de> }\non_ne! { pub fn leaf_ne() { EFF } }\nvis_ne! { pub fn leaf_ne() {} }\n"),
    );
    let (rows, _) = scan(&d, None, &[]);
    for f in ["ci_e", "mv_e", "ps_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f}: the twin's body must be read: {rows:?}");
    }
    assert_eq!(eff(&rows, "ne_x"), vec!["Fs".to_string()], "ne_x: must not hold: {rows:?}");
    // UPDATE A3: `cfg_if!` is another crate's macro, opaque like any other, so `cix_x` keeps its charge and may gain exactly
    // that opacity's `Unknown` (the stated cost of having no `cfg_if!` exception).
    let cix = eff(&rows, "cix_x");
    assert!(cix.contains(&"Fs".to_string()) && cix.iter().all(|e| e == "Fs" || e == "Unknown"), "cix_x: {rows:?}");
    assert_eq!(rows["cix::leaf_cix"].1, vec!["macro:another cfg arm of this fn is hidden by an unexpanded macro".to_string()]);
}

/// UPDATE A2/A3 (Fable RE-CHECK A1/A2, fixtures `fxe`..`fxe7`, executed): there is no `cfg_if!` exception (A3: an identity
/// check was not provable from the source), so `cfg_if!` is opaque like any other foreign macro, which closes: a rename
/// re-exported from another file (c4), the same through a glob (c5), a dependency's own `macro_rules! cfg_if` imported by name
/// (c11) or in scope through `#[macro_use] extern crate` (c10, a crate of its own). Each read `deny Unknown` 0 on 0.42.0, main,
/// 2b and A1. Also pinned, closed by A1: a `$crate::` path in a body a branch reaches (c8), `![]`/`()` delimiters (c9). Must
/// NOT hold, priced: the path form beside a once-defined pair (cx) keeps `Fs` and gains exactly the opacity `Unknown`.
#[test]
fn i1s2b_cfg_if_is_opaque_like_any_foreign_macro() {
    let root = tmpdir("s2ba2");
    let dep = root.join("edep");
    write_crate(
        &dep,
        "edep",
        "",
        "#[macro_export]\nmacro_rules! cfg_if { (\"c10\") => { #[cfg(unix)] pub fn leaf_c10() { EFF } }; (\"c11\") => { #[cfg(unix)] pub fn leaf_c11() { EFF } }; }\n\
         #[macro_export]\nmacro_rules! mint_e { (\"c4\") => { #[cfg(unix)] pub fn leaf_c4() { EFF } }; (\"c5\") => { #[cfg(unix)] pub fn leaf_c5() { EFF } }; }\n",
    );
    let vis = |s: &str| format!("macro_rules! vis_{s} {{ ($($i:item)*) => {{ $( #[cfg(not(unix))] $i )* }} }}\n");
    let on = |s: &str| format!("macro_rules! on_{s} {{ ($($i:item)*) => {{ $( #[cfg(unix)] $i )* }} }}\n");
    let d = root.join("app");
    write_crate(
        &d,
        "s2ba2",
        "[dependencies]\nedep = { path = \"../edep\" }\n",
        "pub mod r;\npub mod c4;\npub mod c5;\npub mod c8;\npub mod c9;\npub mod c11;\npub mod cx;\n\
         pub fn c4_e() { crate::c4::leaf_c4() }\npub fn c5_e() { crate::c5::leaf_c5() }\npub fn c8_e() { crate::c8::leaf_c8() }\n\
         pub fn c9_e() { crate::c9::leaf_c9() }\npub fn c11_e() { crate::c11::leaf_c11() }\npub fn cx_x() { crate::cx::leaf_cx() }\n",
    );
    write_file(&d, "src/r.rs", "pub use edep::mint_e as cfg_if;\n");
    write_file(&d, "src/c4.rs", &(vis("c4") + "use crate::r::cfg_if;\ncfg_if!(\"c4\");\nvis_c4! { pub fn leaf_c4() {} }\n"));
    write_file(&d, "src/c5.rs", &(vis("c5") + "use crate::r::*;\ncfg_if!(\"c5\");\nvis_c5! { pub fn leaf_c5() {} }\n"));
    write_file(
        &d,
        "src/c8.rs",
        &(vis("c8")
            + "macro_rules! inner_c8 { () => { pub fn leaf_c8() { EFF } } }\npub(crate) use inner_c8;\n\
               macro_rules! wrap_c8 { () => { $crate::c8::inner_c8!{} } }\n\
               cfg_if::cfg_if! { if #[cfg(unix)] { wrap_c8!{} } }\nvis_c8! { pub fn leaf_c8() {} }\n"),
    );
    write_file(
        &d,
        "src/c9.rs",
        &(vis("c9")
            + "macro_rules! inner_c9 { () => { pub fn leaf_c9() { EFF } } }\nmacro_rules! mid_c9 { () => { inner_c9![]; } }\n\
               cfg_if::cfg_if! { if #[cfg(unix)] { mid_c9!(); } }\nvis_c9! { pub fn leaf_c9() {} }\n"),
    );
    write_file(&d, "src/c11.rs", &(vis("c11") + "use edep::cfg_if;\ncfg_if!(\"c11\");\nvis_c11! { pub fn leaf_c11() {} }\n"));
    write_file(
        &d,
        "src/cx.rs",
        &(vis("cx")
            + &on("cx")
            + "macro_rules! other_cx { () => { pub fn unrelated_cx() {} } }\n\
               cfg_if::cfg_if! { if #[cfg(unix)] { other_cx!{} } }\non_cx! { pub fn leaf_cx() { EFF } }\nvis_cx! { pub fn leaf_cx() {} }\n"),
    );
    let (rows, _) = scan(&d, None, &[]);
    for f in ["c4_e", "c5_e", "c8_e", "c9_e", "c11_e"] {
        assert!(eff(&rows, f).contains(&"Unknown".to_string()), "{f}: must disclose the twin: {rows:?}");
    }
    // UPDATE A3: no `cfg_if!` exception, so the path form is opaque too: `cx_x` keeps its charge, plus exactly that `Unknown`.
    let cx = eff(&rows, "cx_x");
    assert!(cx.contains(&"Fs".to_string()) && cx.iter().all(|e| e == "Fs" || e == "Unknown"), "cx_x: {rows:?}");
    // …and that `Unknown` is the opacity's own reason on the leaf, nothing else (Fable STAGE2B nit: pinned as for `cix`).
    assert_eq!(rows["cx::leaf_cx"].1, vec!["macro:another cfg arm of this fn is hidden by an unexpanded macro".to_string()]);
    // c10: the dependency's `cfg_if` in textual scope through `#[macro_use] extern crate`, a crate of its own
    let d2 = root.join("app2");
    write_crate(
        &d2,
        "s2ba2b",
        "[dependencies]\nedep = { path = \"../edep\" }\n",
        "#[macro_use]\nextern crate edep;\npub mod c10;\npub fn c10_e() { crate::c10::leaf_c10() }\n",
    );
    write_file(&d2, "src/c10.rs", &(vis("c10") + "cfg_if!(\"c10\");\nvis_c10! { pub fn leaf_c10() {} }\n"));
    let (rows2, _) = scan(&d2, None, &[]);
    assert!(eff(&rows2, "c10_e").contains(&"Unknown".to_string()), "c10_e: must disclose the twin: {rows2:?}");
    let _ = std::fs::remove_dir_all(&root);
}
