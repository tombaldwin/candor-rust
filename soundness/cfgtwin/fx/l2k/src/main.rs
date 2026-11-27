#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use fxdep_l2kd::{SubS, BaseS, Top, OverFar, Flat};
pub trait LSub2: fxdep_l2kd::SupA + fxdep_l2kd::SupB {}
pub trait Lt5: fxdep_l2kd::SupD {}
pub struct L2; impl fxdep_l2kd::SupA for L2 { fn sa() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_l2kd::SupB for L2 { fn sb() {} } impl LSub2 for L2 {}
pub struct L5; impl fxdep_l2kd::Dt for L5 { fn f() {} } impl fxdep_l2kd::SupD for L5 { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt5 for L5 {}
fn g_suppath<X: SubS>() { X::s_fn() } pub fn l2k_suppath_e() { g_suppath::<fxdep_l2kd::DE>() }
fn g_supmeth<X: SubS>(x: &X) { x.s_go() } pub fn l2k_supmeth_e() { g_supmeth(&fxdep_l2kd::DE) }
fn g_proj<I: Iterator>() where I::Item: SubS { I::Item::s_fn() } pub fn l2k_proj_e() { g_proj::<std::iter::Empty<fxdep_l2kd::DE>>() }
fn g_deep<X: Top>() { X::df() } pub fn l2k_deep_e() { g_deep::<fxdep_l2kd::DE>() }
fn g_far<X: OverFar>() { X::ff() } pub fn l2k_far_e() { g_far::<fxdep_l2kd::DE>() }
fn g_flat<X: Flat>() { X::f_fn() } pub fn l2k_flat_e() { g_flat::<fxdep_l2kd::DE>() }
fn g_made<X: fxdep_l2kd::OverMade>() { X::mk() } pub fn l2k_made_e() { g_made::<fxdep_l2kd::DE>() }
fn g_two<X: LSub2>() { X::sa() } pub fn l2k_two_e() { g_two::<L2>() }
fn g_mixed<X: fxdep_l2kd::Dt + Lt5>() { X::sf() } pub fn l2k_mixed_e() { g_mixed::<L5>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l2k_suppath_e" => l2k_suppath_e(),
        "l2k_supmeth_e" => l2k_supmeth_e(),
        "l2k_proj_e" => l2k_proj_e(),
        "l2k_deep_e" => l2k_deep_e(),
        "l2k_far_e" => l2k_far_e(),
        "l2k_flat_e" => l2k_flat_e(),
        "l2k_made_e" => l2k_made_e(),
        "l2k_two_e" => l2k_two_e(),
        "l2k_mixed_e" => l2k_mixed_e(),
        _ => panic!("unknown probe"),
    }
}
