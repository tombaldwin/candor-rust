mod reexp_fl15 { pub use fxdep_fl15::*; }
mod g_fl15 {
    use crate::reexp_fl15::*;
    pub fn free_e() { d_fl15::free() }
    pub fn ty_e() { d_fl15::Lone::m() }
    pub fn pure_q() { d_fl15::pure() }
}
mod lre_fl15 { pub mod dl_fl15 { pub fn pf() {} } }
mod gl_fl15 { use crate::lre_fl15::*; pub fn local_q() { dl_fl15::pf() } }

pub fn fl15_free_e() { g_fl15::free_e(); }
pub fn fl15_ty_e() { g_fl15::ty_e(); }
pub fn fl15_pure_q() { g_fl15::pure_q(); }
pub fn fl15_local_q() { gl_fl15::local_q(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl15_free_e" => fl15_free_e(),
        "fl15_ty_e" => fl15_ty_e(),
        "fl15_pure_q" => fl15_pure_q(),
        "fl15_local_q" => fl15_local_q(),
        _ => panic!("unknown probe"),
    }
}
