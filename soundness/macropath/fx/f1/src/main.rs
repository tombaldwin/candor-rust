#![allow(unused)]
use fxdepf1::mk;
mod other;
mod inner;

pub fn f1_e() { inner::go_f1(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f1_e" => f1_e(),
        _ => panic!("unknown probe"),
    }
}
