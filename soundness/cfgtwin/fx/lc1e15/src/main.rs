#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// Edition 2015: a LEADING `::` is the crate ROOT, so `::fxdep_lc1e15::Sup` is the crate's own `mod fxdep_lc1e15` (the dependency, declared in the
// manifest, is not reachable here without an `extern crate` that the same name would collide with). The local trait IS what the bound names.
pub mod fxdep_lc1e15 { pub trait Sup { fn lsf(); } }
pub trait LSub_lc1e15: ::fxdep_lc1e15::Sup {}
pub struct Noisy_lc1e15; impl fxdep_lc1e15::Sup for Noisy_lc1e15 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub_lc1e15 for Noisy_lc1e15 {}
fn g_lc1e15<T: LSub_lc1e15>() { T::lsf() }
pub fn lc1e15_root_e() { g_lc1e15::<Noisy_lc1e15>() }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lc1e15_root_e" => lc1e15_root_e(),
        _ => panic!("unknown probe"),
    }
}
