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
pub fn eff_r2() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub struct S_r2; pub trait T_r2 { fn t(&self) { eff_r2() } }
#[cfg(not(unix))] impl T_r2 for S_r2 { fn t(&self) {} }
#[cfg(unix)] impl T_r2 for S_r2 {}
fn gen_r2<X: T_r2>(x: &X) { x.t() }
pub struct S2_r2; pub trait T2_r2 { fn t2(&self) { eff_r2() } }
#[cfg(unix)] impl T2_r2 for S2_r2 {}
#[cfg(not(unix))] impl T2_r2 for S2_r2 { fn t2(&self) {} }
pub struct S3_r2; pub trait T3_r2 { fn t3(&self) { eff_r2() } }
#[cfg(not(unix))] mod gm_r2 { impl crate::T3_r2 for crate::S3_r2 { fn t3(&self) {} } }
#[cfg(unix)] mod gm_r2 { impl crate::T3_r2 for crate::S3_r2 {} }
pub struct S4_r2; pub trait T4_r2 { fn t4(&self) { eff_r2() } }
impl T4_r2 for S4_r2 { #[cfg(not(unix))] fn t4(&self) {} }
pub struct S5_r2; pub trait T5_r2<A> { fn t5(&self) { eff_r2() } }
#[cfg(not(unix))] impl T5_r2<u8> for S5_r2 { fn t5(&self) {} }
#[cfg(unix)] impl T5_r2<u8> for S5_r2 {}
pub struct S6_r2; pub trait T6_r2 { fn t6(&self) { eff_r2() } }
#[macro_export] macro_rules! wov_r2 { () => { #[cfg(not(unix))] impl T6_r2 for S6_r2 { fn t6(&self) {} } } }
wov_r2! {}
#[cfg(unix)] pub mod vis_r2 { impl crate::T6_r2 for crate::S6_r2 {} }
#[cfg(not(unix))] pub mod vis_r2 {}
pub struct Pq_r2; pub trait Tq_r2 { fn tq(&self) {} }
#[cfg(not(unix))] impl Tq_r2 for Pq_r2 { fn tq(&self) {} }
#[cfg(unix)] impl Tq_r2 for Pq_r2 {}
pub struct Bo_r2; pub trait Tb_r2 { fn tb(&self) { eff_r2() } }
#[cfg(not(unix))] impl Tb_r2 for Bo_r2 { fn tb(&self) {} }
#[cfg(unix)] impl Tb_r2 for Bo_r2 { fn tb(&self) {} }
pub struct Bm_r2; pub trait Tm_r2 { fn tm(&self) { eff_r2() } }
impl Tm_r2 for Bm_r2 { #[cfg(unix)] fn tm(&self) {} #[cfg(not(unix))] fn tm(&self) {} }

pub fn r2_meth_e() { S_r2.t(); }
pub fn r2_path_e() { S_r2::t(&S_r2); }
pub fn r2_trpath_e() { T_r2::t(&S_r2); }
pub fn r2_ufcs_e() { <S_r2 as T_r2>::t(&S_r2); }
pub fn r2_dyn_e() { let d: &dyn T_r2 = &S_r2; d.t(); }
pub fn r2_gen_e() { gen_r2(&S_r2); }
pub fn r2_ref_e() { let r = &S_r2; r.t(); }
pub fn r2_rev_e() { S2_r2.t2(); }
pub fn r2_gmod_e() { S3_r2.t3(); }
pub fn r2_member_e() { S4_r2.t4(); }
pub fn r2_gentr_e() { S5_r2.t5(); }
pub fn r2_p_e() { S6_r2.t6(); }
pub fn r2_pure_q() { Pq_r2.tq(); }
pub fn r2_bothov_q() { Bo_r2.tb(); }
pub fn r2_bothmem_q() { Bm_r2.tm(); }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "r2_meth_e" => r2_meth_e(),
        "r2_path_e" => r2_path_e(),
        "r2_trpath_e" => r2_trpath_e(),
        "r2_ufcs_e" => r2_ufcs_e(),
        "r2_dyn_e" => r2_dyn_e(),
        "r2_gen_e" => r2_gen_e(),
        "r2_ref_e" => r2_ref_e(),
        "r2_rev_e" => r2_rev_e(),
        "r2_gmod_e" => r2_gmod_e(),
        "r2_member_e" => r2_member_e(),
        "r2_gentr_e" => r2_gentr_e(),
        "r2_p_e" => r2_p_e(),
        "r2_pure_q" => r2_pure_q(),
        "r2_bothov_q" => r2_bothov_q(),
        "r2_bothmem_q" => r2_bothmem_q(),
        _ => panic!("unknown probe"),
    }
}
