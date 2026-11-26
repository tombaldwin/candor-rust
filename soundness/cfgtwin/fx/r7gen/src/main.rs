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
pub fn eff_f_r2gen() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub trait Tr_f_r2gen { fn m(&self) { eff_f_r2gen() } }
pub struct W_f_r2gen<T>(pub T);
#[cfg(unix)] impl Tr_f_r2gen for W_f_r2gen<u8> { fn m(&self) {} }
#[cfg(unix)] impl Tr_f_r2gen for W_f_r2gen<u16> {}

pub fn f_r2gen_q() { W_f_r2gen(1u8).m(); }
pub fn f_r2gen_e() { W_f_r2gen(1u16).m(); }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "f_r2gen_q" => f_r2gen_q(),
        "f_r2gen_e" => f_r2gen_e(),
        _ => panic!("unknown probe"),
    }
}
