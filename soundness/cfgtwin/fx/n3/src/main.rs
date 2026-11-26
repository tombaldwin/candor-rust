#![allow(unused)]
macro_rules! unrelated_n3 { () => {} }
pub mod helpers_n3 { pub fn h_n3() {} }
use helpers_n3::*;
use std::cell::Cell;
thread_local! { static X_n3: Cell<u8> = { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Cell::new(0) }; }
#[cfg(unix)] pub fn gated_n3() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
#[cfg(not(unix))] pub fn gated_n3() {}

pub fn n3_e() { gated_n3(); }
pub fn n3_tl() { X_n3.with(|c| c.get()); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n3_e" => n3_e(),
        "n3_tl" => n3_tl(),
        _ => panic!("unknown probe"),
    }
}
