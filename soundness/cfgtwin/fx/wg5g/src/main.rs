#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_wg5gd::{Flat, FlatQ, BaseS, BaseSQ, SubS, SubSQ};
pub struct Mine; pub struct Quiet;
impl Flat for Mine { fn f_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn f_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }   impl FlatQ for Quiet { fn f_go(&self) {} fn f_fn() {} }
impl BaseS for Mine { fn s_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn s_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }  impl BaseSQ for Quiet { fn s_go(&self) {} fn s_fn() {} }
impl SubS for Mine {}  impl SubSQ for Quiet {}
impl fxdep_wg5gp::LSubD for Mine {}  impl fxdep_wg5gp::LSubDQ for Quiet {}
impl fxdep_wg5gp::LSup for Mine { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }  impl fxdep_wg5gp::LSupQ for Quiet { fn lsf() {} fn lsm(&self) {} }
impl fxdep_wg5gp::LSub for Mine {}  impl fxdep_wg5gp::LSubQ for Quiet {}
pub struct ItE_wg5g; impl Iterator for ItE_wg5g { type Item = Mine; fn next(&mut self) -> Option<Mine> { None } }
pub struct ItQ_wg5g; impl Iterator for ItQ_wg5g { type Item = Quiet; fn next(&mut self) -> Option<Quiet> { None } }
pub fn wg5g_flat_e() { fxdep_wg5gp::p_flat_e(&Mine); }
pub fn wg5g_flat_q() { fxdep_wg5gp::p_flat_q(&Quiet); }
pub fn wg5g_flatfn_e() { fxdep_wg5gp::p_flatfn_e::<Mine>(); }
pub fn wg5g_flatfn_q() { fxdep_wg5gp::p_flatfn_q::<Quiet>(); }
pub fn wg5g_supmeth_e() { fxdep_wg5gp::p_supmeth_e(&Mine); }
pub fn wg5g_supmeth_q() { fxdep_wg5gp::p_supmeth_q(&Quiet); }
pub fn wg5g_suppath_e() { fxdep_wg5gp::p_suppath_e::<Mine>(); }
pub fn wg5g_suppath_q() { fxdep_wg5gp::p_suppath_q::<Quiet>(); }
pub fn wg5g_supqself_e() { fxdep_wg5gp::p_supqself_e::<Mine>(); }
pub fn wg5g_supqself_q() { fxdep_wg5gp::p_supqself_q::<Quiet>(); }
pub fn wg5g_lsdmeth_e() { fxdep_wg5gp::p_lsdmeth_e(&Mine); }
pub fn wg5g_lsdmeth_q() { fxdep_wg5gp::p_lsdmeth_q(&Quiet); }
pub fn wg5g_lsdpath_e() { fxdep_wg5gp::p_lsdpath_e::<Mine>(); }
pub fn wg5g_lsdpath_q() { fxdep_wg5gp::p_lsdpath_q::<Quiet>(); }
pub fn wg5g_proj_e() { fxdep_wg5gp::p_proj_e::<ItE_wg5g>(); }
pub fn wg5g_proj_q() { fxdep_wg5gp::p_proj_q::<ItQ_wg5g>(); }
pub fn wg5g_lsup_e() { fxdep_wg5gp::p_lsup_e::<Mine>(); }
pub fn wg5g_lsup_q() { fxdep_wg5gp::p_lsup_q::<Quiet>(); }
pub fn wg5g_lsupm_e() { fxdep_wg5gp::p_lsupm_e(&Mine); }
pub fn wg5g_lsupm_q() { fxdep_wg5gp::p_lsupm_q(&Quiet); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wg5g_flat_e" => wg5g_flat_e(),
        "wg5g_flat_q" => wg5g_flat_q(),
        "wg5g_flatfn_e" => wg5g_flatfn_e(),
        "wg5g_flatfn_q" => wg5g_flatfn_q(),
        "wg5g_supmeth_e" => wg5g_supmeth_e(),
        "wg5g_supmeth_q" => wg5g_supmeth_q(),
        "wg5g_suppath_e" => wg5g_suppath_e(),
        "wg5g_suppath_q" => wg5g_suppath_q(),
        "wg5g_supqself_e" => wg5g_supqself_e(),
        "wg5g_supqself_q" => wg5g_supqself_q(),
        "wg5g_lsdmeth_e" => wg5g_lsdmeth_e(),
        "wg5g_lsdmeth_q" => wg5g_lsdmeth_q(),
        "wg5g_lsdpath_e" => wg5g_lsdpath_e(),
        "wg5g_lsdpath_q" => wg5g_lsdpath_q(),
        "wg5g_proj_e" => wg5g_proj_e(),
        "wg5g_proj_q" => wg5g_proj_q(),
        "wg5g_lsup_e" => wg5g_lsup_e(),
        "wg5g_lsup_q" => wg5g_lsup_q(),
        "wg5g_lsupm_e" => wg5g_lsupm_e(),
        "wg5g_lsupm_q" => wg5g_lsupm_q(),
        _ => panic!("unknown probe"),
    }
}
