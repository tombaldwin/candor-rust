#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub mod inner_bl7 { pub trait Sup { fn lsf(); } }
pub mod via_glob_bl7 { use super::*; pub trait LSubG: inner_bl7::Sup {} }
pub mod via_mod_use_bl7 { use super::inner_bl7; pub trait LSubM: inner_bl7::Sup {} }
pub mod via_alias_bl7 { use crate::inner_bl7::Sup as DepLike; pub trait LSubA: DepLike {} }
pub mod reexp_bl7 { pub use fxdep_bl7::Sup; }
pub trait LSubR_bl7: reexp_bl7::Sup {}
pub trait LSubSelf_bl7: self::inner_bl7::Sup {}
#[cfg(unix)] pub trait LSubC_bl7: fxdep_bl7::Sup {}
#[cfg(not(unix))] pub trait LSubC_bl7: inner_bl7::Sup {}
pub struct LocLoud_bl7; impl inner_bl7::Sup for LocLoud_bl7 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl via_glob_bl7::LSubG for LocLoud_bl7 {} impl via_mod_use_bl7::LSubM for LocLoud_bl7 {} impl via_alias_bl7::LSubA for LocLoud_bl7 {} impl LSubSelf_bl7 for LocLoud_bl7 {}
pub struct DepQuiet_bl7; impl fxdep_bl7::Sup for DepQuiet_bl7 { fn lsf() {} } impl LSubR_bl7 for DepQuiet_bl7 {} impl LSubC_bl7 for DepQuiet_bl7 {}
pub struct DepLoud_bl7; impl fxdep_bl7::Sup for DepLoud_bl7 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSubR_bl7 for DepLoud_bl7 {}
fn g_glob<T: via_glob_bl7::LSubG>() { T::lsf() }
fn g_moduse<T: via_mod_use_bl7::LSubM>() { T::lsf() }
fn g_alias<T: via_alias_bl7::LSubA>() { T::lsf() }
fn g_self<T: LSubSelf_bl7>() { T::lsf() }
fn g_reexp<T: LSubR_bl7>() { T::lsf() }
fn g_reexpq<T: LSubR_bl7>() { T::lsf() }
fn g_cfg<T: LSubC_bl7>() { T::lsf() }
pub fn bl7_glob_e() { g_glob::<LocLoud_bl7>() }
pub fn bl7_moduse_e() { g_moduse::<LocLoud_bl7>() }
pub fn bl7_alias_e() { g_alias::<LocLoud_bl7>() }
pub fn bl7_self_e() { g_self::<LocLoud_bl7>() }
pub fn bl7_reexp_e() { g_reexp::<DepLoud_bl7>() }
pub fn bl7_reexp_q() { g_reexpq::<DepQuiet_bl7>() }
pub fn bl7_cfg_q() { g_cfg::<DepQuiet_bl7>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl7_glob_e" => bl7_glob_e(),
        "bl7_moduse_e" => bl7_moduse_e(),
        "bl7_alias_e" => bl7_alias_e(),
        "bl7_self_e" => bl7_self_e(),
        "bl7_reexp_e" => bl7_reexp_e(),
        "bl7_reexp_q" => bl7_reexp_q(),
        "bl7_cfg_q" => bl7_cfg_q(),
        _ => panic!("unknown probe"),
    }
}
