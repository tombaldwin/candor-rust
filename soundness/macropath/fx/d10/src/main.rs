#![allow(unused)]

use inner::go_d10 as g_d10;
use inner::qgo_d10 as qg_d10;
mod other;
mod inner;
pub fn d10_e() { inner::go_d10(); }
pub fn d10_q() { inner::qgo_d10(); }
pub fn d10_in_e() { inner::p_inner_d10(); }
pub fn d10_in_q() { inner::q_inner_d10(); }
pub fn d10_use_e() { g_d10(); }
pub fn d10_use_q() { qg_d10(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d10_e" => d10_e(),
        "d10_q" => d10_q(),
        "d10_in_e" => d10_in_e(),
        "d10_in_q" => d10_in_q(),
        "d10_use_e" => d10_use_e(),
        "d10_use_q" => d10_use_q(),
        _ => panic!("unknown probe"),
    }
}
