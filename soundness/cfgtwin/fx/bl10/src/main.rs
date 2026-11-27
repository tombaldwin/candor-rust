#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub trait LSub2_bl10: fxdep_bl10::SupA + fxdep_bl10::SupB {}
pub struct Loud_bl10; impl fxdep_bl10::SupA for Loud_bl10 { fn sa() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_bl10::SupB for Loud_bl10 { fn sb() {} } impl LSub2_bl10 for Loud_bl10 {}
pub struct Quiet_bl10; impl fxdep_bl10::SupA for Quiet_bl10 { fn sa() {} } impl fxdep_bl10::SupB for Quiet_bl10 { fn sb() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub2_bl10 for Quiet_bl10 {}
fn g_sa<T: LSub2_bl10>() { T::sa() }
fn g_saq<T: LSub2_bl10>() { T::sa() }
pub fn bl10_two_e() { g_sa::<Loud_bl10>() }
pub fn bl10_two_q() { g_saq::<Quiet_bl10>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl10_two_e" => bl10_two_e(),
        "bl10_two_q" => bl10_two_q(),
        _ => panic!("unknown probe"),
    }
}
