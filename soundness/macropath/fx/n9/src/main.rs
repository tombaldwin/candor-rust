#![allow(unused)]
use fxdepn9::*;
pub(crate) use other::{mk as mkr, mknop as mkrnop};
mod other;
mod inner;
pub fn n9_e() { inner::go_n9(); }
pub fn n9_q() { inner::q_n9_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n9_e" => n9_e(),
        "n9_q" => n9_q(),
        _ => panic!("unknown probe"),
    }
}
