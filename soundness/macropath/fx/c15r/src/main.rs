#![allow(unused)]
use fxdepc15r::mk;
use fxdepc15r::nop as mknop;
#[macro_use]
mod other;
mod inner;
pub fn c15r_e() { inner::go_c15r(); }
pub fn c15r_q() { inner::q_c15r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c15r_e" => c15r_e(),
        "c15r_q" => c15r_q(),
        _ => panic!("unknown probe"),
    }
}
