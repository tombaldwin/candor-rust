#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub trait Lt { fn lf(); fn lm(&self); }
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub trait HasL { type A: Lt; }
pub struct I0_wg1; impl Lt for I0_wg1 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct I1_wg1; impl Lt for I1_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I2_wg1; impl Lt for I2_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I3_wg1; impl Lt for I3_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I4_wg1; impl Lt for I4_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I5_wg1; impl Lt for I5_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I6_wg1; impl Lt for I6_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I7_wg1; impl Lt for I7_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I8_wg1; impl Lt for I8_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I9_wg1; impl Lt for I9_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I10_wg1; impl Lt for I10_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I11_wg1; impl Lt for I11_wg1 { fn lf() {} fn lm(&self) {} }
pub struct I12_wg1; impl Lt for I12_wg1 { fn lf() {} fn lm(&self) {} }
pub struct S0_wg1; impl LSup for S0_wg1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub for S0_wg1 {}
pub struct S1_wg1; impl LSup for S1_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S1_wg1 {}
pub struct S2_wg1; impl LSup for S2_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S2_wg1 {}
pub struct S3_wg1; impl LSup for S3_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S3_wg1 {}
pub struct S4_wg1; impl LSup for S4_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S4_wg1 {}
pub struct S5_wg1; impl LSup for S5_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S5_wg1 {}
pub struct S6_wg1; impl LSup for S6_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S6_wg1 {}
pub struct S7_wg1; impl LSup for S7_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S7_wg1 {}
pub struct S8_wg1; impl LSup for S8_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S8_wg1 {}
pub struct S9_wg1; impl LSup for S9_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S9_wg1 {}
pub struct S10_wg1; impl LSup for S10_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S10_wg1 {}
pub struct S11_wg1; impl LSup for S11_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S11_wg1 {}
pub struct S12_wg1; impl LSup for S12_wg1 { fn lsf() {} fn lsm(&self) {} } impl LSub for S12_wg1 {}
pub struct It0_wg1; impl Iterator for It0_wg1 { type Item = I0_wg1; fn next(&mut self) -> Option<I0_wg1> { None } }
pub struct It1_wg1; impl Iterator for It1_wg1 { type Item = I1_wg1; fn next(&mut self) -> Option<I1_wg1> { None } }
pub struct H0_wg1; impl HasL for H0_wg1 { type A = I0_wg1; }
pub struct H1_wg1; impl HasL for H1_wg1 { type A = I1_wg1; }
fn g_path_e<T: Lt>() { T::lf() }
pub fn wg1_path_e() { g_path_e::<I0_wg1>(); }
fn g_path_q<T: Lt>() { T::lf() }
pub fn wg1_path_q() { g_path_q::<I1_wg1>(); }
fn g_meth_e<T: Lt>(t: &T) { t.lm() }
pub fn wg1_meth_e() { g_meth_e(&I0_wg1); }
fn g_meth_q<T: Lt>(t: &T) { t.lm() }
pub fn wg1_meth_q() { g_meth_q(&I1_wg1); }
fn g_qself_e<T: Lt>() { <T as Lt>::lf() }
pub fn wg1_qself_e() { g_qself_e::<I0_wg1>(); }
fn g_qself_q<T: Lt>() { <T as Lt>::lf() }
pub fn wg1_qself_q() { g_qself_q::<I1_wg1>(); }
fn g_ufcs_e<T: Lt>(t: &T) { Lt::lm(t) }
pub fn wg1_ufcs_e() { g_ufcs_e(&I0_wg1); }
fn g_ufcs_q<T: Lt>(t: &T) { Lt::lm(t) }
pub fn wg1_ufcs_q() { g_ufcs_q(&I1_wg1); }
fn g_suppath_e<T: LSub>() { T::lsf() }
pub fn wg1_suppath_e() { g_suppath_e::<S0_wg1>(); }
fn g_suppath_q<T: LSub>() { T::lsf() }
pub fn wg1_suppath_q() { g_suppath_q::<S1_wg1>(); }
fn g_supmeth_e<T: LSub>(t: &T) { t.lsm() }
pub fn wg1_supmeth_e() { g_supmeth_e(&S0_wg1); }
fn g_supmeth_q<T: LSub>(t: &T) { t.lsm() }
pub fn wg1_supmeth_q() { g_supmeth_q(&S1_wg1); }
fn g_proj_e<T: Iterator>() where T::Item: Lt { T::Item::lf() }
pub fn wg1_proj_e() { g_proj_e::<It0_wg1>(); }
fn g_proj_q<T: Iterator>() where T::Item: Lt { T::Item::lf() }
pub fn wg1_proj_q() { g_proj_q::<It1_wg1>(); }
fn g_projm_e<T: Iterator>(mut t: T) where T::Item: Lt { if let Some(x) = t.next() { x.lm() } }
pub fn wg1_projm_e() { g_projm_e(std::iter::once(I0_wg1)); }
fn g_projm_q<T: Iterator>(mut t: T) where T::Item: Lt { if let Some(x) = t.next() { x.lm() } }
pub fn wg1_projm_q() { g_projm_q(std::iter::once(I1_wg1)); }
fn g_decl_e<T: HasL>() { T::A::lf() }
pub fn wg1_decl_e() { g_decl_e::<H0_wg1>(); }
fn g_decl_q<T: HasL>() { T::A::lf() }
pub fn wg1_decl_q() { g_decl_q::<H1_wg1>(); }
fn g_qa_e<T: HasL>() { <T as HasL>::A::lf() }
pub fn wg1_qa_e() { g_qa_e::<H0_wg1>(); }
fn g_qa_q<T: HasL>() { <T as HasL>::A::lf() }
pub fn wg1_qa_q() { g_qa_q::<H1_wg1>(); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "wg1_path_e" => wg1_path_e(),
        "wg1_path_q" => wg1_path_q(),
        "wg1_meth_e" => wg1_meth_e(),
        "wg1_meth_q" => wg1_meth_q(),
        "wg1_qself_e" => wg1_qself_e(),
        "wg1_qself_q" => wg1_qself_q(),
        "wg1_ufcs_e" => wg1_ufcs_e(),
        "wg1_ufcs_q" => wg1_ufcs_q(),
        "wg1_suppath_e" => wg1_suppath_e(),
        "wg1_suppath_q" => wg1_suppath_q(),
        "wg1_supmeth_e" => wg1_supmeth_e(),
        "wg1_supmeth_q" => wg1_supmeth_q(),
        "wg1_proj_e" => wg1_proj_e(),
        "wg1_proj_q" => wg1_proj_q(),
        "wg1_projm_e" => wg1_projm_e(),
        "wg1_projm_q" => wg1_projm_q(),
        "wg1_decl_e" => wg1_decl_e(),
        "wg1_decl_q" => wg1_decl_q(),
        "wg1_qa_e" => wg1_qa_e(),
        "wg1_qa_q" => wg1_qa_q(),
        _ => panic!("unknown probe"),
    }
}
