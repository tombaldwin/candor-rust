#![allow(unused)]
use fxdepd2::*;
use inner::go_d2 as g_d2;
use inner::qgo_d2 as qg_d2;
mod other;
mod inner;
pub fn d2_e() { inner::go_d2(); }
pub fn d2_q() { inner::qgo_d2(); }
pub fn d2_in_e() { inner::p_inner_d2(); }
pub fn d2_in_q() { inner::q_inner_d2(); }
pub fn d2_use_e() { g_d2(); }
pub fn d2_use_q() { qg_d2(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d2_e" => d2_e(),
        "d2_q" => d2_q(),
        "d2_in_e" => d2_in_e(),
        "d2_in_q" => d2_in_q(),
        "d2_use_e" => d2_use_e(),
        "d2_use_q" => d2_use_q(),
        _ => panic!("unknown probe"),
    }
}
