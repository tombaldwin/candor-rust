#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
use std::ffi::{OsStr, OsString};
mod a { use std::os::unix::ffi::*; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod b { use std::os::unix::prelude::*; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::ffi::OsString::from_vec(vec![]); } }
mod c { #[allow(unused_imports)] use std::os::unix::ffi::*; use fxdep_l3g::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod d { use fxdep_l3g::FromBOs; pub fn e() { { #[allow(unused_imports)] use std::os::unix::ffi::*; } let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod e { #[allow(unused_imports)] use std::os::unix::ffi::*; pub mod inner { use fxdep_l3g::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod pre { #[allow(unused_imports)] pub use std::os::unix::ffi::*; }
mod f { use fxdep_l3g::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod g { use crate::pre::*; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod h { #[cfg(not(unix))] use std::os::unix::ffi::*; use fxdep_l3g::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod i { #[allow(unused_imports)] use std::os::unix::fs::*; use fxdep_l3g::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod j { #[allow(unused_imports)] use std::os::unix::ffi::OsStrExt; use fxdep_l3g::Enc; pub fn e() { let _ = std::ffi::OsStr::encode_wide(std::ffi::OsStr::new("x")); } }
mod k { #[allow(unused_imports)] use std::os::unix::prelude::*; use fxdep_l3g::Enc; pub fn e() { let _ = std::ffi::OsStr::encode_wide(std::ffi::OsStr::new("x")); } }
mod l { use std::os::unix::{ffi::*, fs::*}; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod m { use std::os::unix::process::*; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::process::ExitStatus::from_raw(0); } }
mod m2 { use fxdep_l3g::FromRawX; pub fn e() { let _ = std::process::ExitStatus::from_raw(0); } }
mod n { use ::std::os::unix::ffi::*; #[allow(unused_imports)] use fxdep_l3g::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
pub fn l3g_glob_ffi_q() { a::q() }
pub fn l3g_glob_prelude_q() { b::q() }
pub fn l3g_glob_othertype_e() { c::e() }
pub fn l3g_glob_block_e() { d::e() }
pub fn l3g_glob_parent_e() { e::inner::e() }
pub fn l3g_glob_reexp_elsewhere_e() { f::e() }
pub fn l3g_glob_localreexp_q() { g::q() }
pub fn l3g_glob_cfgout_e() { h::e() }
pub fn l3g_glob_notexported_e() { i::e() }
pub fn l3g_named_otherplatform_e() { j::e() }
pub fn l3g_glob_otherplatform_e() { k::e() }
pub fn l3g_glob_grouped_q() { l::q() }
pub fn l3g_glob_process_q() { m::q() }
pub fn l3g_process_deponly_e() { m2::e() }
pub fn l3g_glob_leadingcolon_q() { n::q() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "l3g_glob_ffi_q" => l3g_glob_ffi_q(),
        "l3g_glob_prelude_q" => l3g_glob_prelude_q(),
        "l3g_glob_othertype_e" => l3g_glob_othertype_e(),
        "l3g_glob_block_e" => l3g_glob_block_e(),
        "l3g_glob_parent_e" => l3g_glob_parent_e(),
        "l3g_glob_reexp_elsewhere_e" => l3g_glob_reexp_elsewhere_e(),
        "l3g_glob_localreexp_q" => l3g_glob_localreexp_q(),
        "l3g_glob_cfgout_e" => l3g_glob_cfgout_e(),
        "l3g_glob_notexported_e" => l3g_glob_notexported_e(),
        "l3g_named_otherplatform_e" => l3g_named_otherplatform_e(),
        "l3g_glob_otherplatform_e" => l3g_glob_otherplatform_e(),
        "l3g_glob_grouped_q" => l3g_glob_grouped_q(),
        "l3g_glob_process_q" => l3g_glob_process_q(),
        "l3g_process_deponly_e" => l3g_process_deponly_e(),
        "l3g_glob_leadingcolon_q" => l3g_glob_leadingcolon_q(),
        _ => panic!("unknown probe"),
    }
}
