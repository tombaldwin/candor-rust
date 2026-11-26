pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() }
pub use std::thread_local;
