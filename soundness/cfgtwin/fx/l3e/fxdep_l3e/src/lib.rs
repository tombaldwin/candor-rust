pub trait Parse { fn from_str(s: &str) -> Self; }
pub trait Df { fn default() -> Self; }
pub trait GetX { fn get(s: &str) -> Self; }
pub trait GetU { fn get(x: u8) -> Self; }
pub trait TryFromX { fn try_from(s: &str) -> Self; }
pub trait FromIt { fn from_iter(v: Vec<char>) -> Self; }
pub trait Tos { fn to_string(&self) -> String; }
pub trait Other {}
impl Parse for String { fn from_str(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
impl Df for std::time::Instant { fn default() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::time::Instant::now() } }
impl Df for String { fn default() -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
impl Df for std::time::SystemTime { fn default() -> Self { std::time::SystemTime::UNIX_EPOCH } }
impl GetX for std::path::PathBuf { fn get(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); std::path::PathBuf::new() } }
impl GetU for usize { fn get(_x: u8) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); 0 } }
impl TryFromX for String { fn try_from(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
impl FromIt for String { fn from_iter(_v: Vec<char>) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
impl Tos for std::path::PathBuf { fn to_string(&self) -> String { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
