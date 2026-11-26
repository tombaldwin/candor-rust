#![allow(unused)]
#[cfg_attr(unix, macro_use)] extern crate fxdepe29;
pub use inner::gated_e29;
mod inner;
pub fn e29_e() { inner::p_go_e29(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e29_e" => e29_e(),
        "gated_e29" => gated_e29(),
        _ => panic!("unknown probe"),
    }
}
