#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use std::path::PathBuf;
mod a { use fxdep_l3s::Parse; pub fn e() { let _ = String::from_str("x"); } }
mod b { use fxdep_l3s::Df; pub fn e() { let _ = std::time::Instant::default(); } }
mod c { #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::default(); } }
mod d { use std::str::FromStr; #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } }
mod e { #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { use std::str::FromStr; let _ = String::from_str("x"); } }
mod f { #[allow(unused_imports)] use std::str::FromStr; pub mod g { use super::*; #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } } }
mod pre { pub use std::str::FromStr; }
mod h { use crate::pre::FromStr; #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } }
mod i { use std::str::FromStr as _; #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } }
mod j { use std::str::FromStr as Fs; #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } }
mod k { use fxdep_l3s::GetX; pub fn e() { let _ = std::path::PathBuf::get("x"); } }
mod l { use fxdep_l3s::GetU; pub fn e() { let _ = usize::get(1u8); } }
mod m { #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::try_from("x"); } }
mod n { #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_iter(vec!['a']); } }
mod o { use fxdep_l3s::Tos; pub fn e() { let _ = std::path::PathBuf::to_string(&std::path::PathBuf::new()); } }
mod p { use fxdep_l3s::*; pub fn e() { let _ = String::from_str("x"); } }
macro_rules! imp_l3s { () => { use std::str::FromStr; } }
mod r { imp_l3s!(); #[allow(unused_imports)] use fxdep_l3s::Other; pub fn q() { let _ = String::from_str("x"); } }
mod t { use fxdep_l3s::Df; pub fn q() { let _ = std::time::SystemTime::default(); } }
mod u { #[allow(unused_imports)] use std::str::FromStr; pub mod v { use fxdep_l3s::Parse; pub fn e() { let _ = String::from_str("x"); } } }
pub fn l3s_fromstr_dep_e() { a::e() }
pub fn l3s_default_notimpl_e() { b::e() }
pub fn l3s_default_prelude_q() { c::q() }
pub fn l3s_fromstr_import_q() { d::q() }
pub fn l3s_fromstr_fnbody_q() { e::q() }
pub fn l3s_fromstr_superglob_q() { f::g::q() }
pub fn l3s_fromstr_reexport_q() { h::q() }
pub fn l3s_fromstr_asunder_q() { i::q() }
pub fn l3s_fromstr_rename_q() { j::q() }
pub fn l3s_get_pathbuf_e() { k::e() }
pub fn l3s_get_usize_e() { l::e() }
pub fn l3s_tryfrom_2021_q() { m::q() }
pub fn l3s_fromiter_2021_q() { n::q() }
pub fn l3s_tostring_blanket_e() { o::e() }
pub fn l3s_fromstr_depglob_e() { p::e() }
pub fn l3s_fromstr_macrouse_q() { r::q() }
pub fn l3s_default_pureimpl_q() { t::q() }
pub fn l3s_fromstr_parentimport_e() { u::v::e() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l3s_fromstr_dep_e" => l3s_fromstr_dep_e(),
        "l3s_default_notimpl_e" => l3s_default_notimpl_e(),
        "l3s_default_prelude_q" => l3s_default_prelude_q(),
        "l3s_fromstr_import_q" => l3s_fromstr_import_q(),
        "l3s_fromstr_fnbody_q" => l3s_fromstr_fnbody_q(),
        "l3s_fromstr_superglob_q" => l3s_fromstr_superglob_q(),
        "l3s_fromstr_reexport_q" => l3s_fromstr_reexport_q(),
        "l3s_fromstr_asunder_q" => l3s_fromstr_asunder_q(),
        "l3s_fromstr_rename_q" => l3s_fromstr_rename_q(),
        "l3s_get_pathbuf_e" => l3s_get_pathbuf_e(),
        "l3s_get_usize_e" => l3s_get_usize_e(),
        "l3s_tryfrom_2021_q" => l3s_tryfrom_2021_q(),
        "l3s_fromiter_2021_q" => l3s_fromiter_2021_q(),
        "l3s_tostring_blanket_e" => l3s_tostring_blanket_e(),
        "l3s_fromstr_depglob_e" => l3s_fromstr_depglob_e(),
        "l3s_fromstr_macrouse_q" => l3s_fromstr_macrouse_q(),
        "l3s_default_pureimpl_q" => l3s_default_pureimpl_q(),
        "l3s_fromstr_parentimport_e" => l3s_fromstr_parentimport_e(),
        _ => panic!("unknown probe"),
    }
}
