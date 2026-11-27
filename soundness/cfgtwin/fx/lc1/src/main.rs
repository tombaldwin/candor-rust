#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// A supertrait written with a LEADING `::` names an EXTERN crate in edition 2018+, so `::fxdep_lc1::Sup` is the DEPENDENCY's trait even beside a
// local `mod fxdep_lc1` declaring a trait of the same leaf (whose implementor writes). Only dependency implementors are instantiated.
pub mod fxdep_lc1 { pub trait Sup { fn lsf() where Self: Sized; fn lsm(&self); } }
pub trait LSub_lc1: ::fxdep_lc1::Sup {}
pub struct Noisy_lc1; impl fxdep_lc1::Sup for Noisy_lc1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Quiet_lc1; impl ::fxdep_lc1::Sup for Quiet_lc1 { fn lsf() {} } impl LSub_lc1 for Quiet_lc1 {}
pub struct Loud_lc1; impl ::fxdep_lc1::Sup for Loud_lc1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub_lc1 for Loud_lc1 {}
fn g_lc1<T: LSub_lc1>() { T::lsf() }
fn g_lc1q<T: LSub_lc1>() { T::lsf() }
fn gm_lc1<T: LSub_lc1>(t: T) { use ::fxdep_lc1::Sup; t.lsm() }
fn gmq_lc1<T: LSub_lc1>(t: T) { use ::fxdep_lc1::Sup; t.lsm() }
// the control that must stay LOCAL: the same shape written WITHOUT the leading colon, over the local trait, instantiated with a writing local implementor
pub trait LSubL_lc1: self::fxdep_lc1::Sup {}
impl LSubL_lc1 for Noisy_lc1 {}
fn gl_lc1<T: LSubL_lc1>() { T::lsf() }
pub fn lc1_colon_q() { g_lc1q::<Quiet_lc1>() }
pub fn lc1_colon_e() { g_lc1::<Loud_lc1>() }
pub fn lc1_colonm_q() { gmq_lc1(Quiet_lc1) }
pub fn lc1_colonm_e() { gm_lc1(Loud_lc1) }
pub fn lc1_local_e() { gl_lc1::<Noisy_lc1>() }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lc1_colon_q" => lc1_colon_q(),
        "lc1_colon_e" => lc1_colon_e(),
        "lc1_colonm_q" => lc1_colonm_q(),
        "lc1_colonm_e" => lc1_colonm_e(),
        "lc1_local_e" => lc1_local_e(),
        _ => panic!("unknown probe"),
    }
}
