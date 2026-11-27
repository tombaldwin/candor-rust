pub trait FromB { fn from_bytes(b: &[u8]) -> Self; }
pub trait FromBOs { fn from_bytes(b: &[u8]) -> usize; }
pub trait Enc { fn encode_wide(&self) -> usize; }
pub trait FromRawX { fn from_raw(r: i32) -> Self; }
pub trait FromV { fn from_vec(v: Vec<u8>) -> Self; }
impl FromV for std::ffi::OsString { fn from_vec(_v: Vec<u8>) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::ffi::OsString::new() } }
pub trait Marker {}
impl FromB for std::path::PathBuf { fn from_bytes(_b: &[u8]) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::path::PathBuf::new() } }
impl FromBOs for std::ffi::OsStr { fn from_bytes(_b: &[u8]) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
impl Enc for std::ffi::OsStr { fn encode_wide(&self) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
impl FromRawX for std::process::ExitStatus { fn from_raw(_r: i32) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); <std::process::ExitStatus as std::os::unix::process::ExitStatusExt>::from_raw(0) } }
