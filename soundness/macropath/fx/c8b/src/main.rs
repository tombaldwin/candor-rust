#![allow(unused)]
use fxdepc8b::*;
mod other;
mod inner;
pub fn c8b_e() { inner::deep::go_c8b(); }
pub fn c8b_q() { inner::deep::q_c8b_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c8b_e" => c8b_e(),
        "c8b_q" => c8b_q(),
        _ => panic!("unknown probe"),
    }
}
