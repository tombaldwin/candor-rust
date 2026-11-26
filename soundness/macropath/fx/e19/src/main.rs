#![allow(unused)]

pub use inner::gated_e19;
mod inner;
pub fn e19_q() { inner::p_go_e19(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e19_q" => e19_q(),
        "gated_e19" => gated_e19(),
        _ => panic!("unknown probe"),
    }
}
