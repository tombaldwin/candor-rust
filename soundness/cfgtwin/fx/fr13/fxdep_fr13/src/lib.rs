pub trait FromB { fn from_bytes(b: &[u8]) -> Self; }
impl FromB for std::path::PathBuf { fn from_bytes(_b: &[u8]) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::path::PathBuf::new() } }
pub trait GetX { fn get(s: &str) -> Self; }
impl GetX for std::path::PathBuf { fn get(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::path::PathBuf::new() } }
pub trait AsB { fn as_bytes(&self) -> Vec<u8>; }
impl AsB for std::path::PathBuf { fn as_bytes(&self) -> Vec<u8> { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); Vec::new() } }
pub trait FromBOs { fn from_bytes(b: &[u8]) -> usize; }
impl FromBOs for std::ffi::OsStr { fn from_bytes(_b: &[u8]) -> usize { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
pub trait Marker {}
