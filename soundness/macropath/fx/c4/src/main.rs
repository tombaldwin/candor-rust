#![allow(unused)]
pub use other::*;
mod other;
mod inner;
pub fn c4_e() { inner::go_c4(); }
pub fn c4_q() { inner::q_c4_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c4_e" => c4_e(),
        "c4_q" => c4_q(),
        _ => panic!("unknown probe"),
    }
}
