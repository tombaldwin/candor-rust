#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_bm1::{Dt, SupD, SubD, HasA};
pub trait Lt { fn lf(); fn lm(&self); }
pub trait LSup { fn lsf(); } pub trait LSub: LSup {}
pub trait LSubD: fxdep_bm1::SupD {}
pub struct LoudL; impl Lt for LoudL { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct QuietL; impl Lt for QuietL { fn lf() {} fn lm(&self) {} }
impl Lt for String { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct LoudD; impl fxdep_bm1::Dt for LoudD { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn m(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct QuietD; impl fxdep_bm1::Dt for QuietD { fn f() {} fn m(&self) {} }
pub struct LS; impl LSup for LS { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub for LS {}
pub struct OnlySup; impl LSup for OnlySup { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct LSQ; impl LSup for LSQ { fn lsf() {} } impl LSub for LSQ {}
pub struct LDS; impl fxdep_bm1::SupD for LDS { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn sm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubD for LDS {} impl fxdep_bm1::SubD for LDS {}
pub struct LoudFs; impl std::str::FromStr for LoudFs { type Err = (); fn from_str(_s: &str) -> Result<Self, ()> { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Ok(LoudFs) } }
pub struct LoudDef; impl Default for LoudDef { fn default() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); LoudDef } }
pub struct LoudIt; impl Iterator for LoudIt { type Item = u8; fn next(&mut self) -> Option<u8> { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); None } }
// B1 local bound
fn g_b1_path<T: Lt>() { T::lf() }      pub fn bm1_b1_path_e() { g_b1_path::<LoudL>() }
fn g_b1_pathq<T: Lt>() { T::lf() }     pub fn bm1_b1_path_q() { g_b1_pathq::<QuietL>() }
fn g_b1_meth<T: Lt>(t: &T) { t.lm() }  pub fn bm1_b1_meth_e() { g_b1_meth(&LoudL) }
fn g_b1_qs<T: Lt>() { <T as Lt>::lf() } pub fn bm1_b1_qself_e() { g_b1_qs::<LoudL>() }
fn g_b1_ufcs<T: Lt>(t: &T) { Lt::lm(t) } pub fn bm1_b1_ufcs_e() { g_b1_ufcs(&LoudL) }
fn g_b1_std<T: Lt>() { T::lf() }        pub fn bm1_b1_stdimpl_e() { g_b1_std::<String>() }
// B2 dependency bound
fn g_b2_path_in<T: Dt>() { T::f() }     pub fn bm1_b2_path_incrate_e() { g_b2_path_in::<LoudD>() }
fn g_b2_path_inq<T: Dt>() { T::f() }    pub fn bm1_b2_path_incrate_q() { g_b2_path_inq::<QuietD>() }
fn g_b2_path_dep<T: Dt>() { T::f() }    pub fn bm1_b2_path_depty_e() { g_b2_path_dep::<fxdep_bm1::DImpE>() }
fn g_b2_path_std<T: Dt>() { T::f() }    pub fn bm1_b2_path_stdty_e() { g_b2_path_std::<String>() }
fn g_b2_path_stdq<T: Dt>() { T::f() }   pub fn bm1_b2_path_stdty_q() { g_b2_path_stdq::<u16>() }
fn g_b2_meth_in<T: Dt>(t: &T) { t.m() } pub fn bm1_b2_meth_incrate_e() { g_b2_meth_in(&LoudD) }
fn g_b2_qs_in<T: Dt>() { <T as Dt>::f() } pub fn bm1_b2_qself_incrate_e() { g_b2_qs_in::<LoudD>() }
fn g_b2_ufcs_in<T: Dt>(t: &T) { Dt::m(t) } pub fn bm1_b2_ufcs_incrate_e() { g_b2_ufcs_in(&LoudD) }
// B3 std bound, in-crate implementor of the std trait (R1955 class; README documented miss)
fn g_b3_fromstr<T: std::str::FromStr>() { let _ = T::from_str("x"); } pub fn bm1_b3_path_fromstr_e() { g_b3_fromstr::<LoudFs>() }
fn g_b3_fromstrq<T: std::str::FromStr>() { let _ = T::from_str("1"); } pub fn bm1_b3_path_fromstr_q() { g_b3_fromstrq::<u8>() }
fn g_b3_default<T: Default>() { let _ = T::default(); } pub fn bm1_b3_path_default_e() { g_b3_default::<LoudDef>() }
fn g_b3_next<T: Iterator>(mut t: T) { t.next(); } pub fn bm1_b3_meth_next_e() { g_b3_next(LoudIt) }
fn g_b3_nextu<T: Iterator>(mut t: T) { Iterator::next(&mut t); } pub fn bm1_b3_ufcs_next_e() { g_b3_nextu(LoudIt) }
fn g_b3_nextq<T: Iterator>(mut t: T) { <T as Iterator>::next(&mut t); } pub fn bm1_b3_qself_next_e() { g_b3_nextq(LoudIt) }
// B4 mixed: dependency + local bound
fn g_b4_loc<T: Dt + Lt>() { T::lf() }   pub fn bm1_b4_mixed_local_e() { g_b4_loc::<String>() }
fn g_b4_dep<T: Dt + Lt>() { T::f() }    pub fn bm1_b4_mixed_dep_e() { g_b4_dep::<String>() }
fn g_b4_def<T: Lt + Default>() { T::lf() } pub fn bm1_b4_mixed_std_e() { g_b4_def::<String>() }
// B5 supertrait: local, dependency, and a local subtrait over a dependency supertrait
fn g_b5_loc<T: LSub>() { T::lsf() }     pub fn bm1_b5_localsup_e() { g_b5_loc::<LS>() }
fn g_b5_locq<T: LSub>() { T::lsf() }    pub fn bm1_b5_localsup_q() { g_b5_locq::<LSQ>() }
fn g_b5_dep<T: SubD>() { T::sf() }      pub fn bm1_b5_depsup_e() { g_b5_dep::<fxdep_bm1::DSupE>() }
fn g_b5_depin<T: SubD>() { T::sf() }    pub fn bm1_b5_depsup_incrate_e() { g_b5_depin::<LDS>() }
fn g_b5_depm<T: SubD>(t: &T) { t.sm() } pub fn bm1_b5_depsup_meth_e() { g_b5_depm(&fxdep_bm1::DSupE) }
fn g_b5_mix<T: LSubD>() { T::sf() }     pub fn bm1_b5_localsub_depsup_e() { g_b5_mix::<LDS>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bm1_b1_path_e" => bm1_b1_path_e(),
        "bm1_b1_path_q" => bm1_b1_path_q(),
        "bm1_b1_meth_e" => bm1_b1_meth_e(),
        "bm1_b1_qself_e" => bm1_b1_qself_e(),
        "bm1_b1_ufcs_e" => bm1_b1_ufcs_e(),
        "bm1_b1_stdimpl_e" => bm1_b1_stdimpl_e(),
        "bm1_b2_path_incrate_e" => bm1_b2_path_incrate_e(),
        "bm1_b2_path_incrate_q" => bm1_b2_path_incrate_q(),
        "bm1_b2_path_depty_e" => bm1_b2_path_depty_e(),
        "bm1_b2_path_stdty_e" => bm1_b2_path_stdty_e(),
        "bm1_b2_path_stdty_q" => bm1_b2_path_stdty_q(),
        "bm1_b2_meth_incrate_e" => bm1_b2_meth_incrate_e(),
        "bm1_b2_qself_incrate_e" => bm1_b2_qself_incrate_e(),
        "bm1_b2_ufcs_incrate_e" => bm1_b2_ufcs_incrate_e(),
        "bm1_b3_path_fromstr_e" => bm1_b3_path_fromstr_e(),
        "bm1_b3_path_fromstr_q" => bm1_b3_path_fromstr_q(),
        "bm1_b3_path_default_e" => bm1_b3_path_default_e(),
        "bm1_b3_meth_next_e" => bm1_b3_meth_next_e(),
        "bm1_b3_ufcs_next_e" => bm1_b3_ufcs_next_e(),
        "bm1_b3_qself_next_e" => bm1_b3_qself_next_e(),
        "bm1_b4_mixed_local_e" => bm1_b4_mixed_local_e(),
        "bm1_b4_mixed_dep_e" => bm1_b4_mixed_dep_e(),
        "bm1_b4_mixed_std_e" => bm1_b4_mixed_std_e(),
        "bm1_b5_localsup_e" => bm1_b5_localsup_e(),
        "bm1_b5_localsup_q" => bm1_b5_localsup_q(),
        "bm1_b5_depsup_e" => bm1_b5_depsup_e(),
        "bm1_b5_depsup_incrate_e" => bm1_b5_depsup_incrate_e(),
        "bm1_b5_depsup_meth_e" => bm1_b5_depsup_meth_e(),
        "bm1_b5_localsub_depsup_e" => bm1_b5_localsub_depsup_e(),
        _ => panic!("unknown probe"),
    }
}
