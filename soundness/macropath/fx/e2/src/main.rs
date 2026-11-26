#![allow(unused)]

pub use inner::gated_e2;
mod inner;
pub fn e2_e() { inner::p_go_e2(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e2_e" => e2_e(),
        "gated_e2" => gated_e2(),
        _ => panic!("unknown probe"),
    }
}
