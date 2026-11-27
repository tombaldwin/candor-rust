#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub trait LSubD_bl2: fxdep_bl2::SupD {}
pub struct Inh_bl2; impl fxdep_bl2::SupD for Inh_bl2 {} impl LSubD_bl2 for Inh_bl2 {}
pub struct Decl_bl2; impl fxdep_bl2::SupD for Decl_bl2 { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD_bl2 for Decl_bl2 {}
fn g_inh<T: LSubD_bl2>() { T::sf() }
fn g_decl<T: LSubD_bl2>() { T::sf() }
pub fn bl2_inherit_e() { g_inh::<Inh_bl2>() }
pub fn bl2_declares_e() { g_decl::<Decl_bl2>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl2_inherit_e" => bl2_inherit_e(),
        "bl2_declares_e" => bl2_declares_e(),
        _ => panic!("unknown probe"),
    }
}
