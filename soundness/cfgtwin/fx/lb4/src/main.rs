#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// R1980 / R1975 — a supertrait resolved through the crate's use / re-export / alias table, a root counted as a dependency only when it is a KNOWN dependency crate.
extern crate fxdep_lb4 as fxr_lb4;
pub mod inner4 { pub trait Sup { fn lsf() where Self: Sized; fn lsm(&self); } }
// F1 criterion shape: `use dep::float; trait Float: float::Float` (the local trait shares the leaf of its own dependency supertrait)
pub mod st { use fxdep_lb4::float; pub trait Float: float::Float {} impl Float for u8 {} impl Float for u16 {} }
fn g1<T: st::Float>() { T::cast() }
fn g1q<T: st::Float>() { T::cast() }
fn g1m<T: st::Float>(t: T) { use fxdep_lb4::float::Float; t.cm() }
fn g1mq<T: st::Float>(t: T) { use fxdep_lb4::float::Float; t.cm() }
pub fn lb4_self_e() { g1::<u8>() }
pub fn lb4_self_q() { g1q::<u16>() }
pub fn lb4_selfm_e() { g1m(1u8) }
pub fn lb4_selfm_q() { g1mq(1u16) }
// F2 R1975: a supertrait written through a `use ... as` alias of a LOCAL trait
pub mod via_alias { use crate::inner4::Sup as DepLike; pub trait LSubA: DepLike {} }
pub struct LocLoud; impl inner4::Sup for LocLoud { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl via_alias::LSubA for LocLoud {}
fn g2<T: via_alias::LSubA>() { T::lsf() }
fn g2m<T: via_alias::LSubA>(t: T) { t.lsm() }
pub fn lb4_alias_e() { g2::<LocLoud>() }
pub fn lb4_aliasm_e() { g2m(LocLoud) }
// F3 a DEPENDENCY trait re-exported through a local module, beside a local trait of the same leaf (bl7_reexp shape), with only DEPENDENCY implementors
pub mod reexp { pub use fxdep_lb4::Sup; }
pub trait LSubR: reexp::Sup {} impl LSubR for u32 {} impl LSubR for u64 {}
fn g3<T: LSubR>() { T::lsf() }
fn g3q<T: LSubR>() { T::lsf() }
pub fn lb4_reexp_e() { g3::<u32>() }
pub fn lb4_reexp_q() { g3q::<u64>() }
// F4 a LOCAL trait re-exported through a local module
pub mod reexl { pub use crate::inner4::Sup; }
pub trait LSubRL: reexl::Sup {} impl LSubRL for LocLoud {}
fn g4<T: LSubRL>() { T::lsf() }
pub fn lb4_reexl_e() { g4::<LocLoud>() }
// F5 an `extern crate .. as` alias root, beside the colliding local leaf
pub trait LSubX: fxr_lb4::Sup {} impl LSubX for u32 {} impl LSubX for u64 {}
fn g5<T: LSubX>() { T::lsf() }
fn g5q<T: LSubX>() { T::lsf() }
pub fn lb4_extern_e() { g5::<u32>() }
pub fn lb4_extern_q() { g5q::<u64>() }
// F6 the dependency supertrait written as a full path, colliding leaf, dependency implementors only
pub trait LSubP: fxdep_lb4::Sup {} impl LSubP for u32 {} impl LSubP for u64 {}
fn g6<T: LSubP>() { T::lsf() }
fn g6q<T: LSubP>() { T::lsf() }
pub fn lb4_path_e() { g6::<u32>() }
pub fn lb4_path_q() { g6q::<u64>() }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lb4_self_e" => lb4_self_e(),
        "lb4_self_q" => lb4_self_q(),
        "lb4_selfm_e" => lb4_selfm_e(),
        "lb4_selfm_q" => lb4_selfm_q(),
        "lb4_alias_e" => lb4_alias_e(),
        "lb4_aliasm_e" => lb4_aliasm_e(),
        "lb4_reexp_e" => lb4_reexp_e(),
        "lb4_reexp_q" => lb4_reexp_q(),
        "lb4_reexl_e" => lb4_reexl_e(),
        "lb4_extern_e" => lb4_extern_e(),
        "lb4_extern_q" => lb4_extern_q(),
        "lb4_path_e" => lb4_path_e(),
        "lb4_path_q" => lb4_path_q(),
        _ => panic!("unknown probe"),
    }
}
