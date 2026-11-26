#![allow(unused)]
mod other;
mod inner;

pub fn f6_e() { inner::go_f6(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f6_e" => f6_e(),
        _ => panic!("unknown probe"),
    }
}
