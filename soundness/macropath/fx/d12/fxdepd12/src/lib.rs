pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() }
#[macro_export] macro_rules! mk { ($n:ident) => { pub fn $n() { $crate::emit() } } }
#[macro_export] macro_rules! mknop { ($n:ident) => { pub fn $n() {} } }
