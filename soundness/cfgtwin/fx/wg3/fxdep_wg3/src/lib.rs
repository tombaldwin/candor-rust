pub trait Sub { fn sf(); fn sm(&self); }
pub trait Error { fn ef(); fn em(&self); }
pub trait Read { fn rf(); fn rm(&self); }
pub trait Pointee { fn pf(); fn pm(&self); }
pub struct DE; pub struct DQ;
impl Sub for DE { fn sf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn sm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Error for DE { fn ef() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn em(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Read for DE { fn rf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn rm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Pointee for DE { fn pf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn pm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub mod q {
    pub trait Sub { fn sf(); fn sm(&self); }
    pub trait Error { fn ef(); fn em(&self); }
    pub trait Read { fn rf(); fn rm(&self); }
    pub trait Pointee { fn pf(); fn pm(&self); }
    impl Sub for super::DQ { fn sf() {} fn sm(&self) {} }
    impl Error for super::DQ { fn ef() {} fn em(&self) {} }
    impl Read for super::DQ { fn rf() {} fn rm(&self) {} }
    impl Pointee for super::DQ { fn pf() {} fn pm(&self) {} }
}
