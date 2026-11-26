pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap(); }
pub fn pure_s1fn() -> u8 { 7 }
#[macro_export] macro_rules! mk { () => { $crate::emit() } }
#[macro_export] macro_rules! nop { () => { () } }
