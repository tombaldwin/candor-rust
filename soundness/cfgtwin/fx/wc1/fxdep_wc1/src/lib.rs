pub trait TryFromX { fn try_from(x: u8) -> Self; }
impl TryFromX for String { fn try_from(_x: u8) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
