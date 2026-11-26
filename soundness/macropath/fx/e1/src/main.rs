#![allow(unused)]

pub use inner::gated_e1;
mod inner;
pub fn e1_e() { inner::p_go_e1(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e1_e" => e1_e(),
        "gated_e1" => gated_e1(),
        _ => panic!("unknown probe"),
    }
}
