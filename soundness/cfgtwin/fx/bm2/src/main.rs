#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_bm2::{Dt, SupD, SubD, HasA};
pub trait Lt { fn lf(); fn lm(&self); }
pub trait HasL { type A: Lt; }
pub trait LSup { fn lsf(); } pub trait LSub: LSup {}
pub struct LoudL; impl Lt for LoudL { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct QuietL; impl Lt for QuietL { fn lf() {} fn lm(&self) {} }
impl Lt for fxdep_bm2::DImpE { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct HL; impl HasL for HL { type A = LoudL; }
pub struct HLq; impl HasL for HLq { type A = QuietL; }
pub struct LoudD; impl fxdep_bm2::Dt for LoudD { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn m(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct ItL(Option<LoudL>); impl Iterator for ItL { type Item = LoudL; fn next(&mut self) -> Option<LoudL> { None } }
pub struct ItD; impl Iterator for ItD { type Item = LoudD; fn next(&mut self) -> Option<LoudD> { None } }
pub struct ItDS; impl Iterator for ItDS { type Item = fxdep_bm2::DSupE; fn next(&mut self) -> Option<fxdep_bm2::DSupE> { None } }
pub struct ItS; impl Iterator for ItS { type Item = String; fn next(&mut self) -> Option<String> { None } }
pub struct LS; impl LSup for LS { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub for LS {}
pub struct ItLS; impl Iterator for ItLS { type Item = LS; fn next(&mut self) -> Option<LS> { None } }
// P-local declaration (R1037)
fn g_p_ldecl<T: HasL>() { T::A::lf() }         pub fn bm2_localdecl_path_e() { g_p_ldecl::<HL>() }
fn g_p_ldeclq<T: HasL>() { T::A::lf() }        pub fn bm2_localdecl_path_q() { g_p_ldeclq::<HLq>() }
fn g_p_ldecl_qs<T: HasL>() { <T::A as Lt>::lf() } pub fn bm2_localdecl_qpa_e() { g_p_ldecl_qs::<HL>() }
fn g_p_ldecl_q0<T: HasL>() { <T::A>::lf() }    pub fn bm2_localdecl_q0_e() { g_p_ldecl_q0::<HL>() }
fn g_p_ldecl_m<T: HasL>(a: T::A) { a.lm() }    pub fn bm2_localdecl_meth_e() { g_p_ldecl_m::<HL>(LoudL) }
fn g_p_ldecl_qa<T: HasL>() { <T as HasL>::A::lf() } pub fn bm2_localdecl_qa_e() { g_p_ldecl_qa::<HL>() }
// std head, local bound on the projection (R1940, closed)
fn g_p_stdloc<T: Iterator>() where T::Item: Lt { T::Item::lf() } pub fn bm2_stdhead_localproj_e() { g_p_stdloc::<ItL>() }
fn g_p_stdloc_m<T: Iterator>(mut t: T) where T::Item: Lt { if let Some(x) = t.next() { x.lm() } } pub fn bm2_stdhead_localproj_meth_e() { g_p_stdloc_m(std::iter::once(LoudL)) }
// std head, fxdep_bm2ENDENCY bound on the projection
fn g_p_stddep<T: Iterator>() where T::Item: Dt { T::Item::f() } pub fn bm2_stdhead_depproj_incrate_e() { g_p_stddep::<ItD>() }
fn g_p_stddep_sup<T: Iterator>() where T::Item: SubD { T::Item::sf() } pub fn bm2_stdhead_depsupproj_e() { g_p_stddep_sup::<ItDS>() }
fn g_p_stddep_c<T: Iterator<Item: Dt>>() { T::Item::f() } pub fn bm2_stdhead_depconstraint_e() { g_p_stddep_c::<ItD>() }
// std head, local SUBTRAIT bound on the projection, member in the local supertrait (R1953 shape, implementor implements both)
fn g_p_locsup<T: Iterator>() where T::Item: LSub { T::Item::lsf() } pub fn bm2_stdhead_localsupproj_e() { g_p_locsup::<ItLS>() }
// dependency head, local bound on projection (R1939, fc34_loc_e) and dependency head, the dependency's own projection
fn g_p_deploc<T: HasA>() where T::A: Lt { T::A::lf() } pub fn bm2_dephead_localproj_e() { g_p_deploc::<fxdep_bm2::DHas>() }
fn g_p_depown<T: HasA>() where T::A: Dt { T::A::f() } pub fn bm2_dephead_depproj_e() { g_p_depown::<fxdep_bm2::DHas>() }
fn g_p_depbare<T: HasA>() { let _ = 0; } pub fn bm2_dephead_noproj_q() { g_p_depbare::<fxdep_bm2::DHas>() }
// written binding
fn g_p_bindstd<T: Iterator<Item = String>>() { let _ = <T::Item as fxdep_bm2::Parse>::from_str("x"); } pub fn bm2_bind_stdty_named_e() { g_p_bindstd::<ItS>() }
fn g_p_bindstd2<T: Iterator<Item = String>>() { use fxdep_bm2::Parse; let _ = T::Item::from_str("x"); } pub fn bm2_bind_stdty_scope_e() { g_p_bindstd2::<ItS>() }
fn g_p_binddep<T: Iterator<Item = fxdep_bm2::DImpE>>() { T::Item::lf() } pub fn bm2_bind_depty_localtrait_e() { g_p_binddep::<std::iter::Empty<fxdep_bm2::DImpE>>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bm2_localdecl_path_e" => bm2_localdecl_path_e(),
        "bm2_localdecl_path_q" => bm2_localdecl_path_q(),
        "bm2_localdecl_qpa_e" => bm2_localdecl_qpa_e(),
        "bm2_localdecl_q0_e" => bm2_localdecl_q0_e(),
        "bm2_localdecl_meth_e" => bm2_localdecl_meth_e(),
        "bm2_localdecl_qa_e" => bm2_localdecl_qa_e(),
        "bm2_stdhead_localproj_e" => bm2_stdhead_localproj_e(),
        "bm2_stdhead_localproj_meth_e" => bm2_stdhead_localproj_meth_e(),
        "bm2_stdhead_depproj_incrate_e" => bm2_stdhead_depproj_incrate_e(),
        "bm2_stdhead_depsupproj_e" => bm2_stdhead_depsupproj_e(),
        "bm2_stdhead_depconstraint_e" => bm2_stdhead_depconstraint_e(),
        "bm2_stdhead_localsupproj_e" => bm2_stdhead_localsupproj_e(),
        "bm2_dephead_localproj_e" => bm2_dephead_localproj_e(),
        "bm2_dephead_depproj_e" => bm2_dephead_depproj_e(),
        "bm2_dephead_noproj_q" => bm2_dephead_noproj_q(),
        "bm2_bind_stdty_named_e" => bm2_bind_stdty_named_e(),
        "bm2_bind_stdty_scope_e" => bm2_bind_stdty_scope_e(),
        "bm2_bind_depty_localtrait_e" => bm2_bind_depty_localtrait_e(),
        _ => panic!("unknown probe"),
    }
}
