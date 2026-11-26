#![allow(unused)]

use inner::go_d11 as g_d11;
use inner::qgo_d11 as qg_d11;
mod inner;
mod defs;
pub fn d11_e() { inner::go_d11(); }
pub fn d11_q() { inner::qgo_d11(); }
pub fn d11_in_e() { inner::p_inner_d11(); }
pub fn d11_in_q() { inner::q_inner_d11(); }
pub fn d11_use_e() { g_d11(); }
pub fn d11_use_q() { qg_d11(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d11_e" => d11_e(),
        "d11_q" => d11_q(),
        "d11_in_e" => d11_in_e(),
        "d11_in_q" => d11_in_q(),
        "d11_use_e" => d11_use_e(),
        "d11_use_q" => d11_use_q(),
        _ => panic!("unknown probe"),
    }
}
