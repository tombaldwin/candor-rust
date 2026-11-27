#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]
// R1953 — `where T::Item: Lt`, `Lt: Base`, `lf` declared in `Base`: which implementors can be `T::Item`. One trait family per probe.
pub struct It<X>(std::marker::PhantomData<X>); impl<X> Iterator for It<X> { type Item = X; fn next(&mut self) -> Option<X> { None } }
// F1 the row: Other1 implements only Base1 and writes; the instantiation is a pure Lt1 implementor
pub trait Base1 { fn lf(); } pub trait Lt1: Base1 {}
pub struct LoudL1; impl Base1 for LoudL1 { fn lf() {} } impl Lt1 for LoudL1 {}
pub struct Other1; impl Base1 for Other1 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g1<T: Iterator>() where T::Item: Lt1 { T::Item::lf() }
pub fn lb2_supcha_q() { g1::<It<LoudL1>>() }
// F2 the covered control: the Lt2 implementor writes
pub trait Base2 { fn lf(); } pub trait Lt2: Base2 {}
pub struct Loud2; impl Base2 for Loud2 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt2 for Loud2 {}
pub struct Other2; impl Base2 for Other2 { fn lf() {} }
fn g2<T: Iterator>() where T::Item: Lt2 { T::Item::lf() }
pub fn lb2_cover_e() { g2::<It<Loud2>>() }
// F3 a BLANKET impl of the bound: every Base3 implementor may be T::Item (may cover)
pub trait Base3 { fn lf(); } pub trait Lb3: Base3 {} impl<X: Base3> Lb3 for X {}
pub struct Other3; impl Base3 for Other3 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g3<T: Iterator>() where T::Item: Lb3 { T::Item::lf() }
pub fn lb2_blanket_e() { g3::<It<Other3>>() }
// F4 the two impls spell the type differently
pub trait Base4 { fn lf(); } pub trait Lt4: Base4 {}
pub mod m4 { pub struct Sp4; impl super::Lt4 for Sp4 {} }
impl Base4 for crate::m4::Sp4 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct Other4; impl Base4 for Other4 { fn lf() {} }
fn g4<T: Iterator>() where T::Item: Lt4 { T::Item::lf() }
pub fn lb2_spell_e() { g4::<It<m4::Sp4>>() }
// F5 a generic implementor
pub trait Base5 { fn lf(); } pub trait Lt5: Base5 {}
pub struct W5<X>(X); impl<X> Base5 for W5<X> { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl<X> Lt5 for W5<X> {}
pub struct Other5; impl Base5 for Other5 { fn lf() {} }
fn g5<T: Iterator>() where T::Item: Lt5 { T::Item::lf() }
pub fn lb2_generic_e() { g5::<It<W5<u8>>>() }
// F6 thirteen Base6 implementors (all writing, none Lt6); one pure Lt6 implementor
pub trait Base6 { fn lf(); } pub trait Lt6: Base6 {}
pub struct P6; impl Base6 for P6 { fn lf() {} } impl Lt6 for P6 {}
pub struct O6a; impl Base6 for O6a { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6b; impl Base6 for O6b { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6c; impl Base6 for O6c { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6d; impl Base6 for O6d { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6e; impl Base6 for O6e { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6f; impl Base6 for O6f { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6g; impl Base6 for O6g { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6h; impl Base6 for O6h { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6i; impl Base6 for O6i { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6j; impl Base6 for O6j { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6k; impl Base6 for O6k { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6l; impl Base6 for O6l { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct O6m; impl Base6 for O6m { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g6<T: Iterator>() where T::Item: Lt6 { T::Item::lf() }
pub fn lb2_thirteen_q() { g6::<It<P6>>() }
// F7 thirteen Lt7 implementors, one writing: the CHA cannot bound it
pub trait Base7 { fn lf(); } pub trait Lt7: Base7 {}
pub struct L7; impl Base7 for L7 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt7 for L7 {}
pub struct Q7a; impl Base7 for Q7a { fn lf() {} } impl Lt7 for Q7a {}
pub struct Q7b; impl Base7 for Q7b { fn lf() {} } impl Lt7 for Q7b {}
pub struct Q7c; impl Base7 for Q7c { fn lf() {} } impl Lt7 for Q7c {}
pub struct Q7d; impl Base7 for Q7d { fn lf() {} } impl Lt7 for Q7d {}
pub struct Q7e; impl Base7 for Q7e { fn lf() {} } impl Lt7 for Q7e {}
pub struct Q7f; impl Base7 for Q7f { fn lf() {} } impl Lt7 for Q7f {}
pub struct Q7g; impl Base7 for Q7g { fn lf() {} } impl Lt7 for Q7g {}
pub struct Q7h; impl Base7 for Q7h { fn lf() {} } impl Lt7 for Q7h {}
pub struct Q7i; impl Base7 for Q7i { fn lf() {} } impl Lt7 for Q7i {}
pub struct Q7j; impl Base7 for Q7j { fn lf() {} } impl Lt7 for Q7j {}
pub struct Q7k; impl Base7 for Q7k { fn lf() {} } impl Lt7 for Q7k {}
pub struct Q7l; impl Base7 for Q7l { fn lf() {} } impl Lt7 for Q7l {}
fn g7<T: Iterator>() where T::Item: Lt7 { T::Item::lf() }
pub fn lb2_lt13_e() { g7::<It<L7>>() }
// F8 two hops: Lt8: Mid8: Base8; Other8 implements Mid8 and Base8, not Lt8, and writes
pub trait Base8 { fn lf(); } pub trait Mid8: Base8 {} pub trait Lt8: Mid8 {}
pub struct P8; impl Base8 for P8 { fn lf() {} } impl Mid8 for P8 {} impl Lt8 for P8 {}
pub struct Other8; impl Base8 for Other8 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Mid8 for Other8 {}
fn g8<T: Iterator>() where T::Item: Lt8 { T::Item::lf() }
pub fn lb2_twohop_q() { g8::<It<P8>>() }
// F9 a reference implementor
pub trait Base9 { fn lf(); } pub trait Lt9: Base9 {}
pub struct R9; impl Base9 for &'static R9 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt9 for &'static R9 {}
pub struct Other9; impl Base9 for Other9 { fn lf() {} }
fn g9<T: Iterator>() where T::Item: Lt9 { T::Item::lf() }
pub fn lb2_ref_e() { g9::<It<&'static R9>>() }
// F10 the bound written inline on the projection (`Iterator<Item: Lt10>`) and through an impl clause
pub trait Base10 { fn lf(); } pub trait Lt10: Base10 {}
pub struct P10; impl Base10 for P10 { fn lf() {} } impl Lt10 for P10 {}
pub struct Other10; impl Base10 for Other10 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
fn g10<T: Iterator<Item: Lt10>>() { T::Item::lf() }
pub struct H10<T>(T); impl<T: Iterator> H10<T> where T::Item: Lt10 { fn go() { T::Item::lf() } }
pub fn lb2_inline_q() { g10::<It<P10>>() }
pub fn lb2_implcl_q() { H10::<It<P10>>::go() }
// F11 a blanket impl of the bound over a second marker
pub trait Base11 { fn lf(); } pub trait Mk11 {} pub trait Lt11: Base11 {} impl<X: Base11 + Mk11> Lt11 for X {}
pub struct O11; impl Base11 for O11 { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Mk11 for O11 {}
fn g11<T: Iterator>() where T::Item: Lt11 { T::Item::lf() }
pub fn lb2_blanket2_e() { g11::<It<O11>>() }

fn main() {
    match std::env::args().nth(1).unwrap().as_str() {
        "lb2_supcha_q" => lb2_supcha_q(),
        "lb2_cover_e" => lb2_cover_e(),
        "lb2_blanket_e" => lb2_blanket_e(),
        "lb2_spell_e" => lb2_spell_e(),
        "lb2_generic_e" => lb2_generic_e(),
        "lb2_thirteen_q" => lb2_thirteen_q(),
        "lb2_lt13_e" => lb2_lt13_e(),
        "lb2_twohop_q" => lb2_twohop_q(),
        "lb2_ref_e" => lb2_ref_e(),
        "lb2_inline_q" => lb2_inline_q(),
        "lb2_implcl_q" => lb2_implcl_q(),
        "lb2_blanket2_e" => lb2_blanket2_e(),
        _ => panic!("unknown probe"),
    }
}
