#![allow(unused)]

pub use inner::gated_e21;
mod inner;
pub fn e21_e() { inner::p_go_e21(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e21_e" => e21_e(),
        "gated_e21" => gated_e21(),
        _ => panic!("unknown probe"),
    }
}
