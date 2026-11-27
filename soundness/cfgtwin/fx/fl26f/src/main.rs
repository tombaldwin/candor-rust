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
pub mod imp_fl26f { pub fn pur() {} }
#[cfg(unix)] pub mod tw_fl26f { pub fn g() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
#[cfg(not(unix))] pub mod tw_fl26f { pub use crate::imp_fl26f::pur as g; }
#[cfg(unix)] pub mod twq_fl26f { pub fn g() {} }
#[cfg(not(unix))] pub mod twq_fl26f { pub use crate::imp_fl26f::pur as g; }
use tw_fl26f::g as h_fl26f;

pub fn fl26f_body_e() { use tw_fl26f::g; g(); }
pub fn fl26f_mod_e() { h_fl26f(); }
pub fn fl26f_q() { use twq_fl26f::g; g(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl26f_body_e" => fl26f_body_e(),
        "fl26f_mod_e" => fl26f_mod_e(),
        "fl26f_q" => fl26f_q(),
        _ => panic!("unknown probe"),
    }
}
