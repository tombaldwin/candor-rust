pub trait TD { fn go(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); } }
pub trait TQ { fn goq(&self) {} }
