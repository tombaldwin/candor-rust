#![allow(unused)]
use fxdepfimplc::*;
mod other;
mod inner;
pub fn fimplc_e() { inner::go_fimplc(); }
pub fn fimplc_q() { inner::q_fimplc_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fimplc_e" => fimplc_e(),
        "fimplc_q" => fimplc_q(),
        _ => panic!("unknown probe"),
    }
}
