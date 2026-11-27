#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// R1977 — a dependency supertrait (fxdep_lb1::Sup) shares its LEAF with a crate-local trait (m_lb1::Sup). Only the dependency's can run.
pub mod m_lb1 { pub trait Sup { fn lsm(&self); fn lsf() where Self: Sized; } }
pub trait LSub_lb1: fxdep_lb1::Sup {}
pub trait LSubL_lb1: m_lb1::Sup {}
pub struct Noisy_lb1; impl m_lb1::Sup for Noisy_lb1 { fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubL_lb1 for Noisy_lb1 {}
pub struct Quiet_lb1; impl fxdep_lb1::Sup for Quiet_lb1 { fn lsm(&self) {} fn lsf() {} } impl LSub_lb1 for Quiet_lb1 {}
pub struct Loud_lb1; impl fxdep_lb1::Sup for Loud_lb1 { fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub_lb1 for Loud_lb1 {}
pub struct ItQ_lb1; impl Iterator for ItQ_lb1 { type Item = Quiet_lb1; fn next(&mut self) -> Option<Quiet_lb1> { None } }
pub struct ItL_lb1; impl Iterator for ItL_lb1 { type Item = Loud_lb1; fn next(&mut self) -> Option<Loud_lb1> { None } }
fn g_meth_q<T: LSub_lb1>(t: T) { t.lsm() }
fn g_meth_e<T: LSub_lb1>(t: T) { t.lsm() }
fn g_methuse_q<T: LSub_lb1>(t: T) { use fxdep_lb1::Sup; t.lsm() }
fn g_methuse_e<T: LSub_lb1>(t: T) { use fxdep_lb1::Sup; t.lsm() }
fn g_where_q<T>(t: &T) where T: LSub_lb1 { t.lsm() }
fn g_where_e<T>(t: &T) where T: LSub_lb1 { t.lsm() }
fn g_dyn_q(d: &dyn LSub_lb1) { d.lsm() }
fn g_dyn_e(d: &dyn LSub_lb1) { d.lsm() }
fn g_proj_q<T: Iterator>() where T::Item: LSub_lb1 { use fxdep_lb1::Sup; T::Item::lsf() }
fn g_proj_e<T: Iterator>() where T::Item: LSub_lb1 { use fxdep_lb1::Sup; T::Item::lsf() }
fn g_projm_q<T: Iterator>(x: T::Item) where T::Item: LSub_lb1 { use fxdep_lb1::Sup; x.lsm() }
fn g_projm_e<T: Iterator>(x: T::Item) where T::Item: LSub_lb1 { use fxdep_lb1::Sup; x.lsm() }
fn g_local_e<T: LSubL_lb1>(t: T) { t.lsm() }
fn g_localpath_e<T: LSubL_lb1>() { T::lsf() }
fn g_localdyn_e(d: &dyn LSubL_lb1) { d.lsm() }
pub fn lb1_meth_q() { g_meth_q(Quiet_lb1) }
pub fn lb1_meth_e() { g_meth_e(Loud_lb1) }
pub fn lb1_methuse_q() { g_methuse_q(Quiet_lb1) }
pub fn lb1_methuse_e() { g_methuse_e(Loud_lb1) }
pub fn lb1_where_q() { g_where_q(&Quiet_lb1) }
pub fn lb1_where_e() { g_where_e(&Loud_lb1) }
pub fn lb1_dyn_q() { g_dyn_q(&Quiet_lb1) }
pub fn lb1_dyn_e() { g_dyn_e(&Loud_lb1) }
pub fn lb1_proj_q() { g_proj_q::<ItQ_lb1>() }
pub fn lb1_proj_e() { g_proj_e::<ItL_lb1>() }
pub fn lb1_projm_q() { g_projm_q::<ItQ_lb1>(Quiet_lb1) }
pub fn lb1_projm_e() { g_projm_e::<ItL_lb1>(Loud_lb1) }
pub fn lb1_local_e() { g_local_e(Noisy_lb1) }
pub fn lb1_localpath_e() { g_localpath_e::<Noisy_lb1>() }
pub fn lb1_localdyn_e() { g_localdyn_e(&Noisy_lb1) }
// R1846's spelling: `Self::lsf()` / `self.lsm()` in a DEFAULT body of a subtrait whose supertrait is the dependency's same-leaf trait
pub trait LSubD_lb1: fxdep_lb1::Sup { fn go(&self) where Self: Sized { Self::lsf() } fn gm(&self) { self.lsm() } }
pub struct QuietD_lb1; impl fxdep_lb1::Sup for QuietD_lb1 { fn lsm(&self) {} fn lsf() {} } impl LSubD_lb1 for QuietD_lb1 {}
pub struct LoudD_lb1; impl fxdep_lb1::Sup for LoudD_lb1 { fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD_lb1 for LoudD_lb1 {}
fn g_selfp_q<T: LSubD_lb1>(t: T) { t.go() }
fn g_selfp_e<T: LSubD_lb1>(t: T) { t.go() }
fn g_selfm_q<T: LSubD_lb1>(t: T) { t.gm() }
fn g_selfm_e<T: LSubD_lb1>(t: T) { t.gm() }
pub fn lb1_selfp_q() { g_selfp_q(QuietD_lb1) }
pub fn lb1_selfp_e() { g_selfp_e(LoudD_lb1) }
pub fn lb1_selfm_q() { g_selfm_q(QuietD_lb1) }
pub fn lb1_selfm_e() { g_selfm_e(LoudD_lb1) }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lb1_meth_q" => lb1_meth_q(),
        "lb1_meth_e" => lb1_meth_e(),
        "lb1_methuse_q" => lb1_methuse_q(),
        "lb1_methuse_e" => lb1_methuse_e(),
        "lb1_where_q" => lb1_where_q(),
        "lb1_where_e" => lb1_where_e(),
        "lb1_dyn_q" => lb1_dyn_q(),
        "lb1_dyn_e" => lb1_dyn_e(),
        "lb1_proj_q" => lb1_proj_q(),
        "lb1_proj_e" => lb1_proj_e(),
        "lb1_projm_q" => lb1_projm_q(),
        "lb1_projm_e" => lb1_projm_e(),
        "lb1_local_e" => lb1_local_e(),
        "lb1_localpath_e" => lb1_localpath_e(),
        "lb1_localdyn_e" => lb1_localdyn_e(),
        "lb1_selfp_q" => lb1_selfp_q(),
        "lb1_selfp_e" => lb1_selfp_e(),
        "lb1_selfm_q" => lb1_selfm_q(),
        "lb1_selfm_e" => lb1_selfm_e(),
        _ => panic!("unknown probe"),
    }
}
