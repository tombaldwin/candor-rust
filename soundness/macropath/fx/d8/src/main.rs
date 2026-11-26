#![allow(unused)]

mod other;
mod inner;
pub fn d8_e() { <inner::S_d8 as inner::T_d8>::go_d8(&inner::S_d8); }
pub fn d8_q() { <inner::SQ_d8 as inner::TQ_d8>::qgo_d8(&inner::SQ_d8); }
pub fn d8_m_e() { { use inner::T_d8; inner::S_d8.go_d8() }; }
pub fn d8_m_q() { { use inner::TQ_d8; inner::SQ_d8.qgo_d8() }; }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d8_e" => d8_e(),
        "d8_q" => d8_q(),
        "d8_m_e" => d8_m_e(),
        "d8_m_q" => d8_m_q(),
        _ => panic!("unknown probe"),
    }
}
