#![allow(unused)]

use inner::go_d9 as g_d9;
use inner::qgo_d9 as qg_d9;
mod other;
mod inner;
pub fn d9_e() { inner::go_d9(); }
pub fn d9_q() { inner::qgo_d9(); }
pub fn d9_in_e() { inner::p_inner_d9(); }
pub fn d9_in_q() { inner::q_inner_d9(); }
pub fn d9_use_e() { g_d9(); }
pub fn d9_use_q() { qg_d9(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d9_e" => d9_e(),
        "d9_q" => d9_q(),
        "d9_in_e" => d9_in_e(),
        "d9_in_q" => d9_in_q(),
        "d9_use_e" => d9_use_e(),
        "d9_use_q" => d9_use_q(),
        _ => panic!("unknown probe"),
    }
}
