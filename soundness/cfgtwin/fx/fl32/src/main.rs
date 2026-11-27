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
pub mod imp_fl32 { pub fn pur() {} pub fn eff() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
#[cfg(unix)] pub mod tw_fl32 { pub fn g() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
#[cfg(not(unix))] pub mod tw_fl32 { pub use crate::imp_fl32::pur as g; }
#[cfg(unix)] pub mod twq_fl32 { pub fn g() {} }
#[cfg(not(unix))] pub mod twq_fl32 { pub use crate::imp_fl32::pur as g; }
#[cfg(unix)] pub mod twr_fl32 { pub fn g() {} }
#[cfg(not(unix))] pub mod twr_fl32 { pub use crate::imp_fl32::eff as g; }
pub mod fac_fl32 { pub use crate::imp_fl32::pur as k; pub use crate::imp_fl32::eff as g; }
pub mod sb_fl32 { #[cfg(unix)] pub fn g() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } #[cfg(not(unix))] pub use crate::imp_fl32::pur as g; }
pub mod sbq_fl32 { #[cfg(unix)] pub fn g() {} #[cfg(not(unix))] pub use crate::imp_fl32::pur as g; }
use crate::tw_fl32::g as h_fl32;

pub fn fl32_mod_e() { h_fl32(); }
pub fn fl32_body_e() { use crate::tw_fl32::g; g(); }
pub fn fl32_q() { use crate::twq_fl32::g; g(); }
pub fn fl32_path_e() { crate::tw_fl32::g(); }
pub fn fl32r_e() { use crate::twr_fl32::g; g(); }
pub fn fl32fac_e() { use crate::fac_fl32::g; g(); }
pub fn fl32fac_q() { use crate::fac_fl32::k; k(); }
pub fn fl32sb_e() { use crate::sb_fl32::g; g(); }
pub fn fl32sb_q() { use crate::sbq_fl32::g; g(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl32_mod_e" => fl32_mod_e(),
        "fl32_body_e" => fl32_body_e(),
        "fl32_q" => fl32_q(),
        "fl32_path_e" => fl32_path_e(),
        "fl32r_e" => fl32r_e(),
        "fl32fac_e" => fl32fac_e(),
        "fl32fac_q" => fl32fac_q(),
        "fl32sb_e" => fl32sb_e(),
        "fl32sb_q" => fl32sb_q(),
        _ => panic!("unknown probe"),
    }
}
