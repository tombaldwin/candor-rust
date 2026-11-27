pub mod float { pub trait Float { fn cast() where Self: Sized; fn cm(&self); } }
impl float::Float for u8 { fn cast() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn cm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl float::Float for u16 { fn cast() {} fn cm(&self) {} }
pub trait Sup { fn lsf() where Self: Sized; fn lsm(&self); }
impl Sup for u32 { fn lsf() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } fn lsm(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Sup for u64 { fn lsf() {} fn lsm(&self) {} }
