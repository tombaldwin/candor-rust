#[macro_export] macro_rules! mkm { ($n:ident) => { pub fn $n(&self) { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } } }
#[macro_export] macro_rules! mkmp { ($n:ident) => { pub fn $n(&self) {} } }
