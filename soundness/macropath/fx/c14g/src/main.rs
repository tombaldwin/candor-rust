#![allow(unused)]
use fxdepc14g::*;
mod other;
mod inner;
pub fn c14g_e() { inner::go_c14g(); }
pub fn c14g_q() { inner::q_c14g_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "c14g_e" => c14g_e(),
        "c14g_q" => c14g_q(),
        _ => panic!("unknown probe"),
    }
}
