#![allow(unused)]
use fxdeptlpath::*;
crate::thread_local! { static X_tlpath: u8 = 0; }
crate::mkq! { static Y_tlpath: u8 = 0; }
pub fn tlpath_e() { tl_touch(); }
pub fn tlpath_q() { qtl_touch(); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "tlpath_e" => tlpath_e(),
        "tlpath_q" => tlpath_q(),
        _ => panic!("unknown probe"),
    }
}
