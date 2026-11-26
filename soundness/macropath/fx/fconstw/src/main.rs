#![allow(unused)]

mod other;
mod inner;
pub fn fconstw_e() { inner::go_fconstw(); }
pub fn fconstw_q() { inner::q_fconstw_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fconstw_e" => fconstw_e(),
        "fconstw_q" => fconstw_q(),
        _ => panic!("unknown probe"),
    }
}
