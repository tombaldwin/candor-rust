#![allow(unused)]
macro_rules! unrelated_n3d { () => {} }
pub mod helpers_n3d { pub fn h_n3d() {} }
use fxdep_n3d::*;
use std::cell::Cell;
thread_local! { static X_n3d: Cell<u8> = { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Cell::new(0) }; }
#[cfg(unix)] pub fn gated_n3d() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
#[cfg(not(unix))] pub fn gated_n3d() {}

pub fn n3d_e() { gated_n3d(); }
pub fn n3d_tl() { X_n3d.with(|c| c.get()); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n3d_e" => n3d_e(),
        "n3d_tl" => n3d_tl(),
        _ => panic!("unknown probe"),
    }
}
