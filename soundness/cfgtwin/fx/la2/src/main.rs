#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
mod id_private { pub trait IdPrivate { fn new_la2() -> Self; } }
use id_private::IdPrivate;
pub trait Id_la2: IdPrivate {}
pub trait Item_la2 { type Id: Id_la2; }
pub struct IdE_la2; impl IdPrivate for IdE_la2 { fn new_la2() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); IdE_la2 } } impl Id_la2 for IdE_la2 {}
pub struct ItE_la2; impl Item_la2 for ItE_la2 { type Id = IdE_la2; }
fn g_la2<T: Item_la2>() { let _ = T::Id::new_la2(); }
fn g2_la2<T: Id_la2>() { let _ = T::new_la2(); }
pub fn la2_localmod_proj_e() { g_la2::<ItE_la2>() }
pub fn la2_localmod_path_e() { g2_la2::<IdE_la2>() }


fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "la2_localmod_proj_e" => la2_localmod_proj_e(),
        "la2_localmod_path_e" => la2_localmod_path_e(),
        _ => panic!("unknown probe"),
    }
}
