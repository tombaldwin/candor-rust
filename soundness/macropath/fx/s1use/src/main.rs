#![allow(unused)]
mod inner;
pub fn s1use_e() { inner::go_s1use(); }
pub fn s1use_q() { inner::qgo_s1use(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "s1use_e" => s1use_e(),
        "s1use_q" => s1use_q(),
        _ => panic!("unknown probe"),
    }
}
