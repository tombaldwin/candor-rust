#![allow(unused)]

pub use inner::gated_e23;
mod prelude;
mod inner;
pub fn e23_e() { inner::p_go_e23(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e23_e" => e23_e(),
        "gated_e23" => gated_e23(),
        _ => panic!("unknown probe"),
    }
}
