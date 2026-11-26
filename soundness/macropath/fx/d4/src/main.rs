#![allow(unused)]

use inner::go_d4 as g_d4;
use inner::qgo_d4 as qg_d4;
mod other;
mod inner;
pub fn d4_e() { inner::go_d4(); }
pub fn d4_q() { inner::qgo_d4(); }
pub fn d4_in_e() { inner::p_inner_d4(); }
pub fn d4_in_q() { inner::q_inner_d4(); }
pub fn d4_use_e() { g_d4(); }
pub fn d4_use_q() { qg_d4(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d4_e" => d4_e(),
        "d4_q" => d4_q(),
        "d4_in_e" => d4_in_e(),
        "d4_in_q" => d4_in_q(),
        "d4_use_e" => d4_use_e(),
        "d4_use_q" => d4_use_q(),
        _ => panic!("unknown probe"),
    }
}
