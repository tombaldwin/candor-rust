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
pub fn eff_f_r2leaf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub trait Tr_f_r2leaf { fn m(&self) { eff_f_r2leaf() } }
pub mod xa_f_r2leaf { pub struct S; }
pub mod yb_f_r2leaf { pub struct S; }
#[cfg(unix)] impl Tr_f_r2leaf for xa_f_r2leaf::S { fn m(&self) {} }
#[cfg(unix)] impl Tr_f_r2leaf for yb_f_r2leaf::S {}

pub fn f_r2leaf_q() { xa_f_r2leaf::S.m(); }
pub fn f_r2leaf_e() { yb_f_r2leaf::S.m(); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f_r2leaf_q" => f_r2leaf_q(),
        "f_r2leaf_e" => f_r2leaf_e(),
        _ => panic!("unknown probe"),
    }
}
