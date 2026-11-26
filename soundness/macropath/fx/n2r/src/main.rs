#![allow(unused)]
pub(crate) use other::{mk, mknop};
mod other;
mod third;
mod inner;
pub fn n2r_e() { inner::go_n2r(); }
pub fn n2r_q() { inner::q_n2r_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n2r_e" => n2r_e(),
        "n2r_q" => n2r_q(),
        _ => panic!("unknown probe"),
    }
}
