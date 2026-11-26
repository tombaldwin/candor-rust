#![allow(unused)]
#[macro_use] extern crate fxdepe18;
pub use inner::gated_e18;
mod inner;
pub fn e18_e() { inner::p_go_e18(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e18_e" => e18_e(),
        "gated_e18" => gated_e18(),
        _ => panic!("unknown probe"),
    }
}
