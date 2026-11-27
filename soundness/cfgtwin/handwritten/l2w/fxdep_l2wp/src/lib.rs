use fxdep_l2wd::{BaseS, BaseSQ};
pub trait LSub2: fxdep_l2wd::SupA + fxdep_l2wd::SupB {}
pub trait LSub2Q: fxdep_l2wd::SupAQ + fxdep_l2wd::SupBQ {}
pub trait Lt5: fxdep_l2wd::SupD {}
pub trait Lt5Q: fxdep_l2wd::SupDQ {}
pub fn p_flat_e<X: fxdep_l2wd::Flat>(x: &X) { x.f_go() }
pub fn p_flat_q<X: fxdep_l2wd::FlatQ>(x: &X) { x.f_go() }
pub fn p_supmeth_e<X: fxdep_l2wd::SubS>(x: &X) { x.s_go() }
pub fn p_supmeth_q<X: fxdep_l2wd::SubSQ>(x: &X) { x.s_go() }
pub fn p_suppath_e<X: fxdep_l2wd::SubS>() { X::s_fn() }
pub fn p_suppath_q<X: fxdep_l2wd::SubSQ>() { X::s_fn() }
pub fn p_supqself_e<X: fxdep_l2wd::SubS>() { <X as fxdep_l2wd::BaseS>::s_fn() }
pub fn p_supqself_q<X: fxdep_l2wd::SubSQ>() { <X as fxdep_l2wd::BaseSQ>::s_fn() }
pub fn p_proj_e<I: Iterator>() where I::Item: fxdep_l2wd::SubS { I::Item::s_fn() }
pub fn p_proj_q<I: Iterator>() where I::Item: fxdep_l2wd::SubSQ { I::Item::s_fn() }
pub fn p_deep_e<X: fxdep_l2wd::Top>(x: &X) { x.dg() }
pub fn p_deep_q<X: fxdep_l2wd::TopQ>(x: &X) { x.dg() }
pub fn p_deeppath_e<X: fxdep_l2wd::Top>() { X::df() }
pub fn p_deeppath_q<X: fxdep_l2wd::TopQ>() { X::df() }
pub fn p_far_e<X: fxdep_l2wd::OverFar>(x: &X) { x.fg() }
pub fn p_far_q<X: fxdep_l2wd::OverFarQ>(x: &X) { x.fg() }
pub fn p_farpath_e<X: fxdep_l2wd::OverFar>() { X::ff() }
pub fn p_farpath_q<X: fxdep_l2wd::OverFarQ>() { X::ff() }
pub fn p_reexp_e<X: fxdep_l2wd::OverIn>(x: &X) { x.ig() }
pub fn p_reexp_q<X: fxdep_l2wd::OverInQ>(x: &X) { x.ig() }
pub fn p_two_e<X: LSub2>() { X::sa() }
pub fn p_two_q<X: LSub2Q>() { X::sa() }
pub fn p_mixed_e<X: fxdep_l2wd::Dt + Lt5>() { X::sf() }
pub fn p_mixed_q<X: fxdep_l2wd::DtQ + Lt5Q>() { X::sf() }
pub fn p_own_e<X: fxdep_l2wd::OwnSub>(x: &X) { fxdep_l2wd::OwnSub::o_go(x) }
pub fn p_own_q<X: fxdep_l2wd::OwnSubQ>(x: &X) { fxdep_l2wd::OwnSubQ::o_go(x) }
pub mod g {
    use fxdep_l2wd::*;
    pub trait LSubDG: fxdep_l2wd::BaseS {}
    pub trait LSubDGQ: fxdep_l2wd::BaseSQ {}
    pub fn p_gsuppath_e<X: SubS>() { X::s_fn() }
    pub fn p_gsuppath_q<X: SubSQ>() { X::s_fn() }
    pub fn p_gsupqself_e<X: SubS>() { <X as BaseS>::s_fn() }
    pub fn p_gsupqself_q<X: SubSQ>() { <X as BaseSQ>::s_fn() }
    pub fn p_gproj_e<I: Iterator>() where I::Item: SubS { I::Item::s_fn() }
    pub fn p_gproj_q<I: Iterator>() where I::Item: SubSQ { I::Item::s_fn() }
    pub fn p_glsdmeth_e<X: LSubDG>(x: &X) { x.s_go() }
    pub fn p_glsdmeth_q<X: LSubDGQ>(x: &X) { x.s_go() }
}
