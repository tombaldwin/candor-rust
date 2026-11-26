#![allow(unused)]

pub use inner::gated_e6;
mod other;
mod inner;
pub fn e6_q() { inner::p_go_e6(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e6_q" => e6_q(),
        "gated_e6" => gated_e6(),
        _ => panic!("unknown probe"),
    }
}
