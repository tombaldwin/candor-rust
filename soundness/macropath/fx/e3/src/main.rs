#![allow(unused)]
#[macro_use] extern crate fxdepe3;
pub use inner::gated_e3;
mod inner;
pub fn e3_e() { inner::p_go_e3(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e3_e" => e3_e(),
        "gated_e3" => gated_e3(),
        _ => panic!("unknown probe"),
    }
}
