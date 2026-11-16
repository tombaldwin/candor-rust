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

/// The cross-file cfg refusal is lifted: a configuration macro defined in another file (tokio's `cfg_rt!` shape) stamps
/// `#[cfg]` on what it wraps, and with `use`s spliced the twin arm (`cfg_unix! { pub use crate::real::leaf2; }`) stands
/// beside the stub, so the union is whole. XCFG_OFF refuses again and the call discloses (R128) instead of charging.
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
        assert_eq!(eff(&rows, f), vec!["Fs".to_string()], "{f}: {rows:?}");
    }
    assert!(eff(&rows, "xcfgq_x").is_empty() && units.contains("a::leafq"), "the pure twin stays pure: {rows:?}");
    let (off, _) = scan(&d, None, &[("CANDOR_I1S2_XCFG_OFF", "1")]);
    for f in ["xcfg_e", "xcfguse_e"] {
        assert_eq!(eff(&off, f), vec!["Unknown".to_string()], "XCFG_OFF: {f} is R128's disclosure again: {off:?}");
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
