#![allow(unused)]

pub use inner::gated_e5;
mod inner;
pub fn e5_e() { inner::p_go_e5(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e5_e" => e5_e(),
        "gated_e5" => gated_e5(),
        _ => panic!("unknown probe"),
    }
}
