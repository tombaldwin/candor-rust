#![allow(unused)]
fxdepf5::imp!();
mod other;
mod inner;

pub fn f5_e() { inner::go_f5(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f5_e" => f5_e(),
        _ => panic!("unknown probe"),
    }
}
