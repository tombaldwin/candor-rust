#![allow(unused)]
pub(crate) use other::mk;
mod other;
mod inner;

pub fn f7b_e() { inner::go_f7b(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f7b_e" => f7b_e(),
        _ => panic!("unknown probe"),
    }
}
