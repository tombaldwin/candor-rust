#![allow(unused)]
use fxdepc10::*;
pub(crate) use other::{mk, mknop};
mod other;
mod inner;
pub fn c10_e() { inner::go_c10(); }
pub fn c10_q() { inner::q_c10_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c10_e" => c10_e(),
        "c10_q" => c10_q(),
        _ => panic!("unknown probe"),
    }
}
