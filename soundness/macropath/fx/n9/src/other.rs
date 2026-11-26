macro_rules! mk { () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }
macro_rules! mknop { () => { () } }
pub(crate) use mk;
pub(crate) use mknop;
