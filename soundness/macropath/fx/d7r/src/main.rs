#![allow(unused)]
use fxdepd7r::*;
mod other;
mod inner;
pub fn d7r_e() { inner::S_d7r.go_d7r(); }
pub fn d7r_q() { inner::SQ_d7r.qgo_d7r(); }
pub fn d7r_ufcs_e() { inner::S_d7r::go_d7r(&inner::S_d7r); }
pub fn d7r_ufcs_q() { inner::SQ_d7r::qgo_d7r(&inner::SQ_d7r); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d7r_e" => d7r_e(),
        "d7r_q" => d7r_q(),
        "d7r_ufcs_e" => d7r_ufcs_e(),
        "d7r_ufcs_q" => d7r_ufcs_q(),
        _ => panic!("unknown probe"),
    }
}
