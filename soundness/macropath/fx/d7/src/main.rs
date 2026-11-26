#![allow(unused)]

mod other;
mod inner;
pub fn d7_e() { inner::S_d7.go_d7(); }
pub fn d7_q() { inner::SQ_d7.qgo_d7(); }
pub fn d7_ufcs_e() { inner::S_d7::go_d7(&inner::S_d7); }
pub fn d7_ufcs_q() { inner::SQ_d7::qgo_d7(&inner::SQ_d7); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d7_e" => d7_e(),
        "d7_q" => d7_q(),
        "d7_ufcs_e" => d7_ufcs_e(),
        "d7_ufcs_q" => d7_ufcs_q(),
        _ => panic!("unknown probe"),
    }
}
