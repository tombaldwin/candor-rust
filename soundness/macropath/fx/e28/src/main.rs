#![allow(unused)]

pub use inner::gated_e28;
mod prelude;
mod inner;
pub fn e28_q() { inner::p_go_e28(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e28_q" => e28_q(),
        "gated_e28" => gated_e28(),
        _ => panic!("unknown probe"),
    }
}
