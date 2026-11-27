#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_wg5d::{Flat, FlatQ, BaseS, BaseSQ, SubS, SubSQ};
pub struct Mine; pub struct Quiet;
impl Flat for Mine { fn f_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn f_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }   impl FlatQ for Quiet { fn f_go(&self) {} fn f_fn() {} }
impl BaseS for Mine { fn s_go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn s_fn() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }  impl BaseSQ for Quiet { fn s_go(&self) {} fn s_fn() {} }
impl SubS for Mine {}  impl SubSQ for Quiet {}
impl fxdep_wg5p::LSubD for Mine {}  impl fxdep_wg5p::LSubDQ for Quiet {}
impl fxdep_wg5p::LSup for Mine { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }  impl fxdep_wg5p::LSupQ for Quiet { fn lsf() {} fn lsm(&self) {} }
impl fxdep_wg5p::LSub for Mine {}  impl fxdep_wg5p::LSubQ for Quiet {}
pub struct ItE_wg5; impl Iterator for ItE_wg5 { type Item = Mine; fn next(&mut self) -> Option<Mine> { None } }
pub struct ItQ_wg5; impl Iterator for ItQ_wg5 { type Item = Quiet; fn next(&mut self) -> Option<Quiet> { None } }
pub fn wg5_flat_e() { fxdep_wg5p::p_flat_e(&Mine); }
pub fn wg5_flat_q() { fxdep_wg5p::p_flat_q(&Quiet); }
pub fn wg5_flatfn_e() { fxdep_wg5p::p_flatfn_e::<Mine>(); }
pub fn wg5_flatfn_q() { fxdep_wg5p::p_flatfn_q::<Quiet>(); }
pub fn wg5_supmeth_e() { fxdep_wg5p::p_supmeth_e(&Mine); }
pub fn wg5_supmeth_q() { fxdep_wg5p::p_supmeth_q(&Quiet); }
pub fn wg5_suppath_e() { fxdep_wg5p::p_suppath_e::<Mine>(); }
pub fn wg5_suppath_q() { fxdep_wg5p::p_suppath_q::<Quiet>(); }
pub fn wg5_supqself_e() { fxdep_wg5p::p_supqself_e::<Mine>(); }
pub fn wg5_supqself_q() { fxdep_wg5p::p_supqself_q::<Quiet>(); }
pub fn wg5_lsdmeth_e() { fxdep_wg5p::p_lsdmeth_e(&Mine); }
pub fn wg5_lsdmeth_q() { fxdep_wg5p::p_lsdmeth_q(&Quiet); }
pub fn wg5_lsdpath_e() { fxdep_wg5p::p_lsdpath_e::<Mine>(); }
pub fn wg5_lsdpath_q() { fxdep_wg5p::p_lsdpath_q::<Quiet>(); }
pub fn wg5_proj_e() { fxdep_wg5p::p_proj_e::<ItE_wg5>(); }
pub fn wg5_proj_q() { fxdep_wg5p::p_proj_q::<ItQ_wg5>(); }
pub fn wg5_lsup_e() { fxdep_wg5p::p_lsup_e::<Mine>(); }
pub fn wg5_lsup_q() { fxdep_wg5p::p_lsup_q::<Quiet>(); }
pub fn wg5_lsupm_e() { fxdep_wg5p::p_lsupm_e(&Mine); }
pub fn wg5_lsupm_q() { fxdep_wg5p::p_lsupm_q(&Quiet); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wg5_flat_e" => wg5_flat_e(),
        "wg5_flat_q" => wg5_flat_q(),
        "wg5_flatfn_e" => wg5_flatfn_e(),
        "wg5_flatfn_q" => wg5_flatfn_q(),
        "wg5_supmeth_e" => wg5_supmeth_e(),
        "wg5_supmeth_q" => wg5_supmeth_q(),
        "wg5_suppath_e" => wg5_suppath_e(),
        "wg5_suppath_q" => wg5_suppath_q(),
        "wg5_supqself_e" => wg5_supqself_e(),
        "wg5_supqself_q" => wg5_supqself_q(),
        "wg5_lsdmeth_e" => wg5_lsdmeth_e(),
        "wg5_lsdmeth_q" => wg5_lsdmeth_q(),
        "wg5_lsdpath_e" => wg5_lsdpath_e(),
        "wg5_lsdpath_q" => wg5_lsdpath_q(),
        "wg5_proj_e" => wg5_proj_e(),
        "wg5_proj_q" => wg5_proj_q(),
        "wg5_lsup_e" => wg5_lsup_e(),
        "wg5_lsup_q" => wg5_lsup_q(),
        "wg5_lsupm_e" => wg5_lsupm_e(),
        "wg5_lsupm_q" => wg5_lsupm_q(),
        _ => panic!("unknown probe"),
    }
}
