#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub mod m_bl1 { pub trait Sup { fn lsf(); } }
pub trait LSub_bl1: fxdep_bl1::Sup {}
pub struct Noisy_bl1; impl m_bl1::Sup for Noisy_bl1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Quiet_bl1; impl fxdep_bl1::Sup for Quiet_bl1 { fn lsf() {} } impl LSub_bl1 for Quiet_bl1 {}
pub struct Loud_bl1; impl fxdep_bl1::Sup for Loud_bl1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub_bl1 for Loud_bl1 {}
fn g_bl1<T: LSub_bl1>() { T::lsf() }
fn g_bl1q<T: LSub_bl1>() { T::lsf() }
pub fn bl1_collide_q() { g_bl1q::<Quiet_bl1>() }
pub fn bl1_collide_e() { g_bl1::<Loud_bl1>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl1_collide_q" => bl1_collide_q(),
        "bl1_collide_e" => bl1_collide_e(),
        _ => panic!("unknown probe"),
    }
}
