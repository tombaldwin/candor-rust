#![allow(unused)]

mod other;
mod inner;
pub fn fshadow_e() { inner::go_fshadow(); }
pub fn fshadow_q() { inner::q_fshadow_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fshadow_e" => fshadow_e(),
        "fshadow_q" => fshadow_q(),
        _ => panic!("unknown probe"),
    }
}
