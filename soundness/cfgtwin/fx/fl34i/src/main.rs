use fxdep_fl34i::{Af, HasA};
pub struct Loud; impl Af for Loud { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Quiet; impl Af for Quiet { fn f() {} }
pub struct ItE; impl Iterator for ItE { type Item = Loud; fn next(&mut self) -> Option<Loud> { None } } impl HasA for ItE { type A = Loud; }
pub struct ItQ; impl Iterator for ItQ { type Item = Quiet; fn next(&mut self) -> Option<Quiet> { None } } impl HasA for ItQ { type A = Quiet; }
mod g_fl34i {
    use super::Af;
    pub fn iter<T: fxdep_fl34i::HasA + Iterator>() where T::Item: Af { T::Item::f() }
}

pub fn fl34i_e() { g_fl34i::iter::<ItE>(); }
pub fn fl34i_q() { g_fl34i::iter::<ItQ>(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl34i_e" => fl34i_e(),
        "fl34i_q" => fl34i_q(),
        _ => panic!("unknown probe"),
    }
}
