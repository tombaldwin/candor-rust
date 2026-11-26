#![allow(unused)]

mod other;
mod inner;
pub fn fshadowr_e() { inner::go_fshadowr(); }
pub fn fshadowr_q() { inner::q_fshadowr_pure(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fshadowr_e" => fshadowr_e(),
        "fshadowr_q" => fshadowr_q(),
        _ => panic!("unknown probe"),
    }
}
