#![allow(unused)]
pub use other::*;
use inner::go_d5 as g_d5;
use inner::qgo_d5 as qg_d5;
mod other;
mod inner;
pub fn d5_e() { inner::go_d5(); }
pub fn d5_q() { inner::qgo_d5(); }
pub fn d5_in_e() { inner::p_inner_d5(); }
pub fn d5_in_q() { inner::q_inner_d5(); }
pub fn d5_use_e() { g_d5(); }
pub fn d5_use_q() { qg_d5(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d5_e" => d5_e(),
        "d5_q" => d5_q(),
        "d5_in_e" => d5_in_e(),
        "d5_in_q" => d5_in_q(),
        "d5_use_e" => d5_use_e(),
        "d5_use_q" => d5_use_q(),
        _ => panic!("unknown probe"),
    }
}
