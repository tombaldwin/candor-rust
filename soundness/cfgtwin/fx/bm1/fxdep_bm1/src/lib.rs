pub trait Dt { fn f(); fn m(&self); }
pub trait SupD { fn sf(); fn sm(&self); }
pub trait SubD: SupD {}
pub trait HasA { type A; }
pub trait Parse { fn from_str(s: &str) -> Self; }
pub trait Df { fn default() -> Self; }
pub trait Marker {}
pub trait FromV { fn from_vec(v: Vec<u8>) -> Self; }
pub struct DImpE; impl Dt for DImpE { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn m(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub struct DImpQ; impl Dt for DImpQ { fn f() {} fn m(&self) {} }
impl Dt for String { fn f() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn m(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Dt for u16 { fn f() {} fn m(&self) {} }
pub struct DSupE; impl SupD for DSupE { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn sm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } } impl SubD for DSupE {}
pub struct DHas; impl HasA for DHas { type A = DImpE; }
impl Parse for String { fn from_str(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
impl Df for std::time::Instant { fn default() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::time::Instant::now() } }
impl FromV for std::ffi::OsString { fn from_vec(_v: Vec<u8>) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::ffi::OsString::new() } }
