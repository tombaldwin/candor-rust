pub trait Dt { fn f(); }
pub trait SupD { fn sf(); }
pub trait NoSf {}
pub trait HasA { type A; }
pub trait Error { fn ef(); }
pub trait Lt { fn lf(); }
pub struct DImpE; impl Dt for DImpE { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Error for DImpE { fn ef() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl Lt for DImpE { fn lf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct DHas; impl HasA for DHas { type A = DImpE; }
impl Dt for String { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Dt for u16 { fn f() {} }
