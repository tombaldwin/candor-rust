#![allow(unused)]
mod inner;
pub fn s1path_e() { inner::via_s1path(); }
pub fn s1path_q() { inner::qvia_s1path(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "s1path_e" => s1path_e(),
        "s1path_q" => s1path_q(),
        _ => panic!("unknown probe"),
    }
}
