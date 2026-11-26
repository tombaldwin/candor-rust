#![allow(unused)]
use fxdepfstatic::*;
mod other;
mod inner;
pub fn fstatic_e() { inner::go_fstatic(); }
pub fn fstatic_q() { inner::q_fstatic_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fstatic_e" => fstatic_e(),
        "fstatic_q" => fstatic_q(),
        _ => panic!("unknown probe"),
    }
}
