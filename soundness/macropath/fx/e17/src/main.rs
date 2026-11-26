#![allow(unused)]

pub use inner::gated_e17;
mod inner;
pub fn e17_q() { inner::p_go_e17(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e17_q" => e17_q(),
        "gated_e17" => gated_e17(),
        _ => panic!("unknown probe"),
    }
}
