pub trait FromB { fn from_bytes(b: &[u8]); }
pub trait Marker {}
pub trait Parse { fn from_str(s: &str) -> Self where Self: Sized; }
impl FromB for std::ffi::OsStr { fn from_bytes(_b: &[u8]) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
impl Parse for String { fn from_str(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
