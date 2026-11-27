#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_la1::{SupD, HasA};
pub trait Lt { fn lf(); }
pub trait HasL { type A: Lt; }  pub trait HasLS: HasL {}
pub trait HasX { type A; }
pub trait LSup { fn lsf(); } pub trait LMid: LSup {} pub trait LSub2: LMid {} pub trait LSub: LSup {}
pub trait HasS { type A: LSub; }
pub trait LSubD: fxdep_la1::SupD {}  pub trait HasD { type A: LSubD; }
pub trait LSubN: fxdep_la1::NoSf {}  pub trait LsfHave { fn sf(); }
pub struct LoudL; impl Lt for LoudL { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct QuietL_la1; impl Lt for QuietL_la1 { fn lf() {} }
pub struct HL; impl HasL for HL { type A = LoudL; } impl HasLS for HL {}
pub struct HX; impl HasX for HX { type A = LoudL; }
pub struct ItL; impl Iterator for ItL { type Item = LoudL; fn next(&mut self) -> Option<LoudL> { None } }
pub struct LS2; impl LSup for LS2 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LMid for LS2 {} impl LSub2 for LS2 {} impl LSub for LS2 {}
pub struct HS; impl HasS for HS { type A = LS2; }
pub struct LDS; impl fxdep_la1::SupD for LDS { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD for LDS {}
pub struct HD; impl HasD for HD { type A = LDS; }
pub struct LN; impl fxdep_la1::NoSf for LN {} impl LSubN for LN {} impl LsfHave for LN { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct ItS; impl Iterator for ItS { type Item = String; fn next(&mut self) -> Option<String> { None } }
pub struct ItU; impl Iterator for ItU { type Item = u16; fn next(&mut self) -> Option<u16> { None } }
pub struct ItDE; impl Iterator for ItDE { type Item = fxdep_la1::DImpE; fn next(&mut self) -> Option<fxdep_la1::DImpE> { None } }
pub struct Rd; impl std::io::Read for Rd { fn read(&mut self, _b: &mut [u8]) -> std::io::Result<usize> { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Ok(0) } }
fn g_bindstd<T: Iterator<Item = String>>() { use fxdep_la1::Dt; T::Item::f() }
fn g_bindstdq<T: Iterator<Item = u16>>() { use fxdep_la1::Dt; T::Item::f() }
fn g_concrete() { use fxdep_la1::Dt; String::f() }
fn g_qa_std<T: Iterator>() where T::Item: Lt { <T as Iterator>::Item::lf() }
fn g_qa_stdq<T: Iterator>() where T::Item: Lt { <T as Iterator>::Item::lf() }
fn g_qa_sup<T: HasLS>() { <T as HasL>::A::lf() }
fn g_qa_dep<T: HasA>() where T::A: fxdep_la1::Dt { use fxdep_la1::Dt; <T as HasA>::A::f() }
fn g_qa_bind<T: HasX<A = LoudL>>() { <T as HasX>::A::lf() }
fn g_qa_where<T: HasX>() where T::A: Lt { <T as HasX>::A::lf() }
fn g_n2proj<T: HasD>() { T::A::sf() }
fn g_n2guard<T: LSubN + LsfHave>() { T::sf() }
fn g_n1two<T: LSub2>() { T::lsf() }
fn g_n1proj<T: HasS>() { T::A::lsf() }
fn g_qself_read<R: std::io::Read>(mut r: R) { let mut b = [0u8; 1]; let _ = <R as std::io::Read>::read(&mut r, &mut b); }
fn g_ufcs_read<R: std::io::Read>(mut r: R) { let mut b = [0u8; 1]; let _ = std::io::Read::read(&mut r, &mut b); }
fn g_r1952_bare<I: Iterator>() where I::Item: fxdep_la1::Error { use fxdep_la1::Error; I::Item::ef() }
fn g_r1952_bare2<I: Iterator>() where I::Item: Error { I::Item::ef() }
fn g_collide<I: Iterator>() where I::Item: fxdep_la1::Lt { use fxdep_la1::Lt as DLt; I::Item::lf() }
use fxdep_la1::Error;
pub fn la1_bindstd_e() { g_bindstd::<ItS>() }
pub fn la1_bindstd_q() { g_bindstdq::<ItU>() }
pub fn la1_concrete_e() { g_concrete() }
pub fn la1_qa_std_e() { g_qa_std::<ItL>() }
pub fn la1_qa_std_q() { g_qa_stdq::<std::iter::Empty<QuietL_la1>>() }
pub fn la1_qa_sup_e() { g_qa_sup::<HL>() }
pub fn la1_qa_dep_e() { g_qa_dep::<fxdep_la1::DHas>() }
pub fn la1_qa_bind_e() { g_qa_bind::<HX>() }
pub fn la1_qa_where_e() { g_qa_where::<HX>() }
pub fn la1_n2proj_e() { g_n2proj::<HD>() }
pub fn la1_n2guard_e() { g_n2guard::<LN>() }
pub fn la1_n1two_e() { g_n1two::<LS2>() }
pub fn la1_n1proj_e() { g_n1proj::<HS>() }
pub fn la1_qself_read_e() { g_qself_read(Rd) }
pub fn la1_ufcs_read_e() { g_ufcs_read(Rd) }
pub fn la1_r1952_bare_e() { g_r1952_bare::<ItDE>() }
pub fn la1_r1952_bare2_e() { g_r1952_bare2::<ItDE>() }
pub fn la1_collide_e() { g_collide::<ItDE>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "la1_bindstd_e" => la1_bindstd_e(),
        "la1_bindstd_q" => la1_bindstd_q(),
        "la1_concrete_e" => la1_concrete_e(),
        "la1_qa_std_e" => la1_qa_std_e(),
        "la1_qa_std_q" => la1_qa_std_q(),
        "la1_qa_sup_e" => la1_qa_sup_e(),
        "la1_qa_dep_e" => la1_qa_dep_e(),
        "la1_qa_bind_e" => la1_qa_bind_e(),
        "la1_qa_where_e" => la1_qa_where_e(),
        "la1_n2proj_e" => la1_n2proj_e(),
        "la1_n2guard_e" => la1_n2guard_e(),
        "la1_n1two_e" => la1_n1two_e(),
        "la1_n1proj_e" => la1_n1proj_e(),
        "la1_qself_read_e" => la1_qself_read_e(),
        "la1_ufcs_read_e" => la1_ufcs_read_e(),
        "la1_r1952_bare_e" => la1_r1952_bare_e(),
        "la1_r1952_bare2_e" => la1_r1952_bare2_e(),
        "la1_collide_e" => la1_collide_e(),
        _ => panic!("unknown probe"),
    }
}
