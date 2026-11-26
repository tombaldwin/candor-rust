#![allow(unused)]
pub trait Fetch: Sized { fn fetch_all(self, p: &str) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }
pub trait Quiet: Sized { fn hush_zz(self, p: &str) -> usize { p.len() } }
#[macro_export] macro_rules! gen_impls { ($t:ty) => { impl crate::Fetch for $t {} impl crate::Quiet for $t {} impl $t { pub fn inh_zz(&self, p: &str) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }; }
pub mod km { pub struct Kmac(pub String); pub fn kmac(s: &str) -> Kmac { Kmac(s.to_string()) } crate::gen_impls!(Kmac); }

pub fn fr1814_e() { km::kmac("x").fetch_all("p"); }
pub fn fr1814_inh_e() { km::kmac("x").inh_zz("p"); }
pub fn fr1814_q() { let _ = km::kmac("x").hush_zz("p"); }
fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fr1814_e" => fr1814_e(),
        "fr1814_inh_e" => fr1814_inh_e(),
        "fr1814_q" => fr1814_q(),
        _ => panic!("unknown probe"),
    }
}
