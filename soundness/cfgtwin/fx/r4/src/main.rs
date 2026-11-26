#![allow(unused)]
macro_rules! cfg_x { ($($i:item)*) => { $($i)* } }   // "produced": an arm written inside cfg_x!{} is spliced; outside it is hand-written
#[macro_export] macro_rules! wt   { ($(fn $n:ident(&self);)*) => { $( fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } )* } }          // trait-item text -> effectful default(s)
#[macro_export] macro_rules! wfn  { ($v:vis fn $n:ident();) => { $v fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }                     // item text -> effectful free fn
#[macro_export] macro_rules! wim  { ($o:path, $t:path, $n:ident) => { impl $t for $o { fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }  // trait impl, effectful member
#[macro_export] macro_rules! winh { ($o:path, $n:ident) => { impl $o { pub fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }        // inherent effectful method
#[macro_export] macro_rules! wuse { ($n:ident, $src:path) => { pub use $src as $n; } }                      // re-export under a name
#[macro_export] macro_rules! wty  { ($n:ident, $src:path) => { pub type $n = $src; } }                      // type alias
#[macro_export] macro_rules! wmdef{ ($n:ident) => { macro_rules! $n { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }                   // a macro_rules! definition
#[macro_export] macro_rules! wmod { ($m:ident, $n:ident) => { pub mod $m { pub fn $n() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }        // nested module with an effectful fn
pub struct Q;
pub fn eff_r4() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub struct S_r4;
#[cfg(unix)] pub mod m_r4 { pub mod a { pub mod b { pub trait T { fn go(&self) { crate::eff_r4() } } } } }
#[cfg(not(unix))] pub mod m_r4 { pub mod a { pub trait T { fn go(&self) {} } pub mod b { pub use super::*; } } }
impl m_r4::a::b::T for S_r4 {}
fn gen_r4<X: m_r4::a::b::T>(x: &X) { x.go() }
pub struct P_r4;
#[cfg(unix)] #[path = "live_r4.rs"] pub mod pm_r4;
#[cfg(not(unix))] #[path = "pure_r4.rs"] pub mod pm_r4;
impl pm_r4::TP for P_r4 {}
pub struct W_r4;
cfg_x! {
#[cfg(unix)] #[path = "plive_r4.rs"] pub mod wm_r4;
#[cfg(not(unix))] #[path = "ppure_r4.rs"] pub mod wm_r4;
}
impl wm_r4::TW for W_r4 {}
pub struct R_r4;
pub mod u1_r4 { pub trait TU { fn gu(&self) {} } }
pub mod u2_r4 { pub trait TU { fn gu(&self) {} } }
impl u1_r4::TU for R_r4 {}
pub struct R2_r4;
pub mod v1_r4 { pub trait TV { fn gv(&self) {} } }
pub mod v2_r4 { pub trait TV { fn gv(&self) { crate::eff_r4() } } }
impl v1_r4::TV for R2_r4 {}

pub fn r4_meth_e() { use m_r4::a::b::T; S_r4.go(); }
pub fn r4_path_e() { use m_r4::a::b::T; S_r4::go(&S_r4); }
pub fn r4_ufcs_e() { <S_r4 as m_r4::a::b::T>::go(&S_r4); }
pub fn r4_gen_e() { gen_r4(&S_r4); }
pub fn r4_pmeth_e() { use pm_r4::TP; P_r4.gp(); }
pub fn r4_p_e() { use wm_r4::TW; W_r4.gw(); }
pub fn r4_unrel_q() { use u1_r4::TU; R_r4.gu(); }
pub fn r4_unrel2_q() { use v1_r4::TV; R2_r4.gv(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r4_meth_e" => r4_meth_e(),
        "r4_path_e" => r4_path_e(),
        "r4_ufcs_e" => r4_ufcs_e(),
        "r4_gen_e" => r4_gen_e(),
        "r4_pmeth_e" => r4_pmeth_e(),
        "r4_p_e" => r4_p_e(),
        "r4_unrel_q" => r4_unrel_q(),
        "r4_unrel2_q" => r4_unrel2_q(),
        _ => panic!("unknown probe"),
    }
}
