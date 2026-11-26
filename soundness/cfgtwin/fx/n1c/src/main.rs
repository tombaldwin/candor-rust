#![allow(unused)]
macro_rules! unrelated { () => {} }
#[cfg(unix)] pub mod m_n1c { fxdep_n1c::wfn! { pub fn go_n1c(); } pub fn pure_n1c() {} }
#[cfg(not(unix))] pub mod m_n1c { pub fn go_n1c() {} pub fn pure_n1c() {} }

pub fn n1c_e() { m_n1c::go_n1c(); }
pub fn n1c_q() { m_n1c::pure_n1c(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n1c_e" => n1c_e(),
        "n1c_q" => n1c_q(),
        _ => panic!("unknown probe"),
    }
}
