#![allow(unused)]
pub(crate) use other::{mkfn, mkfnp};
use inner::go_d6 as g_d6;
use inner::qgo_d6 as qg_d6;
mod other;
mod inner;
pub fn d6_e() { inner::go_d6(); }
pub fn d6_q() { inner::qgo_d6(); }
pub fn d6_in_e() { inner::p_inner_d6(); }
pub fn d6_in_q() { inner::q_inner_d6(); }
pub fn d6_use_e() { g_d6(); }
pub fn d6_use_q() { qg_d6(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d6_e" => d6_e(),
        "d6_q" => d6_q(),
        "d6_in_e" => d6_in_e(),
        "d6_in_q" => d6_in_q(),
        "d6_use_e" => d6_use_e(),
        "d6_use_q" => d6_use_q(),
        _ => panic!("unknown probe"),
    }
}
