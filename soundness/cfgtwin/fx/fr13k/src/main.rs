#[macro_export] macro_rules! imp_fr13k { () => { use std::os::unix::ffi::OsStrExt; } }
mod k1_fr13k { use fxdep_fr13k::FromBOs; pub fn e() { { use std::os::unix::ffi::OsStrExt; let _ = std::ffi::OsStr::new("a").as_bytes().len(); } let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k2_fr13k { use fxdep_fr13k::FromBOs; pub fn e() { fn helper() -> usize { use std::os::unix::ffi::OsStrExt; std::ffi::OsStr::new("a").as_bytes().len() } helper(); let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k3_fr13k { use fxdep_fr13k::FromBOs; pub fn e() { let c = || { use std::os::unix::ffi::OsStrExt; std::ffi::OsStr::new("a").as_bytes().len() }; c(); let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k4_fr13k { #[cfg(not(unix))] use std::os::unix::ffi::OsStrExt; use fxdep_fr13k::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k5_fr13k { #[cfg(unix)] use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k6_fr13k { use fxdep_fr13k::FromBOs; pub mod inner { use super::*; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod k7_fr13k { use std::os::unix::ffi::OsStrExt; pub mod inner { use super::*; #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod k8_fr13k { crate::imp_fr13k!(); #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k9_fr13k { use std::os::unix::ffi::{OsStrExt, OsStringExt}; #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k10_fr13k { pub(crate) use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k11_fr13k { use crate::k10_fr13k::OsStrExt; #[allow(unused_imports)] use fxdep_fr13k::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }

pub fn fr13k_block_e() { k1_fr13k::e(); }
pub fn fr13k_nestedfn_e() { k2_fr13k::e(); }
pub fn fr13k_closure_e() { k3_fr13k::e(); }
pub fn fr13k_cfgout_e() { k4_fr13k::e(); }
pub fn fr13k_cfgin_q() { k5_fr13k::q(); }
pub fn fr13k_globdep_e() { k6_fr13k::inner::e(); }
pub fn fr13k_globstd_q() { k7_fr13k::inner::q(); }
pub fn fr13k_macrouse_q() { k8_fr13k::q(); }
pub fn fr13k_grouped_q() { k9_fr13k::q(); }
pub fn fr13k_pubcrate_q() { k10_fr13k::q(); }
pub fn fr13k_viareexp_q() { k11_fr13k::q(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fr13k_block_e" => fr13k_block_e(),
        "fr13k_nestedfn_e" => fr13k_nestedfn_e(),
        "fr13k_closure_e" => fr13k_closure_e(),
        "fr13k_cfgout_e" => fr13k_cfgout_e(),
        "fr13k_cfgin_q" => fr13k_cfgin_q(),
        "fr13k_globdep_e" => fr13k_globdep_e(),
        "fr13k_globstd_q" => fr13k_globstd_q(),
        "fr13k_macrouse_q" => fr13k_macrouse_q(),
        "fr13k_grouped_q" => fr13k_grouped_q(),
        "fr13k_pubcrate_q" => fr13k_pubcrate_q(),
        "fr13k_viareexp_q" => fr13k_viareexp_q(),
        _ => panic!("unknown probe"),
    }
}
