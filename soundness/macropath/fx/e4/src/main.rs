#![allow(unused)]
#[macro_use] extern crate fxdepe4;
pub use inner::gated_e4;
mod inner;
pub fn e4_e() { inner::p_go_e4(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e4_e" => e4_e(),
        "gated_e4" => gated_e4(),
        _ => panic!("unknown probe"),
    }
}
