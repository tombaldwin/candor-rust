#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
pub trait LSup_bd1 { fn lsf(); }
pub trait LSub_bd1: LSup_bd1 { fn go() { Self::lsf() } }
pub struct LS_bd1; impl LSup_bd1 for LS_bd1 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl LSub_bd1 for LS_bd1 {}
pub struct LSQ_bd1; impl LSup_bd1 for LSQ_bd1 { fn lsf() {} } impl LSub_bd1 for LSQ_bd1 {}
pub struct LD_bd1; impl fxdep_bd1::SupD for LD_bd1 { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl fxdep_bd1::SubD for LD_bd1 {}
pub struct W_bd1<T>(std::marker::PhantomData<T>);
impl<T: LSub_bd1> W_bd1<T> { pub fn go() { T::lsf() } }
pub struct H_bd1; impl H_bd1 { pub fn gm<T: LSub_bd1>(&self) { T::lsf() } }
fn g_where<T>() where T: LSub_bd1 { T::lsf() }
fn g_whereq<T>() where T: LSub_bd1 { T::lsf() }
fn g_selfdef<T: LSub_bd1>() { T::go() }
pub fn bd1_where_e() { g_where::<LS_bd1>() }
pub fn bd1_where_q() { g_whereq::<LSQ_bd1>() }
pub fn bd1_implgen_e() { W_bd1::<LS_bd1>::go() }
pub fn bd1_method_gen_e() { H_bd1.gm::<LS_bd1>() }
pub fn bd1_selfdef_e() { g_selfdef::<LS_bd1>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "bd1_where_e" => bd1_where_e(),
        "bd1_where_q" => bd1_where_q(),
        "bd1_implgen_e" => bd1_implgen_e(),
        "bd1_method_gen_e" => bd1_method_gen_e(),
        "bd1_selfdef_e" => bd1_selfdef_e(),
        _ => panic!("unknown probe"),
    }
}
