#![allow(unused)]
use fxdepc3::*;
mod other;
mod inner;
pub fn c3_e() { inner::go_c3(); }
pub fn c3_q() { inner::q_c3_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c3_e" => c3_e(),
        "c3_q" => c3_q(),
        _ => panic!("unknown probe"),
    }
}
