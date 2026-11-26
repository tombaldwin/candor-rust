#![allow(unused)]

pub use inner::gated_e27;
mod inner;
pub fn e27_q() { inner::p_go_e27(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e27_q" => e27_q(),
        "gated_e27" => gated_e27(),
        _ => panic!("unknown probe"),
    }
}
