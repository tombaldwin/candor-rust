#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub trait Mk {}
pub trait Lt { fn lf(); fn lm(&self); }
pub struct M_wg2; impl Mk for M_wg2 {}
impl<X: Mk> Lt for X { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct P_wg2; impl Lt for P_wg2 { fn lf() {} fn lm(&self) {} }
pub trait Ln { fn nf(); fn nm(&self); }
pub struct N0_wg2; impl Ln for N0_wg2 { fn nf() {} fn nm(&self) {} }
pub struct N1_wg2; impl Ln for N1_wg2 { fn nf() {} fn nm(&self) {} }
// blanket of D (declaring) and of B (bound) in a supertrait pair
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub struct LM_wg2; impl Mk for LM_wg2 {} impl LSup for LM_wg2 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl<X: Mk + LSup> LSub for X {}                              // blanket of the BOUND B; D (LSup) is explicit
pub trait LSup2 { fn lsf2(); fn lsm2(&self); }  pub trait LSub2: LSup2 {}
impl<X: Mk> LSup2 for X { fn lsf2() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm2(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }                  // blanket of the DECLARING trait D
impl LSub2 for M_wg2 {}
pub trait LSup3 { fn lsf3(); fn lsm3(&self); }  pub trait LSub3: LSup3 {}
impl<X: Mk> LSup3 for X { fn lsf3() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm3(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }                  // blanket of BOTH
impl<X: Mk> LSub3 for X {}
pub trait LSupn { fn nsf(); } pub trait LSubn: LSupn {}
pub struct SN0_wg2; impl LSupn for SN0_wg2 { fn nsf() {} } impl LSubn for SN0_wg2 {}
pub struct It_wg2; impl Iterator for It_wg2 { type Item = M_wg2; fn next(&mut self) -> Option<M_wg2> { None } }
pub struct Itn_wg2; impl Iterator for Itn_wg2 { type Item = N0_wg2; fn next(&mut self) -> Option<N0_wg2> { None } }
fn g_path<T: Lt>() { T::lf() }                   pub fn wg2_path_e() { g_path::<M_wg2>() }
fn g_pathn<T: Ln>() { T::nf() }                  pub fn wg2_path_q() { g_pathn::<N0_wg2>() }
fn g_meth<T: Lt>(t: &T) { t.lm() }               pub fn wg2_meth_e() { g_meth(&M_wg2) }
fn g_methn<T: Ln>(t: &T) { t.nm() }              pub fn wg2_meth_q() { g_methn(&N1_wg2) }
fn g_qself<T: Lt>() { <T as Lt>::lf() }          pub fn wg2_qself_e() { g_qself::<M_wg2>() }
fn g_qselfn<T: Ln>() { <T as Ln>::nf() }         pub fn wg2_qself_q() { g_qselfn::<N0_wg2>() }
fn g_ufcs<T: Lt>(t: &T) { Lt::lm(t) }            pub fn wg2_ufcs_e() { g_ufcs(&M_wg2) }
fn g_bb<T: LSub>() { T::lsf() }                  pub fn wg2_blanketB_e() { g_bb::<LM_wg2>() }
fn g_bbm<T: LSub>(t: &T) { t.lsm() }             pub fn wg2_blanketB_meth_e() { g_bbm(&LM_wg2) }
fn g_bd<T: LSub2>() { T::lsf2() }                pub fn wg2_blanketD_e() { g_bd::<M_wg2>() }
fn g_bboth<T: LSub3>() { T::lsf3() }             pub fn wg2_blanketBoth_e() { g_bboth::<M_wg2>() }
fn g_bdm<T: LSub2>(t: &T) { t.lsm2() }           pub fn wg2_blanketD_meth_e() { g_bdm(&M_wg2) }
fn g_bbothm<T: LSub3>(t: &T) { t.lsm3() }        pub fn wg2_blanketBoth_meth_e() { g_bbothm(&M_wg2) }
fn g_supn<T: LSubn>() { T::nsf() }               pub fn wg2_blanketB_q() { g_supn::<SN0_wg2>() }
fn g_proj<I: Iterator>() where I::Item: Lt { I::Item::lf() }   pub fn wg2_proj_e() { g_proj::<It_wg2>() }
fn g_projn<I: Iterator>() where I::Item: Ln { I::Item::nf() }  pub fn wg2_proj_q() { g_projn::<Itn_wg2>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wg2_path_e" => wg2_path_e(),
        "wg2_path_q" => wg2_path_q(),
        "wg2_meth_e" => wg2_meth_e(),
        "wg2_meth_q" => wg2_meth_q(),
        "wg2_qself_e" => wg2_qself_e(),
        "wg2_qself_q" => wg2_qself_q(),
        "wg2_ufcs_e" => wg2_ufcs_e(),
        "wg2_blanketB_e" => wg2_blanketB_e(),
        "wg2_blanketB_meth_e" => wg2_blanketB_meth_e(),
        "wg2_blanketD_e" => wg2_blanketD_e(),
        "wg2_blanketBoth_e" => wg2_blanketBoth_e(),
        "wg2_blanketD_meth_e" => wg2_blanketD_meth_e(),
        "wg2_blanketBoth_meth_e" => wg2_blanketBoth_meth_e(),
        "wg2_blanketB_q" => wg2_blanketB_q(),
        "wg2_proj_e" => wg2_proj_e(),
        "wg2_proj_q" => wg2_proj_q(),
        _ => panic!("unknown probe"),
    }
}
