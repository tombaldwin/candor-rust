#![allow(unused)]
use fxdepfconst::*;
mod other;
mod inner;
pub fn fconst_e() { inner::go_fconst(); }
pub fn fconst_q() { inner::q_fconst_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fconst_e" => fconst_e(),
        "fconst_q" => fconst_q(),
        _ => panic!("unknown probe"),
    }
}
