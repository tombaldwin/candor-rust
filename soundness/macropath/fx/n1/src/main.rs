#![allow(unused)]
use fxdepn1::{mkfn, mkfnp};
use inner::go_n1 as g_n1;
use inner::qgo_n1 as qg_n1;
mod other;
mod inner;
pub fn n1_e() { inner::go_n1(); }
pub fn n1_q() { inner::qgo_n1(); }
pub fn n1_in_e() { inner::p_inner_n1(); }
pub fn n1_in_q() { inner::q_inner_n1(); }
pub fn n1_use_e() { g_n1(); }
pub fn n1_use_q() { qg_n1(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n1_e" => n1_e(),
        "n1_q" => n1_q(),
        "n1_in_e" => n1_in_e(),
        "n1_in_q" => n1_in_q(),
        "n1_use_e" => n1_use_e(),
        "n1_use_q" => n1_use_q(),
        _ => panic!("unknown probe"),
    }
}
