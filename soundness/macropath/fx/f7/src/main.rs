#![allow(unused)]
pub(crate) use other::mk;
mod other;
mod inner;

pub fn f7_e() { inner::go_f7(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f7_e" => f7_e(),
        _ => panic!("unknown probe"),
    }
}
