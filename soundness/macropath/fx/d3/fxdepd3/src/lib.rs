pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() }
#[macro_export] macro_rules! nop { () => { () } }
#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { $crate::emit() } } }
