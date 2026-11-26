#![allow(unused)]
use fxdepc7::*;
mod other;
mod inner;
pub fn c7_e() { inner::go_c7(); }
pub fn c7_q() { inner::q_c7_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c7_e" => c7_e(),
        "c7_q" => c7_q(),
        _ => panic!("unknown probe"),
    }
}
