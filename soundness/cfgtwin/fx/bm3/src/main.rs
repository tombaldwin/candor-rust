#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]

mod a { use fxdep_bm3::Df; pub fn e() { let _ = std::time::Instant::default(); } }
mod b { use fxdep_bm3::Parse; pub fn e() { let _ = String::from_str("x"); } }
mod c { use std::str::FromStr; use fxdep_bm3::Dt; pub fn q() { let _ = String::from_str("x"); } }
mod d { use std::os::unix::prelude::*; use fxdep_bm3::Marker; pub fn q() { let _ = std::ffi::OsString::from_vec(vec![]); } }
mod e { use fxdep_bm3::FromV; pub fn e() { let _ = std::ffi::OsString::from_vec(vec![]); } }
mod f { use fxdep_bm3::*; pub fn e() { let _ = std::time::Instant::default(); } }
mod g { pub fn e() { let _ = <std::time::Instant as fxdep_bm3::Df>::default(); } }
mod h { use fxdep_bm3::Dt; pub fn e() { <String as Dt>::f(); } pub fn e2() { String::f(); } pub fn e3() { "x".to_string().m(); } }
mod i { use fxdep_bm3::Dt; pub fn q() { u16::f(); } }
pub fn bm3_prelude_name_not_impl_e() { a::e() }
pub fn bm3_nonprelude_name_e() { b::e() }
pub fn bm3_std_trait_in_scope_q() { c::q() }
pub fn bm3_sealed_via_prelude_glob_q() { d::q() }
pub fn bm3_sealed_name_dep_only_e() { e::e() }
pub fn bm3_dep_glob_prelude_name_e() { f::e() }
pub fn bm3_qself_named_e() { g::e() }
pub fn bm3_qself_dt_e() { h::e() }
pub fn bm3_path_dt_e() { h::e2() }
pub fn bm3_meth_dt_e() { h::e3() }
pub fn bm3_path_dt_q() { i::q() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bm3_prelude_name_not_impl_e" => bm3_prelude_name_not_impl_e(),
        "bm3_nonprelude_name_e" => bm3_nonprelude_name_e(),
        "bm3_std_trait_in_scope_q" => bm3_std_trait_in_scope_q(),
        "bm3_sealed_via_prelude_glob_q" => bm3_sealed_via_prelude_glob_q(),
        "bm3_sealed_name_dep_only_e" => bm3_sealed_name_dep_only_e(),
        "bm3_dep_glob_prelude_name_e" => bm3_dep_glob_prelude_name_e(),
        "bm3_qself_named_e" => bm3_qself_named_e(),
        "bm3_qself_dt_e" => bm3_qself_dt_e(),
        "bm3_path_dt_e" => bm3_path_dt_e(),
        "bm3_meth_dt_e" => bm3_meth_dt_e(),
        "bm3_path_dt_q" => bm3_path_dt_q(),
        _ => panic!("unknown probe"),
    }
}
