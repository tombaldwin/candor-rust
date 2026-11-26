#![allow(unused)]
mod inner;
pub fn s1fn_e() { inner::go_s1fn(); }
pub fn s1fn_q() { { inner::fnonly_s1fn(); }; }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "s1fn_e" => s1fn_e(),
        "s1fn_q" => s1fn_q(),
        _ => panic!("unknown probe"),
    }
}
