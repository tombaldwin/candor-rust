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
pub trait Tr_d8v4 { fn go_d8v4(&self); }
cfg_if::cfg_if! {
    if #[cfg(unix)] { pub mod m_d8v4 { crate::wim! { crate::Q, crate::Tr_d8v4, go_d8v4 } pub fn pure_d8v4() {} } } else { pub mod m_d8v4 { impl crate::Tr_d8v4 for crate::Q { fn go_d8v4(&self) {} } pub fn pure_d8v4() {} } }
}

pub fn d8v4_e() { use crate::Tr_d8v4; Q.go_d8v4(); }
pub fn d8v4_q() { m_d8v4::pure_d8v4(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d8v4_e" => d8v4_e(),
        "d8v4_q" => d8v4_q(),
        _ => panic!("unknown probe"),
    }
}
