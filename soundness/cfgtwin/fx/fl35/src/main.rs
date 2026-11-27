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
#[macro_export] macro_rules! wcfg_fl35 { () => {
    #[cfg(unix)] pub mod s { pub struct T; impl T { pub fn go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } }
    #[cfg(not(unix))] pub mod s;
} }
pub mod host_fl35 { crate::wcfg_fl35! {} pub mod t; }

pub fn fl35_let_e() { let t = host_fl35::s::T; t.go(); }
pub fn fl35_ann_e() { let t: host_fl35::s::T = host_fl35::s::T; t.go(); }
pub fn fl35_use_e() { use host_fl35::s::T; T.go(); }
pub fn fl35_ufcs_e() { host_fl35::s::T::go(&host_fl35::s::T); }
pub fn fl35_q() { let u = host_fl35::t::U; u.go(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "fl35_let_e" => fl35_let_e(),
        "fl35_ann_e" => fl35_ann_e(),
        "fl35_use_e" => fl35_use_e(),
        "fl35_ufcs_e" => fl35_ufcs_e(),
        "fl35_q" => fl35_q(),
        _ => panic!("unknown probe"),
    }
}
