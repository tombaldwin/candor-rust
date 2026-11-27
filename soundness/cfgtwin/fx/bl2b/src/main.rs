#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_bl2b::SupD;
pub trait LSubD_bl2b: SupD {}
pub struct Inh_bl2b; impl SupD for Inh_bl2b {} impl LSubD_bl2b for Inh_bl2b {}
pub struct Decl_bl2b; impl SupD for Decl_bl2b { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD_bl2b for Decl_bl2b {}
pub struct DeclQ_bl2b; impl SupD for DeclQ_bl2b { fn sf() {} } impl LSubD_bl2b for DeclQ_bl2b {}
fn g_inh<T: LSubD_bl2b>() { T::sf() }
fn g_decl<T: LSubD_bl2b>() { T::sf() }
fn g_declq<T: LSubD_bl2b>() { T::sf() }
pub fn bl2b_inherit_e() { g_inh::<Inh_bl2b>() }
pub fn bl2b_declares_e() { g_decl::<Decl_bl2b>() }
pub fn bl2b_declares_q() { g_declq::<DeclQ_bl2b>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl2b_inherit_e" => bl2b_inherit_e(),
        "bl2b_declares_e" => bl2b_declares_e(),
        "bl2b_declares_q" => bl2b_declares_q(),
        _ => panic!("unknown probe"),
    }
}
