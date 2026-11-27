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
#[macro_export] macro_rules! mke_fl18 { ($t:ident, $n:ident) => { impl $t for S_fl18 { fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } } }
#[macro_export] macro_rules! mkp_fl18 { ($t:ident, $n:ident) => { impl $t for S_fl18 { fn $n(&self) {} } } }
pub trait Te_fl18 { fn goe(&self); }
pub trait Tp_fl18 { fn gop(&self); }
pub struct S_fl18;
pub fn emit_fl18() { crate::mke_fl18!(Te_fl18, goe); crate::mkp_fl18!(Tp_fl18, gop); }

pub fn fl18_e() { S_fl18.goe(); }
pub fn fl18_q() { S_fl18.gop(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl18_e" => fl18_e(),
        "fl18_q" => fl18_q(),
        _ => panic!("unknown probe"),
    }
}
