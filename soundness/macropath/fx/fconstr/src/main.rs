#![allow(unused)]
use fxdepfconstr::*;
mod other;
mod inner;
pub fn fconstr_e() { inner::go_fconstr(); }
pub fn fconstr_q() { inner::q_fconstr_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fconstr_e" => fconstr_e(),
        "fconstr_q" => fconstr_q(),
        _ => panic!("unknown probe"),
    }
}
