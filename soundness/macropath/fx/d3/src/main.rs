#![allow(unused)]
use fxdepd3::*;
use inner::go_d3 as g_d3;
use inner::qgo_d3 as qg_d3;
mod other;
mod inner;
pub fn d3_e() { inner::go_d3(); }
pub fn d3_q() { inner::qgo_d3(); }
pub fn d3_in_e() { inner::p_inner_d3(); }
pub fn d3_in_q() { inner::q_inner_d3(); }
pub fn d3_use_e() { g_d3(); }
pub fn d3_use_q() { qg_d3(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d3_e" => d3_e(),
        "d3_q" => d3_q(),
        "d3_in_e" => d3_in_e(),
        "d3_in_q" => d3_in_q(),
        "d3_use_e" => d3_use_e(),
        "d3_use_q" => d3_use_q(),
        _ => panic!("unknown probe"),
    }
}
