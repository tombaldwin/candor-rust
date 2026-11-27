use fxdep_fl40::Af;
#[derive(Clone)] pub struct Loud_fl40; impl fxdep_fl40::Af for Loud_fl40 { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Quiet_fl40; impl fxdep_fl40::Af for Quiet_fl40 { fn f() {} }
pub trait Lt_fl40 { fn lf(); fn lm(&self) {} }
pub struct LoudL_fl40; impl fxdep_fl40::Af for LoudL_fl40 { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt_fl40 for LoudL_fl40 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct QuietL_fl40; impl Lt_fl40 for QuietL_fl40 { fn lf() {} }
pub struct ItL_fl40; impl Iterator for ItL_fl40 { type Item = Loud_fl40; fn next(&mut self) -> Option<Loud_fl40> { None } }
pub struct ItQ_fl40; impl Iterator for ItQ_fl40 { type Item = Quiet_fl40; fn next(&mut self) -> Option<Quiet_fl40> { None } }
#[derive(Clone)] pub struct ItLL_fl40; impl Iterator for ItLL_fl40 { type Item = LoudL_fl40; fn next(&mut self) -> Option<LoudL_fl40> { None } }
pub struct ItQL_fl40; impl Iterator for ItQL_fl40 { type Item = QuietL_fl40; fn next(&mut self) -> Option<QuietL_fl40> { None } }
pub struct IiLL_fl40; impl IntoIterator for IiLL_fl40 { type Item = LoudL_fl40; type IntoIter = std::vec::IntoIter<LoudL_fl40>; fn into_iter(self) -> Self::IntoIter { Vec::new().into_iter() } }
pub struct IiQL_fl40; impl IntoIterator for IiQL_fl40 { type Item = QuietL_fl40; type IntoIter = std::vec::IntoIter<QuietL_fl40>; fn into_iter(self) -> Self::IntoIter { Vec::new().into_iter() } }
pub struct DrL_fl40; impl std::ops::Deref for DrL_fl40 { type Target = LoudL_fl40; fn deref(&self) -> &LoudL_fl40 { &LoudL_fl40 } }
pub struct DrQ_fl40; impl std::ops::Deref for DrQ_fl40 { type Target = QuietL_fl40; fn deref(&self) -> &QuietL_fl40 { &QuietL_fl40 } }
pub trait Sub_fl40: Lt_fl40 {}
impl Sub_fl40 for LoudL_fl40 {} impl Sub_fl40 for QuietL_fl40 {}
impl LoudL_fl40 { }
pub trait Hl_fl40 { type A; }
impl Hl_fl40 for ItLL_fl40 { type A = LoudL_fl40; }
pub struct FsL_fl40; impl std::str::FromStr for FsL_fl40 { type Err = (); fn from_str(_: &str) -> Result<Self, ()> { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Ok(FsL_fl40) } }
pub struct ItFs_fl40; impl Iterator for ItFs_fl40 { type Item = FsL_fl40; fn next(&mut self) -> Option<FsL_fl40> { None } }
pub struct W_fl40<T>(std::marker::PhantomData<T>);
impl<T: Iterator> W_fl40<T> where T::Item: Lt_fl40 { pub fn go() { T::Item::lf() } }
mod g_fl40 {
    use fxdep_fl40::Af; use crate::Lt_fl40; use std::str::FromStr;
    pub fn std_dep<T: Iterator>() where T::Item: fxdep_fl40::Af { T::Item::f() }
    pub fn std_loc<T: Iterator>() where T::Item: crate::Lt_fl40 { T::Item::lf() }
    pub fn std_inl<T: Iterator<Item: crate::Lt_fl40>>() { T::Item::lf() }
    pub fn std_into<T: IntoIterator>() where T::Item: crate::Lt_fl40 { T::Item::lf() }
    pub fn std_deref<T: std::ops::Deref>() where T::Target: crate::Lt_fl40 { T::Target::lf() }
    pub fn std_qself<T: Iterator>() where T::Item: crate::Lt_fl40 { <T::Item as crate::Lt_fl40>::lf() }
    pub fn std_two<T: Iterator + Clone>() where T::Item: crate::Lt_fl40 { T::Item::lf() }
    pub fn bind<T: Iterator<Item = crate::LoudL_fl40>>() { T::Item::lf() }
    pub fn bind_q<T: Iterator<Item = crate::QuietL_fl40>>() { T::Item::lf() }
    pub fn loc_dep<T: Iterator>() where T::Item: crate::Lt_fl40 + fxdep_fl40::Af { T::Item::lf() }
    pub fn dep_clone<T: Iterator>() where T::Item: fxdep_fl40::Af + Clone { T::Item::f() }
    pub fn sup<T: Iterator>() where T::Item: crate::Sub_fl40 { T::Item::lf() }
    pub fn recv<T: Iterator>(x: T::Item) where T::Item: crate::Lt_fl40 { T::Item::lm(&x) }
    pub fn fromstr<T: Iterator>() where T::Item: std::str::FromStr { let _ = T::Item::from_str("x"); }
    pub fn fromstr_head<U: std::str::FromStr>() { let _ = U::from_str("x"); }
    pub fn qs0<T: Iterator>() where T::Item: crate::Lt_fl40 { <T::Item>::lf() }
    pub fn lochead<T: crate::Hl_fl40>() where T::A: crate::Lt_fl40 { T::A::lf() }
    pub fn ctl_head<U: fxdep_fl40::Af>() { U::f() }
    pub fn ctl_dep_qself<T: Iterator>() where T::Item: fxdep_fl40::Af { <T::Item as fxdep_fl40::Af>::f() }
}

pub fn fl40_dep_e() { g_fl40::std_dep::<ItL_fl40>(); }
pub fn fl40_dep_q() { g_fl40::std_dep::<ItQ_fl40>(); }
pub fn fl40_loc_e() { g_fl40::std_loc::<ItLL_fl40>(); }
pub fn fl40_loc_q() { g_fl40::std_loc::<ItQL_fl40>(); }
pub fn fl40_inl_e() { g_fl40::std_inl::<ItLL_fl40>(); }
pub fn fl40_into_e() { g_fl40::std_into::<IiLL_fl40>(); }
pub fn fl40_into_q() { g_fl40::std_into::<IiQL_fl40>(); }
pub fn fl40_deref_e() { g_fl40::std_deref::<DrL_fl40>(); }
pub fn fl40_deref_q() { g_fl40::std_deref::<DrQ_fl40>(); }
pub fn fl40_qself_e() { g_fl40::std_qself::<ItLL_fl40>(); }
pub fn fl40_two_e() { g_fl40::std_two::<ItLL_fl40>(); }
pub fn fl40_impl_e() { W_fl40::<ItLL_fl40>::go(); }
pub fn fl40_bind_e() { g_fl40::bind::<ItLL_fl40>(); }
pub fn fl40_bind_q() { g_fl40::bind_q::<ItQL_fl40>(); }
pub fn fl40_locdep_e() { g_fl40::loc_dep::<ItLL_fl40>(); }
pub fn fl40_depclone_e() { g_fl40::dep_clone::<ItL_fl40>(); }
pub fn fl40_sup_e() { g_fl40::sup::<ItLL_fl40>(); }
pub fn fl40_qs0_e() { g_fl40::qs0::<ItLL_fl40>(); }
pub fn fl40_fromstr_e() { g_fl40::fromstr::<ItFs_fl40>(); }
pub fn fl40_fromstrhead_e() { g_fl40::fromstr_head::<FsL_fl40>(); }
pub fn fl40_lochead_e() { g_fl40::lochead::<ItLL_fl40>(); }
pub fn fl40_recv_e() { g_fl40::recv::<ItLL_fl40>(LoudL_fl40); }
pub fn fl40_ctlhead_e() { g_fl40::ctl_head::<Loud_fl40>(); }
pub fn fl40_ctlhead_q() { g_fl40::ctl_head::<Quiet_fl40>(); }
pub fn fl40_ctlqs_e() { g_fl40::ctl_dep_qself::<ItL_fl40>(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl40_dep_e" => fl40_dep_e(),
        "fl40_dep_q" => fl40_dep_q(),
        "fl40_loc_e" => fl40_loc_e(),
        "fl40_loc_q" => fl40_loc_q(),
        "fl40_inl_e" => fl40_inl_e(),
        "fl40_into_e" => fl40_into_e(),
        "fl40_into_q" => fl40_into_q(),
        "fl40_deref_e" => fl40_deref_e(),
        "fl40_deref_q" => fl40_deref_q(),
        "fl40_qself_e" => fl40_qself_e(),
        "fl40_two_e" => fl40_two_e(),
        "fl40_impl_e" => fl40_impl_e(),
        "fl40_bind_e" => fl40_bind_e(),
        "fl40_bind_q" => fl40_bind_q(),
        "fl40_locdep_e" => fl40_locdep_e(),
        "fl40_depclone_e" => fl40_depclone_e(),
        "fl40_sup_e" => fl40_sup_e(),
        "fl40_qs0_e" => fl40_qs0_e(),
        "fl40_fromstr_e" => fl40_fromstr_e(),
        "fl40_fromstrhead_e" => fl40_fromstrhead_e(),
        "fl40_lochead_e" => fl40_lochead_e(),
        "fl40_recv_e" => fl40_recv_e(),
        "fl40_ctlhead_e" => fl40_ctlhead_e(),
        "fl40_ctlhead_q" => fl40_ctlhead_q(),
        "fl40_ctlqs_e" => fl40_ctlqs_e(),
        _ => panic!("unknown probe"),
    }
}
