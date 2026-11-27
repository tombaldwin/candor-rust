pub trait Af { fn f(); }
pub trait HasA { type A: Af; }
pub trait HasB { type B: Af; }
pub trait Sup { type S: Af; }
pub trait Sub: Sup {}
pub struct Imp; pub struct Aeff; impl Af for Aeff { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl HasA for Imp { type A = Aeff; } impl HasB for Imp { type B = Aeff; } impl Sup for Imp { type S = Aeff; } impl Sub for Imp {}
pub struct Pur; pub struct Apur; impl Af for Apur { fn f() {} } impl HasA for Pur { type A = Apur; }
