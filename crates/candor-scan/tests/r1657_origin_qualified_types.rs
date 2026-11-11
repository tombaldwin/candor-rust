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
/// not declared there, so it is not proven. Published: ABSENT over the write (the last twin's pure field). Now: refused and
/// disclosed with a true reason; the declaring path proves its owner and reads pure.
#[test]
fn r1657_an_unproven_reexport_owner_is_disclosed_not_picked() {
    let src = format!("mod runtime {{ mod handle {{ pub struct EngR; impl EngR {{ pub fn run(&self) {{ {W} }} }} pub struct Handle {{ pub inner: EngR }} }}\n\
pub use handle::{{EngR, Handle}}; }}\n\
mod other {{ pub struct EngO; impl EngO {{ pub fn run(&self) {{}} }} pub struct Handle {{ pub inner: EngO }} }}\n\
mod c {{ pub fn rex_param_zq(h: &crate::runtime::Handle) {{ h.inner.run(); }}\npub fn rex_other_zq(h: &crate::other::Handle) {{ h.inner.run(); }} }}\n\
fn main() {{ c::rex_param_zq(&runtime::Handle {{ inner: runtime::EngR }}); c::rex_other_zq(&other::Handle {{ inner: other::EngO }}); }}\n");
    let d = fixture("rex", &[("main.rs", &src)]);
    let (_, on) = scan(&d, None, false);
    assert_eq!(inferred(&on, "c::rex_param_zq"), ["Unknown"], "{on:#}");
    assert!(why(&on, "c::rex_param_zq").iter().any(|w| w.starts_with("ambiguous:same-name types `Handle`")), "{on:#}");
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
