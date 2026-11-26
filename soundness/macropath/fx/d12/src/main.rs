#![allow(unused)]
mod prelude { pub use fxdepd12::*; }
#[macro_export] macro_rules! mk { ($n:ident) => { pub fn $n() {} } }
#[macro_export] macro_rules! mknop { ($n:ident) => { pub fn $n() {} } }
mod inner;

pub fn d12_e() { inner::go_d12(); }
pub fn d12_q() { inner::qgo_d12(); }
pub fn d12_in_e() { inner::p_inner_d12(); }
pub fn d12_in_q() { inner::q_inner_d12(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d12_e" => d12_e(),
        "d12_q" => d12_q(),
        "d12_in_e" => d12_in_e(),
        "d12_in_q" => d12_in_q(),
        _ => panic!("unknown probe"),
    }
}
