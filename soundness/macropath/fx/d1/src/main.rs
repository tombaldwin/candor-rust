#![allow(unused)]

use inner::go_d1 as g_d1;
use inner::qgo_d1 as qg_d1;
mod other;
mod inner;
pub fn d1_e() { inner::go_d1(); }
pub fn d1_q() { inner::qgo_d1(); }
pub fn d1_in_e() { inner::p_inner_d1(); }
pub fn d1_in_q() { inner::q_inner_d1(); }
pub fn d1_use_e() { g_d1(); }
pub fn d1_use_q() { qg_d1(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d1_e" => d1_e(),
        "d1_q" => d1_q(),
        "d1_in_e" => d1_in_e(),
        "d1_in_q" => d1_in_q(),
        "d1_use_e" => d1_use_e(),
        "d1_use_q" => d1_use_q(),
        _ => panic!("unknown probe"),
    }
}
