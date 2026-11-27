#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_bm4::{Dt, SupD, SubD};
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub trait LSubD: fxdep_bm4::SupD {}
pub struct LS; impl LSup for LS { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub for LS {}
pub struct LDS; impl fxdep_bm4::SupD for LDS { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn sm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD for LDS {}
fn g4_lsm<T: LSub>(t: &T) { t.lsm() }        pub fn bm4_localsup_meth_e() { g4_lsm(&LS) }
fn g4_lsq<T: LSub>() { <T as LSup>::lsf() }  pub fn bm4_localsup_qself_e() { g4_lsq::<LS>() }
fn g4_mixm<T: LSubD>(t: &T) { t.sm() }       pub fn bm4_localsub_depsup_meth_e() { g4_mixm(&LDS) }
fn g4_mixq<T: LSubD>() { <T as SupD>::sf() } pub fn bm4_localsub_depsup_qself_e() { g4_mixq::<LDS>() }
fn g4_depq<T: SubD>() { <T as SupD>::sf() }  pub fn bm4_depsup_qself_e() { g4_depq::<fxdep_bm4::DSupE>() }
mod h { use fxdep_bm4::Dt; pub fn e1() { let s = String::new(); s.m(); } pub fn e2() { String::new().m(); } }
pub fn bm4_meth_dt_let_e() { h::e1() }
pub fn bm4_meth_dt_ctor_e() { h::e2() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bm4_localsup_meth_e" => bm4_localsup_meth_e(),
        "bm4_localsup_qself_e" => bm4_localsup_qself_e(),
        "bm4_localsub_depsup_meth_e" => bm4_localsub_depsup_meth_e(),
        "bm4_localsub_depsup_qself_e" => bm4_localsub_depsup_qself_e(),
        "bm4_depsup_qself_e" => bm4_depsup_qself_e(),
        "bm4_meth_dt_let_e" => bm4_meth_dt_let_e(),
        "bm4_meth_dt_ctor_e" => bm4_meth_dt_ctor_e(),
        _ => panic!("unknown probe"),
    }
}
