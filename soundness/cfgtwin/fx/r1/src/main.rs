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
pub fn eff_r1() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } pub fn pur_r1() {}
#[cfg(unix)] pub mod tw_r1 { pub use crate::eff_r1 as g; }
#[cfg(not(unix))] pub mod tw_r1 { pub use crate::pur_r1 as g; }
#[cfg(not(unix))] pub mod tr_r1 { pub use crate::pur_r1 as g; }
#[cfg(unix)] pub mod tr_r1 { pub use crate::eff_r1 as g; }
pub mod in_r1 { #[cfg(unix)] pub use crate::eff_r1 as g; #[cfg(not(unix))] pub use crate::pur_r1 as g; }
#[cfg(unix)] pub mod t3_r1 { pub use crate::eff_r1 as g; }
#[cfg(all(not(unix), windows))] pub mod t3_r1 { pub use crate::pur_r1 as g; }
#[cfg(all(not(unix), not(windows)))] pub mod t3_r1 { pub use crate::pur_r1 as g; }
#[cfg(unix)] pub mod rx_r1 { pub use crate::eff_r1 as gx_r1; }
#[cfg(not(unix))] pub mod rx_r1 { pub use crate::pur_r1 as gx_r1; }
pub use rx_r1::gx_r1;
pub mod user_r1 { use crate::tw_r1::g; pub fn run() { g() } }
pub mod sole_r1 { pub use crate::eff_r1 as g; }
cfg_x! {
#[cfg(unix)] pub mod pw_r1 { wuse! { g, crate::eff_r1 } pub use crate::pur_r1 as keep; }
#[cfg(not(unix))] pub mod pw_r1 { pub use crate::pur_r1 as g; pub use crate::pur_r1 as keep; }
}

pub fn r1_tw_e() { tw_r1::g(); }
pub fn r1_rev_e() { tr_r1::g(); }
pub fn r1_in_e() { in_r1::g(); }
pub fn r1_three_e() { t3_r1::g(); }
pub fn r1_crate_e() { crate::tw_r1::g(); }
pub fn r1_bodyuse_e() { use crate::tw_r1::g; g(); }
pub fn r1_bodyrel_e() { use tw_r1::g; g(); }
pub fn r1_moduse_e() { user_r1::run(); }
pub fn r1_fnptr_e() { let f = tw_r1::g; f(); }
pub fn r1_reexp_e() { gx_r1(); }
pub fn r1_reexp2_e() { crate::gx_r1(); }
pub fn r1_sole_e() { use sole_r1::g; g(); }
pub fn r1_p_e() { pw_r1::g(); }
pub fn r1_p_q() { pw_r1::keep(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r1_tw_e" => r1_tw_e(),
        "r1_rev_e" => r1_rev_e(),
        "r1_in_e" => r1_in_e(),
        "r1_three_e" => r1_three_e(),
        "r1_crate_e" => r1_crate_e(),
        "r1_bodyuse_e" => r1_bodyuse_e(),
        "r1_bodyrel_e" => r1_bodyrel_e(),
        "r1_moduse_e" => r1_moduse_e(),
        "r1_fnptr_e" => r1_fnptr_e(),
        "r1_reexp_e" => r1_reexp_e(),
        "r1_reexp2_e" => r1_reexp2_e(),
        "r1_sole_e" => r1_sole_e(),
        "r1_p_e" => r1_p_e(),
        "r1_p_q" => r1_p_q(),
        _ => panic!("unknown probe"),
    }
}
