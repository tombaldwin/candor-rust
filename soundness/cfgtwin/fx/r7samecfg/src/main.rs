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
pub fn eff_f_r2samecfg() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub trait Tr_f_r2samecfg { fn m(&self) { eff_f_r2samecfg() } fn n(&self) { eff_f_r2samecfg() } }
pub struct S_f_r2samecfg;
#[cfg(unix)] impl Tr_f_r2samecfg for S_f_r2samecfg { fn m(&self) {} }
pub struct U_f_r2samecfg;
#[cfg(unix)] impl Tr_f_r2samecfg for U_f_r2samecfg { fn n(&self) {} }

pub fn f_r2samecfg_q() { S_f_r2samecfg.m(); }
pub fn f_r2samecfg_q2() { U_f_r2samecfg.n(); }
pub fn f_r2samecfg_e() { S_f_r2samecfg.n(); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f_r2samecfg_q" => f_r2samecfg_q(),
        "f_r2samecfg_q2" => f_r2samecfg_q2(),
        "f_r2samecfg_e" => f_r2samecfg_e(),
        _ => panic!("unknown probe"),
    }
}
