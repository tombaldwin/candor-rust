#![allow(unused)]

pub use inner::gated_e26;
mod prelude;
mod inner;
pub fn e26_q() { inner::p_go_e26(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e26_q" => e26_q(),
        "gated_e26" => gated_e26(),
        _ => panic!("unknown probe"),
    }
}
