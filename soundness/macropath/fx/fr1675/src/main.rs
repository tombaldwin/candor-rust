#![allow(unused)]
#[macro_export]
macro_rules! select { () => { mod kq; use kq as sys; } }
crate::select!();
pub struct Poller { poller: sys::Poller }
impl Poller {
    pub fn new() -> Poller { Poller { poller: sys::Poller } }
    pub fn wait(&self) -> u8 { self.poller.wait() }
}

pub fn fr1675_e() { let _ = Poller::new().wait(); }
pub fn fr1675_q() { let _ = Poller::new(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fr1675_e" => fr1675_e(),
        "fr1675_q" => fr1675_q(),
        _ => panic!("unknown probe"),
    }
}
