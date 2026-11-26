#![allow(unused)]
use fxdepc15::mk;
use fxdepc15::nop as mknop;
#[macro_use]
mod other;
mod inner;
pub fn c15_e() { inner::go_c15(); }
pub fn c15_q() { inner::q_c15_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c15_e" => c15_e(),
        "c15_q" => c15_q(),
        _ => panic!("unknown probe"),
    }
}
