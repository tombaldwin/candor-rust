#![allow(unused)]
#[cfg(unix)] #[path = "live_n2.rs"] pub mod m_n2;
#[cfg(not(unix))] #[path = "pure_n2.rs"] pub mod m_n2;

pub fn n2_e() { m_n2::go_n2(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "n2_e" => n2_e(),
        _ => panic!("unknown probe"),
    }
}
