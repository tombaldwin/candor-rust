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
#[cfg(unix)] #[macro_use] mod a1_r3 { macro_rules! k1_r3 { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[cfg(not(unix))] #[macro_use] mod a1_r3 { macro_rules! k1_r3 { () => {} } }
#[cfg(not(unix))] #[macro_use] mod a2_r3 { macro_rules! k2_r3 { () => {} } }
#[cfg(unix)] #[macro_use] mod a2_r3 { macro_rules! k2_r3 { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[cfg(unix)] #[macro_use] mod a3_r3 { #[macro_use] mod n { macro_rules! k3_r3 { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } } }
#[cfg(not(unix))] #[macro_use] mod a3_r3 { #[macro_use] mod n { macro_rules! k3_r3 { () => {} } } }
#[cfg(unix)] #[macro_use] mod a4_r3 { wmdef! { k4_r3 } }
#[cfg(not(unix))] #[macro_use] mod a4_r3 { macro_rules! k4_r3 { () => {} } }
#[cfg(unix)] mod a6_r3 { #[macro_export] macro_rules! k6_r3 { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[cfg(not(unix))] mod a6_r3 { #[macro_export] macro_rules! k6_r3 { () => {} } }
pub fn pk7_r3() {}
#[cfg(unix)] #[macro_use] mod a7_r3 { macro_rules! k7_r3 { () => { crate::pk7_r3() } } }
#[cfg(not(unix))] #[macro_use] mod a7_r3 { macro_rules! k7_r3 { () => { crate::pk7_r3() } } }
pub mod user_r3 { pub fn run() { k1_r3!(); } }

pub fn r3_inline_e() { k1_r3!(); }
pub fn r3_rev_e() { k2_r3!(); }
pub fn r3_nested_e() { k3_r3!(); }
pub fn r3_p_e() { k4_r3!(); }
pub fn r3_child_e() { user_r3::run(); }
pub fn r3_export_e() { crate::k6_r3!(); }
pub fn r3_same_q() { k7_r3!(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r3_inline_e" => r3_inline_e(),
        "r3_rev_e" => r3_rev_e(),
        "r3_nested_e" => r3_nested_e(),
        "r3_p_e" => r3_p_e(),
        "r3_child_e" => r3_child_e(),
        "r3_export_e" => r3_export_e(),
        "r3_same_q" => r3_same_q(),
        _ => panic!("unknown probe"),
    }
}
