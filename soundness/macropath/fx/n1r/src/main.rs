#![allow(unused)]
use fxdepn1r::{mkfn, mkfnp};
use inner::go_n1r as g_n1r;
use inner::qgo_n1r as qg_n1r;
mod other;
mod inner;
pub fn n1r_e() { inner::go_n1r(); }
pub fn n1r_q() { inner::qgo_n1r(); }
pub fn n1r_in_e() { inner::p_inner_n1r(); }
pub fn n1r_in_q() { inner::q_inner_n1r(); }
pub fn n1r_use_e() { g_n1r(); }
pub fn n1r_use_q() { qg_n1r(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n1r_e" => n1r_e(),
        "n1r_q" => n1r_q(),
        "n1r_in_e" => n1r_in_e(),
        "n1r_in_q" => n1r_in_q(),
        "n1r_use_e" => n1r_use_e(),
        "n1r_use_q" => n1r_use_q(),
        _ => panic!("unknown probe"),
    }
}
