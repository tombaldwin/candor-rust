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
pub mod k_r7idmatch { pub struct S; pub trait T { fn t(&self) { crate::eff_r7idmatch() } } }
pub fn eff_r7idmatch() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub mod one_r7idmatch { use crate::k_r7idmatch::{S, T}; #[cfg(not(unix))] impl T for S { fn t(&self) {} } }
pub mod two_r7idmatch { #[cfg(unix)] impl crate::k_r7idmatch::T for crate::k_r7idmatch::S {} }
pub struct V_r7idmatch; pub trait TV_r7idmatch { fn tv(&self) { eff_r7idmatch() } }
pub mod three_r7idmatch { #[cfg(not(unix))] impl super::TV_r7idmatch for super::V_r7idmatch { fn tv(&self) {} } }
#[cfg(unix)] impl TV_r7idmatch for V_r7idmatch {}

pub fn r7idmatch_e() { use k_r7idmatch::T; k_r7idmatch::S.t(); }
pub fn r7idmatch_sup_e() { V_r7idmatch.tv(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r7idmatch_e" => r7idmatch_e(),
        "r7idmatch_sup_e" => r7idmatch_sup_e(),
        _ => panic!("unknown probe"),
    }
}
