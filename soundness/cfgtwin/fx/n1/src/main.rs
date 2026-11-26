#![allow(unused)]
#[cfg(unix)] pub mod m_n1 { fxdep_n1::wfn! { pub fn go_n1(); } pub fn pure_n1() {} }
#[cfg(not(unix))] pub mod m_n1 { pub fn go_n1() {} pub fn pure_n1() {} }

pub fn n1_e() { m_n1::go_n1(); }
pub fn n1_q() { m_n1::pure_n1(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n1_e" => n1_e(),
        "n1_q" => n1_q(),
        _ => panic!("unknown probe"),
    }
}
