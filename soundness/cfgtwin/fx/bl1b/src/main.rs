#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub mod m_bl1b { pub trait Default { fn default() -> Self; } }
pub trait LSub_bl1b: std::default::Default {}
pub struct Noisy_bl1b; impl m_bl1b::Default for Noisy_bl1b { fn default() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Noisy_bl1b } }
#[derive(Default)] pub struct Quiet_bl1b; impl LSub_bl1b for Quiet_bl1b {}
fn g_bl1b<T: LSub_bl1b>() { let _ = T::default(); }
pub fn bl1b_stdcollide_q() { g_bl1b::<Quiet_bl1b>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bl1b_stdcollide_q" => bl1b_stdcollide_q(),
        _ => panic!("unknown probe"),
    }
}
