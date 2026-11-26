#![allow(unused)]
use fxdepn4::*;
mod inner;
pub fn n4_e() { inner::go_n4(); }
pub fn n4_q() { inner::q_n4_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n4_e" => n4_e(),
        "n4_q" => n4_q(),
        _ => panic!("unknown probe"),
    }
}
