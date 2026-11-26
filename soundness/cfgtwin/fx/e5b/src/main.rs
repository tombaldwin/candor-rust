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
pub struct QG_e5b;
pub trait ExtG_e5b { fn go_e5b(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }  }

#[macro_export] macro_rules! wov_e5b { () => { #[cfg(not(unix))] impl ExtG_e5b for QG_e5b { fn go_e5b(&self) {} } } }
crate::wov_e5b! {}
cfg_x! { #[cfg(unix)] impl ExtG_e5b for QG_e5b { fn go_e5b(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }

pub fn e5b_e() { use ExtG_e5b; QG_e5b.go_e5b(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "e5b_e" => e5b_e(),
        _ => panic!("unknown probe"),
    }
}
