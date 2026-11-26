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
pub mod imp_r6 { pub fn eff() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
#[cfg(unix)] pub mod tw_r6 { pub use crate::imp_r6::eff as g; }
#[cfg(not(unix))] pub mod tw_r6 { pub fn g() {} }
pub mod user_r6 { use crate::tw_r6::g; pub fn run() { g() } }

pub fn r6_body_e() { use tw_r6::g; g(); }
pub fn r6_mod_e() { user_r6::run(); }
pub fn r6_path_e() { tw_r6::g(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r6_body_e" => r6_body_e(),
        "r6_mod_e" => r6_mod_e(),
        "r6_path_e" => r6_path_e(),
        _ => panic!("unknown probe"),
    }
}
