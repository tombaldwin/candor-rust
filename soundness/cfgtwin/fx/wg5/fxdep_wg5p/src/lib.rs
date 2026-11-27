use fxdep_wg5d::{Flat, FlatQ, BaseS, BaseSQ, SubS, SubSQ};
pub trait LSubD: fxdep_wg5d::BaseS {}
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub trait LSubDQ: fxdep_wg5d::BaseSQ {}
pub trait LSupQ { fn lsf(); fn lsm(&self); } pub trait LSubQ: LSupQ {}
pub fn p_flat_e<X: Flat>(x: &X) { x.f_go() }
pub fn p_flat_q<X: FlatQ>(x: &X) { x.f_go() }
pub fn p_flatfn_e<X: Flat>() { X::f_fn() }
pub fn p_flatfn_q<X: FlatQ>() { X::f_fn() }
pub fn p_supmeth_e<X: SubS>(x: &X) { x.s_go() }
pub fn p_supmeth_q<X: SubSQ>(x: &X) { x.s_go() }
pub fn p_suppath_e<X: SubS>() { X::s_fn() }
pub fn p_suppath_q<X: SubSQ>() { X::s_fn() }
pub fn p_supqself_e<X: SubS>() { <X as BaseS>::s_fn() }
pub fn p_supqself_q<X: SubSQ>() { <X as BaseSQ>::s_fn() }
pub fn p_lsdmeth_e<X: LSubD>(x: &X) { x.s_go() }
pub fn p_lsdmeth_q<X: LSubDQ>(x: &X) { x.s_go() }
pub fn p_lsdpath_e<X: LSubD>() { X::s_fn() }
pub fn p_lsdpath_q<X: LSubDQ>() { X::s_fn() }
pub fn p_proj_e<I: Iterator>() where I::Item: SubS { I::Item::s_fn() }
pub fn p_proj_q<I: Iterator>() where I::Item: SubSQ { I::Item::s_fn() }
pub fn p_lsup_e<X: LSub>() { X::lsf() }
pub fn p_lsup_q<X: LSubQ>() { X::lsf() }
pub fn p_lsupm_e<X: LSub>(x: &X) { x.lsm() }
pub fn p_lsupm_q<X: LSubQ>(x: &X) { x.lsm() }
