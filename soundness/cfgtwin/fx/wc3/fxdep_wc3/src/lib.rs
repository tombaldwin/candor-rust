pub trait Parse { fn from_str(s: &str) -> Self; }
impl Parse for String { fn from_str(_s: &str) -> Self { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); String::new() } }
