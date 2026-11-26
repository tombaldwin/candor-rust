#![allow(unused)]
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
use fxdepc11::*;
mod other;
mod inner;
pub fn c11_e() { inner::go_c11(); }
pub fn c11_q() { inner::q_c11_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c11_e" => c11_e(),
        "c11_q" => c11_q(),
        _ => panic!("unknown probe"),
    }
}
