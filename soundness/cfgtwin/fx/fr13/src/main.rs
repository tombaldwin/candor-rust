use std::path::{Path, PathBuf}; use std::ffi::{OsStr, OsString};
mod a_fr13 { use std::os::unix::ffi::OsStrExt; use fxdep_fr13::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod b_fr13 { use std::slice::SliceIndex; use fxdep_fr13::GetX; pub fn e() { let _ = std::path::PathBuf::get("x"); } }
mod c_fr13 { use std::os::unix::ffi::OsStrExt; use fxdep_fr13::AsB; pub fn e() { let _ = std::path::PathBuf::from("x").as_bytes(); } }
mod d_fr13 { use std::os::unix::ffi::OsStrExt; pub fn e() { let _ = <std::ffi::OsStr as fxdep_fr13::FromBOs>::from_bytes(b"x"); } }
mod o_fr13 { use std::os::unix::ffi::OsStrExt; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); }
    pub mod inner_fr13 { use fxdep_fr13::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod p_fr13 { use fxdep_fr13::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); }
    pub mod inner2_fr13 { use std::os::unix::ffi::OsStrExt; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod prelude_fr13 { pub use std::os::unix::ffi::OsStrExt; }
mod f_fr13 { use crate::prelude_fr13::OsStrExt; #[allow(unused_imports)] use fxdep_fr13::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod g_fr13 { use std::os::unix::ffi::OsStrExt as _; use fxdep_fr13::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod h_fr13 { pub fn q() { use std::os::unix::ffi::OsStrExt; let _ = std::ffi::OsStr::from_bytes(b"x"); } pub fn e() { use fxdep_fr13::FromBOs; let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod i_fr13 { use std::os::unix::ffi::OsStringExt; use fxdep_fr13::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod j_fr13 { use fxdep_fr13::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }

pub fn fr13_pathbuf_e() { a_fr13::e(); }
pub fn fr13_sliceidx_e() { b_fr13::e(); }
pub fn fr13_method_e() { c_fr13::e(); }
pub fn fr13_ufcs_e() { d_fr13::e(); }
pub fn fr13_outer_q() { o_fr13::q(); }
pub fn fr13_inner_e() { o_fr13::inner_fr13::e(); }
pub fn fr13_outer2_e() { p_fr13::e(); }
pub fn fr13_inner2_q() { p_fr13::inner2_fr13::q(); }
pub fn fr13_reexp_q() { f_fr13::q(); }
pub fn fr13_alias_e() { g_fr13::e(); }
pub fn fr13_fnuse_q() { h_fr13::q(); }
pub fn fr13_fnuse_e() { h_fr13::e(); }
pub fn fr13_othersealed_e() { i_fr13::e(); }
pub fn fr13_deponly_e() { j_fr13::e(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fr13_pathbuf_e" => fr13_pathbuf_e(),
        "fr13_sliceidx_e" => fr13_sliceidx_e(),
        "fr13_method_e" => fr13_method_e(),
        "fr13_ufcs_e" => fr13_ufcs_e(),
        "fr13_outer_q" => fr13_outer_q(),
        "fr13_inner_e" => fr13_inner_e(),
        "fr13_outer2_e" => fr13_outer2_e(),
        "fr13_inner2_q" => fr13_inner2_q(),
        "fr13_reexp_q" => fr13_reexp_q(),
        "fr13_alias_e" => fr13_alias_e(),
        "fr13_fnuse_q" => fr13_fnuse_q(),
        "fr13_fnuse_e" => fr13_fnuse_e(),
        "fr13_othersealed_e" => fr13_othersealed_e(),
        "fr13_deponly_e" => fr13_deponly_e(),
        _ => panic!("unknown probe"),
    }
}
