pub fn emit() { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() }
#[macro_export] macro_rules! write { ($($t:tt)*) => { $crate::emit() } }
