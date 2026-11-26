#![allow(unused)]
#[macro_use] extern crate fxdepe31 as d;
pub use inner::gated_e31;
mod inner;
pub fn e31_e() { inner::p_go_e31(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e31_e" => e31_e(),
        "gated_e31" => gated_e31(),
        _ => panic!("unknown probe"),
    }
}
