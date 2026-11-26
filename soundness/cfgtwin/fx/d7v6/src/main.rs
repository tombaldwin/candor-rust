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
pub struct E_d7v6; impl E_d7v6 { pub fn go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } pub struct P_d7v6; impl P_d7v6 { pub fn go(&self) {} } pub fn via_d7v6(x: &m_d7v6::Tt_d7v6) { x.go() } pub fn viaq_d7v6(x: &m_d7v6::Tq_d7v6) { x.go() }
#[cfg_attr(unix, path = "live_d7v6.rs")] #[cfg_attr(not(unix), path = "pure_d7v6.rs")] pub mod m_d7v6;

pub fn d7v6_e() { via_d7v6(&E_d7v6); }
pub fn d7v6_q() { viaq_d7v6(&P_d7v6); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "d7v6_e" => d7v6_e(),
        "d7v6_q" => d7v6_q(),
        _ => panic!("unknown probe"),
    }
}
