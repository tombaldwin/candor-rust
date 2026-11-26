#![allow(unused)]
macro_rules! unrelated { () => {} }
#[cfg(unix)] #[path = "live_n2c.rs"] pub mod m_n2c;
#[cfg(not(unix))] #[path = "pure_n2c.rs"] pub mod m_n2c;

pub fn n2c_e() { m_n2c::go_n2c(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n2c_e" => n2c_e(),
        _ => panic!("unknown probe"),
    }
}
