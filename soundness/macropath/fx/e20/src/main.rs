#![allow(unused)]

pub use inner::gated_e20;
mod inner;
pub fn e20_e() { inner::p_go_e20(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e20_e" => e20_e(),
        "gated_e20" => gated_e20(),
        _ => panic!("unknown probe"),
    }
}
