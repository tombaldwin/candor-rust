//! SOUNDNESS R1652 / R1657 (stage 1a) — A BARE TYPE NAME IS QUALIFIED WHERE IT IS MINTED, AND THE AMBIGUOUS-LEAF FIELD PICK
//! IS RETIRED.
//!
//! A value's type was a bare leaf (`Holder`) wherever the declaring module wrote it bare, so two same-leaf structs in
//! different modules answered to one string and `fields[leaf]` kept the LAST twin merged. A receiver whose owner path was
//! not proven read the other twin's field type. Now a bare name a module declares as a `struct`/`enum`/`union` is spelled
//! `crate::<module>::<Name>` at its origin (`lang::anchor_origin`), which makes the owner provable, and an owner that is
//! still unproven is REFUSED and disclosed (`ambiguous:same-name types ...`) instead of picked.
//!
//! Every fixture below is EXECUTED ground truth (lane scratch `rustagent-stage1a/fx/<name>`, `cargo run`; the effectful
//! bodies write a temp file). Fixture fns carry a unique `_zq` terminator because `deny <E> <fn>` matches by PREFIX.
//! On published 0.41.3 and on 19f9f61: `consume_a_zq` (fxi) ABSENT over a write, `consume_a_zq` (fxii) charged `Fs` over a
//! pure body, `elem_idx_a_zq` / `elem_for_a_zq` ABSENT over a write, `rex_param_zq` ABSENT over a write.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn fixture(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("candor-r1657-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\n").unwrap();
    for (p, s) in files {
        let f = d.join("src").join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, s).unwrap();
    }
    d
}

/// The report (`--json`), and the exit code of `policy` when one is given. `off` sets `CANDOR_R1657_OFF`.
fn scan(dir: &Path, policy: Option<&str>, off: bool) -> (i32, serde_json::Value) {
    let mut cmd = Command::new(bin());
    cmd.arg(dir.to_string_lossy().as_ref()).arg("--json");
    if let Some(p) = policy {
        let pf = dir.join("policy.txt");
        std::fs::write(&pf, p).unwrap();
        cmd.arg("--policy").arg(&pf);
    }
    cmd.env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env_remove("CANDOR_R1657_OFF").env_remove("CANDOR_R1624_OFF");
    if off {
        cmd.env("CANDOR_R1657_OFF", "1");
    }
    let out = cmd.output().expect("run candor-scan");
    let code = out.status.code().unwrap_or(-1);
    let refused_by_switch = off && policy.is_some() && code == 2;
    assert!(code == 0 || code == 1 || refused_by_switch, "scan failed ({code}): {}", String::from_utf8_lossy(&out.stderr));
    (code, serde_json::from_slice(&out.stdout).expect("report json"))
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

const W: &str = "std::fs::write(std::env::temp_dir().join(\"r1657\"), b\"x\").unwrap();";

/// R1652's two executed fixtures: `a`/`b` each `struct Holder { inner: EngineX }`, consumer `let h = make_a(); h.inner.run()`.
fn twins(a_eff: bool) -> Vec<(&'static str, String)> {
    let (ea, eb) = if a_eff { (W, "") } else { ("", W) };
    vec![
        ("a.rs", format!("pub struct EngineA;\nimpl EngineA {{ pub fn run(&self) {{ {ea} }} }}\npub struct Holder {{ pub inner: EngineA }}\npub fn make() -> Holder {{ Holder {{ inner: EngineA }} }}\n")),
        ("b.rs", format!("pub struct EngineB;\nimpl EngineB {{ pub fn run(&self) {{ {eb} }} }}\npub struct Holder {{ pub inner: EngineB }}\npub fn make() -> Holder {{ Holder {{ inner: EngineB }} }}\n")),
        ("c.rs", "use crate::a::make as make_a;\nuse crate::b::make as make_b;\npub fn consume_a_zq() { let h = make_a(); h.inner.run(); }\npub fn consume_b_zq() { let h = make_b(); h.inner.run(); }\n".to_string()),
        ("main.rs", "mod a;\nmod b;\nmod c;\nfn main() { c::consume_a_zq(); }\n".to_string()),
    ]
}

/// The same two twins as `twins`, in ONE file as inline modules. The kill-switch (OFF) arms use this form: with `CANDOR_R1657_OFF`
/// the pre-change reading keeps the LAST same-leaf twin merged (`fields[leaf]`), and "last" is the order `scan` walks the files in,
/// which is the directory's own `read_dir` order and so differs by filesystem (alphabetical on APFS and a plain overlay;
/// REVERSE creation order on tmpfs and on CI's runner, where the OFF arms of the two-file form read the other twin and
/// failed with the silence/fabrication "not restored"). Within one file the last twin is the one written last, on every
/// filesystem, so the legacy pick is deterministic and the attribution is checkable.
fn twins_one_file(a_eff: bool) -> String {
    let (ea, eb) = if a_eff { (W, "") } else { ("", W) };
    format!("mod a {{ pub struct EngineA; impl EngineA {{ pub fn run(&self) {{ {ea} }} }} pub struct Holder {{ pub inner: EngineA }} pub fn make() -> Holder {{ Holder {{ inner: EngineA }} }} }}\n\
mod b {{ pub struct EngineB; impl EngineB {{ pub fn run(&self) {{ {eb} }} }} pub struct Holder {{ pub inner: EngineB }} pub fn make() -> Holder {{ Holder {{ inner: EngineB }} }} }}\n\
mod c {{ use crate::a::make as make_a; use crate::b::make as make_b;\n\
pub fn consume_a_zq() {{ let h = make_a(); h.inner.run(); }}\npub fn consume_b_zq() {{ let h = make_b(); h.inner.run(); }} }}\n\
fn main() {{ c::consume_a_zq(); c::consume_b_zq(); }}\n")
}

fn files<'a>(v: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    v.iter().map(|(p, s)| (*p, s.as_str())).collect()
}

#[test]
fn r1657_fxi_the_silence_closes_and_the_kill_switch_restores_it() {
    let src = twins(true);
    let d = fixture("fxi", &files(&src));
    let (rc, _) = scan(&d, Some("deny Fs c::consume_a_zq\n"), false);
    assert_eq!(rc, 1, "fxi: deny Fs over the executed write");
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "c::consume_a_zq"), ["Env", "Fs"], "{on:#}");
    assert!(inferred(&on, "c::consume_b_zq").is_empty(), "the pure twin's consumer stays pure");
    let _ = std::fs::remove_dir_all(&d);
    // The attribution arm: one file, so the pre-change last-twin pick does not depend on the filesystem's directory order.
    let d = fixture("fxi1", &[("main.rs", &twins_one_file(true))]);
    let (_, on1) = scan(&d, None, false);
    assert_eq!(inferred(&on1, "c::consume_a_zq"), ["Env", "Fs"], "{on1:#}");
    let (_, off) = scan(&d, None, true);
    assert!(!inferred(&off, "c::consume_a_zq").contains(&"Fs".to_string()), "CANDOR_R1657_OFF restores the silence");
    assert_eq!(off["switches"], serde_json::json!(["CANDOR_R1657_OFF"]), "the switched report names the switch");
    let (refused, _) = scan(&d, Some("deny Fs c::consume_a_zq\n"), true);
    assert_eq!(refused, 2, "a gate under CANDOR_R1657_OFF is unanswerable");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn r1657_fxii_the_fabrication_is_removed_and_the_effectful_twin_stays_charged() {
    let src = twins(false);
    let d = fixture("fxii", &files(&src));
    let (rc, on) = scan(&d, Some("deny Fs c::consume_a_zq\n"), false);
    assert_eq!(rc, 0, "fxii: nothing is written through `consume_a_zq`:\n{on:#}");
    let (_, on) = scan(&d, None, false);
    assert!(inferred(&on, "c::consume_a_zq").is_empty(), "fxii pure consumer {on:#}");
    assert_eq!(inferred(&on, "c::consume_b_zq"), ["Env", "Fs"], "the effectful twin's consumer stays charged");
    let _ = std::fs::remove_dir_all(&d);
    // The attribution arm: one file (see `twins_one_file`).
    let d = fixture("fxii1", &[("main.rs", &twins_one_file(false))]);
    let (_, on1) = scan(&d, None, false);
    assert!(inferred(&on1, "c::consume_a_zq").is_empty(), "fxii pure consumer {on1:#}");
    let (_, off) = scan(&d, None, true);
    assert!(inferred(&off, "c::consume_a_zq").contains(&"Fs".to_string()), "CANDOR_R1657_OFF restores the fabrication");
    let _ = std::fs::remove_dir_all(&d);
}

/// The element route (`field_elem`), both directions, through an index and through a `for` over `.iter()`.
#[test]
fn r1657_element_route_twins() {
    let src = format!("mod a {{ pub struct EngA; impl EngA {{ pub fn run(&self) {{ {W} }} }} pub struct Holder {{ pub items: Vec<EngA> }} pub fn make() -> Holder {{ Holder {{ items: vec![EngA] }} }} }}\n\
mod b {{ pub struct EngB; impl EngB {{ pub fn run(&self) {{}} }} pub struct Holder {{ pub items: Vec<EngB> }} pub fn make() -> Holder {{ Holder {{ items: vec![EngB] }} }} }}\n\
mod c {{ use crate::a::make as make_a; use crate::b::make as make_b;\n\
pub fn elem_idx_a_zq() {{ let h = make_a(); h.items[0].run(); }}\npub fn elem_idx_b_zq() {{ let h = make_b(); h.items[0].run(); }}\n\
pub fn elem_for_a_zq() {{ let h = make_a(); for e in h.items.iter() {{ e.run(); }} }}\npub fn elem_for_b_zq() {{ let h = make_b(); for e in h.items.iter() {{ e.run(); }} }} }}\n\
fn main() {{ c::elem_idx_a_zq(); c::elem_idx_b_zq(); c::elem_for_a_zq(); c::elem_for_b_zq(); }}\n");
    let d = fixture("elem", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    for f in ["c::elem_idx_a_zq", "c::elem_for_a_zq"] {
        assert_eq!(inferred(&on, f), ["Env", "Fs"], "{f}: {on:#}");
    }
    for f in ["c::elem_idx_b_zq", "c::elem_for_b_zq"] {
        assert!(inferred(&on, f).is_empty(), "{f} charged {:?}", inferred(&on, f));
    }
    let (rc, _) = scan(&d, Some("deny Fs c::elem_idx_a_zq\n"), false);
    assert_eq!(rc, 1);
    let _ = std::fs::remove_dir_all(&d);
}

/// HAZARD (i): a generic parameter that shadows a nominal name its module declares is the PARAMETER. Both directions;
/// without the guard (seeded on the lane's scratch binary) `gsf_field_param_zq` read `Fs` and `gss_field_param_zq` ABSENT.
#[test]
fn r1657_a_generic_parameter_shadowing_a_declared_name_is_not_qualified() {
    let gsf = format!("mod m {{ pub trait Go {{ fn go(&self); }}\npub struct State;\nimpl State {{ pub fn go(&self) {{ {W} }} }}\n\
pub struct Calm;\nimpl Go for Calm {{ fn go(&self) {{}} }}\npub struct W<State> {{ pub s: State }}\n\
impl<State: Go> W<State> {{ pub fn gsf_field_param_zq(&self) {{ self.s.go(); }} }}\n\
pub fn gsf_real_state_zq(s: &State) {{ s.go(); }} }}\nfn main() {{ m::W {{ s: m::Calm }}.gsf_field_param_zq(); }}\n");
    let d = fixture("gsf", &[("main.rs", &gsf)]);
    let (_, on) = scan(&d, None, false);
    assert!(inferred(&on, "m::W::gsf_field_param_zq").is_empty(), "fabrication through a shadowed name {on:#}");
    assert_eq!(inferred(&on, "m::gsf_real_state_zq"), ["Env", "Fs"], "the declared type itself is still that type");
    let _ = std::fs::remove_dir_all(&d);
    let gss = format!("mod m {{ pub trait Go {{ fn go(&self); }}\npub struct State;\nimpl State {{ pub fn go(&self) {{}} }}\n\
pub struct Loud;\nimpl Go for Loud {{ fn go(&self) {{ {W} }} }}\npub struct W<State> {{ pub s: State }}\n\
impl<State: Go> W<State> {{ pub fn gss_field_param_zq(&self) {{ self.s.go(); }} }} }}\nfn main() {{ m::W {{ s: m::Loud }}.gss_field_param_zq(); }}\n");
    let d = fixture("gss", &[("main.rs", &gss)]);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "m::W::gss_field_param_zq"), ["Env", "Fs"], "silence through a shadowed name {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// HAZARD (viii) / R1153 g2: a name declared ONLY under `#[cfg]` is not qualified to the (here inactive) local declaration;
/// the glob candidate keeps its disclosure. Seeded without the guard, both callers went ABSENT over a write.
#[test]
fn r1657_a_cfg_only_declaration_is_not_the_answer() {
    let src = format!("mod eng {{ pub struct Tool; impl Tool {{ pub fn go(&self) {{ {W} }} }} }}\n\
mod g {{ use crate::eng::*;\n#[cfg(windows)] pub struct Tool;\n#[cfg(windows)] impl Tool {{ pub fn go(&self) {{}} }}\n\
pub struct Holder {{ pub t: Tool }}\npub fn cfgl_param_zq(t: &Tool) {{ t.go(); }}\npub fn cfgl_field_zq(h: &Holder) {{ h.t.go(); }} }}\n\
mod z {{ pub struct Tool; impl Tool {{ pub fn go(&self) {{}} }} pub struct Holder {{ pub t: Tool }} }}\n\
fn main() {{ g::cfgl_param_zq(&eng::Tool); g::cfgl_field_zq(&g::Holder {{ t: eng::Tool }}); let _ = z::Holder {{ t: z::Tool }}; }}\n");
    let d = fixture("cfgl", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    for f in ["g::cfgl_param_zq", "g::cfgl_field_zq"] {
        assert!(!inferred(&on, f).is_empty(), "{f} must not read pure over the glob's write {on:#}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// HAZARD (iii): a type name inside a `macro_rules!` body resolves where the macro is INVOKED.
#[test]
fn r1657_macro_bodies_qualify_at_the_invoking_module() {
    let src = format!("mod m {{ pub struct Tool; impl Tool {{ pub fn go(&self) {{ {W} }} }} pub struct Holder {{ pub t: Tool }}\n\
#[macro_export] macro_rules! body_user {{ ($h:expr) => {{{{ let x: &Holder = $h; x.t.go(); }}}}; }}\n\
pub fn mac_m_body_zq(h: &Holder) {{ crate::body_pure!(h); }} }}\n\
mod n {{ pub struct Tool; impl Tool {{ pub fn go(&self) {{}} }} pub struct Holder {{ pub t: Tool }}\n\
#[macro_export] macro_rules! body_pure {{ ($h:expr) => {{{{ let x: &Holder = $h; x.t.go(); }}}}; }}\n\
pub fn mac_n_body_zq(h: &Holder) {{ crate::body_user!(h); }} }}\n\
fn main() {{ m::mac_m_body_zq(&m::Holder {{ t: m::Tool }}); n::mac_n_body_zq(&n::Holder {{ t: n::Tool }}); }}\n");
    let d = fixture("mac", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "m::mac_m_body_zq"), ["Env", "Fs"], "invoked in m: m's Holder {on:#}");
    assert!(!inferred(&on, "n::mac_n_body_zq").contains(&"Fs".to_string()), "invoked in n: n's pure Holder {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// HAZARD (ii): an owner spelled through a RE-EXPORT (`crate::runtime::Handle` for `runtime::handle::Handle`) is anchored but
/// not declared there. Published: ABSENT over the write (the last twin's pure field). Stage 1a: refused and disclosed with a
/// true reason. STAGE 2 (the re-export table, `lang::reexport_lookup`): RESOLVED to the declaration the re-export names, so
/// the write is charged; the stage-1a disclosure is what `CANDOR_R1657S2_OFF` gives back (`tests/r1657s2_reexport_table.rs`
/// pins that arm). The declaring path proves its owner and reads pure on every arm.
#[test]
fn r1657_an_unproven_reexport_owner_is_disclosed_not_picked() {
    let src = format!("mod runtime {{ mod handle {{ pub struct EngR; impl EngR {{ pub fn run(&self) {{ {W} }} }} pub struct Handle {{ pub inner: EngR }} }}\n\
pub use handle::{{EngR, Handle}}; }}\n\
mod other {{ pub struct EngO; impl EngO {{ pub fn run(&self) {{}} }} pub struct Handle {{ pub inner: EngO }} }}\n\
mod c {{ pub fn rex_param_zq(h: &crate::runtime::Handle) {{ h.inner.run(); }}\npub fn rex_other_zq(h: &crate::other::Handle) {{ h.inner.run(); }} }}\n\
fn main() {{ c::rex_param_zq(&runtime::Handle {{ inner: runtime::EngR }}); c::rex_other_zq(&other::Handle {{ inner: other::EngO }}); }}\n");
    let d = fixture("rex", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "c::rex_param_zq"), ["Env", "Fs"], "stage 2 resolves the re-export owner {on:#}");
    assert!(inferred(&on, "c::rex_other_zq").is_empty(), "the proven owner reads its own pure field {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// HAZARD (v): trait positions are never qualified (only `struct`/`enum`/`union` names are), so dispatch is unchanged.
#[test]
fn r1657_trait_positions_and_macro_declared_types_are_untouched() {
    let src = format!("mod m {{ pub trait Tr {{ fn go(&self); }}\npub struct Loud;\nimpl Tr for Loud {{ fn go(&self) {{ {W} }} }}\n\
pub fn trt_dyn_zq(x: &dyn Tr) {{ x.go(); }}\npub fn trt_impl_zq(x: impl Tr) {{ x.go(); }}\npub fn trt_bound_zq<T: Tr>(x: T) {{ x.go(); }}\n\
macro_rules! decl_ty {{ ($n:ident) => {{ pub struct $n; impl $n {{ pub fn hit(&self) {{ {W} }} }} }}; }}\ndecl_ty!(Made);\n\
pub fn trt_made_zq(x: &Made) {{ x.hit(); }} }}\n\
fn main() {{ m::trt_dyn_zq(&m::Loud); m::trt_impl_zq(m::Loud); m::trt_bound_zq(m::Loud); m::trt_made_zq(&m::Made); }}\n");
    let d = fixture("trt", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    let (_, off) = scan(&d, None, true);
    for f in ["m::trt_dyn_zq", "m::trt_impl_zq", "m::trt_bound_zq", "m::trt_made_zq"] {
        assert_eq!(inferred(&on, f), ["Env", "Fs"], "{f} {on:#}");
        assert_eq!(inferred(&on, f), inferred(&off, f), "{f}: unchanged by R1657");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// The first A/B's two regressions, pinned. (1) core2's shape: the crate root declares `mod io` under one cfg and re-exports
/// `std::io as io` under the other; the field `inner: BufWriter`, qualified at its origin as `crate::io::buffered::BufWriter`,
/// was re-expanded through the root alias to `std::io::..` and `lw_flush_zq` went ABSENT over the write (EXECUTED: the bin
/// writes the marker). (2) hashbrown's shape: three same-leaf `Entry` enums whose `Occupied(OccupiedEntry)` payloads
/// agreed as bare strings and, qualified, withdrew each other; the payload binding went untyped and `Entry::insert` lost its
/// edge and its disclosure. Both read as they did before origin qualification.
#[test]
fn r1657_a_root_alias_or_a_same_leaf_payload_does_not_untype_the_value() {
    let lib = "#[cfg(not(feature = \"std\"))]\npub mod io;\n#[cfg(feature = \"std\")]\npub use std::io as io;\n";
    let buffered = format!("pub struct BufWriter {{ pub n: u8 }}\nimpl BufWriter {{ pub fn flush(&mut self) {{ {W} }} }}\n\
pub struct LineWriter {{ inner: BufWriter }}\nimpl LineWriter {{ pub fn lw_flush_zq(&mut self) {{ self.inner.flush() }} }}\n");
    let d = fixture("rootio", &[("lib.rs", lib), ("io/mod.rs", "mod buffered;\npub use buffered::{BufWriter, LineWriter};\n"), ("io/buffered.rs", &buffered)]);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "io::buffered::LineWriter::lw_flush_zq"), ["Env", "Fs"], "{on:#}");
    let _ = std::fs::remove_dir_all(&d);
    let twin = |m: &str, eff: &str| format!("pub mod {m} {{ pub struct OccupiedEntry; impl OccupiedEntry {{ pub fn insert(&self) {{ {eff} }} }}\n\
pub enum Entry {{ Occupied(OccupiedEntry), Vacant(u8) }}\n\
impl Entry {{ pub fn ins_{m}_zq(self) {{ match self {{ Entry::Occupied(e) => e.insert(), Entry::Vacant(_) => {{}} }} }} }} }}\n");
    let src = format!("{}{}", twin("map", W), twin("set", ""));
    let d = fixture("payload", &[("lib.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    let (_, off) = scan(&d, None, true);
    let f = "map::Entry::ins_map_zq";
    assert!(!inferred(&on, f).is_empty(), "{f} must not read pure over its write {on:#}");
    assert_eq!(inferred(&on, f), inferred(&off, f), "{f}: unchanged by R1657");
    // The payload is TYPED (the module-blind leaf, as before), not left to the untyped-receiver floor: in hashbrown the
    // callee was `Unknown`-only, the floor does not fire for such a candidate, and the row went silent. Discriminates the
    // first cut, which read `dispatch:untyped receiver of insert` here.
    assert_eq!(why(&on, f), why(&off, f), "{f}: the same disclosure as before R1657 {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS R1661 — a `crate::`-rooted receiver base must not form a dependency provenance. `s.f.0.go()` over a tuple field
/// (a type the receiver walk cannot name) formed an inert `crate::<untyped>` marker that suppressed the R1095 untyped-receiver
/// floor. EXECUTED (`rustagent-stage1a/fx/m2x`, all three write the marker): on 0.41.3 `m2_qual_zq` (`s: &crate::m::S`) read
/// ABSENT; origin qualification put the bare spelling (`m2_bare_zq`) on the same path. Both now disclose.
#[test]
fn r1661_a_crate_rooted_base_keeps_the_untyped_receiver_floor() {
    let src = format!("pub struct Writer;\nimpl Writer {{ pub fn go(&self) {{ {W} }} }}\n\
pub mod m {{ use super::*; pub struct S {{ pub f: (Writer, u8) }}\nimpl S {{ pub fn m2_self_zq(&self) {{ self.f.0.go(); }} }}\n\
pub fn m2_bare_zq(s: &S) {{ s.f.0.go(); }} }}\n\
pub mod n {{ pub fn m2_qual_zq(s: &crate::m::S) {{ s.f.0.go(); }} }}\n");
    let d = fixture("m2x", &[("lib.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    for f in ["m::S::m2_self_zq", "m::m2_bare_zq", "n::m2_qual_zq"] {
        assert_eq!(inferred(&on, f), ["Unknown"], "{f}: {on:#}");
        assert!(why(&on, f).iter().any(|w| w.starts_with("dispatch:untyped receiver of `go`")), "{f}: {on:#}");
    }
    let (rc, _) = scan(&d, Some("deny Unknown n::m2_qual_zq\n"), false);
    assert_eq!(rc, 1, "the disclosure reaches the gate");
    let _ = std::fs::remove_dir_all(&d);
}

/// The second A/B's regression, pinned: two same-named statics in two modules (jiff's `DEFAULT_DATETIME_PARSER` in
/// `fmt::rfc2822` and `fmt::temporal`) typed by same-leaf types agreed as the bare leaf before origin qualification; withdrawing
/// the name left `Date::from_str` untyped and, its candidates being `Unknown`-only, ABSENT.
#[test]
fn r1657_same_named_statics_of_same_leaf_types_keep_their_type() {
    // Two FILES: the cross-file merge (`cache::merge_decls`) is the half that withdrew the name in jiff.
    let p = |eff: &str| format!("pub struct P;\nimpl P {{ pub const fn new() -> P {{ P }} pub fn parse_date(&self) {{ {eff} }} }}\n\
pub(crate) static DEFAULT_P: P = P::new();\n");
    let (rfc, temporal) = (p(""), p(W));
    let lib = "pub mod rfc;\npub mod temporal;\npub mod civil { use crate::temporal::DEFAULT_P; pub fn st_from_str_zq() { DEFAULT_P.parse_date() } }\n";
    let d = fixture("stat", &[("lib.rs", lib), ("rfc.rs", &rfc), ("temporal.rs", &temporal)]);
    let (_, on) = scan(&d, None, false);
    let (_, off) = scan(&d, None, true);
    let f = "civil::st_from_str_zq";
    assert!(!inferred(&on, f).is_empty(), "must not read pure over the write {on:#}");
    // TYPED by the static (the module-blind leaf, as before), not left to the untyped floor: in jiff the candidates were
    // `Unknown`-only and the floor did not fire. Discriminates the first cut, which read `dispatch:untyped receiver`.
    assert_eq!(why(&on, f), why(&off, f), "{f}: the same disclosure as before R1657 {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The final-arm audit's regressions, pinned (MIR checker: `LOST-STILL-CALLED`). (1) regex-automata's shape: an outer
/// `Lazy<T, F>(lazy::Lazy<T, F>)` whose `get(this)` calls `this.0.get()`, beside a same-leaf `Lazy<'i, 'c>` in ANOTHER file:
/// once `this`'s owner was proven, `deref_param_recv` read the argument `T` as a type (its parameter test used the leaf-keyed
/// fact the twin withdraws) and formed `T::get`. (2) rustc-serialize's shape: a payload written `self::Array` for a `type`
/// alias kept its alias re-expansion.
///
/// STATED LIMIT: this reduced shape reads `Unknown` on the pre-fix binary too, so it is a CONTROL, not a discriminator. The
/// discriminating reproduction is regex-automata 0.4.16's own `util/lazy.rs` (two `#[cfg]` inner `mod lazy` arms) beside the
/// twin file (lane scratch `rustagent-stage1a/fx/ralazy`: `Lazy::get`'s edge `[]` before the fix, restored after).
#[test]
fn r1657_a_proven_owner_reads_its_own_parameters_and_an_alias_payload_still_expands() {
    let lazy = "pub struct Lazy<T, F = fn() -> T>(lazy::Lazy<T, F>);\n\
impl<T, F: Fn() -> T> Lazy<T, F> { pub fn lz_get_zq(this: &Lazy<T, F>) -> &T { this.0.get() } }\n\
impl<T, F: Fn() -> T> core::ops::Deref for Lazy<T, F> { type Target = T; fn deref(&self) -> &T { Lazy::lz_get_zq(self) } }\n\
mod lazy { pub(super) struct Lazy<T, F> { pub(super) v: Option<T>, pub(super) f: F }\n\
impl<T, F: Fn() -> T> Lazy<T, F> { pub(super) fn get(&self) -> &T { let _ = (self.f)(); self.v.as_ref().unwrap() } } }\n";
    let hybrid = "pub struct Dfa;\nstruct Lazy<'i> { d: &'i Dfa }\nimpl<'i> Lazy<'i> { fn new(d: &'i Dfa) -> Lazy<'i> { Lazy { d } } fn c(&self) -> u8 { 0 } }\n\
pub fn use_it(d: &Dfa) -> u8 { Lazy::new(d).c() }\n";
    let d = fixture("lazy", &[("lib.rs", "pub mod util { pub mod lazy; }\npub mod hybrid;\n"), ("util/lazy.rs", lazy), ("hybrid.rs", hybrid)]);
    let (_, on) = scan(&d, None, false);
    let f = "util::lazy::Lazy::lz_get_zq";
    assert!(inferred(&on, f).contains(&"Unknown".to_string()), "{f} keeps its callee's Unknown {on:#}");
    let _ = std::fs::remove_dir_all(&d);
}

/// SOUNDNESS R1667 — a PROVEN owner whose own declaration has no indexed entry for a field (`#[cfg(feature = "x")] dfa:
/// dense::Builder` is not indexed) must not take a same-leaf twin's entry (`hybrid::regex::Builder { dfa: dfa::Builder }`).
/// regex-automata's shape, EXECUTED both directions (lane fixtures `r1667` / `r1667b`): the dense arm writes and the caller
/// was ABSENT on 0.41.3 (edge to the hybrid twin's pure `configure`); swapped, the pure dense caller was charged the hybrid
/// twin's `Fs`. Now refused and disclosed in both; the hybrid caller keeps its own field.
#[test]
fn r1667_a_cfg_gated_field_of_a_proven_owner_never_takes_a_twins_entry() {
    let tree = |dense_eff: &str, hy_eff: &str| -> Vec<(&'static str, String)> {
        vec![
            ("lib.rs", "pub mod dfa;\npub mod hybrid;\n".to_string()),
            ("dfa/mod.rs", "pub mod dense;\npub mod regex;\n".to_string()),
            ("dfa/dense.rs", format!("#[cfg(feature = \"x\")]\npub struct Builder {{ pub n: u8 }}\n#[cfg(feature = \"x\")]\n\
impl Builder {{ pub fn new() -> Builder {{ Builder {{ n: 0 }} }} pub fn configure(&mut self) {{ {dense_eff} }} }}\n")),
            ("dfa/regex.rs", "use crate::dfa::dense;\npub struct Builder {\n    #[cfg(feature = \"x\")]\n    dfa: dense::Builder,\n}\n\
impl Builder {\n    #[cfg(feature = \"x\")]\n    pub fn r1667_dense_zq(&mut self) -> &mut Builder { self.dfa.configure(); self }\n}\n".to_string()),
            ("hybrid/mod.rs", "pub mod dfa;\npub mod regex;\n".to_string()),
            ("hybrid/dfa.rs", format!("pub struct Builder {{ pub m: u16 }}\nimpl Builder {{ pub fn new() -> Builder {{ Builder {{ m: 0 }} }} pub fn configure(&mut self) {{ {hy_eff} }} }}\n")),
            ("hybrid/regex.rs", "use crate::hybrid::dfa;\npub struct Builder { dfa: dfa::Builder }\n\
impl Builder { pub fn r1667_hy_regex_zq(&mut self) -> &mut Builder { self.dfa.configure(); self } }\n".to_string()),
        ]
    };
    for (tag, dense_eff, hy_eff) in [("silent", W, ""), ("fabricated", "", W)] {
        let src = tree(dense_eff, hy_eff);
        let d = fixture(&format!("r1667{tag}"), &src.iter().map(|(p, s)| (*p, s.as_str())).collect::<Vec<_>>());
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[features]\ndefault = [\"x\"]\nx = []\n").unwrap();
        let (_, on) = scan(&d, None, false);
        let f = "dfa::regex::Builder::r1667_dense_zq";
        assert_eq!(inferred(&on, f), ["Unknown"], "{tag}: refused and disclosed, never the twin's effect {on:#}");
        assert!(why(&on, f).iter().any(|w| w.starts_with("ambiguous:same-name types `Builder` declare field `dfa`")), "{tag}: {on:#}");
        let hy = inferred(&on, "hybrid::regex::Builder::r1667_hy_regex_zq");
        assert_eq!(hy.is_empty(), hy_eff.is_empty(), "{tag}: the hybrid caller keeps its own field's answer {on:#}");
        let (_, off) = scan(&d, None, true);
        if tag == "fabricated" {
            assert!(inferred(&off, f).contains(&"Fs".to_string()), "CANDOR_R1657_OFF restores the fabrication");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// SOUNDNESS R1667 (second spelling) — a pattern that WRITES a dependency's enum (`dep::Data::Struct(v)`) is not a variant of
/// this crate's same-leaf enum (`ast::Data::Struct(Fields)`). darling_core's shape (`impl UsesTypeParams for syn::Data`).
/// EXECUTED (lane fixture `depenum`): the dependency arm writes nothing and was charged the local `Fields::uses`'s `Fs` on
/// 0.41.3; the local-enum arm writes and stays charged.
#[test]
fn r1667_a_dependency_enum_pattern_is_not_typed_by_a_local_same_leaf_enum() {
    let base = std::env::temp_dir().join(format!("candor-r1667-depenum-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("dep/src")).unwrap();
    std::fs::create_dir_all(base.join("app/src")).unwrap();
    std::fs::write(base.join("dep/Cargo.toml"), "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
    std::fs::write(base.join("dep/src/lib.rs"), "pub struct DataStruct; pub enum Data { Struct(DataStruct), Other(u8) }\n").unwrap();
    std::fs::write(base.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ndep = { path = \"../dep\" }\n").unwrap();
    std::fs::write(base.join("app/src/lib.rs"), format!("pub mod ast {{ pub struct Fields; impl Fields {{ pub fn uses(&self) {{ {W} }} }}\n\
pub enum Data {{ Struct(Fields), Other(u8) }} }}\npub trait Uses {{ fn uses(&self); }}\nimpl Uses for dep::DataStruct {{ fn uses(&self) {{}} }}\n\
pub fn depenum_zq(d: &dep::Data) {{ match d {{ dep::Data::Struct(v) => v.uses(), dep::Data::Other(_) => {{}} }} }}\n\
pub fn localenum_zq(d: &ast::Data) {{ match d {{ ast::Data::Struct(v) => v.uses(), ast::Data::Other(_) => {{}} }} }}\n")).unwrap();
    let (_, on) = scan(&base.join("app"), None, false);
    assert!(!inferred(&on, "depenum_zq").contains(&"Fs".to_string()), "the local twin's effect is not the dependency payload's {on:#}");
    assert_eq!(inferred(&on, "localenum_zq"), ["Env", "Fs"], "{on:#}");
    let (_, off) = scan(&base.join("app"), None, true);
    assert!(inferred(&off, "depenum_zq").contains(&"Fs".to_string()), "CANDOR_R1657_OFF restores the fabrication");
    let _ = std::fs::remove_dir_all(&base);
}

/// SOUNDNESS R1667 (third spelling) — the ONE-segment dependency enum pattern, `use dep::Data; Data::Struct(v)`, beside a local
/// `ast::Data { Struct(Fields) }`. EXECUTED (lane fixtures `depenum1a`, `depenum1b`, `cargo run`): on bb24901 and before, `v` was
/// typed by the local enum's payload, so the caller was charged the local `Fields::uses`' `Fs` over a pure dependency impl (a) and
/// read ABSENT over an effectful one (b). Now untyped, so the untyped-receiver floor discloses; `CANDOR_R1667_OFF` restores both.
#[test]
fn r1667_a_one_segment_dependency_enum_pattern_is_not_typed_by_a_local_same_leaf_enum() {
    for (tag, local_writes) in [("a", true), ("b", false)] {
        let base = std::env::temp_dir().join(format!("candor-r1667-depenum1{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("dep/src")).unwrap();
        std::fs::create_dir_all(base.join("app/src")).unwrap();
        std::fs::write(base.join("dep/Cargo.toml"), "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(base.join("dep/src/lib.rs"), "pub struct DataStruct; pub enum Data { Struct(DataStruct), Other(u8) }\n").unwrap();
        std::fs::write(base.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ndep = { path = \"../dep\" }\n").unwrap();
        let (lw, dw) = if local_writes { (W, "") } else { ("", W) };
        std::fs::write(base.join("app/src/lib.rs"), format!("pub mod ast {{ pub struct Fields; impl Fields {{ pub fn uses(&self) {{ {lw} }} }}\n\
pub enum Data {{ Struct(Fields), Other(u8) }} }}\npub trait Uses {{ fn uses(&self); }}\nimpl Uses for dep::DataStruct {{ fn uses(&self) {{ {dw} }} }}\n\
pub mod user {{ use crate::Uses; use dep::Data;\n\
pub fn onesegment_zq(d: &Data) {{ match d {{ Data::Struct(v) => v.uses(), Data::Other(_) => {{}} }} }} }}\n")).unwrap();
        let f = "user::onesegment_zq";
        let (_, on) = scan(&base.join("app"), None, false);
        assert_eq!(inferred(&on, f), ["Unknown"], "{tag}: {on:#}");
        assert!(why(&on, f).iter().any(|w| w.starts_with("dispatch:untyped receiver of `uses`")), "{tag}: {on:#}");
        let out = Command::new(bin()).arg(base.join("app").to_string_lossy().as_ref()).arg("--json")
            .env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env("CANDOR_R1667_OFF", "1")
            .output().expect("run candor-scan");
        let off: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
        if local_writes {
            assert!(inferred(&off, f).contains(&"Fs".to_string()), "CANDOR_R1667_OFF restores the fabrication {off:#}");
        } else {
            assert!(inferred(&off, f).is_empty(), "CANDOR_R1667_OFF restores the silence {off:#}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}

/// SOUNDNESS R1660 — a chain hop whose DECLARED return was withdrawn (same-leaf twins declare different returns) is a guessed
/// hop, like one declared to return another type (R1663). lettre's `conn.unpark().abort()`: sync `ParkedConnection::unpark ->
/// SmtpConnection` beside an async twin of the same leaf. Once R1657 typed `conn` to its own module's twin, the `ambiguous:` hedge
/// that covered the site was gone, and the walk's guess `ParkedConnection::abort` named no unit: ABSENT over a write. EXECUTED
/// (lane fixture `reexret`, `cargo run` writes the marker); disclosed through the untyped-receiver floor now.
#[test]
fn r1660_a_withdrawn_hop_return_is_a_guessed_hop() {
    let d = fixture("r1660", &[("lib.rs", &format!("pub mod smtp {{
    pub mod client {{
        pub use self::{{ connection::SmtpConnection, }};
        mod connection {{
            pub struct SmtpConnection(pub u8);
            impl SmtpConnection {{ pub fn abort(&mut self) {{ {W} }} }}
        }}
    }}
    pub mod pool {{
        pub mod sync_impl {{
            use super::{{super::client::SmtpConnection}};
            pub struct ParkedConnection {{ conn: SmtpConnection }}
            impl ParkedConnection {{ fn unpark(self) -> SmtpConnection {{ self.conn }} }}
            pub fn hop_zq(conn: ParkedConnection) {{ conn.unpark().abort(); }}
            pub fn shadow_zq(conn: ParkedConnection) {{ let mut conn = conn.unpark(); conn.abort(); }}
        }}
        pub mod async_impl {{
            pub struct ParkedConnection(u8);
            impl ParkedConnection {{ pub fn unpark(self) -> u8 {{ self.0 }} }}
        }}
    }}
}}
"))]);
    let (_, on) = scan(&d, None, false);
    let out = Command::new(bin()).arg(d.to_string_lossy().as_ref()).arg("--json")
        .env_remove("CANDOR_DEPS").env_remove("CANDOR_CONFIG").env_remove("CANDOR_POLICY").env("CANDOR_R1660_OFF", "1")
        .output().expect("run candor-scan");
    let off: serde_json::Value = serde_json::from_slice(&out.stdout).expect("report json");
    // the chained spelling (R1663's floor) and the bound self-shadowing one (`let mut conn = conn.unpark()`, lettre's Drop)
    for f in ["smtp::pool::sync_impl::hop_zq", "smtp::pool::sync_impl::shadow_zq"] {
        assert_eq!(inferred(&on, f), ["Unknown"], "{f} {on:#}");
        assert!(why(&on, f).iter().any(|w| w.starts_with("dispatch:untyped receiver of `abort`")), "{f} {on:#}");
        assert!(inferred(&off, f).is_empty(), "CANDOR_R1660_OFF restores the silence for {f} {off:#}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
