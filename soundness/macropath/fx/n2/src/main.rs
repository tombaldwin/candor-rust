#![allow(unused)]
pub(crate) use other::{mk, mknop};
mod other;
mod third;
mod inner;
pub fn n2_e() { inner::go_n2(); }
pub fn n2_q() { inner::q_n2_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n2_e" => n2_e(),
        "n2_q" => n2_q(),
        _ => panic!("unknown probe"),
    }
}
