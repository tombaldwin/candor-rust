#![allow(unused)]
use fxdepc10r::*;
pub(crate) use other::{mk, mknop};
mod other;
mod inner;
pub fn c10r_e() { inner::go_c10r(); }
pub fn c10r_q() { inner::q_c10r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c10r_e" => c10r_e(),
        "c10r_q" => c10r_q(),
        _ => panic!("unknown probe"),
    }
}
