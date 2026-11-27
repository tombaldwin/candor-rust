#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// R1964 (N8) — `T: dep::Dt + Lt`, `T::lf()` with `lf` declared by the LOCAL bound: a dependency key `dep#Dt::lf` names a member Dt cannot declare (E0034).
use fxdep_lb3::Dt;
pub trait Lt1 { fn lf(); } pub struct Loud1; impl Dt for Loud1 { fn f() {} } impl Lt1 for Loud1 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g1<T: fxdep_lb3::Dt + Lt1>() { T::lf() }
pub fn lb3_mixed_e() { g1::<Loud1>() }
// seeded C3: the only implementor is pure, so the dependency key was the row's only `invisible`
pub trait Lt2 { fn lf(); } pub struct P2; impl Dt for P2 { fn f() {} } impl Lt2 for P2 { fn lf() {} }
fn g2<T: fxdep_lb3::Dt + Lt2>() { T::lf() }
pub fn lb3_pure_q() { g2::<P2>() }
// thirteen implementors (one writing): the CHA hedges
pub trait Lt3 { fn lf(); } pub struct L3; impl Dt for L3 { fn f() {} } impl Lt3 for L3 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Q3a; impl Lt3 for Q3a { fn lf() {} }
pub struct Q3b; impl Lt3 for Q3b { fn lf() {} }
pub struct Q3c; impl Lt3 for Q3c { fn lf() {} }
pub struct Q3d; impl Lt3 for Q3d { fn lf() {} }
pub struct Q3e; impl Lt3 for Q3e { fn lf() {} }
pub struct Q3f; impl Lt3 for Q3f { fn lf() {} }
pub struct Q3g; impl Lt3 for Q3g { fn lf() {} }
pub struct Q3h; impl Lt3 for Q3h { fn lf() {} }
pub struct Q3i; impl Lt3 for Q3i { fn lf() {} }
pub struct Q3j; impl Lt3 for Q3j { fn lf() {} }
pub struct Q3k; impl Lt3 for Q3k { fn lf() {} }
pub struct Q3l; impl Lt3 for Q3l { fn lf() {} }
fn g3<T: fxdep_lb3::Dt + Lt3>() { T::lf() }
pub fn lb3_many_e() { g3::<L3>() }
// the member is declared by a LOCAL supertrait of the local bound
pub trait LSup4 { fn lf(); } pub trait Lt4: LSup4 {} pub struct L4; impl Dt for L4 { fn f() {} } impl LSup4 for L4 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt4 for L4 {}
fn g4<T: fxdep_lb3::Dt + Lt4>() { T::lf() }
pub fn lb3_supl_e() { g4::<L4>() }
// KEEP: the member comes from a DEPENDENCY supertrait of the local bound
pub trait Lt5: fxdep_lb3::SupD {} pub struct L5; impl Dt for L5 { fn f() {} } impl fxdep_lb3::SupD for L5 { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt5 for L5 {}
fn g5<T: fxdep_lb3::Dt + Lt5>() { T::sf() }
pub fn lb3_depsup_e() { g5::<L5>() }
// KEEP: a dependency supertrait shares its LEAF with a local trait that declares the member
pub mod m7 { pub trait Sup { fn lf(); } } pub struct N7; impl m7::Sup for N7 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub trait Lt7: fxdep_lb3::Sup {} pub struct L7; impl Dt for L7 { fn f() {} } impl fxdep_lb3::Sup for L7 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt7 for L7 {}
pub struct Q7; impl Dt for Q7 { fn f() {} } impl fxdep_lb3::Sup for Q7 { fn lf() {} } impl Lt7 for Q7 {}
fn g7<T: fxdep_lb3::Dt + Lt7>() { T::lf() }
fn g7q<T: fxdep_lb3::Dt + Lt7>() { T::lf() }
pub fn lb3_collide_e() { g7::<L7>() }
pub fn lb3_collide_q() { g7q::<Q7>() }
// the method spelling, a where clause, an impl-level bound, the dependency bound written bare
pub trait Lt9 { fn lm(&self); } pub struct L9; impl Dt for L9 { fn f() {} } impl Lt9 for L9 { fn lm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g9<T: fxdep_lb3::Dt + Lt9>(t: T) { t.lm() }
pub fn lb3_meth_e() { g9(L9) }
pub trait Lt10 { fn lf(); } pub struct L10; impl Dt for L10 { fn f() {} } impl Lt10 for L10 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g10<T>() where T: fxdep_lb3::Dt + Lt10 { T::lf() }
pub fn lb3_where_e() { g10::<L10>() }
pub trait Lt11 { fn lf(); } pub struct L11; impl Dt for L11 { fn f() {} } impl Lt11 for L11 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct H11<T>(T); impl<T: fxdep_lb3::Dt + Lt11> H11<T> { fn go() { T::lf() } }
pub fn lb3_impl_e() { H11::<L11>::go() }
pub trait Lt12 { fn lf(); } pub struct L12; impl Dt for L12 { fn f() {} } impl Lt12 for L12 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g12<T: Dt + Lt12>() { T::lf() }
pub fn lb3_bare_e() { g12::<L12>() }
// the dependency member itself, through the same mixed bound (the control that keeps its key)
pub trait Lt13 { fn lf(); } pub struct L13; impl Dt for L13 { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt13 for L13 { fn lf() {} }
fn g13<T: fxdep_lb3::Dt + Lt13>() { T::f() }
pub fn lb3_depmem_e() { g13::<L13>() }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lb3_mixed_e" => lb3_mixed_e(),
        "lb3_pure_q" => lb3_pure_q(),
        "lb3_many_e" => lb3_many_e(),
        "lb3_supl_e" => lb3_supl_e(),
        "lb3_depsup_e" => lb3_depsup_e(),
        "lb3_collide_e" => lb3_collide_e(),
        "lb3_collide_q" => lb3_collide_q(),
        "lb3_meth_e" => lb3_meth_e(),
        "lb3_where_e" => lb3_where_e(),
        "lb3_impl_e" => lb3_impl_e(),
        "lb3_bare_e" => lb3_bare_e(),
        "lb3_depmem_e" => lb3_depmem_e(),
        _ => panic!("unknown probe"),
    }
}
