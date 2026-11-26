#![allow(unused)]
#[macro_use] extern crate fxdepe13;
pub use inner::gated_e13;
mod inner;
pub fn e13_e() { inner::p_go_e13(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e13_e" => e13_e(),
        "gated_e13" => gated_e13(),
        _ => panic!("unknown probe"),
    }
}
