#![allow(unused)]

pub use inner::gated_e7;
mod inner;
pub fn e7_q() { inner::p_go_e7(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e7_q" => e7_q(),
        "gated_e7" => gated_e7(),
        _ => panic!("unknown probe"),
    }
}
