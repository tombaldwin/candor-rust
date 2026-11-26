#![allow(unused)]
macro_rules! imp { () => { use fxdepf4::mk; } }
imp!();
mod other;
mod inner;

pub fn f4_e() { inner::go_f4(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f4_e" => f4_e(),
        _ => panic!("unknown probe"),
    }
}
