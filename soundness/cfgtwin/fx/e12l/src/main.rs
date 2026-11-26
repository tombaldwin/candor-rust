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
pub struct QG_e12l;
pub fn via_e12l<I: Iterator>(i: I) { i.count(); }
#[macro_export] macro_rules! wov_e12l { () => { #[cfg(unix)] impl Iterator for QG_e12l { type Item = u8; fn next(&mut self) -> Option<u8> { None } fn count(self) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } } } }
crate::wov_e12l! {}
cfg_x! { #[cfg(not(unix))] impl Iterator for QG_e12l { type Item = u8; fn next(&mut self) -> Option<u8> { None } } }

pub fn e12l_e() { QG_e12l.count(); }
pub fn e12l_b() { via_e12l(QG_e12l); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e12l_e" => e12l_e(),
        "e12l_b" => e12l_b(),
        _ => panic!("unknown probe"),
    }
}
