mod a_fl41 { use fxdep_fl41::FromB; pub fn dep_e() { std::ffi::OsStr::from_bytes(b"x") } }
mod b_fl41 { use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_fl41::Marker; pub fn std_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod c_fl41 { use std::os::unix::ffi::*; #[allow(unused_imports)] use fxdep_fl41::Marker; pub fn glob_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod d_fl41 { use std::os::unix::ffi::OsStrExt as _; #[allow(unused_imports)] use fxdep_fl41::Marker; pub fn alias_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod e_fl41 { use fxdep_fl41::Parse; pub fn ordt_e() { let _ = String::from_str("x"); } }

pub fn fl41_dep_e() { a_fl41::dep_e(); }
pub fn fl41_std_q() { b_fl41::std_q(); }
pub fn fl41_glob_q() { c_fl41::glob_q(); }
pub fn fl41_alias_q() { d_fl41::alias_q(); }
pub fn fl41_ordt_e() { e_fl41::ordt_e(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl41_dep_e" => fl41_dep_e(),
        "fl41_std_q" => fl41_std_q(),
        "fl41_glob_q" => fl41_glob_q(),
        "fl41_alias_q" => fl41_alias_q(),
        "fl41_ordt_e" => fl41_ordt_e(),
        _ => panic!("unknown probe"),
    }
}
